// Modified by FreeType; compare with WowType 5656e136 for the original implementation.
//! 输入会话：一个输入框里的编辑状态和按键规则（见 docs/PORTING.md）。
//!
//! 目标是让 fcitx5 和 macOS 两个外壳只做「平台按键 → 会话 → 按结果画界面、上屏」，
//! 输入行为都在这里。现在是一块一块从 fcitx5 外壳（`fcitx5-addon/src/wowtype_engine.cpp`）
//! 挪过来的，已经挪进来的：
//!
//! - 编辑区里的原始输入、已选词栈（长句先选开头的词）；
//! - 字母和隔音符进编辑区，退格（先撤销最后一次选词，没有选词了才删字母），Esc 清空；
//! - Ctrl+Delete（还有 Ctrl+退格、Shift+Delete）忘掉光标处学错的词，候选原地重算；
//! - 候选列表（引擎的候选 + 计算器、emoji 推荐、数字按键 emoji、日期时间），
//!   编辑区的分音节显示（`nihao` 显示成 `ni hao`，双拼显示成全拼）。
//!
//! - 选词（句首词进已选栈，整句上屏）和学习（选词记录、分几次选出来的整句也记一次）；
//! - 空格、回车、数字、分号引号选词、计算器和双拼分号进编辑区，翻页、移光标和
//!   横排下键展开的候选面板（页和光标照 fcitx5 的候选列表：各管各的，光标可以不在这一页上）。
//!
//! 还没挪的键 [`Session::key`] 返回 `handled = false`，外壳接着用自己的旧逻辑处理
//! （打字途中的标点、其他可见字符、吃掉的光标键）。鼠标点候选按全局序号调
//! [`Session::choose`]，表情面板往编辑区加搜索词用 [`Session::append`]，这些过渡用法挪完以后会删掉。

use crate::datetime::LocalTime;
use crate::Engine;

mod compose;
mod keys;
mod paging;
mod sentence;
#[path = "session_trace.rs"]
mod trace;
mod commit;

/// 平台无关的键码：XKB keysym 的子集（fcitx5 直接用，macOS 外壳把 kVK_* 翻成这个）。
pub mod keysym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const KP_DELETE: u32 = 0xff9f;
    pub const DELETE: u32 = 0xffff;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    pub const KP_MULTIPLY: u32 = 0xffaa;
    pub const KP_ADD: u32 = 0xffab;
    pub const KP_SUBTRACT: u32 = 0xffad;
    pub const KP_DECIMAL: u32 = 0xffae;
    pub const KP_DIVIDE: u32 = 0xffaf;
    pub const KP_0: u32 = 0xffb0;
    pub const KP_9: u32 = 0xffb9;
    pub const SPACE: u32 = 0x20;
    pub const PERCENT: u32 = 0x25;
    pub const APOSTROPHE: u32 = 0x27;
    pub const PAREN_LEFT: u32 = 0x28;
    pub const PAREN_RIGHT: u32 = 0x29;
    pub const ASTERISK: u32 = 0x2a;
    pub const PLUS: u32 = 0x2b;
    pub const COMMA: u32 = 0x2c;
    pub const MINUS: u32 = 0x2d;
    pub const PERIOD: u32 = 0x2e;
    pub const SLASH: u32 = 0x2f;
    pub const DIGIT_0: u32 = 0x30;
    pub const DIGIT_9: u32 = 0x39;
    pub const SEMICOLON: u32 = 0x3b;
    pub const EQUAL: u32 = 0x3d;
    pub const BRACKET_LEFT: u32 = 0x5b;
    pub const BRACKET_RIGHT: u32 = 0x5d;
    pub const ASCII_CIRCUM: u32 = 0x5e;
}

/// 修饰键位掩码，和 X11/fcitx5 的 KeyState 取值一样（fcitx5 外壳可以直接传）。
pub mod modifier {
    pub const SHIFT: u32 = 1 << 0;
    pub const CAPS: u32 = 1 << 1;
    pub const CTRL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    pub const NUMLOCK: u32 = 1 << 4;
    pub const SUPER: u32 = 1 << 6;
}

/// 外壳按平台的全局热键认出来的候选动作（fcitx5 的「上一页」「下一个候选」热键，
/// 候选框上的翻页按钮）。方向键不走这个，按横排 / 竖排照会话自己的规则。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAction {
    #[default]
    None,
    PrevPage,
    NextPage,
    PrevCandidate,
    NextCandidate,
}

/// 一次按键。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub keysym: u32,
    pub modifiers: u32,
    /// 松开（按下是 false）。
    pub release: bool,
    /// 光标前一个字符，不知道是 None（以后挪标点、小数点时用）。
    pub prev_char: Option<char>,
    /// 外壳认出来的全局热键（翻页、移光标），没有是 None。
    pub action: KeyAction,
}

impl Key {
    /// 按下一个键，不带修饰键。
    pub fn press(keysym: u32) -> Self {
        Self {
            keysym,
            ..Self::default()
        }
    }

    /// 按下一个可见 ASCII 字符（大写字母带上 Shift，和 fcitx5 收到的一样）。
    pub fn char(c: char) -> Self {
        let modifiers = if c.is_ascii_uppercase() {
            modifier::SHIFT
        } else {
            0
        };
        Self {
            keysym: c as u32,
            modifiers,
            ..Self::default()
        }
    }

    /// 候选框上的翻页按钮、全局热键：不是按键，只带动作。
    pub fn action(action: KeyAction) -> Self {
        Self {
            action,
            ..Self::default()
        }
    }

    /// 忘掉光标处的候选：Ctrl+Delete、Ctrl+退格（Mac 笔记本没有向前删除键）、Shift+Delete。
    /// 带 Alt、Super 的不算。
    fn is_forget(&self) -> bool {
        let m = self.modifiers & !(modifier::CAPS | modifier::NUMLOCK);
        let delete = matches!(self.keysym, keysym::DELETE | keysym::KP_DELETE);
        match m {
            modifier::CTRL => delete || self.keysym == keysym::BACKSPACE,
            modifier::SHIFT => delete,
            _ => false,
        }
    }

    fn has_command_modifier(&self) -> bool {
        self.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0
    }

    /// 数字键（主键盘和小键盘）的值；带了 CapsLock、NumLock 以外的修饰键不算（和 fcitx5 一样）。
    fn digit(&self) -> Option<usize> {
        if self.modifiers & !(modifier::CAPS | modifier::NUMLOCK) != 0 {
            return None;
        }
        match self.keysym {
            k @ keysym::DIGIT_0..=keysym::DIGIT_9 => Some((k - keysym::DIGIT_0) as usize),
            k @ keysym::KP_0..=keysym::KP_9 => Some((k - keysym::KP_0) as usize),
            _ => None,
        }
    }
}

/// V 模式计算器认的字符（数字、运算符、括号，小键盘也算；不看修饰键）。
fn calc_char(sym: u32) -> Option<char> {
    Some(match sym {
        keysym::DIGIT_0..=keysym::DIGIT_9 => char::from(b'0' + (sym - keysym::DIGIT_0) as u8),
        keysym::KP_0..=keysym::KP_9 => char::from(b'0' + (sym - keysym::KP_0) as u8),
        keysym::PLUS | keysym::KP_ADD => '+',
        keysym::MINUS | keysym::KP_SUBTRACT => '-',
        keysym::ASTERISK | keysym::KP_MULTIPLY => '*',
        keysym::SLASH | keysym::KP_DIVIDE => '/',
        keysym::PERIOD | keysym::KP_DECIMAL => '.',
        keysym::PERCENT => '%',
        keysym::ASCII_CIRCUM => '^',
        keysym::PAREN_LEFT => '(',
        keysym::PAREN_RIGHT => ')',
        _ => return None,
    })
}

/// 已经选好、还没上屏的一个词（长句先选开头的词）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub text: String,
    /// 词库里的写法：开着繁体时 text 是转换过的字，拼成词去学时用这个。空的当和 text 一样。
    pub plain: String,
    pub pinyin: String,
    /// 用掉的原始输入字节数。
    pub bytes: usize,
}

/// 一次调用之后外壳要做的事。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// 会话处理了这个键。false：会话还不管这个键（外壳接着处理，挪完以后就是交给应用）。
    pub handled: bool,
    /// 要上屏的字。
    pub commit: Option<String>,
    /// 提交的候选在全部列表中的序号，从 1 起。
    pub selected_rank: usize,
    /// 编辑区：已选的词 + 还没选的输入（分好音节，双拼显示成全拼）。
    pub preedit: String,
    /// 编辑区光标，UTF-8 字节偏移。
    pub preedit_cursor: usize,
    /// 候选框是普通的一页还是展开的面板。
    pub panel: Panel,
    /// 候选框里显示的是全部候选里从 cand_start 起的 cand_count 个（一页，或展开面板的一屏）。
    pub cand_start: usize,
    pub cand_count: usize,
    /// 光标在显示的这些里的位置；不在这一页上是 None（翻页按钮翻走以后会这样）。
    pub cand_cursor: Option<usize>,
    pub has_prev: bool,
    pub has_next: bool,
    /// 这次调用忘掉了一个学过的词（Ctrl+Delete）：外壳该马上存用户词库和输入习惯。
    pub forgot: bool,
}

/// 候选框的样子。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    /// 一页候选（横排或竖排）。
    #[default]
    Normal,
    /// 横排时按下键展开的面板：[`GRID_COLS`] 列，一屏 [`GRID_ROWS`] 行，光标所在行标 1–6。
    Expanded,
}

/// 展开面板一行几个候选。
pub const GRID_COLS: usize = 6;
/// 展开面板一屏几行。
pub const GRID_ROWS: usize = 6;
const GRID_SCREEN: usize = GRID_COLS * GRID_ROWS;

/// 设置里能选的翻页键（[`Config::page_keys`] 的位掩码）。
pub mod page_keys {
    /// - =
    pub const MINUS: u32 = 1 << 0;
    /// [ ]
    pub const BRACKET: u32 = 1 << 1;
    /// , .
    pub const COMMA: u32 = 1 << 2;
    /// Shift+Tab Tab
    pub const TAB: u32 = 1 << 3;
}

/// 分号、引号选词。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectKeys {
    #[default]
    Off,
    /// 分号选第 2 个。
    Semicolon,
    /// 分号选第 2 个、引号选第 3 个。
    SemicolonQuote,
}

/// 打字途中方向键做什么（#129）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArrowKeys {
    /// 都用来挑候选，跟着候选排列：横排左右键选词、上键上一页、下键展开面板；竖排上下键选词、左右键翻页。
    #[default]
    Candidates,
    /// 上下键选词（到了一页的两头接着翻页），左右键在还没选词的拼音里移动光标：
    /// 字母插在光标处，退格删光标前一个，Home、End、Delete 也管光标。方向键不翻页。
    Caret,
}

/// 候选框的状态（过渡：fcitx5 外壳按它摆自己的候选列表）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// 普通候选框在第几页。
    pub page: usize,
    /// 普通候选框的光标（全部候选里的序号，可以不在这一页上）。
    pub cursor: usize,
    pub expanded: bool,
    /// 展开面板的光标（全部候选里的序号）。
    pub grid_cursor: usize,
}

/// 会话的设置（外壳从自己的配置里抄过来）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// 独立整句编辑区与对应快捷键；关闭时保留普通候选操作。
    pub sentence_editor: bool,
    /// 一页几个候选（数字按键 emoji 只在第一页里找数字字）。
    pub page_size: usize,
    /// V 模式计算器（v 后面打算式）。双拼时用大写 V（小写 v 是声母键）。
    pub calculator: bool,
    /// 打 riqi、shijian 这类时给日期时间候选。
    pub date_time: bool,
    /// 打字时推荐 emoji（开心 → 😊，八 → 8️⃣）。
    pub emoji_suggest: bool,
    /// 候选横排；false 竖排。方向键怎么用见 [`ArrowKeys`]。
    pub horizontal: bool,
    /// 翻页键，[`page_keys`] 的位掩码（PgUp PgDn 一直能用）。
    pub page_keys: u32,
    pub select_keys: SelectKeys,
    /// 以词定字：打拼音途中 `[` 上屏光标处候选的第一个字、`]` 上屏最后一个字（#83）。
    /// `[ ]` 设成翻页键时让给翻页。
    pub pick_char: bool,
    /// 双拼时编辑区显示打的键（uivs），不翻成全拼（shi'zhong）；中英混打时看得出打了什么（#122）。
    pub shuangpin_keys: bool,
    /// 打字途中方向键做什么（#129）。
    pub arrow_keys: ArrowKeys,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sentence_editor: false,
            page_size: 9,
            calculator: true,
            date_time: true,
            emoji_suggest: true,
            horizontal: true,
            page_keys: page_keys::MINUS,
            select_keys: SelectKeys::Off,
            pick_char: true,
            shuangpin_keys: false,
            arrow_keys: ArrowKeys::Candidates,
        }
    }
}

/// 候选的种类：选中以后怎么上屏、学不学。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CandidateKind {
    /// 引擎的候选：选中要学习。
    #[default]
    Word,
    /// 计算结果、日期时间、带换行的自定义词：原样上屏，不学习。
    Plain,
    /// emoji：上屏 [`Candidate::commit`]，记进最近用过的 emoji，不学习。
    Emoji,
}

/// 会话给出的一个候选。
pub type ManualPins = std::collections::BTreeMap<String, std::collections::BTreeMap<u8, String>>;

impl Engine {
    /// 拷贝一份手动排位供 Windows 宿主原子保存。
    pub fn manual_pins(&self) -> ManualPins {
        self.manual_pins.read().unwrap().clone()
    }
    pub fn set_manual_pins(&self, pins: ManualPins) {
        *self.manual_pins.write().unwrap() = pins;
    }

    fn pin_scope(&self, input: &str) -> String {
        let scheme = if self.config().shuangpin {
            self.config().shuangpin_scheme.name()
        } else {
            "quanpin"
        };
        format!("{scheme}:{}", input.to_ascii_lowercase())
    }

    fn apply_pins(&self, input: &str, candidates: &mut Vec<Candidate>) {
        let pins = self.manual_pins.read().unwrap();
        if pins.is_empty() {
            return;
        }
        let Some(slots) = pins.get(&self.pin_scope(input)) else {
            return;
        };
        let mut fixed = Vec::with_capacity(slots.len());
        for (&position, key) in slots {
            if let Some(index) = candidates
                .iter()
                .position(|item| item.pin_key.as_ref() == Some(key))
            {
                let mut item = candidates.remove(index);
                item.pin_position = position;
                fixed.push(item);
            }
        }
        for item in fixed {
            let index = usize::from(item.pin_position.saturating_sub(1)).min(candidates.len());
            candidates.insert(index, item);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidate {
    /// 候选框里显示的字。
    pub text: String,
    /// 拼音（空格分音节）；计算结果、emoji 这类为空。
    pub pinyin: String,
    /// 用掉了还没选词那部分输入的多少字节；比它短是句首词（选了以后剩下的接着选）。
    pub consumed: usize,
    pub kind: CandidateKind,
    /// 纠错标志只供外壳显示，不参与上屏。
    pub corrected: bool,
    /// 长词补全来源；上屏与学习使用完整词和完整读音，不作为显示标记。
    pub long_completion: bool,
    pub pin_key: Option<String>,
    pub pin_position: u8,
    /// 选中时上屏的字：emoji 推荐在句尾对上时是「整句 + emoji」，其余和 text 一样。
    pub commit: String,
    /// emoji 本身（推荐候选显示的是「开心😊」，这里是「😊」）；不是 emoji 为空。
    pub emoji: String,
    /// Decoder word boundaries are retained for local sentence editing.
    pub words: Vec<(String, String)>,
}

impl Candidate {
    fn plain(text: String, consumed: usize) -> Self {
        Self {
            commit: text.clone(),
            text,
            consumed,
            kind: CandidateKind::Plain,
            ..Self::default()
        }
    }
}

/// 读本地时间的函数（测试里换成固定时间）。
pub type Clock = fn() -> Option<LocalTime>;

/// 选了开头的词以后，拿已上屏的字 + 已选的词当上文，最多这么多字节。
const CONTEXT_BYTES: usize = 24;
/// emoji 推荐插在第几个（首选和次选后面）。
const EMOJI_SLOT: usize = 2;

/// 一个输入框的会话。只能在一个线程里用。
///
/// 引擎是后台载入的，载入好之前 `engine` 是 None（照样能打字，只是没有候选）。
#[derive(Clone)]
pub struct Session<'e> {
    enhancement_key: Option<String>,
    sentence: crate::sentence::State,
    trace: trace::Trace,
    /// 全部原始输入（包括已经选过词的部分）。
    input: String,
    selections: Vec<Selection>,
    selection_learning: Vec<Option<(u64, u64)>>,
    committed_learning: Vec<(u64, u64)>,
    no_learn: bool,
    learning_changed: bool,
    /// 编辑区光标在 input 里的字节位置；None 是在末尾（没移动过，或移回了末尾）。
    /// 只在还没选词的那段里，读的时候用 [`Session::caret`] 夹进去。
    caret: Option<usize>,
    engine: Option<&'e Engine>,
    config: Config,
    /// 光标前已经上屏的字（外壳记的）。
    context: String,
    clock: Clock,
    /// 候选缓存：输入、已选词、引擎、上文、设置变了就作废，用的时候再算。
    candidates: Vec<Candidate>,
    fresh: bool,
    /// 每重算一次候选加一（C 接口按它判断要不要重建字符串）。
    generation: u64,
    /// 候选框：页、光标（全局序号）、展开面板和它的光标。候选重算时回到开头。
    page: usize,
    cursor: usize,
    expanded: bool,
    grid_cursor: usize,
    /// 这次调用要上屏的字（frame 取走）。
    pending: Option<String>,
    /// 这次调用上屏的字是选哪个候选来的；原样上屏输入时是 None。
    last_choice: Option<Candidate>,
    pending_rank: usize,
    /// 这次调用忘掉了学过的词（frame 取走）。
    forgot: bool,
    /// Windows 预判副本：Some(false) 尚未遇到写操作，Some(true) 必须在实际按键时重算。
    /// 克隆会话仍共享引擎，不能让预判提前学习或删除记录。
    preview: Option<bool>,
}

impl Default for Session<'_> {
    fn default() -> Self {
        Self {
            sentence: crate::sentence::State::default(),
            enhancement_key: None,
            trace: trace::Trace::default(),
            input: String::new(),
            selections: Vec::new(),
            selection_learning: Vec::new(),
            committed_learning: Vec::new(),
            no_learn: false,
            learning_changed: false,
            caret: None,
            engine: None,
            config: Config::default(),
            context: String::new(),
            clock: system_time,
            candidates: Vec::new(),
            fresh: false,
            generation: 0,
            page: 0,
            cursor: 0,
            expanded: false,
            grid_cursor: 0,
            pending: None,
            last_choice: None,
            pending_rank: 0,
            forgot: false,
            preview: None,
        }
    }
}

impl std::fmt::Debug for Session<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("input", &self.input)
            .field("selections", &self.selections)
            .field("has_engine", &self.engine.is_some())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<'e> Session<'e> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 非破坏性按键预判。只复制编辑状态，用户学习数据仍由真正接受按键的会话修改。
    pub fn preview(&self) -> Self {
        let mut preview = self.clone();
        preview.preview = Some(false);
        preview
    }

    /// 没有触及学习数据的预判可以直接接纳，省去重复算候选；否则调用方要对原会话重放按键。
    pub fn accept_preview(&mut self) -> bool {
        let accepted = self.preview.take() == Some(false);
        if accepted {
            self.queue_enhancement();
        }
        accepted
    }

    fn defer_learning(&mut self) -> bool {
        if let Some(changed) = &mut self.preview {
            *changed = true;
            true
        } else {
            false
        }
    }

    /// 上屏联想（FreeType）：光标前的字后面最可能接的词，见 [`crate::predict`]。没有引擎时是空的。
    pub fn predictions(&self, limit: usize) -> Vec<crate::predict::Prediction> {
        self.engine.map_or_else(Vec::new, |engine| {
            engine.predict_after(&self.context, limit)
        })
    }

    /// 用户选了联想词 `word`（词库里的写法）：记下它和上一个词的搭配。
    /// 在联想词上屏、上文更新之前调。预判副本里不记（和选词一样留给真正的按键）。
    pub fn learn_prediction(&mut self, word: &str) {
        let Some(engine) = self.engine else {
            return;
        };
        if self.no_learn || self.defer_learning() {
            return;
        }
        self.committed_learning.clear();
        let mark = engine.learn_mark();
        engine.set_context(utf8_tail(&self.context, CONTEXT_BYTES));
        engine.record_prediction(word);
        let end = engine.learn_mark();
        if end > mark {
            self.committed_learning.push((mark, end));
            self.learning_changed = true;
        }
    }

    /// 独立英文候选共用学习日志和 TSF 预判规则；存完整词/短语编码，不存截短前缀。
    pub fn learn_external_word(&mut self, word: &str, code: &str) {
        let Some(engine) = self.engine else {
            return;
        };
        if self.no_learn || self.defer_learning() {
            return;
        }
        self.committed_learning.clear();
        let mark = engine.learn_mark();
        engine.record_selection_with_pinyin(word, Some(code));
        let end = engine.learn_mark();
        if end > mark {
            self.committed_learning.push((mark, end));
            self.learning_changed = true;
        }
    }

    /// 带引擎建会话。
    pub fn with_engine(engine: &'e Engine) -> Self {
        Self {
            engine: Some(engine),
            ..Self::default()
        }
    }

    /// 换引擎（外壳的词库在后台载入好了）。
    pub fn set_engine(&mut self, engine: Option<&'e Engine>) {
        let same = match (self.engine, engine) {
            (Some(a), Some(b)) => std::ptr::eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.sentence = crate::sentence::State::default();
            self.enhancement_key = None;
            // A mark from the old engine cannot be interpreted by its replacement.
            self.selection_learning.fill(None);
            self.committed_learning.clear();
            self.engine = engine;
            self.fresh = false;
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn configure(&mut self, config: Config) {
        if self.config.sentence_editor && !config.sentence_editor {
            // 关闭区域后不能留下不可见的词块焦点或编辑约束。
            self.sentence = crate::sentence::State::default();
            self.caret = None;
        }
        if self.config != config {
            if config.arrow_keys == ArrowKeys::Candidates {
                self.caret = None; // 方向键换回选词：拼音里的光标回到末尾
            }
            self.config = config;
            self.fresh = false;
        }
    }

    /// 光标前已经上屏的字（选了开头的词以后，剩下的部分拿它和已选的词当上文排序）。
    pub fn set_context(&mut self, text: &str) {
        if self.context != text {
            self.context.clear();
            self.context.push_str(text);
            self.fresh = false;
        }
    }

    /// 换掉读本地时间的函数（测试用）。
    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
        self.fresh = false;
    }

    /// 外壳标记敏感输入时不向引擎记录选词、联想或原样上屏。
    pub fn set_no_learn(&mut self, value: bool) {
        if value && !self.no_learn {
            self.no_learn = true;
            self.trace_sensitive();
            self.clear();
        }
        self.no_learn = value;
    }

    pub fn learning_disabled(&self) -> bool {
        self.no_learn
    }

    pub fn has_commit_learning(&self) -> bool {
        !self.committed_learning.is_empty()
    }

    /// 只有外壳确认刚上屏的文字已被紧接的退格修改时才调用。
    pub fn undo_commit_learning(&mut self) -> bool {
        self.trace_commit_backspace();
        let marks = std::mem::take(&mut self.committed_learning);
        for (start, end) in marks.into_iter().rev() {
            self.undo_range(start, end);
        }
        self.take_learning_change()
    }

    pub fn take_learning_change(&mut self) -> bool {
        std::mem::take(&mut self.learning_changed)
    }

    fn undo_range(&mut self, start: u64, end: u64) {
        if !self.defer_learning()
            && self
                .engine
                .is_some_and(|engine| engine.undo_learning(start, end))
        {
            self.learning_changed = true;
        }
    }

    fn pop_selection(&mut self) -> Option<Selection> {
        if let Some(Some((start, end))) = self.selection_learning.pop() {
            self.undo_range(start, end);
        }
        let selection = self.selections.pop();
        if selection.is_some() {
            self.trace_undo_selection();
        }
        selection
    }

    fn push_input(&mut self, c: char) {
        let at = self.caret();
        self.sentence.edit(at, 0, c.len_utf8());
        if at < self.input.len() {
            // 光标在拼音中间（#129）：插在光标处，光标跟着往后
            self.input.insert(at, c);
            self.caret = Some(at + c.len_utf8());
        } else {
            self.input.push(c);
        }
        self.fresh = false;
    }

    /// 清空编辑区。commit_pending：把已选的词和还没选的原始输入上屏（切换输入法时），
    /// 和回车一样也记一次选词。
    pub fn reset(&mut self, commit_pending: bool) -> Frame {
        self.trace_via("reset");
        self.last_choice = None;
        if commit_pending && self.composing() {
            self.commit_input_now();
        } else {
            self.clear();
        }
        self.frame(true)
    }

    /// 现在的编辑区和候选框（不处理按键）。这次调用要上屏的字也在里面。
    pub fn frame(&mut self, handled: bool) -> Frame {
        let n = self.candidates().len();
        let selected = self.selected_text();
        let (shown, shown_caret) = self.segmented_remaining();
        let preedit_cursor = selected.len() + shown_caret;
        let preedit = selected + &shown;
        let mut frame = Frame {
            handled,
            commit: self.pending.take(),
            selected_rank: std::mem::take(&mut self.pending_rank),
            forgot: std::mem::take(&mut self.forgot),
            preedit_cursor,
            preedit,
            ..Frame::default()
        };
        if n == 0 {
            return frame;
        }
        if self.expanded {
            let cursor = self.grid_cursor.min(n - 1);
            let start = cursor / GRID_SCREEN * GRID_SCREEN;
            frame.panel = Panel::Expanded;
            frame.cand_start = start;
            frame.cand_count = (n - start).min(GRID_SCREEN);
            frame.cand_cursor = Some(cursor - start);
            frame.has_prev = start > 0;
            frame.has_next = start + GRID_SCREEN < n;
        } else {
            let (start, len) = self.page_range(n);
            frame.cand_start = start;
            frame.cand_count = len;
            frame.cand_cursor = self.cursor_on_page();
            frame.has_prev = self.has_prev_page();
            frame.has_next = self.has_next_page(n);
        }
        frame
    }

    /// 候选框的状态（过渡：fcitx5 外壳按它摆自己的候选列表）。
    pub fn view(&mut self) -> View {
        self.candidates();
        View {
            page: self.page,
            cursor: self.cursor,
            expanded: self.expanded,
            grid_cursor: self.grid_cursor,
        }
    }

    /// 候选框里显示的候选（一页，或展开面板的一屏）。
    pub fn shown(&mut self) -> &[Candidate] {
        let frame = self.frame(false);
        // frame 会取走要上屏的字；这里只是看，放回去
        self.pending = frame.commit;
        self.forgot = frame.forgot;
        &self.candidates[frame.cand_start..frame.cand_start + frame.cand_count]
    }

    /// 所选项必须是完整输入所匹配的词，日期、联想、纠错和句首截取不进入固定排位。
    pub fn pin_target(&mut self, index: usize) -> Option<(String, String)> {
        let scope = self.engine?.pin_scope(self.remaining());
        let key = self.shown().get(index)?.pin_key.clone()?;
        Some((scope, key))
    }

    pub fn refresh_candidates(&mut self) {
        self.fresh = false;
    }
    /// 上一次调用上屏的字是选哪个候选来的；原样上屏输入、没上屏是 None。
    pub fn last_choice(&self) -> Option<&Candidate> {
        self.last_choice.as_ref()
    }

    fn clear(&mut self) {
        self.enhancement_key = None;
        self.sentence = crate::sentence::State::default();
        self.trace_abandon();
        self.committed_learning.clear();
        let picks = std::mem::take(&mut self.selection_learning);
        for (start, end) in picks.into_iter().rev().flatten() {
            self.undo_range(start, end);
        }
        self.input.clear();
        self.selections.clear();
        self.caret = None;
        self.fresh = false;
    }

    /// 还没选词的那段在 input 里从哪个字节开始。
    fn rest_start(&self) -> usize {
        self.selected_bytes().min(self.input.len())
    }

    /// 编辑区光标在 input 里的字节位置：在还没选词的那段里，没移动过时在末尾。
    fn caret(&self) -> usize {
        self.caret.map_or(self.input.len(), |at| {
            at.clamp(self.rest_start(), self.input.len())
        })
    }

    /// 候选重算过几次（C 接口用）。
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// 正在输入（编辑区不空）。
    pub fn composing(&self) -> bool {
        !self.input.is_empty()
    }

    /// 全部原始输入。
    pub fn input(&self) -> &str {
        &self.input
    }

    /// 已选的词用掉的字节数。
    pub fn selected_bytes(&self) -> usize {
        self.selections.iter().map(|s| s.bytes).sum()
    }

    /// 还没选词的那部分输入。
    pub fn remaining(&self) -> &str {
        self.input.get(self.selected_bytes()..).unwrap_or("")
    }

    /// 已选的词连起来。
    /// 已选词保留汉字，未选的部分显示实际按键和对应的光标位置。
    pub fn raw_preedit(&self) -> (String, usize) {
        let selected = self.selected_text();
        let caret = selected.len() + self.caret() - self.rest_start();
        (selected + self.remaining(), caret)
    }

    pub fn selected_text(&self) -> String {
        self.selections.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }
}

/// 只留尾部 max_bytes 字节，起点对齐到字符边界。
fn utf8_tail(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// 本地时间（日期时间候选用）。
#[cfg(unix)]
pub fn system_time() -> Option<LocalTime> {
    let mut now: libc::time_t = 0;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: time 和 localtime_r 只写我们给的两个变量
    unsafe {
        libc::time(&mut now);
        if libc::localtime_r(&now, &mut tm).is_null() {
            return None;
        }
    }
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1).max(0) as u32,
        day: tm.tm_mday.max(0) as u32,
        hour: tm.tm_hour.max(0) as u32,
        minute: tm.tm_min.max(0) as u32,
        second: tm.tm_sec.max(0) as u32,
        weekday: tm.tm_wday.max(0) as u32,
    })
}

#[cfg(windows)]
pub fn system_time() -> Option<LocalTime> {
    let mut now: libc::time_t = 0;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: time 和 localtime_s 只写我们给的两个变量
    unsafe {
        libc::time(&mut now);
        if libc::localtime_s(&mut tm, &now) != 0 {
            return None;
        }
    }
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1).max(0) as u32,
        day: tm.tm_mday.max(0) as u32,
        hour: tm.tm_hour.max(0) as u32,
        minute: tm.tm_min.max(0) as u32,
        second: tm.tm_sec.max(0) as u32,
        weekday: tm.tm_wday.max(0) as u32,
    })
}

#[cfg(not(any(unix, windows)))]
pub fn system_time() -> Option<LocalTime> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &str) -> Session<'static> {
        let mut session = Session::new();
        for c in keys.chars() {
            session.key(&Key::char(c));
        }
        session
    }

    #[test]
    fn letters_and_apostrophe() {
        let session = typed("xi'an");
        assert_eq!(session.input(), "xi'an");
        // 开头的隔音符不进编辑区
        let mut session = Session::new();
        assert!(!session.key(&Key::char('\'')).handled);
        assert!(!session.composing());
    }

    #[test]
    fn command_modifiers_and_release_are_not_ours() {
        let mut session = typed("ni");
        let ctrl_a = Key {
            keysym: u32::from(b'a'),
            modifiers: modifier::CTRL,
            ..Key::default()
        };
        assert!(!session.key(&ctrl_a).handled);
        let release = Key {
            release: true,
            ..Key::press(keysym::BACKSPACE)
        };
        assert!(!session.key(&release).handled);
        assert_eq!(session.input(), "ni");
    }

    #[test]
    fn keys_outside_composition_are_not_ours() {
        let mut session = Session::new();
        assert!(!session.key(&Key::press(keysym::BACKSPACE)).handled);
        assert!(!session.key(&Key::press(keysym::ESCAPE)).handled);
    }

    #[test]
    fn reset_commits_selected_and_raw() {
        let mut session = typed("nihaozhongguo");
        session.push_selection(Selection {
            text: "你好".into(),
            pinyin: "ni hao".into(),
            bytes: 5,
            ..Default::default()
        });
        let frame = session.reset(true);
        assert_eq!(frame.commit.as_deref(), Some("你好zhongguo"));
        assert!(frame.preedit.is_empty());
        assert!(!session.composing());
        assert_eq!(session.reset(true).commit, None);
    }

    #[test]
    fn preedit_cursor_is_byte_offset() {
        let mut session = typed("nihaoma");
        session.push_selection(Selection {
            text: "你好".into(),
            pinyin: "ni hao".into(),
            bytes: 5,
            ..Default::default()
        });
        let frame = session.frame(true);
        assert_eq!(frame.preedit, "你好ma");
        assert_eq!(frame.preedit_cursor, "你好ma".len());
    }
}
