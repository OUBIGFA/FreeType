//! 引擎宿主：一个引擎，每个输入法连接一个输入会话。只在一个线程里用。
//!
//! 会话（`wowtype_core::session::Session`）借用引擎；引擎要换掉或改设置（`&mut Engine`）时，
//! 先让所有会话放开引用（[`Host::detach`]），改完再接回去（[`Host::attach`]）。
//! 引擎放在 `Box` 里，地址不会因为 `Host` 移动而变。
//!
//! 按键规则在 [`crate::input`]；这里管连接、中英文状态（全局或按应用）、快捷键、设置、
//! 学到的词，以及通知桌面上的状态栏（[`Desktop`]）。
//!
//! 记住的中英文状态之外还有两件事影响实际的输入状态：
//!
//! - **大写锁定**：开着时一律按英文输入（[`Host::effective`]），不改记住的状态，关掉就回去；
//! - **指定软件的预设**：切入时建立当前程序的临时状态（[`Host::foreground`]），
//!   软件内可手动切换；离开即丢弃，不覆盖普通程序的状态或全局记忆。
//!
//! TSF 先问「要不要这个键」（test）再真正按下：test 在会话副本上算一遍并记下结果，
//! 普通编辑键直接接纳这份结果；涉及学习、删除记录的键只在真正按下时执行。test 说不要的键不会再真按，这时副本直接留下
//! （交给应用的字记进了上文）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use freetype_protocol::{
    keysym, modifier, AppWindow, BackupPart, BrandIcon, CandidateAction, Command, Config, DefaultMode, Frame, ImportReport, Key, Mode, Request, Response,
    Status, UserWord, PROTOCOL_VERSION,
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
    pub scale: u32,
    pub icon_size: u32,
    pub hide_seconds: u32,
    pub shadow: bool,
    pub parts: u8,
    pub mode: Mode,
    pub chinese_punct: bool,
    pub traditional: bool,
}

/// 服务进程的桌面部分（状态栏、打开设置程序）。测试里没有。
pub trait Desktop {
    fn update(&self, state: &BarState);
    fn open_settings(&self);
    /// 已保存的品牌图像变化时通知已激活的语言栏；接收方只读取图像。
    fn brand_icon_changed(&self) {}
    /// 现在前台窗口属于哪个程序（小写文件名；认不出的程序是空串）。
    /// None：没有前台窗口，或者是 FreeType 自己的窗口，不算换了程序。
    fn foreground_app(&self) -> Option<String> {
        None
    }
    /// 现在开着窗口的程序（设置里添加指定软件时挑）。
    fn open_apps(&self) -> Vec<AppWindow> {
        Vec::new()
    }
}

struct Client {
    state: ClientState,
    /// test 算出来的结果：(键, 按下以后的状态, 应答)。
    pending: Option<(Key, ClientState, Out)>,
    revision: u64,
}

pub struct Host {
    clients: HashMap<ClientId, Client>,
    /// 会话借用它；见模块说明里的规则。
    engine: Option<Box<Engine>>,
    engine_error: Option<String>,
    store: Store,
    config: Config,
    brand_icon_revision: u64,
    /// 最后主动选择的中英文状态；全局模式和没有输入焦点时使用。
    mode: Mode,
    /// 按应用记的中英文状态（mode_scope 是 app 时用）。
    app_modes: HashMap<String, Mode>,
    /// 大写锁定开着（DLL 报的）：实际输入状态一律是英文，`mode`、`app_modes` 不动。
    caps: bool,
    /// 最近到前台的程序；指定软件的预设只在它变了的那一刻套用一次。
    foreground: String,
    /// 当前指定软件的临时状态；手动切换也只改它，离开后恢复基础状态。
    preset_mode: Option<Mode>,
    /// 预设刚切换了状态：下一次输入框得到焦点时在光标旁提示一次。
    preset_hint: bool,
    /// 工具栏只读取独立常用语；快捷短语由引擎按编码匹配。
    phrases: Vec<String>,
    recent_emoji: Vec<String>,
    /// 正在用的输入框（状态栏按它显示）。
    focused: Option<ClientId>,
    next_page_revision: u64,
    desktop: Option<Box<dyn Desktop>>,
    last_bar: Option<BarState>,
    /// 正在用的主词库文件。
    lexicon_in_use: String,
    /// 设置里换了词库，等主循环在后台重新载入。
    lexicon_change: Option<String>,
    /// 学到了新东西，还没存盘。
    dirty: bool,
    saver: crate::saver::Saver,
    stats: crate::stats::Stats,
    // 包含尚未存盘的学习变化；同步导出、轮询和远端恢复不更新此时间。
    sync_modified_ms: u64,
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
    engine.set_typo_kinds(config.typo_kinds);
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
    let index = resources.join("lexicon.ftd");
    let lexicon: std::sync::Arc<dyn wowtype_core::Lexicon> =
        if lexicon_path == resources.join("lexicon.tsv") && index.exists() {
            std::sync::Arc::new(wowtype_core::lexicon::MappedLexicon::open(&lexicon_path, &index)
                .map_err(|e| format!("默认词库索引载入失败（{}）：{e:#}", index.display()))?)
        } else {
            // Existing TSV-only installations and user-selected dictionaries remain readable.
            std::sync::Arc::new(wowtype_core::MemoryLexicon::from_tsv_path_cached(&lexicon_path)
                .map_err(|e| format!("词库载入失败（{}）：{e:#}", lexicon_path.display()))?)
        };
    let lm: std::sync::Arc<dyn wowtype_core::LanguageModel> =
        match wowtype_core::lm::load_lm(resources.join("words.lm")) {
            Ok(lm) => lm,
            Err(_) => std::sync::Arc::new(wowtype_core::BigramLm::default()),
        };
    let engine = Engine::new(lexicon, lm);
    // 模糊音和默认开启的纠错兜底都使用此索引。必须在后台载入阶段建好，
    // 否则 ready 后的首次按键仍会遍历整份词库，可能超过 TSF 的 1.5 秒等待上限。
    engine.prepare_fuzzy();
    let _ = engine.load_english(resources.join("english.txt"));
    let _ = engine.load_conversion(resources.join("s2t.tsv"));
    let store = Store::new(store_dir.to_path_buf());
    for (path, history) in [(store.user_dict_path(), false), (store.history_path(), true)] {
        if !path.exists() { continue; }
        let loaded = if history { engine.load_history(&path) } else { engine.load_user_dict(&path) };
        if let Err(error) = loaded {
            if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() != std::io::ErrorKind::InvalidData) {
                return Err(format!("学习文件读取失败：{error:#}"));
            }
            let bad = path.with_extension(format!("tsv.{}-{}.bad", std::process::id(), wowtype_core::now_ms()));
            std::fs::rename(&path, &bad).map_err(|failure| format!("损坏学习文件无法留底：{failure}"))?;
            crate::log("检测到损坏的学习文件，已保留 .bad 副本");
        }
    }
    // 用户导入的模型优先；不可用时仍可使用随安装包发布的固定模型。
    let external = store.dir().join("wetype").join("nlrgpt2_nnlm.bin");
    for model in [external, resources.join("nnlm.bin")] {
        if !model.is_file() { continue; }
        match engine.load_nnlm(&model) {
            Ok(_) => break,
            Err(error) => crate::log(&format!("整句增强模型载入失败：{error:#}")),
        }
    }
    Ok(engine)
}

fn read_pins(path: &Path) -> Result<wowtype_core::session::ManualPins, String> {
    match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).map_err(|error| format!("固定排位文件无法读取：{error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(format!("读取固定排位失败：{error}")),
    }
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
        let mut stats = crate::stats::Stats::new(store.dir().to_path_buf());
        if config.typing_stats { stats.prepare(); }
        let mut host = Self {
            clients: HashMap::new(),
            engine: None,
            engine_error: None,
            mode: config.default_mode.resolve(store.load_mode()),
            brand_icon_revision: config.brand_icon_revision(),
            config,
            store,
            app_modes: HashMap::new(),
            caps: false,
            foreground: String::new(),
            preset_mode: None,
            preset_hint: false,
            phrases: Vec::new(),
            recent_emoji,
            focused: None,
            next_page_revision: 0,
            desktop: None,
            last_bar: None,
            lexicon_in_use: String::new(),
            lexicon_change: None,
            dirty: false,
            saver: crate::saver::Saver::default(),
            stats,
            sync_modified_ms: 0,
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
            client.revision = 0;
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
        if let Err(error) = self.save_learning() { crate::log(&error); }
        Some(change)
    }

    /// 记下正在用的词库文件（状态里显示）。
    pub fn set_lexicon_in_use(&mut self, path: String) {
        self.lexicon_in_use = path;
    }

    /// 后台载入好了：装上引擎，套用设置和用户数据。
    pub fn install_engine(&mut self, result: Result<Engine, String>) {
        let result = result.and_then(|engine| {
            if let Some(current) = self.engine.as_deref() {
                // 载入期间旧引擎仍接受输入，磁盘快照可能已过期。整体接续最新学习状态，
                // 既保留刚学到的词，也避免合并快照把刚删除的词重新带回来。
                let learning = current.export_learning().map_err(|e| format!("读取当前学习记录失败：{e:#}"))?;
                engine.clear_learned();
                engine.import_learning(&learning).map_err(|e| format!("接续学习记录失败：{e:#}"))?;
                engine.inherit_pending_words(current);
            }
            Ok(engine)
        });
        self.detach();
        match result {
            Ok(mut engine) => {
                let pins = self.store.dir().join("candidate-pins.json");
                if pins.is_file() {
                    match read_pins(&pins) {
                        Ok(pins) => engine.set_manual_pins(pins),
                        Err(error) => crate::log(&error),
                    }
                }
                configure_engine(&mut engine, &self.config);
                self.engine = Some(Box::new(engine));
                self.engine_error = None;
                self.apply_user_data();
                self.prepare_prediction();
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
        self.apply_quick_phrases();
        self.apply_dictionaries();
    }

    fn apply_quick_phrases(&self) {
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

    fn apply_dictionaries(&self) {
        let Some(engine) = self.engine.as_deref() else { return; };
        let words = self
            .store
            .load_dictionaries()
            .into_iter()
            .flat_map(|(_, entries)| entries)
            .map(|e| (e.pinyin, e.word, f64::from(e.weight.max(1))))
            .collect();
        engine.lexicon().set_extra_words(words);
    }

    /// 开着上屏联想：提前建好它的词频表（只建一次），免得第一次联想时那个键卡一下。
    fn prepare_prediction(&self) {
        if let (true, Some(engine)) = (self.config.predict_next, self.engine.as_deref()) {
            engine.prepare_prediction();
        }
    }

    fn set_config(&mut self, config: Config) {
        // 保存设置、改变范围时只迁移基础状态，不能把当前软件的临时模式变成全局记忆。
        let current_mode = self.base_mode_of(self.current_app());
        let old_scope = self.config.mode_scope.clone();
        let old_lexicon = self.config.lexicon.clone();
        let quick_phrases_changed = config.quick_phrases != self.config.quick_phrases;
        if !config.typing_stats || !self.config.typing_stats { self.stats.break_activity(); }
        if config.typing_stats && !self.config.typing_stats { self.stats.prepare(); }
        self.config = config.sanitized();
        let brand_revision = self.config.brand_icon_revision();
        if brand_revision != self.brand_icon_revision {
            self.brand_icon_revision = brand_revision;
            if let Some(desktop) = &self.desktop { desktop.brand_icon_changed(); }
        }
        if !self.config.app_presets || !self.config.app_preset_rules.iter().any(|rule| rule.app == self.foreground) {
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
        if quick_phrases_changed { self.apply_quick_phrases(); }
        self.prepare_prediction();
        self.notify_bar();
    }

    fn save_config(&mut self, config: Config) -> Result<(), String> {
        let config = config.sanitized();
        if config.default_mode == DefaultMode::Global && self.config.default_mode != DefaultMode::Global {
            self.store.save_mode(self.base_mode_of(self.current_app())).map_err(|e| format!("保存中英文状态失败：{e}"))?;
        }
        self.store.save_config(&config).map_err(|e| format!("保存设置失败：{e}"))?;
        self.set_config(config);
        Ok(())
    }

    /// 改一项设置并存盘（快捷键、状态栏切换的）。
    fn change_config(&mut self, change: impl FnOnce(&mut Config)) -> Result<(), String> {
        let mut config = self.config.clone();
        change(&mut config);
        self.save_config(config)
    }

    pub fn connect(&mut self, id: ClientId, app: String) {
        let state = ClientState::new(self.engine_ref(), &self.config, app);
        self.clients.insert(id, Client { state, pending: None, revision: 0 });
    }

    pub fn disconnect(&mut self, id: ClientId) {
        if let Some(mut client) = self.clients.remove(&id) {
            client.state.clear();
            if client.state.session.take_learning_change() { self.dirty = true; self.mark_sync_change(); }
        }
        if self.focused == Some(id) {
            self.focused = None;
            self.notify_bar();
        }
    }

    /// 存用户词库和输入习惯（有新东西时）。
    pub fn save_if_dirty(&mut self) {
        if let Err(error) = self.stats.save(false) { crate::log(&error); }
        if let Err(error) = self.saver.result(false) {
            crate::log(&error);
            self.dirty = true;
        }
        if !self.dirty { return; }
        let Some(engine) = self.engine.as_deref() else { return; };
        let snapshot = engine.learning_snapshot();
        let dict = self.store.user_dict_path();
        let history = self.store.history_path();
        let job = Box::new(move || {
            std::fs::create_dir_all(dict.parent().unwrap()).map_err(|error| format!("创建用户目录失败：{error}"))?;
            snapshot.save(&dict, &history).map_err(|error| format!("{error:#}"))
        });
        match self.saver.enqueue("learning", job) {
            Ok(()) => self.dirty = false,
            Err(error) => crate::log(&error),
        }
    }

    fn save_learning(&mut self) -> Result<(), String> {
        // 等旧快照写完，避免它在用户手动保存或清空以后覆盖新数据。
        if let Err(error) = self.saver.result(true) { crate::log(&error); }
        self.dirty = true;
        let engine = self.engine.as_deref().ok_or("词库还在载入，请稍后再试")?;
        std::fs::create_dir_all(self.store.dir()).map_err(|error| format!("创建用户目录失败：{error}"))?;
        engine.learning_snapshot().save(&self.store.user_dict_path(), &self.store.history_path())
            .map_err(|error| format!("{error:#}"))?;
        self.dirty = false;
        Ok(())
    }

    fn mode_of(&self, app: &str) -> Mode {
        if app == self.foreground {
            if let Some(mode) = self.preset_mode {
                return mode;
            }
        }
        self.base_mode_of(app)
    }

    fn base_mode_of(&self, app: &str) -> Mode {
        if app.is_empty() {
            return self.mode;
        }
        if self.config.default_mode != DefaultMode::Global && self.config.mode_scope == "app" {
            self.app_modes.get(app).copied().unwrap_or(self.config.default_mode.resolve(self.mode))
        } else {
            self.mode
        }
    }

    fn set_mode(&mut self, app: &str, mode: Mode) -> Result<(), String> {
        if app == self.foreground && self.preset_mode.is_some() {
            self.preset_mode = Some(mode);
        } else {
            if self.config.default_mode == DefaultMode::Global {
                self.store.save_mode(mode).map_err(|e| format!("保存中英文状态失败：{e}"))?;
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

    fn app_of(&self, id: ClientId) -> String {
        self.clients.get(&id).map(|c| c.state.app.clone()).unwrap_or_default()
    }

    /// 实际的输入状态：大写锁定开着时一律是英文（字母、数字、标点都原样交给应用），
    /// 记住的状态不动，关掉大写锁定就回到它。按键、状态显示都用这个。
    fn effective(&self, mode: Mode) -> Mode {
        if self.caps {
            Mode::English
        } else {
            mode
        }
    }

    /// 光标旁提示的状态：大写锁定开着是「A」，否则是「中」「英」。
    fn hint_text(&self, mode: Mode) -> String {
        if self.caps {
            "A".to_string()
        } else {
            mode_hint(mode)
        }
    }

    /// 记下大写锁定的状态；变了返回 true（旧状态下缓存的按键结果随之作废）。
    fn set_caps(&mut self, on: bool) -> bool {
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
    fn caps_lock(&mut self, id: ClientId, on: bool) -> Response {
        if !self.set_caps(on) {
            // 没变（输入框得到焦点时 DLL 也会报一次）：不提示
            return self.with_client(id, |state, env| state.current(false, env));
        }
        let hint = self.config.hint_on_toggle.then(|| self.hint_text(self.mode_of(&self.app_of(id))));
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
        if self.desktop.as_ref().is_some_and(|desktop| desktop.foreground_app().as_deref() != Some(app)) {
            return;
        }
        if app == self.foreground {
            return;
        }
        let previous = self.current_mode();
        self.foreground = app.to_string();
        self.preset_mode = self.config.app_presets.then(|| {
            self.config.app_preset_rules.iter().find(|rule| rule.app == app).map(|rule| rule.mode)
        }).flatten();
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
    fn refresh_foreground(&mut self) {
        if let Some(app) = self.desktop.as_ref().and_then(|desktop| desktop.foreground_app()) {
            self.foreground(&app);
        }
    }

    fn current_app(&self) -> &str {
        if !self.foreground.is_empty() {
            &self.foreground
        } else {
            self.focused.and_then(|id| self.clients.get(&id)).map(|client| client.state.app.as_str()).unwrap_or("")
        }
    }

    /// 状态栏、托盘跟随当前前台；不能被旧输入框尚未送达的失焦通知拖回旧模式。
    fn current_mode(&self) -> Mode {
        self.mode_of(self.current_app())
    }

    fn notify_bar(&mut self) {
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
            mode: self.effective(self.current_mode()),
            chinese_punct: self.config.fullwidth_punct,
            traditional: self.config.traditional,
            data_dir: self.store.dir().display().to_string(),
            log_dir: crate::local_dir().join("logs").display().to_string(),
            lexicon: self.lexicon_in_use.clone(),
            neural_model: self.engine.as_ref().and_then(|engine| engine.model_path()).map_or_else(String::new, |path|
                if path == self.store.dir().join("wetype/nlrgpt2_nnlm.bin") { "external".into() } else { "bundled".into() }),
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

    fn style(&self, frame: &mut Frame, mode: Mode) {
        frame.mode = mode;
        frame.chinese_punct = self.config.fullwidth_punct;
        frame.traditional = self.config.traditional;
        frame.font_size = self.config.font_size;
        frame.font_family = self.config.font_family.clone();
        frame.candidate_style = self.config.candidate_style.clone();
        frame.candidate_wheel = self.config.candidate_wheel;
        frame.hide_candidate_preedit = !self.config.candidate_preedit;
        frame.preedit_size = self.config.preedit_size;
        frame.lock_candidate_position = !self.config.candidate_follow;
        frame.wrap_candidates = !self.config.candidate_single_line;
        frame.font_fallbacks = self.config.font_fallbacks.clone();
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
    fn hint_frame(&mut self, id: ClientId, hint: Option<String>) -> Response {
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

    fn toggle_punct(&mut self, id: ClientId) -> Response {
        if let Err(error) = self.change_config(|c| c.fullwidth_punct = !c.fullwidth_punct) {
            return Response::error(error);
        }
        let hint = if self.config.fullwidth_punct { "中文标点" } else { "英文标点" };
        self.hint_frame(id, Some(hint.into()))
    }

    fn toggle_traditional(&mut self, id: ClientId) -> Response {
        if let Err(error) = self.change_config(|c| c.traditional = !c.traditional) {
            return Response::error(error);
        }
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
            let mode = self.effective(self.mode_of(&self.app_of(id)));
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
        self.refresh_foreground();
        // 每个键都带着大写锁定的状态：在没有 FreeType 的窗口里开关过也能跟上
        if self.set_caps(key.modifiers & modifier::CAPS != 0) {
            self.notify_bar();
        }
        let app = self.app_of(id);
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
            self.refresh_foreground();
            // TSF 的异步编辑会话可能在应用已退到后台后才报告焦点，不能抢占当前输入框。
            if self.desktop.is_some() && !self.foreground.is_empty() && self.app_of(id) != self.foreground {
                return self.with_client(id, |state, env| state.current(false, env));
            }
        }
        if active {
            self.focused = Some(id);
        } else if self.focused == Some(id) {
            self.focused = None;
        }
        // 预设刚切换了状态：和手动切换一样提示一次
        let preset_hint = active && std::mem::take(&mut self.preset_hint) && self.config.hint_on_toggle;
        let hint_on_focus = self.config.hint_on_focus;
        let mut response = self.with_client(id, |state, env| {
            // 换了输入框：上一个输入框里的联想词不带过来
            state.dismiss_prediction();
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

    fn candidate_action(&mut self, id: ClientId, revision: u64, action: CandidateAction) -> Response {
        if revision == 0 || self.clients.get(&id).is_none_or(|client| client.revision != revision) {
            return self.with_client(id, |state, env| state.current(true, env));
        }
        match action {
            CandidateAction::Select { index } => self.with_client(id, |state, env| state.select(index, env)),
            CandidateAction::Page { next } => self.with_client(id, |state, env| state.page(next, env)),
            CandidateAction::Forget { index } => self.forget(id, index),
            CandidateAction::Pin { index, place } => self.pin(id, index, place),
        }
    }

    fn pin(&mut self, id: ClientId, index: usize, place: u8) -> Response {
        if u32::from(place) > self.config.page_size { return Response::error("固定排位不在本页范围内"); }
        let Some(client) = self.clients.get_mut(&id) else { return Response::error("输入会话已结束"); };
        if client.state.tool.is_some() || client.state.raw.is_some() || client.state.session.learning_disabled() {
            return Response::error("当前候选不能固定排位");
        }
        let Some((scope, key)) = client.state.session.pin_target(index) else { return Response::error("当前候选不能固定排位"); };
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库未加载"); };
        let path = self.store.dir().join("candidate-pins.json");
        // 不覆盖损坏记录；保留给用户排查，重新打开仍可使用输入法。
        let mut pins = match read_pins(&path) { Ok(pins) => pins, Err(error) => return Response::error(error) };
        let slots = pins.entry(scope.clone()).or_default();
        slots.retain(|_, existing| *existing != key);
        if place > 0 { slots.insert(place, key); }
        if slots.is_empty() { pins.remove(&scope); }
        let saved = serde_json::to_string(&pins).map_err(std::io::Error::other).and_then(|data| crate::store::write_atomic(&path, &data));
        if let Err(error) = saved { return Response::error(format!("保存固定排位失败：{error}")); }
        engine.set_manual_pins(pins);
        for client in self.clients.values_mut() {
            client.state.session.refresh_candidates();
            client.pending = None;
            client.revision = 0;
        }
        self.with_client(id, |state, env| state.current(true, env))
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

    fn report(result: Result<ImportReport, String>) -> Response {
        match result {
            Ok(report) => Response {
                report: Some(report),
                ..Response::ok()
            },
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

    /// 导出全部设置备份；`parts` 是要导出的类别。
    fn export_settings(&self, path: &Path, parts: &[BackupPart]) -> Response {
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库还在载入，请稍后再试"); };
        Self::report(backup::export(&self.store, &self.config, self.mode, &self.recent_emoji, engine, path, parts))
    }

    fn import_settings(&mut self, path: &Path) -> Response {
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库还在载入，请稍后再试"); };
        // 确认运行中引擎的学习状态可读，再准备文件；失败不能先覆盖其他设置。
        if let Err(error) = engine.export_learning() { return Response::error(format!("读取当前学习记录失败：{error:#}")); }
        let prepared = match backup::read(path) { Ok(data) => data, Err(error) => return Response::error(error) };
        // 只替换备份里有的类别；没有的类别（文件、引擎里的状态）保持不变。
        let config = prepared.restored_config(&self.store, &self.config);
        // 排空旧学习快照，再替换备份文件，保证重启仍读取恢复后的数据。
        if prepared.has(BackupPart::Learning) {
            if let Err(error) = self.saver.result(true) {
                self.dirty = true;
                crate::log(&error);
            }
        }
        let cleanup_note = match prepared.restore_files(&self.store, &self.config) { Ok(note) => note, Err(error) => return Response::error(error) };
        if prepared.has(BackupPart::Learning) {
            engine.clear_learned();
            if let Err(error) = engine.import_learning(&prepared.data.learning.to_string()) {
                return Response::error(format!("备份文件已恢复，但输入引擎尚未更新：{error:#}。请重新打开 FreeType"));
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
        if let Some(note) = cleanup_note { report.note = Some(format!("{}；{note}", report.note.unwrap_or_default())); }
        let common = match self.store.load_common_phrases() { Ok(common) => common, Err(error) => return Response::error(error) };
        Response {
            config: Some(self.config.clone()), status: Some(self.status()),
            quick_phrases: Some(self.store.list_quick_phrases()), common_phrases: Some(common),
            dictionaries: Some(self.store.list_dictionaries()), user_words: Some(self.user_words()), report: Some(report),
            ..Response::ok()
        }
    }

    /// 数据同步的恢复：把另一台设备（或本机以前）的快照并进当前数据，不删除本机已有的内容。
    ///
    /// 学习记录按核心的规则合并（重复合并不累加）；设置只采纳那台设备上改过的项；
    /// 中英文状态、主词库、状态栏位置是各设备自己的，不动。
    fn merge_settings(&mut self, path: &Path) -> Response {
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库还在载入，请稍后再试"); };
        // 整份先校验完再动数据
        let prepared = match backup::read(path) { Ok(data) => data, Err(error) => return Response::error(error) };
        let counts = match prepared.merge_files(&self.store) { Ok(counts) => counts, Err(error) => return Response::error(error) };
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
        let common = match self.store.load_common_phrases() { Ok(common) => common, Err(error) => return Response::error(error) };
        let report = ImportReport {
            added: counts.quick_phrases + counts.common_phrases + counts.dictionaries + learned,
            note: Some(format!(
                "新增 {} 份词库、{} 条快捷短语、{} 条常用语，合并 {} 条学习记录",
                counts.dictionaries, counts.quick_phrases, counts.common_phrases, learned
            )),
            ..ImportReport::default()
        };
        Response {
            config: Some(self.config.clone()), status: Some(self.status()),
            quick_phrases: Some(self.store.list_quick_phrases()), common_phrases: Some(common),
            dictionaries: Some(self.store.list_dictionaries()), user_words: Some(self.user_words()), report: Some(report),
            ..Response::ok()
        }
    }

    fn mark_sync_change(&mut self) {
        self.sync_modified_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    }

    fn export_sync_settings(&self, path: &Path) -> Response {
        let mut response = self.export_settings(path, &BackupPart::ALL);
        if response.ok {
            match backup::modified_unix_ms(&self.store, &self.config) {
                Ok(stamp) => response.modified_unix_ms = Some(stamp.max(self.sync_modified_ms)),
                Err(error) => return Response::error(error),
            }
        }
        response
    }

    fn import_sync_settings(&mut self, path: &Path, expected_path: &Path) -> Response {
        if self.clients.values().any(|client| client.state.busy()) {
            return Response::error("正在输入，自动同步将在稍后重试");
        }
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库还在载入，请稍后再试"); };
        let current = match backup::snapshot(&self.store, &self.config, self.mode, &self.recent_emoji, engine) {
            Ok((text, _)) => text,
            Err(error) => return Response::error(error),
        };
        let expected = match std::fs::read_to_string(expected_path) {
            Ok(text) => text,
            Err(error) => return Response::error(format!("读取同步比较快照失败：{error}")),
        };
        if current != expected { return Response::error("本机数据在下载期间已修改，将重新比较版本"); }
        // 检查和恢复在同一宿主请求中完成，期间不处理其他输入或设置修改。
        self.import_settings(path)
    }

    /// 处理一条请求；里面出了 panic 时记日志、丢掉这个连接的输入状态，不让服务进程退出。
    pub fn handle(&mut self, id: ClientId, request: Request) -> Response {
        let may_edit = matches!(&request,
            Request::SetConfig { .. } | Request::SetQuickPhrases { .. } | Request::ImportQuickPhrases { .. }
            | Request::SetCommonPhrases { .. } | Request::ImportCommonPhrases { .. }
            | Request::ImportDictionary { .. } | Request::RemoveDictionary { .. }
            | Request::ImportLearning { .. } | Request::ImportSettings { .. } | Request::MergeSettings { .. }
            | Request::ForgetWords { .. } | Request::AddUserWord { .. } | Request::ClearLearned
            | Request::ToggleMode { .. } | Request::Command { .. } | Request::Reload);
        let scan = match &request {
            Request::PhysicalKey { key, scan, .. } if !key.test => Some(*scan),
            _ => None,
        };
        let preview = matches!(&request, Request::Key { key: Key { test: true, .. }, .. } | Request::PhysicalKey { key: Key { test: true, .. }, .. });
        let actual = matches!(&request,
            Request::Key { key: Key { test: false, .. }, .. }
            | Request::PhysicalKey { key: Key { test: false, .. }, .. } | Request::Select { .. }
            | Request::CandidateAction { action: CandidateAction::Select { .. }, .. });
        if matches!(&request, Request::SensitiveFocus { active: true } | Request::Focus { active: false, .. }) {
            self.stats.break_activity();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle_inner(id, request)));
        match result {
            Ok(mut response) => {
                if !preview {
                    if let (Some(frame), Some(client)) = (&mut response.frame, self.clients.get_mut(&id)) {
                        if frame.candidates.is_empty() { client.revision = 0; }
                        else if frame.handled || client.revision == 0 {
                            self.next_page_revision = self.next_page_revision.wrapping_add(1).max(1);
                            client.revision = self.next_page_revision;
                        }
                        frame.page_revision = client.revision;
                    }
                }
                if let (Some(client), Some(frame)) = (self.clients.get(&id), &mut response.frame) {
                    frame.pair_enabled = frame.mode == Mode::Chinese && client.state.pairs_enabled(&self.config);
                }
                let learned = self.clients.get_mut(&id).is_some_and(|client| client.state.session.take_learning_change());
                self.dirty |= learned;
                if response.ok && (may_edit || learned) { self.mark_sync_change(); }
                if self.config.typing_stats && actual && self.clients.get(&id).is_some_and(|client| !client.state.session.learning_disabled()) {
                    if let Some(frame) = response.frame.as_ref().filter(|frame| frame.handled) {
                        self.stats.record(id, scan, frame.commit.as_deref(), frame.selected_rank, self.config.stats_retention);
                    }
                }
                response
            },
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

    fn change_neural_model(&mut self, path: Option<String>) -> Response {
        use std::io::Read;
        use wowtype_core::nnlm::Nnlm;
        const MAX_BYTES: u64 = 32 << 20;
        let Some(engine) = self.engine.as_deref() else { return Response::error("词库还在载入，请稍后再试"); };
        let target = self.store.dir().join("wetype/nlrgpt2_nnlm.bin");
        let result = (|| -> Result<(), String> {
            if let Some(source) = path {
                let file = std::fs::File::open(&source).map_err(|e| format!("打开模型失败：{e}"))?;
                if !file.metadata().map_err(|e| e.to_string())?.is_file() { return Err("请选择模型文件".into()); }
                let mut data = Vec::new();
                file.take(MAX_BYTES + 1).read_to_end(&mut data).map_err(|e| format!("读取模型失败：{e}"))?;
                if data.len() as u64 > MAX_BYTES { return Err("模型文件超过 32 MB".into()); }
                let model = Nnlm::from_bytes(&data).map_err(|e| format!("模型校验失败：{e:#}"))?;
                crate::store::write_atomic_bytes(&target, &data).map_err(|e| format!("保存模型失败：{e}"))?;
                engine.install_nnlm(std::sync::Arc::new(model), target);
            } else {
                let bundled = crate::resource_dir().join("nnlm.bin");
                let model = Nnlm::load(&bundled).map_err(|e| format!("随包模型无法读取：{e:#}"))?;
                match std::fs::remove_file(&target) {
                    Ok(()) => {},
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(error) => return Err(format!("移除外部模型失败：{error}")),
                }
                engine.install_nnlm(std::sync::Arc::new(model), bundled);
            }
            Ok(())
        })();
        if let Err(error) = result { return Response::error(error); }
        for client in self.clients.values_mut() {
            client.state.session.refresh_candidates();
            client.state.dismiss_prediction();
            client.revision = 0;
            client.pending = None;
        }
        Response { status: Some(self.status()), ..Response::ok() }
    }

    fn config_response(&self) -> Response {
        use wowtype_core::shuangpin::{encode, Scheme};
        let shuangpin_example = self.config.shuangpin.then(|| {
            let scheme = Scheme::from_name(&self.config.shuangpin_scheme)?;
            let keys: Vec<_> = ["ni", "hao"].iter().filter_map(|syllable| {
                let pair = *encode(scheme, syllable).first()?;
                String::from_utf8(pair.to_vec()).ok()
            }).collect();
            (keys.len() == 2).then(|| keys.join(" "))
        }).flatten();
        Response { config: Some(self.config.clone()), shuangpin_example, ..Response::ok() }
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
            Request::PhysicalKey { key, before, shown_revision, .. } => {
                let selects = matches!(key.keysym,
                    keysym::SPACE_DOUBLE | 0x20 | 0x27 | 0x3b | 0x5b | 0x5d | 0x30..=0x39 | keysym::KP_0..=keysym::KP_9
                );
                if selects && shown_revision.is_some_and(|revision| self.clients.get(&id).is_some_and(|client| {
                    client.revision != revision && (client.state.busy() || revision != 0)
                })) {
                    return self.with_client(id, |state, env| state.current(true, env));
                }
                self.key(id, key, before)
            }
            Request::CandidateAction { revision, action } => self.candidate_action(id, revision, action),
            Request::ListTypingStats => match self.stats.snapshot(self.config.stats_retention) {
                Ok(days) => Response { typing_stats: Some(days), ..Response::ok() },
                Err(error) => Response::error(error),
            },
            Request::ClearTypingStats => match self.stats.clear() {
                Ok(()) => Response::ok(),
                Err(error) => Response::error(error),
            },
            Request::SensitiveFocus { active } => {
                if active && self.focused == Some(id) { self.focused = None; }
                let response = self.with_client(id, |state, env| {
                    state.set_sensitive(active);
                    state.current(false, env)
                });
                self.notify_bar();
                response
            }
            Request::ClosePair => self.with_client(id, |state, env| state.close_pair(env)),
            Request::UndoCommitLearning => {
                let changed = self.clients.get_mut(&id).is_some_and(|client| {
                    client.pending = None;
                    client.state.dismiss_prediction();
                    client.state.session.undo_commit_learning()
                });
                if changed {
                    self.dirty = true;
                    self.mark_sync_change();
                }
                Response::ok()
            }
            Request::Select { index } => self.with_client(id, |state, env| state.select(index, env)),
            Request::Page { next } => self.with_client(id, |state, env| state.page(next, env)),
            Request::Forget { index } => self.forget(id, index),
            Request::Reset { commit } => self.with_client(id, |state, env| state.reset(commit, env)),
            Request::Focus { active, before } => self.focus(id, active, before),
            Request::ToggleMode { shift } => self.toggle_mode(id, shift),
            Request::Command { command } => self.command(id, command),
            Request::CapsLock { on } => self.caps_lock(id, on),
            Request::Status => Response {
                status: Some(self.status()),
                ..Response::ok()
            },
            Request::ListApps => Response {
                apps: Some(self.desktop.as_ref().map(|desktop| desktop.open_apps()).unwrap_or_default()),
                ..Response::ok()
            },
            Request::GetConfig => self.config_response(),
            Request::GetBrandIcon { known_revision } => Response {
                brand_icon: (known_revision != Some(self.brand_icon_revision)).then(|| BrandIcon {
                    revision: self.brand_icon_revision,
                    appearance: self.config.brand_icon.clone(),
                    custom_png: if self.config.brand_icon == "custom" { self.config.custom_brand_icon.clone() } else { String::new() },
                }),
                ..Response::ok()
            },
            Request::SetNeuralModel { path } => self.change_neural_model(path),
            Request::SetConfig { config } => match self.save_config(config) {
                Ok(()) => self.config_response(),
                Err(e) => Response::error(e),
            },
            Request::ListQuickPhrases => Response {
                quick_phrases: Some(self.store.list_quick_phrases()),
                ..Response::ok()
            },
            Request::SetQuickPhrases { phrases } => {
                if let Err(error) = self.store.set_quick_phrases(&phrases) { return Response::error(error); }
                self.apply_quick_phrases();
                Response::ok()
            }
            Request::ImportQuickPhrases { path, replace } => {
                let response = Self::report(self.store.import_quick_phrases(Path::new(&path), replace));
                if response.ok { self.apply_quick_phrases(); }
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
                if let Err(error) = self.store.set_common_phrases(phrases) { return Response::error(error); }
                self.load_phrases();
                Response::ok()
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
                if response.ok { self.apply_dictionaries(); }
                response
            }
            Request::RemoveDictionary { name } => {
                if let Err(error) = self.store.remove_dictionary(&name) { return Response::error(error); }
                self.apply_dictionaries();
                Response::ok()
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
                match crate::store::write_atomic(Path::new(&path), &msime::format_dictionary(&entries)) {
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
            Request::ExportSettings { path, parts } => {
                self.export_settings(Path::new(&path), parts.as_deref().unwrap_or(&BackupPart::ALL))
            }
            Request::ExportSyncSettings { path } => self.export_sync_settings(Path::new(&path)),
            Request::ImportSyncSettings { path, expected_path } => self.import_sync_settings(Path::new(&path), Path::new(&expected_path)),
            Request::InspectSettingsBackup { path } => Self::report(backup::read(Path::new(&path)).map(|data| data.report())),
            Request::ImportSettings { path } => self.import_settings(Path::new(&path)),
            Request::MergeSettings { path } => self.merge_settings(Path::new(&path)),
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
                if let Err(error) = self.stats.save(true) { crate::log(&error); }
                for client in self.clients.values_mut() {
                    client.state.clear();
                    self.dirty |= client.state.session.take_learning_change();
                }
                if let Err(error) = self.saver.result(true) {
                    crate::log(&error);
                    self.dirty = true;
                }
                if self.dirty && self.engine.is_some() {
                    if let Err(error) = self.save_learning() { return Response::error(error); }
                }
                Response::ok()
            }
            Request::Reload => {
                let config = self.store.load_config();
                self.set_config(config);
                self.apply_user_data();
                self.lexicon_change = Some(self.config.lexicon.clone());
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
