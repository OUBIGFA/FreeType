//! 中英文状态、按应用预设、大写锁定、焦点、状态栏与切换快捷键。
use super::*;

/// 键是不是设置里选的快捷键（`name` 是 [`Config`] 里的写法）。
pub(super) fn is_hotkey(key: &Key, name: &str) -> bool {
    let m = key.modifiers & (modifier::SHIFT | modifier::CTRL | modifier::ALT | modifier::SUPER);
    let c = char::from_u32(key.keysym).map(|c| c.to_ascii_lowercase());
    let (want_mods, want) = match name {
        "ctrl+shift+e" => (modifier::CTRL | modifier::SHIFT, 'e'),
        "ctrl+alt+e" => (modifier::CTRL | modifier::ALT, 'e'),
        "ctrl+shift+y" => (modifier::CTRL | modifier::SHIFT, 'y'),
        "ctrl+alt+y" => (modifier::CTRL | modifier::ALT, 'y'),
        "ctrl+shift+u" => (modifier::CTRL | modifier::SHIFT, 'u'),
        "ctrl+alt+u" => (modifier::CTRL | modifier::ALT, 'u'),
        "ctrl+period" => (modifier::CTRL, '.'),
        "ctrl+shift+f" => (modifier::CTRL | modifier::SHIFT, 'f'),
        "ctrl+shift+t" => (modifier::CTRL | modifier::SHIFT, 't'),
        "ctrl+space" => (modifier::CTRL, ' '),
        _ => return false,
    };
    m == want_mods && c == Some(want)
}

pub(super) fn mode_hint(mode: Mode) -> String {
    if mode == Mode::Chinese { "中" } else { "英" }.to_string()
}

impl Host {
    pub(super) fn mode_of(&self, app: &str) -> Mode {
        if app == self.foreground {
            if let Some(mode) = self.preset_mode {
                return mode;
            }
        }
        self.base_mode_of(app)
    }

    pub(super) fn base_mode_of(&self, app: &str) -> Mode {
        if app.is_empty() {
            return self.mode;
        }
        if self.config.default_mode != DefaultMode::Global && self.config.mode_scope == "app" {
            self.app_modes
                .get(app)
                .copied()
                .unwrap_or(self.config.default_mode.resolve(self.mode))
        } else {
            self.mode
        }
    }

    pub(super) fn set_mode(&mut self, app: &str, mode: Mode) -> Result<(), String> {
        if app == self.foreground && self.preset_mode.is_some() {
            self.preset_mode = Some(mode);
        } else {
            if self.config.default_mode == DefaultMode::Global {
                self.store
                    .save_mode(mode)
                    .map_err(|e| format!("保存中英文状态失败：{e}"))?;
            }
            if self.config.default_mode != DefaultMode::Global && self.config.mode_scope == "app" {
                self.app_modes.insert(app.to_string(), mode);
            }
            // 打开设置后原应用会失焦；保留最后的普通模式，供启用全局时沿用。
            self.mode = mode;
        }
        // 别的连接（托盘、其他应用）也能切状态，旧状态下缓存的按键结果随之作废。
        for client in self.clients.values_mut() {
            client.pending = None;
        }
        Ok(())
    }

    pub(super) fn app_of(&self, id: ClientId) -> String {
        self.clients
            .get(&id)
            .map(|c| c.state.app.clone())
            .unwrap_or_default()
    }

    /// 实际的输入状态：大写锁定开着时一律是英文（字母、数字、标点都原样交给应用），
    /// 记住的状态不动，关掉大写锁定就回到它。按键、状态显示都用这个。
    pub(super) fn effective(&self, mode: Mode) -> Mode {
        if self.caps {
            Mode::English
        } else {
            mode
        }
    }

    /// 光标旁提示的状态：大写锁定开着是「A」，否则是「中」「英」。
    pub(super) fn hint_text(&self, mode: Mode) -> String {
        if self.caps {
            "A".to_string()
        } else {
            mode_hint(mode)
        }
    }

    /// 记下大写锁定的状态；变了返回 true（旧状态下缓存的按键结果随之作废）。
    pub(super) fn set_caps(&mut self, on: bool) -> bool {
        if self.caps == on {
            return false;
        }
        self.caps = on;
        for client in self.clients.values_mut() {
            client.pending = None;
        }
        true
    }

    /// DLL 报大写锁定开了 / 关了。
    pub(super) fn caps_lock(&mut self, id: ClientId, on: bool) -> Response {
        if !self.set_caps(on) {
            // 没变（输入框得到焦点时 DLL 也会报一次）：不提示
            return self.with_client(id, |state, env| state.current(false, env));
        }
        let hint = self
            .config
            .hint_on_toggle
            .then(|| self.hint_text(self.mode_of(&self.app_of(id))));
        let mut response = self.with_client(id, |state, env| {
            // 打到一半开了大写锁定：和切到英文一样，字母原样上屏，面板关掉
            if on && state.busy() {
                state.reset(true, env)
            } else {
                state.current(false, env)
            }
        });
        if let Some(frame) = response.frame.as_mut() {
            frame.hint = hint;
        }
        self.notify_bar();
        response
    }

    /// 前台窗口换到了 `app` 这个程序（小写文件名；桌面线程的事件，或输入框得到焦点时查到的）。
    ///
    /// 指定软件的预设只在这里套用：切入时建立临时状态，离开时丢弃。
    /// 同一个程序里换窗口（查找框、保存对话框）不算，用户在里面手动切换的结果留着；
    /// 普通程序始终使用自己的基础状态。即使切入时模式相同，也要隔离后续手动切换。
    pub fn foreground(&mut self, app: &str) {
        // 桌面事件与管道请求经过不同队列。焦点请求可能已处理新窗口，旧事件才送达；
        // 只有仍与系统前台一致的事件才能改变状态，FreeType 自己的窗口也不算离开。
        if self
            .desktop
            .as_ref()
            .is_some_and(|desktop| desktop.foreground_app().as_deref() != Some(app))
        {
            return;
        }
        if app == self.foreground {
            return;
        }
        let previous = self.current_mode();
        self.foreground = app.to_string();
        self.preset_mode = self
            .config
            .app_presets
            .then(|| {
                self.config
                    .app_preset_rules
                    .iter()
                    .find(|rule| rule.app == app)
                    .map(|rule| rule.mode)
            })
            .flatten();
        if self.focused.is_some_and(|id| self.app_of(id) != app) {
            self.focused = None;
        }
        for client in self.clients.values_mut() {
            client.pending = None;
        }
        self.preset_hint = previous != self.current_mode();
        self.notify_bar();
    }

    /// 首个焦点或按键可能先于桌面事件到达，处理前先同步真实前台。
    pub(super) fn refresh_foreground(&mut self) {
        if let Some(app) = self
            .desktop
            .as_ref()
            .and_then(|desktop| desktop.foreground_app())
        {
            self.foreground(&app);
        }
    }

    pub(super) fn current_app(&self) -> &str {
        if !self.foreground.is_empty() {
            &self.foreground
        } else {
            self.focused
                .and_then(|id| self.clients.get(&id))
                .map(|client| client.state.app.as_str())
                .unwrap_or("")
        }
    }

    /// 状态栏、托盘跟随当前前台；不能被旧输入框尚未送达的失焦通知拖回旧模式。
    pub(super) fn current_mode(&self) -> Mode {
        self.mode_of(self.current_app())
    }

    pub(super) fn notify_bar(&mut self) {
        let state = BarState {
            visible: self.config.status_bar && self.focused.is_some(),
            autohide: self.config.status_bar_autohide,
            scale: self.config.bar_scale,
            icon_size: self.config.bar_icon_size,
            hide_seconds: self.config.bar_hide_seconds,
            shadow: self.config.bar_shadow,
            parts: self.config.bar_parts,
            mode: self.effective(self.current_mode()),
            chinese_punct: self.config.fullwidth_punct,
            traditional: self.config.traditional,
            brand_icon: self.config.brand_icon.clone(),
            brand_revision: self.brand_icon_revision,
            // 普通按键只复制共享引用，不能反复复制自定义 PNG 的 Base64 内容。
            custom_brand_icon: self
                .last_bar
                .as_ref()
                .filter(|state| state.brand_revision == self.brand_icon_revision)
                .map(|state| state.custom_brand_icon.clone())
                .unwrap_or_else(|| self.config.custom_brand_icon.clone().into()),
        };
        if self.last_bar.as_ref() == Some(&state) {
            return;
        }
        if let Some(desktop) = &self.desktop {
            desktop.update(&state);
        }
        self.last_bar = Some(state);
    }

    pub(super) fn toggle_punct(&mut self, id: ClientId) -> Response {
        if let Err(error) = self.change_config(|c| c.fullwidth_punct = !c.fullwidth_punct) {
            return Response::error(error);
        }
        let hint = if self.config.fullwidth_punct {
            "中文标点"
        } else {
            "英文标点"
        };
        self.hint_frame(id, Some(hint.into()))
    }

    pub(super) fn toggle_traditional(&mut self, id: ClientId) -> Response {
        if let Err(error) = self.change_config(|c| c.traditional = !c.traditional) {
            return Response::error(error);
        }
        let hint = if self.config.traditional {
            "繁體"
        } else {
            "简体"
        };
        self.hint_frame(id, Some(hint.into()))
    }

    /// 设置里选的快捷键：处理了返回应答。test 只回答「要这个键」，不动状态。
    pub(super) fn hotkey(&mut self, id: ClientId, key: &Key) -> Option<Response> {
        let hit = |name: &str| is_hotkey(key, name);
        let action = if hit(&self.config.hotkey_english) {
            5
        } else if hit(&self.config.hotkey_radical) {
            6
        } else if hit(&self.config.hotkey_punct) {
            1
        } else if hit(&self.config.hotkey_traditional) {
            2
        } else if hit(&self.config.hotkey_emoji) {
            3
        } else if hit(&self.config.hotkey_mode) {
            4
        } else {
            return None;
        };
        if key.test {
            let mode = self.effective(self.mode_of(&self.app_of(id)));
            let mut frame = Frame {
                handled: true,
                ..Frame::default()
            };
            self.style(&mut frame, mode);
            return Some(Response::with_frame(frame));
        }
        Some(match action {
            5 => self.with_client(id, |state, env| {
                state.open_lookup(crate::lookup_mode::Kind::English, env)
            }),
            6 => self.with_client(id, |state, env| {
                state.open_lookup(crate::lookup_mode::Kind::Radical, env)
            }),
            1 => self.toggle_punct(id),
            2 => self.toggle_traditional(id),
            3 => self.with_client(id, |state, env| {
                // 和上游一样，同一个表情快捷键再次按下时关闭面板。
                if state
                    .tool
                    .as_ref()
                    .is_some_and(|tool| tool.page == Page::Emoji)
                {
                    state.tool = None;
                    state.current(true, env)
                } else {
                    state.open_tool(Page::Emoji, env)
                }
            }),
            _ => self.toggle_mode(id, None),
        })
    }

    pub(super) fn toggle_mode(&mut self, id: ClientId, shift: Option<String>) -> Response {
        self.refresh_foreground();
        if let Some(side) = shift.as_deref() {
            let allowed = match self.config.shift_toggle.as_str() {
                "both" => true,
                "left" => side == "left",
                "right" => side == "right",
                _ => false,
            };
            if !allowed {
                let mode = self.effective(self.mode_of(&self.app_of(id)));
                let mut frame = Frame::default();
                self.style(&mut frame, mode);
                return Response::with_frame(frame);
            }
        }
        if self.caps {
            // 大写锁定开着时打出来的总是英文，切换了也看不出来：不切，提示原因
            return self.hint_frame(id, Some(self.hint_text(Mode::English)));
        }
        let app = match self.clients.get(&id) {
            Some(client) => client.state.app.clone(),
            None => self.current_app().to_string(),
        };
        let current = if app.is_empty() {
            self.current_mode()
        } else {
            self.mode_of(&app)
        };
        let mode = match current {
            Mode::Chinese => Mode::English,
            Mode::English => Mode::Chinese,
        };
        if let Err(error) = self.set_mode(&app, mode) {
            return Response::error(error);
        }
        let hint = self.config.hint_on_toggle.then(|| mode_hint(mode));
        let engine = self.engine_ref();
        let out = match self.clients.get_mut(&id) {
            Some(client) => {
                client.pending = None;
                let env = Env {
                    config: &self.config,
                    mode,
                    engine,
                    phrases: &self.phrases,
                    recent_emoji: &self.recent_emoji,
                };
                let state = &mut client.state;
                // 切到英文：打到一半的字母原样上屏，面板关掉
                if mode == Mode::English && state.busy() {
                    state.reset(true, &env)
                } else {
                    state.current(true, &env)
                }
            }
            None => Out {
                frame: Frame {
                    handled: true,
                    ..Frame::default()
                },
                ..Out::default()
            },
        };
        let mut response = self.finish(mode, out);
        if let Some(frame) = response.frame.as_mut() {
            frame.hint = hint;
        }
        self.notify_bar();
        response
    }

    pub(super) fn focus(&mut self, id: ClientId, active: bool, before: Option<String>) -> Response {
        if active {
            self.refresh_foreground();
            // TSF 的异步编辑会话可能在应用已退到后台后才报告焦点，不能抢占当前输入框。
            if self.desktop.is_some()
                && !self.foreground.is_empty()
                && self.app_of(id) != self.foreground
            {
                return self.with_client(id, |state, env| state.current(false, env));
            }
        }
        if active {
            self.focused = Some(id);
        } else if self.focused == Some(id) {
            self.focused = None;
        }
        // 预设刚切换了状态：和手动切换一样提示一次
        let preset_hint =
            active && std::mem::take(&mut self.preset_hint) && self.config.hint_on_toggle;
        let hint_on_focus = self.config.hint_on_focus;
        let mut response = self.with_client(id, |state, env| {
            // 换了输入框：上一个输入框里的联想词不带过来
            state.dismiss_prediction();
            state.focus_context();
            if let Some(before) = &before {
                state.set_before(before);
            }
            state.current(false, env)
        });
        if let Some(frame) = response.frame.as_mut() {
            if active && (hint_on_focus || preset_hint) {
                frame.hint = Some(self.hint_text(frame.mode));
            }
        }
        self.notify_bar();
        response
    }
}
