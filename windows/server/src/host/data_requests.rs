//! 个人数据：注音、个人词、学习与全部设置的导入导出。
use super::*;

impl Host {
    /// 给只有词语的词条注音：读正在用的词库文件，只收要用到的词和字。
    pub(super) fn reading(&self, words: &[String]) -> Box<dyn dictfmt::Reading> {
        let path = Path::new(&self.lexicon_in_use);
        // 默认词库只有编译索引（随安装包或更新包）；注音读同名的文本词库（若存在）。
        let source = if path.extension().is_some_and(|ext| ext == "ftd") {
            path.with_extension("tsv")
        } else {
            path.into()
        };
        let text = std::fs::read_to_string(source).unwrap_or_default();
        Box::new(LexiconReading::from_tsv(&text, words))
    }

    pub(super) fn add_user_word(&mut self, word: String, pinyin: String) -> Response {
        let word = word.trim().to_string();
        if word.is_empty() || word.contains(['\t', '\n', '\r']) {
            return Response::error("词语不能为空，也不能有换行");
        }
        let english = word.bytes().any(|b| b.is_ascii_alphabetic())
            && word.bytes().all(|b| b.is_ascii_alphanumeric());
        let pinyin = if pinyin.trim().is_empty() && english {
            word.to_ascii_lowercase()
        } else if pinyin.trim().is_empty() {
            match dictfmt::Reading::pinyin(self.reading(&[word.clone()]).as_ref(), &word) {
                Some(p) => p,
                None => return Response::error(format!("认不出「{word}」的读音，请填上拼音")),
            }
        } else {
            pinyin
        };
        let pinyin = if word.chars().any(|c| c.is_ascii_alphabetic()) {
            pinyin
        } else {
            let Some(pinyin) = msime::normalize_pinyin(&pinyin) else {
                return Response::error("拼音不对：写成 ni hao、ni'hao 或 nihao");
            };
            pinyin
        };
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        if let Err(e) = engine.add_user_word(&pinyin, &word, ADDED_WORD_FREQ) {
            return Response::error(format!("添加词语失败：{e}"));
        }
        if let Err(error) = self.learned_changed() {
            return Response::error(error);
        }
        Response {
            user_words: Some(self.user_words()),
            report: Some(ImportReport {
                added: 1,
                note: Some(format!("{word}（{pinyin}）")),
                ..ImportReport::default()
            }),
            ..Response::ok()
        }
    }

    pub(super) fn user_words(&self) -> Vec<UserWord> {
        let Some(engine) = self.engine.as_deref() else {
            return Vec::new();
        };
        let mut words: Vec<UserWord> = engine
            .user_words()
            .into_iter()
            .map(|(pinyin, word, freq)| {
                let display = wowtype_core::autospace::format_text(
                    &word,
                    None,
                    crate::input::autospace_flags(&self.config),
                );
                let display = if display == word {
                    String::new()
                } else {
                    display
                };
                UserWord {
                    word,
                    display,
                    pinyin,
                    freq,
                }
            })
            .collect();
        words.sort_by(|a, b| {
            b.freq
                .total_cmp(&a.freq)
                .then_with(|| a.pinyin.cmp(&b.pinyin))
        });
        words
    }

    /// 改了引擎学到的东西：各输入框的候选作废，马上存盘。
    pub(super) fn learned_changed(&mut self) -> Result<(), String> {
        for client in self.clients.values_mut() {
            client.pending = None;
            client.state.clear();
        }
        self.dirty = true;
        self.save_learning()
    }

    pub(super) fn export_learning(&self, path: &Path) -> Response {
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        let data = match engine.export_learning() {
            Ok(data) => data,
            Err(error) => return Response::error(format!("读取学习记录失败：{error:#}")),
        };
        match crate::store::write_atomic(path, &data) {
            Ok(()) => Response {
                report: Some(ImportReport {
                    added: engine.user_words().len(),
                    note: Some("已导出学习备份，包含词条、选词次数和搭配".into()),
                    ..ImportReport::default()
                }),
                ..Response::ok()
            },
            Err(error) => Response::error(format!("写入学习备份失败：{error}")),
        }
    }

    pub(super) fn import_learning(&mut self, path: &Path) -> Response {
        let data = match std::fs::read_to_string(path) {
            Ok(data) => data,
            Err(error) => return Response::error(format!("读取学习备份失败：{error}")),
        };
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        let changed = match engine.import_learning(&data) {
            Ok(changed) => changed,
            Err(error) => return Response::error(format!("导入学习备份失败：{error:#}")),
        };
        if let Err(error) = self.learned_changed() {
            return Response::error(format!(
                "学习记录已合并，但保存失败：{error}；请修复后重新导入"
            ));
        }
        Response {
            report: Some(ImportReport {
                added: changed,
                ..ImportReport::default()
            }),
            user_words: Some(self.user_words()),
            ..Response::ok()
        }
    }

    /// 导出全部设置备份；`parts` 是要导出的类别。
    pub(super) fn export_settings(&self, path: &Path, parts: &[BackupPart]) -> Response {
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        Self::report(backup::export(
            &self.store,
            &self.config,
            self.mode,
            &self.recent_emoji,
            engine,
            path,
            parts,
        ))
    }

    pub(super) fn import_settings(&mut self, path: &Path) -> Response {
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        // 确认运行中引擎的学习状态可读，再准备文件；失败不能先覆盖其他设置。
        if let Err(error) = engine.export_learning() {
            return Response::error(format!("读取当前学习记录失败：{error:#}"));
        }
        let mut prepared = match backup::read(path) {
            Ok(data) => data,
            Err(error) => return Response::error(error),
        };
        // 主动整份恢复采用旧正文，但继续使用本机的变更历史。之后按本次用户操作
        // 生成新版本；只有真正的逐项合并才接纳远端记录版本。
        prepared.data.sync_records = None;
        self.import_prepared(prepared)
    }

    pub(super) fn import_prepared(&mut self, prepared: backup::PreparedBackup) -> Response {
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        // 只替换备份里有的类别；没有的类别（文件、引擎里的状态）保持不变。
        let config = prepared.restored_config(&self.store, &self.config);
        // 排空旧学习快照，再替换备份文件，保证重启仍读取恢复后的数据。
        if prepared.has(BackupPart::Learning) {
            if let Err(error) = self.saver.result(true) {
                self.dirty = true;
                crate::log(&error);
            }
        }
        let cleanup_note = match prepared.restore_files(&self.store, &self.config) {
            Ok(note) => note,
            Err(error) => return Response::error(error),
        };
        if prepared.has(BackupPart::Learning) {
            engine.clear_learned();
            if let Err(error) = engine.import_learning(&prepared.data.learning.to_string()) {
                return Response::error(format!(
                    "备份已恢复，但输入引擎更新失败：{error:#}；请重新打开 FreeType"
                ));
            }
            self.dirty = false;
            self.recent_emoji = prepared.data.recent_emoji.clone();
        }
        self.set_config(config);
        self.apply_user_data();
        // 同一路径的主词库内容也可能变了，不能只按路径字符串判断是否重新载入。
        if prepared.has(BackupPart::Dictionaries) && prepared.data.main_lexicon.is_some() {
            self.lexicon_change = Some(self.config.lexicon.clone());
        }
        if prepared.has(BackupPart::Settings) {
            self.mode = prepared.data.mode;
            self.app_modes.clear();
        }
        for client in self.clients.values_mut() {
            client.pending = None;
            client.state.clear();
            client.state.set_before("");
        }
        self.notify_bar();
        let mut report = prepared.report();
        if let Some(note) = cleanup_note {
            report.note = Some(format!("{}；{note}", report.note.unwrap_or_default()));
        }
        let common = match self.store.load_common_phrases() {
            Ok(common) => common,
            Err(error) => return Response::error(error),
        };
        Response {
            config: Some(self.config.clone()),
            status: Some(self.status()),
            quick_phrases: Some(self.store.list_quick_phrases()),
            common_phrases: Some(common),
            dictionaries: Some(self.store.list_dictionaries()),
            user_words: Some(self.user_words()),
            report: Some(report),
            ..Response::ok()
        }
    }

    /// 数据同步的恢复：把另一台设备（或本机以前）的快照并进当前数据，不删除本机已有的内容。
    ///
    /// 学习记录按核心的规则合并（重复合并不累加）；设置只采纳那台设备上改过的项；
    /// 中英文状态、主词库、状态栏位置是各设备自己的，不动。
    pub(super) fn merge_settings(&mut self, path: &Path) -> Response {
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        // 整份先校验完再动数据
        let prepared = match backup::read(path) {
            Ok(data) => data,
            Err(error) => return Response::error(error),
        };
        let counts = match prepared.merge_files(&self.store) {
            Ok(counts) => counts,
            Err(error) => return Response::error(error),
        };
        // 备份里没有学习记录（只导出了一部分数据的文件）时不合并
        let mut learned = 0;
        if prepared.has(BackupPart::Learning) {
            learned = match engine.import_learning(&prepared.data.learning.to_string()) {
                Ok(changed) => changed,
                Err(error) => return Response::error(format!("合并学习记录失败：{error:#}")),
            };
            if let Err(error) = self.learned_changed() {
                return Response::error(format!("学习记录已合并，但尚未保存到磁盘：{error}"));
            }
        }
        for emoji in &prepared.data.recent_emoji {
            if !self.recent_emoji.contains(emoji) {
                self.recent_emoji.push(emoji.clone());
            }
        }
        self.recent_emoji.truncate(RECENT_EMOJI);
        let _ = self.store.save_recent_emoji(&self.recent_emoji);
        if let Err(error) = self.save_config(prepared.merged_config(&self.config)) {
            return Response::error(error);
        }
        self.apply_user_data();
        let common = match self.store.load_common_phrases() {
            Ok(common) => common,
            Err(error) => return Response::error(error),
        };
        let report = ImportReport {
            added: counts.quick_phrases + counts.common_phrases + counts.dictionaries + learned,
            note: Some(format!(
                "新增 {} 份词库、{} 条快捷短语、{} 条常用语，合并 {} 条学习记录",
                counts.dictionaries, counts.quick_phrases, counts.common_phrases, learned
            )),
            ..ImportReport::default()
        };
        Response {
            config: Some(self.config.clone()),
            status: Some(self.status()),
            quick_phrases: Some(self.store.list_quick_phrases()),
            common_phrases: Some(common),
            dictionaries: Some(self.store.list_dictionaries()),
            user_words: Some(self.user_words()),
            report: Some(report),
            ..Response::ok()
        }
    }
}
