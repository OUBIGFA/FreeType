//! 输入会话请求：按键、候选操作、固定排位、删除学习和命令。
use super::*;

impl Host {
    /// 对一个连接的输入状态做点事。
    pub(super) fn with_client(
        &mut self,
        id: ClientId,
        f: impl FnOnce(&mut ClientState, &Env) -> Out,
    ) -> Response {
        let app = self.app_of(id);
        let mode = self.effective(self.mode_of(&app));
        let engine = self.engine_ref();
        let Some(client) = self.clients.get_mut(&id) else {
            return Response::error("这个连接没有输入会话（先发 hello，session 为 true）");
        };
        client.pending = None;
        let env = Env {
            config: &self.config,
            mode,
            engine,
            phrases: &self.phrases,
            recent_emoji: &self.recent_emoji,
        };
        let out = f(&mut client.state, &env);
        self.finish(mode, out)
    }

    /// 只带提示的应答（快捷键、命令）。
    pub(super) fn hint_frame(&mut self, id: ClientId, hint: Option<String>) -> Response {
        let enabled = self.config.hint_on_toggle;
        let mut response = self.with_client(id, |state, env| state.current(true, env));
        if !response.ok {
            let mode = self.effective(self.current_mode());
            let mut frame = Frame {
                handled: true,
                ..Frame::default()
            };
            self.style(&mut frame, mode);
            response = Response::with_frame(frame);
        }
        if let Some(frame) = response.frame.as_mut() {
            frame.hint = hint.filter(|_| enabled);
        }
        self.notify_bar();
        response
    }

    /// 补上候选窗的样式和状态；记下有没有学到东西。
    pub(super) fn finish(&mut self, mode: Mode, out: Out) -> Response {
        if let Some(emoji) = out.emoji {
            self.remember_emoji(&emoji);
        }
        let mut frame = out.frame;
        if frame.commit.is_some() {
            self.dirty = true;
            self.mark_sync_change();
        }
        if out.forgot {
            self.dirty = true;
            self.mark_sync_change();
            self.save_if_dirty();
        }
        self.style(&mut frame, mode);
        Response::with_frame(frame)
    }

    pub(super) fn style(&self, frame: &mut Frame, mode: Mode) {
        // 开机后词库还没载好时只能原样上屏字母；空着的候选窗要说明原因，不让人以为卡住了。
        if self.engine.is_none()
            && !frame.preedit.is_empty()
            && frame.candidates.is_empty()
            && frame.title.is_none()
        {
            frame.title = Some(LOADING_TITLE.into());
        }
        frame.mode = mode;
        frame.chinese_punct = self.config.fullwidth_punct;
        frame.traditional = self.config.traditional;
        frame.font_size = self.config.font_size;
        frame.font_family = self.config.font_family.clone();
        frame.candidate_style = self.config.candidate_style.clone();
        frame.candidate_skin =
            (self.config.candidate_style == "custom").then(|| self.config.candidate_skin.clone());
        frame.candidate_wheel = self.config.candidate_wheel;
        frame.hide_candidate_preedit = !self.config.candidate_preedit;
        frame.sentence_editor_above = self.config.sentence_editor_position == "above";
        frame.preedit_size = self.config.preedit_size;
        frame.lock_candidate_position = !self.config.candidate_follow;
        frame.wrap_candidates = !self.config.candidate_single_line;
        // 词典注解较长，查询模式按行展示；普通候选的横竖排偏好保持独立。
        frame.font_fallbacks = self.config.font_fallbacks.clone();
        frame.vertical = !frame.annotations.is_empty() || !self.config.horizontal;
        frame.hint_position = self.config.hint_position.clone();
        frame.want_before = self.config.autospace;
    }

    pub(super) fn remember_emoji(&mut self, emoji: &str) {
        self.recent_emoji.retain(|e| e != emoji);
        self.recent_emoji.insert(0, emoji.to_string());
        self.recent_emoji.truncate(RECENT_EMOJI);
        let _ = self.store.save_recent_emoji(&self.recent_emoji);
    }

    pub(super) fn key(&mut self, id: ClientId, key: Key, before: Option<String>) -> Response {
        self.refresh_foreground();
        // 每个键都带着大写锁定的状态：在没有 FreeType 的窗口里开关过也能跟上
        if self.set_caps(key.modifiers & modifier::CAPS != 0) {
            self.notify_bar();
        }
        let app = self.app_of(id);
        if key.keysym == keysym::ESCAPE
            && key.modifiers
                & (modifier::SHIFT
                    | modifier::CTRL
                    | modifier::ALT
                    | modifier::SUPER
                    | modifier::REPEAT)
                == 0
            && !self.caps
            && self.mode_of(&app) == Mode::Chinese
            && self.config.vim_mode_apps.contains(&app)
            && self
                .clients
                .get(&id)
                .is_some_and(|client| client.state.vim_escape_idle())
        {
            // TSF 的预询问只预约正式处理；正式处理切状态但把 Esc 留给编辑器。
            if key.test {
                return self.with_client(id, |state, env| state.current(true, env));
            }
            if let Err(error) = self.set_mode(&app, Mode::English) {
                return Response::error(error);
            }
            self.notify_bar();
            return self.with_client(id, |state, env| state.current(false, env));
        }
        if let Some(response) = self.hotkey(id, &key) {
            if let Some(client) = self.clients.get_mut(&id) {
                client.pending = None;
            }
            return response;
        }
        let mode = self.effective(self.mode_of(&app));
        let engine = self.engine_ref();
        let Some(client) = self.clients.get_mut(&id) else {
            return Response::error("这个连接没有输入会话（先发 hello，session 为 true）");
        };
        let env = Env {
            config: &self.config,
            mode,
            engine,
            phrases: &self.phrases,
            recent_emoji: &self.recent_emoji,
        };
        let real = Key { test: false, ..key };
        if key.test {
            let mut state = client.state.clone();
            state.session = state.session.preview();
            if let Some(before) = &before {
                state.set_before(before);
            }
            let out = state.key(&real, &env);
            let mut frame = out.frame.clone();
            if frame.handled {
                client.pending = Some((real, state, out));
                self.style(&mut frame, mode);
                return Response::with_frame(frame);
            }
            // 不要的键不会再真按：交给应用的字记进上文
            client.pending = None;
            if state.session.accept_preview() {
                client.state = state;
            }
            self.style(&mut frame, mode);
            return Response::with_frame(frame);
        }
        if let Some((pending, mut state, mut out)) = client.pending.take() {
            if pending == real && before.is_none() && state.session.accept_preview() {
                out.frame.enhancement_pending = state.session.enhancement_pending();
                client.state = state;
                return self.finish(mode, out);
            }
        }
        if let Some(before) = &before {
            client.state.set_before(before);
        }
        let out = client.state.key(&real, &env);
        self.finish(mode, out)
    }

    pub(super) fn candidate_action(
        &mut self,
        id: ClientId,
        revision: u64,
        action: CandidateAction,
    ) -> Response {
        if revision == 0
            || self
                .clients
                .get(&id)
                .is_none_or(|client| client.revision != revision)
        {
            return self.with_client(id, |state, env| state.current(true, env));
        }
        match action {
            CandidateAction::SentenceFocus { at } => self.with_client(id, |state, env| {
                state.sentence_action(wowtype_core::sentence::Action::Focus { at }, env)
            }),
            CandidateAction::SentenceCaret { at } => self.with_client(id, |state, env| {
                state.sentence_action(wowtype_core::sentence::Action::Caret { at }, env)
            }),
            CandidateAction::SentencePage { next } => self.with_client(id, |state, env| {
                state.sentence_action(wowtype_core::sentence::Action::Page { next }, env)
            }),
            CandidateAction::SentenceChoose { index } => self.with_client(id, |state, env| {
                let page = state.session.sentence_view().map_or(0, |v| v.page);
                state.sentence_action(
                    wowtype_core::sentence::Action::Choose {
                        index: index + page * 9,
                    },
                    env,
                )
            }),
            CandidateAction::SentenceResize { expand } => self.with_client(id, |state, env| {
                state.sentence_action(wowtype_core::sentence::Action::Resize { expand }, env)
            }),
            CandidateAction::SentenceLeave => self.with_client(id, |state, env| {
                state.sentence_action(wowtype_core::sentence::Action::Leave, env)
            }),
            CandidateAction::Select { index } => {
                self.with_client(id, |state, env| state.select(index, env))
            }
            CandidateAction::Page { next } => {
                self.with_client(id, |state, env| state.page(next, env))
            }
            CandidateAction::Forget { index } => self.forget(id, index),
            CandidateAction::Pin { index, place } => self.pin(id, index, place),
        }
    }

    pub(super) fn pin(&mut self, id: ClientId, index: usize, place: u8) -> Response {
        if u32::from(place) > self.config.page_size {
            return Response::error("固定排位不在本页范围内");
        }
        let Some(client) = self.clients.get_mut(&id) else {
            return Response::error("输入会话已结束");
        };
        if client.state.tool.is_some()
            || client.state.raw.is_some()
            || client.state.session.learning_disabled()
        {
            return Response::error("当前候选不能固定排位");
        }
        let Some((scope, key)) = client.state.session.pin_target(index) else {
            return Response::error("当前候选不能固定排位");
        };
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库未加载");
        };
        let path = self.store.dir().join("candidate-pins.json");
        // 不覆盖损坏记录；保留给用户排查，重新打开仍可使用输入法。
        let mut pins = match read_pins(&path) {
            Ok(pins) => pins,
            Err(error) => return Response::error(error),
        };
        let slots = pins.entry(scope.clone()).or_default();
        slots.retain(|_, existing| *existing != key);
        if place > 0 {
            slots.insert(place, key);
        }
        if slots.is_empty() {
            pins.remove(&scope);
        }
        let saved = serde_json::to_string(&pins)
            .map_err(std::io::Error::other)
            .and_then(|data| crate::store::write_atomic(&path, &data));
        if let Err(error) = saved {
            return Response::error(format!("保存固定排位失败：{error}"));
        }
        engine.set_manual_pins(pins);
        for client in self.clients.values_mut() {
            client.state.session.refresh_candidates();
            client.pending = None;
            client.revision = 0;
        }
        self.with_client(id, |state, env| state.current(true, env))
    }

    pub(super) fn forget(&mut self, id: ClientId, index: usize) -> Response {
        let mut forgot = None;
        let mut word = String::new();
        let mut response = self.with_client(id, |state, env| {
            if state.tool.is_none() && state.raw.is_none() {
                let shown = state.current(true, env);
                word = shown
                    .frame
                    .candidates
                    .get(index)
                    .cloned()
                    .unwrap_or_default();
                forgot = state.session.forget(index);
            }
            let mut out = state.current(true, env);
            out.forgot = forgot.is_some();
            out
        });
        if let Some(frame) = response.frame.as_mut() {
            frame.hint = Some(match &forgot {
                Some(w) => format!("已删除「{w}」的记忆"),
                None if word.is_empty() => "这里没有能删的词".to_string(),
                None => format!("「{word}」不是学来的，删不掉"),
            });
        }
        response
    }

    pub(super) fn command(&mut self, id: ClientId, command: Command) -> Response {
        match command {
            Command::EnglishCandidates => self.with_client(id, |state, env| {
                state.open_lookup(crate::lookup_mode::Kind::English, env)
            }),
            Command::RadicalLookup => self.with_client(id, |state, env| {
                state.open_lookup(crate::lookup_mode::Kind::Radical, env)
            }),
            Command::TogglePunct => self.toggle_punct(id),
            Command::ToggleTraditional => self.toggle_traditional(id),
            Command::EmojiPanel => {
                self.with_client(id, |state, env| state.open_tool(Page::Emoji, env))
            }
            Command::Toolbar => self.with_client(id, |state, env| state.open_tool(Page::Menu, env)),
            Command::OpenSettings => {
                if let Some(desktop) = &self.desktop {
                    desktop.open_settings();
                }
                Response::ok()
            }
            Command::ToggleStatusBar => {
                if let Err(error) = self.change_config(|c| c.status_bar = !c.status_bar) {
                    return Response::error(error);
                }
                Response {
                    config: Some(self.config.clone()),
                    ..Response::ok()
                }
            }
        }
    }

    pub(super) fn report(result: Result<ImportReport, String>) -> Response {
        match result {
            Ok(report) => Response {
                report: Some(report),
                ..Response::ok()
            },
            Err(error) => Response::error(error),
        }
    }
}
