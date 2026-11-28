//! 设置与资源：快捷短语、导入词库、配置保存、整句模型和词库更新状态。
use super::*;

pub(super) fn read_pins(path: &Path) -> Result<wowtype_core::session::ManualPins, String> {
    match std::fs::read_to_string(path) {
        Ok(data) => {
            serde_json::from_str(&data).map_err(|error| format!("固定排位文件无法读取：{error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(format!("读取固定排位失败：{error}")),
    }
}

impl Host {
    pub(super) fn load_phrases(&mut self) {
        self.phrases = match self.store.load_common_phrases() {
            Ok(phrases) => phrases,
            Err(error) => {
                crate::log(&error);
                Vec::new()
            }
        };
    }

    /// 快捷短语、导入的词库交给引擎（不用 &mut）。
    pub(super) fn apply_user_data(&mut self) {
        self.load_phrases();
        self.apply_quick_phrases();
        self.apply_dictionaries();
    }

    pub(super) fn apply_quick_phrases(&self) {
        let Some(engine) = self.engine.as_deref() else {
            return;
        };
        let phrases = if self.config.quick_phrases {
            msime::quick_phrases_to_custom_tsv(&self.store.load_quick_phrases())
        } else {
            String::new()
        };
        engine.lexicon().set_custom_tsv(&phrases);
    }

    pub(super) fn apply_dictionaries(&self) {
        let Some(engine) = self.engine.as_deref() else {
            return;
        };
        let words = self
            .store
            .load_dictionaries()
            .into_iter()
            .flat_map(|(_, entries)| entries)
            .map(|e| (e.pinyin, e.word, f64::from(e.weight.max(1))))
            .collect();
        engine.lexicon().set_extra_words(words);
    }

    pub(super) fn refresh_dictionary_candidates(&mut self) {
        self.apply_dictionaries();
        for client in self.clients.values_mut() {
            client.pending = None;
            client.state.session.refresh_candidates();
            client.state.dismiss_prediction();
        }
    }

    /// 开着上屏联想：提前建好它的词频表（只建一次），免得第一次联想时那个键卡一下。
    pub(super) fn prepare_prediction(&self) {
        if let (true, Some(engine)) = (self.config.predict_next, self.engine.as_deref()) {
            engine.prepare_prediction();
        }
    }

    pub(super) fn set_config(&mut self, config: Config) {
        // 保存设置、改变范围时只迁移基础状态，不能把当前软件的临时模式变成全局记忆。
        let current_mode = self.base_mode_of(self.current_app());
        let old_scope = self.config.mode_scope.clone();
        let old_lexicon = self.config.lexicon.clone();
        let quick_phrases_changed = config.quick_phrases != self.config.quick_phrases;
        if !config.typing_stats || !self.config.typing_stats {
            self.stats.break_activity();
        }
        if config.typing_stats && !self.config.typing_stats {
            self.stats.prepare();
        }
        self.config = config.sanitized();
        let brand_revision = self.config.brand_icon_revision();
        if brand_revision != self.brand_icon_revision {
            self.brand_icon_revision = brand_revision;
            if let Some(desktop) = &self.desktop {
                desktop.brand_icon_changed();
            }
        }
        if !self.config.app_presets
            || !self
                .config
                .app_preset_rules
                .iter()
                .any(|rule| rule.app == self.foreground)
        {
            self.preset_mode = None;
            self.preset_hint = false;
        }
        if self.config.lexicon != old_lexicon {
            self.lexicon_change = Some(self.config.lexicon.clone());
        }
        if self.config.mode_scope != old_scope {
            self.app_modes.clear();
        }
        // 切到全局时沿用当前应用的状态；改变范围也不应突然中英翻转。
        if self.config.default_mode == DefaultMode::Global || self.config.mode_scope != old_scope {
            self.mode = current_mode;
        }
        self.detach();
        if let Some(engine) = self.engine.as_deref_mut() {
            configure_engine(engine, &self.config);
        }
        self.attach();
        // 改外观、标点等普通设置时，输入线程无需重新读入所有导入词库。
        if quick_phrases_changed {
            self.apply_quick_phrases();
        }
        self.prepare_prediction();
        self.notify_bar();
    }

    pub(super) fn save_config(&mut self, config: Config) -> Result<(), String> {
        let config = config.sanitized();
        if config.default_mode == DefaultMode::Global
            && self.config.default_mode != DefaultMode::Global
        {
            self.store
                .save_mode(self.base_mode_of(self.current_app()))
                .map_err(|e| format!("保存中英文状态失败：{e}"))?;
        }
        self.store
            .save_config(&config)
            .map_err(|e| format!("保存设置失败：{e}"))?;
        self.set_config(config);
        Ok(())
    }

    /// 改一项设置并存盘（快捷键、状态栏切换的）。
    pub(super) fn change_config(&mut self, change: impl FnOnce(&mut Config)) -> Result<(), String> {
        let mut config = self.config.clone();
        change(&mut config);
        self.save_config(config)
    }

    pub(super) fn change_neural_model(&mut self, path: Option<String>) -> Response {
        use std::io::Read;
        use wowtype_core::nnlm::Nnlm;
        const MAX_BYTES: u64 = 32 << 20;
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        let target = self.store.dir().join("wetype/nlrgpt2_nnlm.bin");
        let result = (|| -> Result<(), String> {
            if let Some(source) = path {
                let file =
                    std::fs::File::open(&source).map_err(|e| format!("打开模型失败：{e}"))?;
                if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                    return Err("请选择模型文件".into());
                }
                let mut data = Vec::new();
                file.take(MAX_BYTES + 1)
                    .read_to_end(&mut data)
                    .map_err(|e| format!("读取模型失败：{e}"))?;
                if data.len() as u64 > MAX_BYTES {
                    return Err("模型文件超过 32 MB".into());
                }
                let model = Nnlm::from_bytes(&data).map_err(|e| format!("模型校验失败：{e:#}"))?;
                crate::store::write_atomic_bytes(&target, &data)
                    .map_err(|e| format!("保存模型失败：{e}"))?;
                engine.install_nnlm(std::sync::Arc::new(model), target);
            } else {
                let bundled = crate::resource_dir().join("nnlm.bin");
                let model = Nnlm::load(&bundled).map_err(|e| format!("内置模型读取失败：{e:#}"))?;
                match std::fs::remove_file(&target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(format!("移除外部模型失败：{error}")),
                }
                engine.install_nnlm(std::sync::Arc::new(model), bundled);
            }
            Ok(())
        })();
        if let Err(error) = result {
            return Response::error(error);
        }
        for client in self.clients.values_mut() {
            client.state.session.refresh_candidates();
            client.state.dismiss_prediction();
            client.revision = 0;
            client.pending = None;
        }
        Response {
            status: Some(self.status()),
            ..Response::ok()
        }
    }

    pub(super) fn config_response(&self) -> Response {
        use wowtype_core::shuangpin::{encode, Scheme};
        let shuangpin_example = self
            .config
            .shuangpin
            .then(|| {
                let scheme = Scheme::from_name(&self.config.shuangpin_scheme)?;
                let keys: Vec<_> = ["ni", "hao"]
                    .iter()
                    .filter_map(|syllable| {
                        let pair = *encode(scheme, syllable).first()?;
                        String::from_utf8(pair.to_vec()).ok()
                    })
                    .collect();
                (keys.len() == 2).then(|| keys.join(" "))
            })
            .flatten();
        Response {
            config: Some(self.config.clone()),
            shuangpin_example,
            ..Response::ok()
        }
    }

    /// 词库更新的操作只读写一个小记录文件；校验大文件在设置程序和启动载入线程里做。
    pub(super) fn dictionary_update_response(&self, result: Result<(), String>) -> Response {
        let status = crate::dictionary_updates::status(
            &crate::dictionary_updates::root(),
            &crate::resource_dir(),
        );
        Response {
            dictionary_update: Some(status),
            ..result.map_or_else(Response::error, |()| Response::ok())
        }
    }
}
