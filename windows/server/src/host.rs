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
use std::sync::Arc;

use freetype_protocol::{
    keysym, modifier, AppWindow, BackupPart, BrandIcon, CandidateAction, Command, Config,
    DefaultMode, Frame, ImportReport, Key, Mode, Request, Response, Status, UserWord,
    PROTOCOL_VERSION,
};
use wowtype_core::msime;
use wowtype_core::Engine;

use crate::backup;
use crate::dictfmt::{self, LexiconReading};
use crate::input::{session_config, ClientState, Env, Out};
use crate::store::Store;
use crate::tasks::{Job, Step};
use crate::tools::Page;

pub type ClientId = u64;

/// 整句候选个数上限（和 Linux 版一样）。
const MAX_CANDIDATES: usize = 1000;
/// 最近用过的 emoji 记多少个。
const RECENT_EMOJI: usize = 32;
/// 自己加的词的词频（学到的词里算高的，排在前面）。
const ADDED_WORD_FREQ: f64 = 50_000.0;
/// 词库还在载入时编辑区上方的说明；载好后下一个键就有候选。
pub(crate) const LOADING_TITLE: &str = "词库载入中；稍候即可选词";

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
    pub brand_icon: String,
    pub brand_revision: u64,
    pub custom_brand_icon: std::sync::Arc<str>,
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
    enhancement: Arc<crate::enhancement::Worker>,
    clients: HashMap<ClientId, Client>,
    /// 会话借用它；见模块说明里的规则。
    engine: Option<Box<Engine>>,
    engine_error: Option<String>,
    lexicon_warning: Option<String>,
    lexicon_loading: bool,
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
    diagnostics: crate::candidate_diagnostics::Diagnostics,
    // 包含尚未存盘的学习变化；同步导出、轮询和远端恢复不更新此时间。
    sync_modified_ms: u64,
    /// 最近一次实际按键或选词的时间。
    last_input: Option<std::time::Instant>,
    /// 同步计算、同步记录登记和旧引擎释放的后台线程。
    tasks: crate::tasks::Tasks,
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
    engine.set_long_word_completion(config.long_word_completion);
    engine.set_traditional(config.traditional);
    engine.set_rare_ext_chars(config.rare_chars);
}

pub struct LoadedEngine {
    pub engine: Engine,
    pub path: PathBuf,
    warning: Option<String>,
    invalid_custom_format: bool,
}

impl Host {
    pub fn new(store: Store) -> Self {
        let enhancement = crate::enhancement::Worker::new(store.dir().to_path_buf());
        if std::fs::read_to_string(store.dir().join("enhancement/enabled.txt"))
            .is_ok_and(|s| s.trim() == "1")
        {
            enhancement.enable(true);
        }
        let diagnostic_dir = if store.dir() == crate::data_dir() {
            crate::local_dir()
        } else {
            store.dir().join("local")
        };
        let config = store.load_config();
        let recent_emoji = store.load_recent_emoji();
        let mut stats = crate::stats::Stats::new(store.dir().to_path_buf());
        if config.typing_stats {
            stats.prepare();
        }
        let mut host = Self {
            enhancement,
            clients: HashMap::new(),
            engine: None,
            engine_error: None,
            lexicon_warning: None,
            lexicon_loading: true,
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
            diagnostics: crate::candidate_diagnostics::Diagnostics::new(
                diagnostic_dir.join("candidate-diagnostics.jsonl"),
            ),
            sync_modified_ms: 0,
            last_input: None,
            tasks: Default::default(),
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
        self.lexicon_loading = true;
        self.lexicon_warning = None;
        self.engine_error = None;
        self.dirty = true;
        if let Err(error) = self.save_learning() {
            crate::log(&error);
        }
        Some(change)
    }

    pub fn finish_loading(&mut self, result: Result<LoadedEngine, String>) {
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => return self.install_engine(Err(error)),
        };
        self.install_engine(Ok(loaded.engine));
        if self.engine_error.is_some() {
            return;
        }
        self.lexicon_in_use = loaded.path.display().to_string();
        self.lexicon_warning = loaded.warning;
        if loaded.invalid_custom_format {
            self.config.lexicon.clear();
            if let Err(error) = self.store.save_config(&self.config) {
                if let Some(warning) = &mut self.lexicon_warning {
                    warning.push_str(&format!("；无法保存默认词库设置：{error}"));
                }
            }
        }
        if let Some(warning) = &self.lexicon_warning {
            crate::log(warning);
        }
    }

    /// 后台载入好了：装上引擎，套用设置和用户数据。
    pub fn install_engine(&mut self, result: Result<Engine, String>) {
        self.lexicon_loading = false;
        let result = result.and_then(|engine| {
            if let Some(current) = self.engine.as_deref() {
                // 载入期间旧引擎仍接受输入，磁盘快照可能已过期。整体接续最新学习状态，
                // 既保留刚学到的词，也避免合并快照把刚删除的词重新带回来。
                let learning = current
                    .export_learning()
                    .map_err(|e| format!("读取当前学习记录失败：{e:#}"))?;
                engine.clear_learned();
                engine
                    .import_learning(&learning)
                    .map_err(|e| format!("接续学习记录失败：{e:#}"))?;
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
                if self.enhancement.status().enabled {
                    engine.set_enhancement(Some(self.enhancement.clone()));
                }
                if let Some(previous) = self.engine.replace(Box::new(engine)) {
                    // detach 已移除全部会话借用。大词库及模型析构可能耗时数秒，
                    // 交给现有低优先级任务线程，输入线程只交换已就绪的引擎。
                    self.discard_engine(previous);
                }
                self.engine_error = None;
                self.apply_user_data();
                self.prepare_prediction();
            }
            Err(error) => self.engine_error = Some(error),
        }
        self.attach();
    }

    /// 在后台释放已脱离会话的旧引擎，或被更新一次载入取代的过期结果。
    /// 所有权交给任务线程，确保词库及模型的最后一次释放不会阻塞按键。
    pub fn discard_engine(&self, engine: Box<Engine>) {
        self.tasks.submit(
            Box::new(move || {
                drop(engine);
                Step::Done
            }),
            None,
        );
    }

    pub fn connect(&mut self, id: ClientId, app: String) {
        let state = ClientState::new(self.engine_ref(), &self.config, app);
        self.clients.insert(
            id,
            Client {
                state,
                pending: None,
                revision: 0,
            },
        );
    }

    pub fn disconnect(&mut self, id: ClientId) {
        if let Some(mut client) = self.clients.remove(&id) {
            client.state.clear();
            if self.config.candidate_diagnostics {
                self.diagnostics
                    .record(client.state.session.take_trace(), &client.state.app, id);
            }
            if client.state.session.take_learning_change() {
                self.dirty = true;
                self.mark_sync_change();
            }
        }
        if self.focused == Some(id) {
            self.focused = None;
            self.notify_bar();
        }
    }

    /// 存用户词库和输入习惯（有新东西时）。
    pub fn save_if_dirty(&mut self) {
        if let Err(error) = self.stats.save(false) {
            crate::log(&error);
        }
        if let Err(error) = self.saver.result(false) {
            crate::log(&error);
            self.dirty = true;
        }
        if !self.dirty {
            return;
        }
        let Some(engine) = self.engine.as_deref() else {
            return;
        };
        let snapshot = engine.learning_snapshot();
        let dict = self.store.user_dict_path();
        let history = self.store.history_path();
        let job = Box::new(move || {
            std::fs::create_dir_all(dict.parent().unwrap())
                .map_err(|error| format!("创建用户目录失败：{error}"))?;
            snapshot
                .save(&dict, &history)
                .map_err(|error| format!("{error:#}"))
        });
        match self.saver.enqueue("learning", job) {
            Ok(()) => self.dirty = false,
            Err(error) => crate::log(&error),
        }
    }

    fn save_learning(&mut self) -> Result<(), String> {
        // 等旧快照写完，避免它在用户手动保存或清空以后覆盖新数据。
        if let Err(error) = self.saver.result(true) {
            crate::log(&error);
        }
        self.dirty = true;
        let engine = self.engine.as_deref().ok_or("词库还在载入，请稍后再试")?;
        std::fs::create_dir_all(self.store.dir())
            .map_err(|error| format!("创建用户目录失败：{error}"))?;
        engine
            .learning_snapshot()
            .save(&self.store.user_dict_path(), &self.store.history_path())
            .map_err(|error| format!("{error:#}"))?;
        self.dirty = false;
        Ok(())
    }

    fn status(&self) -> Status {
        Status {
            protocol: PROTOCOL_VERSION,
            version: option_env!("FREETYPE_BUILD_VERSION")
                .unwrap_or(env!("CARGO_PKG_VERSION"))
                .to_string(),
            ready: self.engine.is_some(),
            mode: self.effective(self.current_mode()),
            chinese_punct: self.config.fullwidth_punct,
            traditional: self.config.traditional,
            data_dir: self.store.dir().display().to_string(),
            log_dir: crate::local_dir().join("logs").display().to_string(),
            lexicon: self.lexicon_in_use.clone(),
            neural_model: self
                .engine
                .as_ref()
                .and_then(|engine| engine.model_path())
                .map_or_else(String::new, |path| {
                    if path == self.store.dir().join("wetype/nlrgpt2_nnlm.bin") {
                        "external".into()
                    } else {
                        "bundled".into()
                    }
                }),
            error: self.engine_error.clone(),
            lexicon_warning: self.lexicon_warning.clone(),
            lexicon_loading: self.lexicon_loading,
            input_idle_seconds: self.last_input.map(|at| at.elapsed().as_secs()),
        }
    }
}

#[cfg(test)]
mod tests;

mod loading;
pub use loading::{load_engine, load_engine_from};
mod input_requests;
mod modes;
mod settings_requests;
use settings_requests::*;
mod data_requests;
mod requests;
mod sync_requests;
