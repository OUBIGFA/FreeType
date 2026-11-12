//! 引擎宿主：一个引擎，每个输入法连接一个输入会话。只在一个线程里用。
//!
//! 会话（`wowtype_core::session::Session`）借用引擎；引擎要换掉或改设置（`&mut Engine`）时，
//! 先让所有会话放开引用（[`Host::detach`]），改完再接回去（[`Host::attach`]）。
//! 引擎放在 `Box` 里，地址不会因为 `Host` 移动而变。
//!
//! 会话还没挪进核心的输入规则在这里补上（Linux 版在 fcitx5 外壳里）：
//!
//! - 中 / 英两种状态，切换时把编辑区的字母原样上屏；
//! - 中文状态下的标点：没在组字时按全角规则转换；组字时先上屏光标处的候选，再上屏标点；
//! - 组字时会话不认的键一律吃掉，不让它漏给应用；
//! - TSF 先问「要不要这个键」（test）再真正按下：test 在会话副本上算一遍并记下结果，
//!   紧接着的真按键直接用这份结果，不算两遍。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use freetype_protocol::{
    modifier, Config, Frame, ImportReport, Key, Mode, Request, Response, Status, PROTOCOL_VERSION,
};
use wowtype_core::msime;
use wowtype_core::session::{self, KeyAction, Session};
use wowtype_core::symbols::{self, PunctStyle};
use wowtype_core::Engine;

use crate::store::Store;

pub type ClientId = u64;

/// 光标前记多少个字当上文（选了开头的词以后给剩下的部分排序用）。
const CONTEXT_CHARS: usize = 16;
/// 标点上屏前最多连按几次空格把句首词选完。
const MAX_HEAD_COMMITS: usize = 32;
/// 整句候选个数上限（和 Linux 版一样）。
const MAX_CANDIDATES: usize = 1000;

#[derive(Clone)]
struct ClientState {
    session: Session<'static>,
    /// 这个输入框里最近上屏的字。
    context: String,
}

struct Client {
    state: ClientState,
    /// test 算出来的结果：(键, 按下以后的状态, 应答)。
    pending: Option<(Key, ClientState, Frame)>,
}

pub struct Host {
    clients: HashMap<ClientId, Client>,
    /// 会话借用它；见模块说明里的规则。
    engine: Option<Box<Engine>>,
    engine_error: Option<String>,
    store: Store,
    config: Config,
    mode: Mode,
    /// 学到了新东西，还没存盘。
    dirty: bool,
}

impl Drop for Host {
    fn drop(&mut self) {
        // 先丢会话再丢引擎
        self.clients.clear();
    }
}

fn shown_text(text: &str) -> String {
    text.replace('\n', "⏎").replace('\t', "⇥")
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// 可见的 ASCII 标点（字母、数字以外）。
fn punct_char(key: &Key) -> Option<char> {
    if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
        return None;
    }
    let c = char::from_u32(key.keysym)?;
    (c.is_ascii_graphic() && !c.is_ascii_alphanumeric()).then_some(c)
}

fn is_letter(key: &Key) -> bool {
    char::from_u32(key.keysym).is_some_and(|c| c.is_ascii_alphabetic())
}

fn core_key(key: &Key, prev: Option<char>) -> session::Key {
    session::Key {
        keysym: key.keysym,
        modifiers: key.modifiers,
        release: false,
        prev_char: prev,
        action: KeyAction::None,
    }
}

impl ClientState {
    fn new(engine: Option<&'static Engine>, config: session::Config) -> Self {
        let mut session = Session::new();
        session.set_engine(engine);
        session.configure(config);
        Self {
            session,
            context: String::new(),
        }
    }

    fn remember(&mut self, text: &str) {
        self.context.push_str(text);
        let n = self.context.chars().count();
        if n > CONTEXT_CHARS {
            self.context = self.context.chars().skip(n - CONTEXT_CHARS).collect();
        }
        self.session.set_context(&self.context.clone());
    }

    /// 核心的结果翻成应答，顺便记下上屏的字。
    fn frame(&mut self, core: session::Frame, mode: Mode) -> Frame {
        if let Some(text) = &core.commit {
            self.remember(text);
        }
        let candidates = if core.cand_count > 0 {
            self.session.candidates()[core.cand_start..core.cand_start + core.cand_count]
                .iter()
                .map(|c| shown_text(&c.text))
                .collect()
        } else {
            Vec::new()
        };
        Frame {
            handled: core.handled,
            commit: core.commit,
            preedit_cursor: utf16_len(&core.preedit[..core.preedit_cursor.min(core.preedit.len())]),
            preedit: core.preedit,
            candidates,
            cursor: core.cand_cursor,
            has_prev: core.has_prev,
            has_next: core.has_next,
            mode,
            ..Frame::default()
        }
    }

    /// 只看当前状态，不按键。
    fn current(&mut self, handled: bool, mode: Mode) -> Frame {
        let core = self.session.frame(handled);
        self.frame(core, mode)
    }

    fn key(&mut self, key: &Key, mode: Mode, fullwidth: bool) -> Frame {
        let composing = self.session.composing();
        if mode == Mode::English {
            return self.current(false, mode);
        }
        if key.modifiers & modifier::CAPS != 0 && !composing && is_letter(key) {
            // 大写锁定：字母原样交给应用
            return self.current(false, mode);
        }
        let prev = self.context.chars().last();
        let core = self.session.key(&core_key(key, prev));
        if core.handled || !composing {
            if core.handled {
                return self.frame(core, mode);
            }
            // 没在组字：中文标点
            let style = if fullwidth {
                PunctStyle::Fullwidth
            } else {
                PunctStyle::Halfwidth
            };
            if let Some(text) = punct_char(key).and_then(|c| symbols::punct(c, style, prev)) {
                let mut frame = self.frame(core, mode);
                frame.handled = true;
                frame.commit = Some(text.to_string());
                self.remember(text);
                return frame;
            }
            return self.frame(core, mode);
        }
        // 组字时会话不认的键
        let Some(c) = punct_char(key) else {
            // 方向键以外的功能键、Ctrl 组合键：吃掉，不漏给应用
            let mut frame = self.frame(core, mode);
            frame.handled = true;
            return frame;
        };
        // 标点：先把候选选完上屏，再上屏标点
        let mut text = String::new();
        for _ in 0..MAX_HEAD_COMMITS {
            if !self.session.composing() {
                break;
            }
            let space = self.session.key(&session::Key::press(session::keysym::SPACE));
            if let Some(commit) = space.commit {
                text.push_str(&commit);
            }
            if !space.handled {
                break;
            }
        }
        if self.session.composing() {
            // 没有候选可选：原样上屏字母
            if let Some(commit) = self.session.commit_input().commit {
                text.push_str(&commit);
            }
        }
        let style = if fullwidth {
            PunctStyle::Fullwidth
        } else {
            PunctStyle::Halfwidth
        };
        let prev = text.chars().last().or(prev);
        match symbols::punct(c, style, prev) {
            Some(p) => text.push_str(p),
            None => text.push(c),
        }
        let mut frame = self.current(true, mode);
        self.remember(&text);
        frame.commit = Some(text);
        frame
    }
}

fn session_config(config: &Config) -> session::Config {
    session::Config {
        page_size: config.page_size as usize,
        calculator: config.calculator,
        date_time: config.date_time,
        emoji_suggest: config.emoji_suggest,
        horizontal: config.horizontal,
        ..session::Config::default()
    }
}

/// 改设置里引擎的部分。
fn configure_engine(engine: &mut Engine, config: &Config) {
    engine.set_max_candidates(MAX_CANDIDATES);
    engine.set_fuzzy(config.fuzzy);
    engine.set_shuangpin(config.shuangpin);
    if let Some(scheme) = wowtype_core::shuangpin::Scheme::from_name(&config.shuangpin_scheme) {
        engine.set_shuangpin_scheme(scheme);
    }
    engine.set_typo_correction(config.typo_correction);
    engine.set_traditional(config.traditional);
}

/// 从安装目录的 `data` 载入引擎（慢：几秒），在后台线程里调。
pub fn load_engine(resources: &Path, store_dir: &Path) -> Result<Engine, String> {
    let lexicon_path = resources.join("lexicon.tsv");
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_path_cached(&lexicon_path)
        .map_err(|e| format!("词库载入失败（{}）：{e:#}", lexicon_path.display()))?;
    let lm: std::sync::Arc<dyn wowtype_core::LanguageModel> =
        match wowtype_core::lm::load_lm(resources.join("words.lm")) {
            Ok(lm) => lm,
            Err(_) => std::sync::Arc::new(wowtype_core::BigramLm::default()),
        };
    let engine = Engine::new(std::sync::Arc::new(lexicon), lm);
    let _ = engine.load_english(resources.join("english.txt"));
    let _ = engine.load_conversion(resources.join("s2t.tsv"));
    let store = Store::new(store_dir.to_path_buf());
    if store.user_dict_path().exists() {
        let _ = engine.load_user_dict(store.user_dict_path());
    }
    if store.history_path().exists() {
        let _ = engine.load_history(store.history_path());
    }
    Ok(engine)
}

impl Host {
    pub fn new(store: Store) -> Self {
        let config = store.load_config();
        Self {
            clients: HashMap::new(),
            engine: None,
            engine_error: None,
            store,
            config,
            mode: Mode::Chinese,
            dirty: false,
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.store.dir().to_path_buf()
    }

    /// 会话用的引擎引用。
    fn engine_ref(&self) -> Option<&'static Engine> {
        // SAFETY: 引擎在 Box 里，地址稳定；换掉或改它之前 detach() 会让所有会话放开引用，
        // Drop 时先丢会话。所以会话手里的引用不会比引擎活得久，也不会和 &mut 同时存在。
        self.engine
            .as_deref()
            .map(|engine| unsafe { &*(engine as *const Engine) })
    }

    fn detach(&mut self) {
        for client in self.clients.values_mut() {
            client.state.session.set_engine(None);
            client.pending = None;
        }
    }

    fn attach(&mut self) {
        let engine = self.engine_ref();
        let config = session_config(&self.config);
        for client in self.clients.values_mut() {
            client.state.session.set_engine(engine);
            client.state.session.configure(config);
        }
    }

    /// 后台载入好了：装上引擎，套用设置和用户数据。
    pub fn install_engine(&mut self, result: Result<Engine, String>) {
        self.detach();
        match result {
            Ok(mut engine) => {
                configure_engine(&mut engine, &self.config);
                self.engine = Some(Box::new(engine));
                self.engine_error = None;
                self.apply_user_data();
            }
            Err(error) => self.engine_error = Some(error),
        }
        self.attach();
    }

    /// 快捷短语、导入的词库交给引擎（不用 &mut）。
    fn apply_user_data(&self) {
        let Some(engine) = self.engine.as_deref() else {
            return;
        };
        let phrases = if self.config.quick_phrases {
            msime::quick_phrases_to_custom_tsv(&self.store.load_quick_phrases())
        } else {
            String::new()
        };
        engine.lexicon().set_custom_tsv(&phrases);
        let words = self
            .store
            .load_dictionaries()
            .into_iter()
            .flat_map(|(_, entries)| entries)
            .map(|e| (e.pinyin, e.word, f64::from(e.weight.max(1))))
            .collect();
        engine.lexicon().set_extra_words(words);
    }

    fn set_config(&mut self, config: Config) {
        self.config = config.sanitized();
        self.detach();
        if let Some(engine) = self.engine.as_deref_mut() {
            configure_engine(engine, &self.config);
        }
        self.attach();
        self.apply_user_data();
    }

    pub fn connect(&mut self, id: ClientId) {
        let state = ClientState::new(self.engine_ref(), session_config(&self.config));
        self.clients.insert(id, Client { state, pending: None });
    }

    pub fn disconnect(&mut self, id: ClientId) {
        self.clients.remove(&id);
    }

    /// 存用户词库和输入习惯（有新东西时）。
    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(engine) = self.engine.as_deref() else {
            return;
        };
        let _ = std::fs::create_dir_all(self.store.dir());
        let saved = engine.save_user_dict(self.store.user_dict_path()).is_ok()
            && engine.save_history(self.store.history_path()).is_ok();
        if saved {
            self.dirty = false;
        }
    }

    fn status(&self) -> Status {
        Status {
            protocol: PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").to_string(),
            ready: self.engine.is_some(),
            mode: self.mode,
            data_dir: self.store.dir().display().to_string(),
            error: self.engine_error.clone(),
        }
    }

    /// 记下有没有学到东西，补上候选窗的样式。
    fn finish(&mut self, mut frame: Frame) -> Response {
        if frame.commit.is_some() {
            self.dirty = true;
        }
        frame.font_size = self.config.font_size;
        frame.vertical = !self.config.horizontal;
        Response::with_frame(frame)
    }

    fn with_client(&mut self, id: ClientId, f: impl FnOnce(&mut ClientState, Mode, &Config) -> Frame) -> Response {
        let mode = self.mode;
        let config = self.config.clone();
        let Some(client) = self.clients.get_mut(&id) else {
            return Response::error("这个连接没有输入会话（先发 hello，session 为 true）");
        };
        client.pending = None;
        let frame = f(&mut client.state, mode, &config);
        self.finish(frame)
    }

    fn key(&mut self, id: ClientId, key: Key) -> Response {
        let mode = self.mode;
        let fullwidth = self.config.fullwidth_punct;
        let Some(client) = self.clients.get_mut(&id) else {
            return Response::error("这个连接没有输入会话（先发 hello，session 为 true）");
        };
        let real = Key { test: false, ..key };
        if key.test {
            let mut state = client.state.clone();
            let frame = state.key(&real, mode, fullwidth);
            client.pending = Some((real, state, frame.clone()));
            return self.finish(frame);
        }
        let frame = match client.pending.take() {
            Some((pending, state, frame)) if pending == real => {
                client.state = state;
                frame
            }
            _ => client.state.key(&real, mode, fullwidth),
        };
        self.finish(frame)
    }

    fn toggle_mode(&mut self, id: ClientId) -> Response {
        self.mode = match self.mode {
            Mode::Chinese => Mode::English,
            Mode::English => Mode::Chinese,
        };
        let mode = self.mode;
        let hint = Some(if mode == Mode::Chinese { "中" } else { "英" }.to_string());
        let frame = match self.clients.get_mut(&id) {
            Some(client) => {
                client.pending = None;
                let state = &mut client.state;
                let core = if mode == Mode::English && state.session.composing() {
                    state.session.commit_input()
                } else {
                    state.session.frame(true)
                };
                state.frame(core, mode)
            }
            None => Frame {
                handled: true,
                mode,
                ..Frame::default()
            },
        };
        self.finish(Frame { hint, ..frame })
    }

    fn report(result: Result<ImportReport, String>) -> Response {
        match result {
            Ok(report) => Response {
                report: Some(report),
                ..Response::ok()
            },
            Err(error) => Response::error(error),
        }
    }

    fn done(&self, result: Result<(), String>) -> Response {
        match result {
            Ok(()) => {
                self.apply_user_data();
                Response::ok()
            }
            Err(error) => Response::error(error),
        }
    }

    /// 处理一条请求。
    pub fn handle(&mut self, id: ClientId, request: Request) -> Response {
        match request {
            Request::Hello { session } => {
                if session && !self.clients.contains_key(&id) {
                    self.connect(id);
                }
                Response {
                    status: Some(self.status()),
                    ..Response::ok()
                }
            }
            Request::Key { key } => self.key(id, key),
            Request::Select { index } => self.with_client(id, |state, mode, _| {
                let core = state.session.select(index);
                state.frame(core, mode)
            }),
            Request::Page { next } => self.with_client(id, |state, mode, _| {
                let action = if next {
                    KeyAction::NextPage
                } else {
                    KeyAction::PrevPage
                };
                let core = state.session.key(&session::Key::action(action));
                state.frame(core, mode)
            }),
            Request::Reset { commit } => self.with_client(id, |state, mode, _| {
                let core = if commit && state.session.composing() {
                    state.session.commit_input()
                } else {
                    state.session.reset(false)
                };
                state.frame(core, mode)
            }),
            Request::ToggleMode => self.toggle_mode(id),
            Request::Status => Response {
                status: Some(self.status()),
                ..Response::ok()
            },
            Request::GetConfig => Response {
                config: Some(self.config.clone()),
                ..Response::ok()
            },
            Request::SetConfig { config } => match self.store.save_config(&config.clone().sanitized()) {
                Ok(()) => {
                    self.set_config(config);
                    Response {
                        config: Some(self.config.clone()),
                        ..Response::ok()
                    }
                }
                Err(e) => Response::error(format!("保存设置失败：{e}")),
            },
            Request::ListQuickPhrases => Response {
                quick_phrases: Some(self.store.list_quick_phrases()),
                ..Response::ok()
            },
            Request::SetQuickPhrases { phrases } => {
                let result = self.store.set_quick_phrases(&phrases);
                self.done(result)
            }
            Request::ImportQuickPhrases { path, replace } => {
                let response = Self::report(self.store.import_quick_phrases(Path::new(&path), replace));
                self.apply_user_data();
                response
            }
            Request::ExportQuickPhrases { path } => match self.store.export_quick_phrases(Path::new(&path)) {
                Ok(count) => Response {
                    report: Some(ImportReport {
                        added: count,
                        ..ImportReport::default()
                    }),
                    ..Response::ok()
                },
                Err(error) => Response::error(error),
            },
            Request::ListDictionaries => Response {
                dictionaries: Some(self.store.list_dictionaries()),
                ..Response::ok()
            },
            Request::ImportDictionary { path, name } => {
                let response = Self::report(self.store.import_dictionary(Path::new(&path), &name));
                self.apply_user_data();
                response
            }
            Request::RemoveDictionary { name } => {
                let result = self.store.remove_dictionary(&name);
                self.done(result)
            }
            Request::ExportUserDictionary { path } => {
                let Some(engine) = self.engine.as_deref() else {
                    return Response::error("词库还在载入，请稍后再试");
                };
                let entries: Vec<msime::DictEntry> = engine
                    .lexicon()
                    .words()
                    .into_iter()
                    .map(|(pinyin, word, freq)| msime::DictEntry {
                        word,
                        pinyin,
                        weight: freq.clamp(1.0, f64::from(u32::MAX)) as u32,
                    })
                    .collect();
                match std::fs::write(&path, msime::format_dictionary(&entries)) {
                    Ok(()) => Response {
                        report: Some(ImportReport {
                            added: entries.len(),
                            ..ImportReport::default()
                        }),
                        ..Response::ok()
                    },
                    Err(e) => Response::error(format!("写入文件失败：{e}")),
                }
            }
            Request::Shutdown => {
                self.save_if_dirty();
                Response::ok()
            }
            Request::Reload => {
                let config = self.store.load_config();
                self.set_config(config);
                Response {
                    status: Some(self.status()),
                    ..Response::ok()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str) -> Host {
        let dir = std::env::temp_dir().join(format!("freetype-host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut host = Host::new(Store::new(dir));
        let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
            "ni hao\t你好\t1000\nni\t你\t900\nhao\t好\t800\nshi jie\t世界\t700\nyun\t韵\t10\n",
        )
        .unwrap();
        let engine = Engine::new(
            std::sync::Arc::new(lexicon),
            std::sync::Arc::new(wowtype_core::BigramLm::default()),
        );
        host.install_engine(Ok(engine));
        host.handle(1, Request::Hello { session: true });
        host
    }

    fn press(host: &mut Host, sym: u32) -> Frame {
        let key = Key {
            keysym: sym,
            modifiers: 0,
            test: false,
        };
        host.handle(1, Request::Key { key }).frame.unwrap()
    }

    fn typing(host: &mut Host, text: &str) -> Frame {
        let mut frame = Frame::default();
        for c in text.chars() {
            frame = press(host, c as u32);
        }
        frame
    }

    #[test]
    fn letters_compose_and_space_commits() {
        let mut host = host("compose");
        let frame = typing(&mut host, "nihao");
        assert!(frame.handled);
        assert_eq!(frame.candidates[0], "你好");
        assert!(!frame.preedit.is_empty());
        let frame = press(&mut host, 0x20);
        assert_eq!(frame.commit.as_deref(), Some("你好"));
        assert!(frame.preedit.is_empty());
        assert!(frame.candidates.is_empty());
    }

    #[test]
    fn punctuation_commits_candidate_then_fullwidth_mark() {
        let mut host = host("punct");
        typing(&mut host, "nihao");
        let frame = press(&mut host, ',' as u32);
        assert!(frame.handled);
        assert_eq!(frame.commit.as_deref(), Some("你好，"));
        let frame = press(&mut host, '.' as u32);
        assert_eq!(frame.commit.as_deref(), Some("。"));
        let frame = press(&mut host, '1' as u32);
        assert!(!frame.handled, "没在组字时数字交给应用");
    }

    #[test]
    fn test_then_real_key_uses_one_result() {
        let mut host = host("test-key");
        let key = Key {
            keysym: 'n' as u32,
            modifiers: 0,
            test: true,
        };
        let tested = host.handle(1, Request::Key { key }).frame.unwrap();
        assert!(tested.handled);
        let real = host
            .handle(1, Request::Key { key: Key { test: false, ..key } })
            .frame
            .unwrap();
        assert_eq!(real, tested);
        let frame = press(&mut host, 'i' as u32);
        assert_eq!(frame.candidates[0], "你");
    }

    #[test]
    fn test_alone_does_not_change_state() {
        let mut host = host("test-only");
        let key = Key {
            keysym: 'n' as u32,
            modifiers: 0,
            test: true,
        };
        host.handle(1, Request::Key { key });
        let frame = host.handle(1, Request::Status);
        assert!(frame.ok);
        let frame = press(&mut host, 0xff1b);
        assert!(!frame.handled, "没有真的进编辑区，Esc 交给应用");
    }

    #[test]
    fn english_mode_passes_keys_and_toggle_commits_letters() {
        let mut host = host("mode");
        typing(&mut host, "nih");
        let frame = host.handle(1, Request::ToggleMode).frame.unwrap();
        assert_eq!(frame.mode, Mode::English);
        assert_eq!(frame.hint.as_deref(), Some("英"));
        assert_eq!(frame.commit.as_deref(), Some("nih"));
        let frame = press(&mut host, 'a' as u32);
        assert!(!frame.handled);
        let frame = host.handle(1, Request::ToggleMode).frame.unwrap();
        assert_eq!(frame.mode, Mode::Chinese);
        assert!(press(&mut host, 'a' as u32).handled);
    }

    #[test]
    fn caps_lock_letters_go_to_app() {
        let mut host = host("caps");
        let key = Key {
            keysym: 'A' as u32,
            modifiers: modifier::CAPS,
            test: false,
        };
        assert!(!host.handle(1, Request::Key { key }).frame.unwrap().handled);
    }

    #[test]
    fn composing_swallows_ctrl_combinations() {
        let mut host = host("ctrl");
        typing(&mut host, "ni");
        let key = Key {
            keysym: 'c' as u32,
            modifiers: modifier::CTRL,
            test: false,
        };
        let frame = host.handle(1, Request::Key { key }).frame.unwrap();
        assert!(frame.handled);
        assert!(frame.commit.is_none());
    }

    #[test]
    fn quick_phrases_and_dictionaries_change_candidates() {
        let mut host = host("user-data");
        let file = host.data_dir().with_extension("qp.txt");
        std::fs::write(&file, "yyjq\t韵酒\n").unwrap();
        let response = host.handle(
            1,
            Request::ImportQuickPhrases {
                path: file.display().to_string(),
                replace: false,
            },
        );
        assert_eq!(response.report.unwrap().added, 1);
        assert_eq!(typing(&mut host, "yyjq").candidates[0], "韵酒");
        press(&mut host, 0xff1b);

        std::fs::write(&file, "示例词\tshi li ci\t5000\n").unwrap();
        let response = host.handle(
            1,
            Request::ImportDictionary {
                path: file.display().to_string(),
                name: "示例".into(),
            },
        );
        assert!(response.ok, "{:?}", response.error);
        assert!(typing(&mut host, "shilici").candidates.iter().any(|c| c == "示例词"));

        let mut config = host.handle(1, Request::GetConfig).config.unwrap();
        config.quick_phrases = false;
        host.handle(1, Request::SetConfig { config });
        press(&mut host, 0xff1b);
        assert!(!typing(&mut host, "yyjq").candidates.iter().any(|c| c == "韵酒"));
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_dir_all(host.data_dir());
    }

    #[test]
    fn select_and_page_and_reset() {
        let mut host = host("select");
        typing(&mut host, "ni");
        let frame = host.handle(1, Request::Select { index: 0 }).frame.unwrap();
        assert_eq!(frame.commit.as_deref(), Some("你"));
        typing(&mut host, "hao");
        let frame = host.handle(1, Request::Reset { commit: true }).frame.unwrap();
        assert_eq!(frame.commit.as_deref(), Some("hao"));
        assert!(host.handle(1, Request::Page { next: true }).ok);
    }

    #[test]
    fn keys_before_engine_loads_still_compose() {
        let dir = std::env::temp_dir().join(format!("freetype-host-noengine-{}", std::process::id()));
        let mut host = Host::new(Store::new(dir));
        host.handle(7, Request::Hello { session: true });
        let key = Key {
            keysym: 'n' as u32,
            modifiers: 0,
            test: false,
        };
        let frame = host.handle(7, Request::Key { key }).frame.unwrap();
        assert!(frame.handled);
        assert!(!host.handle(7, Request::Status).status.unwrap().ready);
    }
}
