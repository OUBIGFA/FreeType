//! 引擎宿主：一个引擎，每个输入法连接一个输入会话。只在一个线程里用。
//!
//! 会话（`wowtype_core::session::Session`）借用引擎；引擎要换掉或改设置（`&mut Engine`）时，
//! 先让所有会话放开引用（[`Host::detach`]），改完再接回去（[`Host::attach`]）。
//! 引擎放在 `Box` 里，地址不会因为 `Host` 移动而变。
//!
//! 按键规则在 [`crate::input`]；这里管连接、中英文状态（全局或按应用）、快捷键、设置、
//! 学到的词，以及通知桌面上的状态栏（[`Desktop`]）。
//!
//! TSF 先问「要不要这个键」（test）再真正按下：test 在会话副本上算一遍并记下结果，
//! 普通编辑键直接接纳这份结果；涉及学习、删除记录的键只在真正按下时执行。test 说不要的键不会再真按，这时副本直接留下
//! （交给应用的字记进了上文）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use freetype_protocol::{
    modifier, Command, Config, DefaultMode, Frame, ImportReport, Key, Mode, Request, Response, Status, UserWord,
    PROTOCOL_VERSION,
};
use wowtype_core::msime;
use wowtype_core::Engine;

use crate::dictfmt::{self, LexiconReading};
use crate::backup;
use crate::input::{session_config, ClientState, Env, Out};
use crate::store::Store;
use crate::tools::Page;

pub type ClientId = u64;

/// 整句候选个数上限（和 Linux 版一样）。
const MAX_CANDIDATES: usize = 1000;
/// 最近用过的 emoji 记多少个。
const RECENT_EMOJI: usize = 32;
/// 自己加的词的词频（学到的词里算高的，排在前面）。
const ADDED_WORD_FREQ: f64 = 50_000.0;

/// 桌面上的状态栏要显示的。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarState {
    /// 有输入框在用 FreeType，且设置里开着状态栏。
    pub visible: bool,
    pub autohide: bool,
    pub mode: Mode,
    pub chinese_punct: bool,
    pub traditional: bool,
}

/// 服务进程的桌面部分（状态栏、打开设置程序）。测试里没有。
pub trait Desktop {
    fn update(&self, state: &BarState);
    fn open_settings(&self);
}

struct Client {
    state: ClientState,
    /// test 算出来的结果：(键, 按下以后的状态, 应答)。
    pending: Option<(Key, ClientState, Out)>,
}

pub struct Host {
    clients: HashMap<ClientId, Client>,
    /// 会话借用它；见模块说明里的规则。
    engine: Option<Box<Engine>>,
    engine_error: Option<String>,
    store: Store,
    config: Config,
    /// 最后主动选择的中英文状态；全局模式和没有输入焦点时使用。
    mode: Mode,
    /// 按应用记的中英文状态（mode_scope 是 app 时用）。
    app_modes: HashMap<String, Mode>,
    /// 工具栏只读取独立常用语；快捷短语由引擎按编码匹配。
    phrases: Vec<String>,
    recent_emoji: Vec<String>,
    /// 正在用的输入框（状态栏按它显示）。
    focused: Option<ClientId>,
    desktop: Option<Box<dyn Desktop>>,
    last_bar: Option<BarState>,
    /// 正在用的主词库文件。
    lexicon_in_use: String,
    /// 设置里换了词库，等主循环在后台重新载入。
    lexicon_change: Option<String>,
    /// 学到了新东西，还没存盘。
    dirty: bool,
}

impl Drop for Host {
    fn drop(&mut self) {
        // 先丢会话再丢引擎
        self.clients.clear();
    }
}

/// 改设置里引擎的部分。
fn configure_engine(engine: &mut Engine, config: &Config) {
    engine.set_max_candidates(MAX_CANDIDATES);
    engine.set_fuzzy(config.fuzzy);
    engine.set_fuzzy_groups(config.fuzzy_groups);
    engine.set_shuangpin(config.shuangpin);
    if let Some(scheme) = wowtype_core::shuangpin::Scheme::from_name(&config.shuangpin_scheme) {
        engine.set_shuangpin_scheme(scheme);
    }
    engine.set_typo_correction(config.typo_correction);
    engine.set_traditional(config.traditional);
    engine.set_rare_ext_chars(config.rare_chars);
}

/// 主词库文件：设置里指定的（文件在的话），否则安装目录 `data` 里的。
pub fn lexicon_path(resources: &Path, configured: &str) -> PathBuf {
    let configured = Path::new(configured);
    if !configured.as_os_str().is_empty() && configured.is_file() {
        return configured.to_path_buf();
    }
    resources.join("lexicon.tsv")
}

/// 载入引擎（慢：几秒），在后台线程里调。`lexicon` 是设置里的词库路径（空串用默认的）。
pub fn load_engine(resources: &Path, store_dir: &Path, lexicon: &str) -> Result<Engine, String> {
    let lexicon_path = lexicon_path(resources, lexicon);
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
    // 可选：微信输入法的神经语言模型（用户自己放进来，不随安装包分发）
    let nnlm = store.dir().join("wetype").join("nlrgpt2_nnlm.bin");
    if nnlm.exists() {
        let _ = engine.load_nnlm(&nnlm);
    }
    Ok(engine)
}

fn mode_hint(mode: Mode) -> String {
    if mode == Mode::Chinese { "中" } else { "英" }.to_string()
}

/// 键是不是设置里选的快捷键（`name` 是 [`Config`] 里的写法）。
fn is_hotkey(key: &Key, name: &str) -> bool {
    let m = key.modifiers & (modifier::SHIFT | modifier::CTRL | modifier::ALT | modifier::SUPER);
    let c = char::from_u32(key.keysym).map(|c| c.to_ascii_lowercase());
    let (want_mods, want) = match name {
        "ctrl+shift+e" => (modifier::CTRL | modifier::SHIFT, 'e'),
        "ctrl+alt+e" => (modifier::CTRL | modifier::ALT, 'e'),
        "ctrl+period" => (modifier::CTRL, '.'),
        "ctrl+shift+f" => (modifier::CTRL | modifier::SHIFT, 'f'),
        "ctrl+shift+t" => (modifier::CTRL | modifier::SHIFT, 't'),
        "ctrl+space" => (modifier::CTRL, ' '),
        _ => return false,
    };
    m == want_mods && c == Some(want)
}

impl Host {
    pub fn new(store: Store) -> Self {
        let config = store.load_config();
        let recent_emoji = store.load_recent_emoji();
        let mut host = Self {
            clients: HashMap::new(),
            engine: None,
            engine_error: None,
            mode: config.default_mode.resolve(store.load_mode()),
            config,
            store,
            app_modes: HashMap::new(),
            phrases: Vec::new(),
            recent_emoji,
            focused: None,
            desktop: None,
            last_bar: None,
            lexicon_in_use: String::new(),
            lexicon_change: None,
            dirty: false,
        };
        host.load_phrases();
        host
    }

    pub fn set_desktop(&mut self, desktop: Box<dyn Desktop>) {
        self.desktop = Some(desktop);
        self.last_bar = None;
        self.notify_bar();
    }

    pub fn data_dir(&self) -> PathBuf {
        self.store.dir().to_path_buf()
    }

    pub fn config(&self) -> &Config {
        &self.config
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

    /// 设置里换了词库：返回新路径（主循环在后台载入，载好了交给 [`Host::install_engine`]）。
    /// 先把学到的词存盘，新引擎会从文件读回来。
    pub fn take_lexicon_change(&mut self) -> Option<String> {
        let change = self.lexicon_change.take()?;
        self.dirty = true;
        self.save_if_dirty();
        Some(change)
    }

    /// 记下正在用的词库文件（状态里显示）。
    pub fn set_lexicon_in_use(&mut self, path: String) {
        self.lexicon_in_use = path;
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

    fn load_phrases(&mut self) {
        self.phrases = match self.store.load_common_phrases() {
            Ok(phrases) => phrases,
            Err(error) => {
                crate::log(&error);
                Vec::new()
            }
        };
    }

    /// 快捷短语、导入的词库交给引擎（不用 &mut）。
    fn apply_user_data(&mut self) {
        self.load_phrases();
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
        let current_mode = self.current_mode();
        let old_scope = self.config.mode_scope.clone();
        let old_lexicon = self.config.lexicon.clone();
        self.config = config.sanitized();
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
        self.apply_user_data();
        self.notify_bar();
    }

    fn save_config(&mut self, config: Config) -> Result<(), String> {
        let config = config.sanitized();
        if config.default_mode == DefaultMode::Global && self.config.default_mode != DefaultMode::Global {
            self.store.save_mode(self.current_mode()).map_err(|e| format!("保存中英文状态失败：{e}"))?;
        }
        self.store.save_config(&config).map_err(|e| format!("保存设置失败：{e}"))?;
        self.set_config(config);
        Ok(())
    }

    /// 改一项设置并存盘（快捷键、状态栏切换的）。
    fn change_config(&mut self, change: impl FnOnce(&mut Config)) {
        let mut config = self.config.clone();
        change(&mut config);
        let _ = self.store.save_config(&config.clone().sanitized());
        self.set_config(config);
    }

    pub fn connect(&mut self, id: ClientId, app: String) {
        let state = ClientState::new(self.engine_ref(), &self.config, app);
        self.clients.insert(id, Client { state, pending: None });
    }

    pub fn disconnect(&mut self, id: ClientId) {
        self.clients.remove(&id);
        if self.focused == Some(id) {
            self.focused = None;
            self.notify_bar();
        }
    }

    /// 存用户词库和输入习惯（有新东西时）。
    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        if let Err(error) = self.save_learning() {
            crate::log(&error);
        }
    }

    fn save_learning(&mut self) -> Result<(), String> {
        let engine = self.engine.as_deref().ok_or("词库还在载入，请稍后再试")?;
        std::fs::create_dir_all(self.store.dir()).map_err(|error| format!("创建用户目录失败：{error}"))?;
        engine.save_user_dict(self.store.user_dict_path())
            .map_err(|error| format!("保存学习词条失败：{error:#}"))?;
        engine.save_history(self.store.history_path())
            .map_err(|error| format!("保存输入习惯失败：{error:#}"))?;
        self.dirty = false;
        Ok(())
    }

    fn mode_of(&self, app: &str) -> Mode {
        if self.config.default_mode != DefaultMode::Global && self.config.mode_scope == "app" {
            self.app_modes.get(app).copied().unwrap_or(self.config.default_mode.resolve(self.mode))
        } else {
            self.mode
        }
    }

    fn set_mode(&mut self, app: &str, mode: Mode) -> Result<(), String> {
        if self.config.default_mode == DefaultMode::Global {
            self.store.save_mode(mode).map_err(|e| format!("保存中英文状态失败：{e}"))?;
        }
        if self.config.default_mode != DefaultMode::Global && self.config.mode_scope == "app" {
            self.app_modes.insert(app.to_string(), mode);
        }
        // 打开设置后原应用会失焦；仍需保留最后的主动选择，供启用全局时沿用。
        self.mode = mode;
        // 别的连接（托盘、其他应用）也能切状态，旧状态下缓存的按键结果随之作废。
        for client in self.clients.values_mut() {
            client.pending = None;
        }
        Ok(())
    }

    fn app_of(&self, id: ClientId) -> String {
        self.clients.get(&id).map(|c| c.state.app.clone()).unwrap_or_default()
    }

    /// 状态栏、托盘看的状态：有焦点时取当前应用，否则保留最后的主动选择。
    fn current_mode(&self) -> Mode {
        self.focused.map(|id| self.mode_of(&self.app_of(id))).unwrap_or(self.mode)
    }

    fn notify_bar(&mut self) {
        let state = BarState {
            visible: self.config.status_bar && self.focused.is_some(),
            autohide: self.config.status_bar_autohide,
            mode: self.current_mode(),
            chinese_punct: self.config.fullwidth_punct,
            traditional: self.config.traditional,
        };
        if self.last_bar.as_ref() == Some(&state) {
            return;
        }
        if let Some(desktop) = &self.desktop {
            desktop.update(&state);
        }
        self.last_bar = Some(state);
    }

    fn status(&self) -> Status {
        Status {
            protocol: PROTOCOL_VERSION,
            version: option_env!("FREETYPE_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")).to_string(),
            ready: self.engine.is_some(),
            mode: self.current_mode(),
            chinese_punct: self.config.fullwidth_punct,
            traditional: self.config.traditional,
            data_dir: self.store.dir().display().to_string(),
            log_dir: crate::local_dir().join("logs").display().to_string(),
            lexicon: self.lexicon_in_use.clone(),
            error: self.engine_error.clone(),
        }
    }

    /// 补上候选窗的样式和状态；记下有没有学到东西。
    fn finish(&mut self, mode: Mode, out: Out) -> Response {
        if let Some(emoji) = out.emoji {
            self.remember_emoji(&emoji);
        }
        let mut frame = out.frame;
        if frame.commit.is_some() {
            self.dirty = true;
        }
        if out.forgot {
            self.dirty = true;
            self.save_if_dirty();
        }
        self.style(&mut frame, mode);
        Response::with_frame(frame)
    }

    fn style(&self, frame: &mut Frame, mode: Mode) {
        frame.mode = mode;
        frame.chinese_punct = self.config.fullwidth_punct;
        frame.traditional = self.config.traditional;
        frame.font_size = self.config.font_size;
        frame.font_family = self.config.font_family.clone();
        frame.candidate_style = self.config.candidate_style.clone();
        frame.vertical = !self.config.horizontal;
        frame.hint_position = self.config.hint_position.clone();
        frame.want_before = self.config.autospace;
    }

    fn remember_emoji(&mut self, emoji: &str) {
        self.recent_emoji.retain(|e| e != emoji);
        self.recent_emoji.insert(0, emoji.to_string());
        self.recent_emoji.truncate(RECENT_EMOJI);
        let _ = self.store.save_recent_emoji(&self.recent_emoji);
    }

    /// 对一个连接的输入状态做点事。
    fn with_client(&mut self, id: ClientId, f: impl FnOnce(&mut ClientState, &Env) -> Out) -> Response {
        let app = self.app_of(id);
        let mode = self.mode_of(&app);
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
    fn hint_frame(&mut self, id: ClientId, hint: Option<String>) -> Response {
        let enabled = self.config.hint_on_toggle;
        let mut response = self.with_client(id, |state, env| state.current(true, env));
        if !response.ok {
            let mode = self.current_mode();
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

    fn toggle_punct(&mut self, id: ClientId) -> Response {
        self.change_config(|c| c.fullwidth_punct = !c.fullwidth_punct);
        let hint = if self.config.fullwidth_punct { "中文标点" } else { "英文标点" };
        self.hint_frame(id, Some(hint.into()))
    }

    fn toggle_traditional(&mut self, id: ClientId) -> Response {
        self.change_config(|c| c.traditional = !c.traditional);
        let hint = if self.config.traditional { "繁體" } else { "简体" };
        self.hint_frame(id, Some(hint.into()))
    }

    /// 设置里选的快捷键：处理了返回应答。test 只回答「要这个键」，不动状态。
    fn hotkey(&mut self, id: ClientId, key: &Key) -> Option<Response> {
        let hit = |name: &str| is_hotkey(key, name);
        let action = if hit(&self.config.hotkey_punct) {
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
            let mode = self.mode_of(&self.app_of(id));
            let mut frame = Frame {
                handled: true,
                ..Frame::default()
            };
            self.style(&mut frame, mode);
            return Some(Response::with_frame(frame));
        }
        Some(match action {
            1 => self.toggle_punct(id),
            2 => self.toggle_traditional(id),
            3 => self.with_client(id, |state, env| {
                // 和上游一样，同一个表情快捷键再次按下时关闭面板。
                if state.tool.as_ref().is_some_and(|tool| tool.page == Page::Emoji) {
                    state.tool = None;
                    state.current(true, env)
                } else {
                    state.open_tool(Page::Emoji, env)
                }
            }),
            _ => self.toggle_mode(id, None),
        })
    }

    fn key(&mut self, id: ClientId, key: Key, before: Option<String>) -> Response {
        if let Some(response) = self.hotkey(id, &key) {
            if let Some(client) = self.clients.get_mut(&id) {
                client.pending = None;
            }
            return response;
        }
        let app = self.app_of(id);
        let mode = self.mode_of(&app);
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
        if let Some((pending, mut state, out)) = client.pending.take() {
            if pending == real && before.is_none() && state.session.accept_preview() {
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

    fn toggle_mode(&mut self, id: ClientId, shift: Option<String>) -> Response {
        if let Some(side) = shift.as_deref() {
            let allowed = match self.config.shift_toggle.as_str() {
                "both" => true,
                "left" => side == "left",
                "right" => side == "right",
                _ => false,
            };
            if !allowed {
                let mode = self.mode_of(&self.app_of(id));
                let mut frame = Frame::default();
                self.style(&mut frame, mode);
                return Response::with_frame(frame);
            }
        }
        let app = match self.clients.get(&id) {
            Some(client) => client.state.app.clone(),
            None => self.focused.map(|f| self.app_of(f)).unwrap_or_default(),
        };
        let current = if app.is_empty() { self.current_mode() } else { self.mode_of(&app) };
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

    fn focus(&mut self, id: ClientId, active: bool, before: Option<String>) -> Response {
        if active {
            self.focused = Some(id);
        } else if self.focused == Some(id) {
            self.focused = None;
        }
        let hint_on_focus = self.config.hint_on_focus;
        let mut response = self.with_client(id, |state, env| {
            if let Some(before) = &before {
                state.set_before(before);
            }
            state.current(false, env)
        });
        if let Some(frame) = response.frame.as_mut() {
            if active && hint_on_focus {
                frame.hint = Some(mode_hint(frame.mode));
            }
        }
        self.notify_bar();
        response
    }

    fn forget(&mut self, id: ClientId, index: usize) -> Response {
        let mut forgot = None;
        let mut word = String::new();
        let mut response = self.with_client(id, |state, env| {
            if state.tool.is_none() && state.raw.is_none() {
                let shown = state.current(true, env);
                word = shown.frame.candidates.get(index).cloned().unwrap_or_default();
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

    fn command(&mut self, id: ClientId, command: Command) -> Response {
        match command {
            Command::TogglePunct => self.toggle_punct(id),
            Command::ToggleTraditional => self.toggle_traditional(id),
            Command::EmojiPanel => self.with_client(id, |state, env| state.open_tool(Page::Emoji, env)),
            Command::Toolbar => self.with_client(id, |state, env| state.open_tool(Page::Menu, env)),
            Command::OpenSettings => {
                if let Some(desktop) = &self.desktop {
                    desktop.open_settings();
                }
                Response::ok()
            }
            Command::ToggleStatusBar => {
                self.change_config(|c| c.status_bar = !c.status_bar);
                Response {
                    config: Some(self.config.clone()),
                    ..Response::ok()
                }
            }
        }
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

    fn done(&mut self, result: Result<(), String>) -> Response {
        match result {
            Ok(()) => {
                self.apply_user_data();
                Response::ok()
            }
            Err(error) => Response::error(error),
        }
    }

    /// 给只有词语的词条注音：读正在用的词库文件，只收要用到的词和字。
    fn reading(&self, words: &[String]) -> Box<dyn dictfmt::Reading> {
        let text = std::fs::read_to_string(&self.lexicon_in_use).unwrap_or_default();
        Box::new(LexiconReading::from_tsv(&text, words))
    }

    fn add_user_word(&mut self, word: String, pinyin: String) -> Response {
        let word = word.trim().to_string();
        if word.is_empty() || word.contains(['\t', '\n', '\r']) {
            return Response::error("词语不能为空，也不能有换行");
        }
        let english = word.bytes().any(|b| b.is_ascii_alphabetic()) && word.bytes().all(|b| b.is_ascii_alphanumeric());
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
            return Response::error(format!("加不进去：{e}"));
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

    fn user_words(&self) -> Vec<UserWord> {
        let Some(engine) = self.engine.as_deref() else {
            return Vec::new();
        };
        let mut words: Vec<UserWord> = engine
            .user_words()
            .into_iter()
            .map(|(pinyin, word, freq)| {
                let display = wowtype_core::autospace::format_text(&word, None, crate::input::autospace_flags(&self.config));
                let display = if display == word { String::new() } else { display };
                UserWord { word, display, pinyin, freq }
            })
            .collect();
        words.sort_by(|a, b| b.freq.total_cmp(&a.freq).then_with(|| a.pinyin.cmp(&b.pinyin)));
        words
    }

    /// 改了引擎学到的东西：各输入框的候选作废，马上存盘。
    fn learned_changed(&mut self) -> Result<(), String> {
        for client in self.clients.values_mut() {
            client.pending = None;
            client.state.clear();
        }
        self.dirty = true;
        self.save_learning()
    }

    fn export_learning(&self, path: &Path) -> Response {
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

    fn import_learning(&mut self, path: &Path) -> Response {
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
            return Response::error(format!("学习记录已合并，但尚未保存到磁盘：{error}。请修复后重新导入"));
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

    fn export_settings(&self, path: &Path) -> Response {
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库还在载入，请稍后再试"); };
        Self::report(backup::export(&self.store, &self.config, self.mode, &self.recent_emoji, engine, path))
    }

    fn import_settings(&mut self, path: &Path) -> Response {
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库还在载入，请稍后再试"); };
        // 确认运行中引擎的学习状态可读，再准备文件；失败不能先覆盖其他设置。
        if let Err(error) = engine.export_learning() { return Response::error(format!("读取当前学习记录失败：{error:#}")); }
        let prepared = match backup::read(path) { Ok(data) => data, Err(error) => return Response::error(error) };
        let cleanup_note = match prepared.restore_files(&self.store) { Ok(note) => note, Err(error) => return Response::error(error) };
        engine.clear_learned();
        if let Err(error) = engine.import_learning(&prepared.data.learning.to_string()) {
            return Response::error(format!("备份文件已恢复，但输入引擎尚未更新：{error:#}。请重新打开 FreeType"));
        }
        self.dirty = false;
        self.recent_emoji = prepared.data.recent_emoji.clone();
        self.set_config(prepared.restored_config(&self.store));
        // 同一路径的主词库内容也可能变了，不能只按路径字符串判断是否重新载入。
        if prepared.data.main_lexicon.is_some() { self.lexicon_change = Some(self.config.lexicon.clone()); }
        self.mode = prepared.data.mode;
        self.app_modes.clear();
        for client in self.clients.values_mut() {
            client.pending = None;
            client.state.clear();
            client.state.set_before("");
        }
        self.notify_bar();
        let mut report = prepared.report();
        if let Some(note) = cleanup_note { report.note = Some(format!("{}；{note}", report.note.unwrap_or_default())); }
        Response {
            config: Some(self.config.clone()), status: Some(self.status()),
            quick_phrases: Some(self.store.list_quick_phrases()), common_phrases: Some(prepared.data.common_phrases),
            dictionaries: Some(self.store.list_dictionaries()), user_words: Some(self.user_words()), report: Some(report),
            ..Response::ok()
        }
    }

    /// 处理一条请求；里面出了 panic 时记日志、丢掉这个连接的输入状态，不让服务进程退出。
    pub fn handle(&mut self, id: ClientId, request: Request) -> Response {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle_inner(id, request)));
        match result {
            Ok(response) => response,
            Err(_) => {
                crate::log(&format!("处理请求时出错（连接 {id}），已重置这个输入框"));
                let engine = self.engine_ref();
                if let Some(client) = self.clients.get_mut(&id) {
                    let app = client.state.app.clone();
                    client.pending = None;
                    client.state = ClientState::new(engine, &self.config, app);
                }
                Response::error("服务进程内部出错，已重置这个输入框")
            }
        }
    }

    fn handle_inner(&mut self, id: ClientId, request: Request) -> Response {
        match request {
            Request::Hello { session, app } => {
                if session && !self.clients.contains_key(&id) {
                    self.connect(id, app.to_ascii_lowercase());
                }
                Response {
                    status: Some(self.status()),
                    ..Response::ok()
                }
            }
            Request::Key { key, before } => self.key(id, key, before),
            Request::Select { index } => self.with_client(id, |state, env| state.select(index, env)),
            Request::Page { next } => self.with_client(id, |state, env| state.page(next, env)),
            Request::Forget { index } => self.forget(id, index),
            Request::Reset { commit } => self.with_client(id, |state, env| state.reset(commit, env)),
            Request::Focus { active, before } => self.focus(id, active, before),
            Request::ToggleMode { shift } => self.toggle_mode(id, shift),
            Request::Command { command } => self.command(id, command),
            Request::Status => Response {
                status: Some(self.status()),
                ..Response::ok()
            },
            Request::GetConfig => Response {
                config: Some(self.config.clone()),
                ..Response::ok()
            },
            Request::SetConfig { config } => match self.save_config(config) {
                Ok(()) => {
                    Response {
                        config: Some(self.config.clone()),
                        ..Response::ok()
                    }
                }
                Err(e) => Response::error(e),
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
            Request::ListCommonPhrases => match self.store.load_common_phrases() {
                Ok(phrases) => Response { common_phrases: Some(phrases), ..Response::ok() },
                Err(error) => Response::error(error),
            },
            Request::SetCommonPhrases { phrases } => {
                let result = self.store.set_common_phrases(phrases);
                self.done(result)
            }
            Request::ImportCommonPhrases { path, replace } => {
                let result = self.store.import_common_phrases(Path::new(&path), replace);
                if result.is_ok() {
                    self.load_phrases();
                }
                Self::report(result)
            }
            Request::ExportCommonPhrases { path } => {
                Self::report(self.store.export_common_phrases(Path::new(&path)).map(|added| ImportReport {
                    added, ..ImportReport::default()
                }))
            }
            Request::ImportDictionary { path, name } => {
                let reading = |words: &[String]| self.reading(words);
                let response = Self::report(self.store.import_dictionary(Path::new(&path), &name, &reading));
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
            Request::ExportLearning { path } => self.export_learning(Path::new(&path)),
            Request::ImportLearning { path } => self.import_learning(Path::new(&path)),
            Request::ExportSettings { path } => self.export_settings(Path::new(&path)),
            Request::InspectSettingsBackup { path } => Self::report(backup::read(Path::new(&path)).map(|data| data.report())),
            Request::ImportSettings { path } => self.import_settings(Path::new(&path)),
            Request::ListUserWords => Response {
                user_words: Some(self.user_words()),
                ..Response::ok()
            },
            Request::ForgetWords { words } => {
                let Some(engine) = self.engine.as_deref() else {
                    return Response::error("词库还在载入，请稍后再试");
                };
                let removed = words.iter().filter(|w| engine.forget_word(w)).count();
                if let Err(error) = self.learned_changed() {
                    return Response::error(error);
                }
                Response {
                    report: Some(ImportReport {
                        added: removed,
                        ..ImportReport::default()
                    }),
                    user_words: Some(self.user_words()),
                    ..Response::ok()
                }
            }
            Request::AddUserWord { word, pinyin } => self.add_user_word(word, pinyin),
            Request::ClearLearned => {
                let Some(engine) = self.engine.as_deref() else {
                    return Response::error("词库还在载入，请稍后再试");
                };
                engine.clear_learned();
                if let Err(error) = self.learned_changed() {
                    return Response::error(error);
                }
                Response {
                    user_words: Some(Vec::new()),
                    ..Response::ok()
                }
            }
            Request::ClearClipboard => {
                wowtype_core::clipboard::clear();
                Response::ok()
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
mod tests;
