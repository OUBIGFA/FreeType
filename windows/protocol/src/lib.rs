//! FreeType 服务进程的命名管道消息。
//!
//! 输入法 DLL（每个应用进程里一份）和设置程序都连到同一个服务进程：管道是字节模式，
//! 每条请求、每条应答都是一行 JSON（UTF-8，`\n` 结尾）。一个连接里请求和应答一问一答。
//!
//! 管道名带当前用户的 SID：`\\.\pipe\FreeType-<SID>`，见 [`pipe_name`]。

use serde::{Deserialize, Serialize};

/// 协议版本：改了已有字段的含义时加一。服务进程在 `hello` 的应答里带上它。
pub const PROTOCOL_VERSION: u32 = 2;

/// 输入法文本服务的 COM 类 ID（注册后不能再改）。
pub const TEXT_SERVICE_CLSID: &str = "6AEA9A65-33E4-4FFA-94E3-399C2ADC4CF1";
/// 简体中文语言配置的 GUID（注册后不能再改）。
pub const PROFILE_GUID: &str = "2C829DC6-FA03-477E-85D0-C64E1C64B209";
/// 简体中文（中国）的 LANGID。
pub const LANGID_ZH_CN: u16 = 0x0804;
/// 显示在系统输入法列表里的名字。
pub const DISPLAY_NAME: &str = "FreeType";
/// 候选窗默认字体。
pub const DEFAULT_FONT: &str = "Microsoft YaHei";

/// 管道名。`sid` 是当前用户 SID 的字符串形式（S-1-5-21-...）。
pub fn pipe_name(sid: &str) -> String {
    format!(r"\\.\pipe\FreeType-{sid}")
}

/// 平台无关键码（和 `wowtype_core::session::keysym` 一致），DLL 把 Windows 虚拟键翻成它。
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
    pub const KP_MULTIPLY: u32 = 0xffaa;
    pub const KP_ADD: u32 = 0xffab;
    pub const KP_SUBTRACT: u32 = 0xffad;
    pub const KP_DECIMAL: u32 = 0xffae;
    pub const KP_DIVIDE: u32 = 0xffaf;
    pub const KP_0: u32 = 0xffb0;
    pub const KP_9: u32 = 0xffb9;
    pub const DELETE: u32 = 0xffff;
    pub const SHIFT_L: u32 = 0xffe1;
    pub const SHIFT_R: u32 = 0xffe2;
}

/// 修饰键位掩码（和 `wowtype_core::session::modifier` 一致）。
pub mod modifier {
    pub const SHIFT: u32 = 1 << 0;
    pub const CAPS: u32 = 1 << 1;
    pub const CTRL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    pub const NUMLOCK: u32 = 1 << 4;
    pub const SUPER: u32 = 1 << 6;
}

/// 翻页键（[`Config::page_keys`] 的位掩码，和核心的 `session::page_keys` 一致）。
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

/// 模糊音分组（[`Config::fuzzy_groups`] 的位掩码，和核心一致）。
pub mod fuzzy {
    pub const Z_ZH: u32 = 1;
    pub const C_CH: u32 = 2;
    pub const S_SH: u32 = 4;
    pub const N_L: u32 = 8;
    pub const F_H: u32 = 16;
    pub const R_L: u32 = 32;
    pub const AN_ANG: u32 = 64;
    pub const EN_ENG: u32 = 128;
    pub const IN_ING: u32 = 256;
    pub const IAN_IANG: u32 = 512;
    pub const UAN_UANG: u32 = 1024;
    pub const UN_ONG: u32 = 2048;
    /// 打开模糊音时默认的几组（和 WowType 一样）。
    pub const DEFAULT: u32 = Z_ZH | C_CH | S_SH | AN_ANG | EN_ENG | IN_ING | IAN_IANG | UAN_UANG;
}

/// 输入状态：中文（拼音）或英文（按键原样交给应用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Chinese,
    English,
}

/// 用户设置，存为 `%APPDATA%\FreeType\config.json`。缺的字段取默认值。
///
/// 取值是字符串的几项（快捷键、位置等）只认下面列出的写法，别的在 [`Config::sanitized`] 里换成默认值。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // —— 候选窗 ——
    /// 一页几个候选（3–10）。
    pub page_size: u32,
    /// 候选横排。
    pub horizontal: bool,
    /// 候选窗字号（像素，按 96 DPI 计）。
    pub font_size: u32,
    /// 候选窗字体（字体族名）。
    pub font_family: String,

    // —— 拼音 ——
    /// 模糊音总开关；打开时用 [`Config::fuzzy_groups`] 里的几组。
    pub fuzzy: bool,
    pub fuzzy_groups: u32,
    /// 双拼及方案：ziranma xiaohe microsoft sogou abc ziguang pinyinjiajia。
    pub shuangpin: bool,
    pub shuangpin_scheme: String,
    /// 按错键纠错。
    pub typo_correction: bool,
    /// 也出 CJK 扩展 B 区以后的生僻单字（常见字体显示不了，默认不出）。
    pub rare_chars: bool,
    /// 主词库文件（`拼音<Tab>词<Tab>词频`）的路径；空串用安装包里的雾凇词库。
    /// 文件不在时也用默认词库。
    pub lexicon: String,

    // —— 按键 ——
    /// 单按哪个 Shift 切换中英文：both left right off。
    pub shift_toggle: String,
    /// 翻页键（[`page_keys`]），PgUp PgDn 一直能用。
    pub page_keys: u32,
    /// 分号、引号选第 2、3 个候选：off semicolon quote。
    pub select_keys: String,
    /// 以词定字：`[` 上屏候选的第一个字、`]` 最后一个字。
    pub pick_char: bool,
    /// 小键盘的点直接上屏半角「.」；打字途中按小键盘的点进入网址、邮箱输入。
    pub numpad_dot: bool,

    // —— 中英文与标点 ——
    /// 中文状态下标点用中文（，。）；关掉是英文标点。
    pub fullwidth_punct: bool,
    /// 成对标点自动补全：打（ “ 《 【 时补上另一半，光标放中间。
    pub auto_pair: bool,
    /// 中英文之间自动加空格（总开关，默认开）；字母、数字两种分别控制。
    pub autospace: bool,
    pub autospace_letters: bool,
    pub autospace_digits: bool,
    /// 启动、切到新应用时的状态。
    pub default_mode: Mode,
    /// 中英文状态：global 全局统一，app 每个应用各记各的。
    pub mode_scope: String,
    /// 输出繁体。
    pub traditional: bool,

    // —— 实用功能 ——
    /// 快捷短语混入候选（编码完整对上时排第一）。
    pub quick_phrases: bool,
    /// 打字时推荐 emoji。
    pub emoji_suggest: bool,
    /// riqi、shijian 给日期时间候选。
    pub date_time: bool,
    /// V 模式计算器、数字转大写。
    pub calculator: bool,
    /// 只打 v 再按空格打开工具栏（剪贴板、常用语、符号、表情）。
    pub toolbar: bool,
    /// 记录剪贴板历史（只在内存里），工具栏里能选。
    pub clipboard_history: bool,
    /// 邮箱、网址补全：打 @ 出常用邮箱后缀，打 gmail. 出 .com 等。
    pub email_completion: bool,

    // —— 快捷键（off 表示不用） ——
    /// 表情面板：off ctrl+shift+e ctrl+alt+e。
    pub hotkey_emoji: String,
    /// 中英文标点：off ctrl+period。
    pub hotkey_punct: String,
    /// 简繁切换：off ctrl+shift+f ctrl+shift+t。
    pub hotkey_traditional: String,
    /// 切换中英文（Shift 以外再加一个）：off ctrl+space。
    pub hotkey_mode: String,

    // —— 状态提示 ——
    /// 通知区域（托盘）里的 FreeType 图标。
    pub tray_icon: bool,
    /// 桌面上的悬浮状态栏。
    pub status_bar: bool,
    /// 状态栏一会儿不用就隐藏，切换状态时再出来。
    pub status_bar_autohide: bool,
    /// 切换中英、标点、简繁时在光标旁提示。
    pub hint_on_toggle: bool,
    /// 换到另一个输入框时在光标旁提示当前中英文状态。
    pub hint_on_focus: bool,
    /// 提示在光标的哪边：top-left top top-right bottom-left bottom bottom-right。
    pub hint_position: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            page_size: 7,
            horizontal: true,
            font_size: 18,
            font_family: DEFAULT_FONT.into(),
            fuzzy: false,
            fuzzy_groups: fuzzy::DEFAULT,
            shuangpin: false,
            shuangpin_scheme: "xiaohe".into(),
            typo_correction: true,
            rare_chars: false,
            lexicon: String::new(),
            shift_toggle: "both".into(),
            page_keys: page_keys::MINUS,
            select_keys: "off".into(),
            pick_char: true,
            numpad_dot: true,
            fullwidth_punct: true,
            auto_pair: true,
            autospace: true,
            autospace_letters: true,
            autospace_digits: true,
            default_mode: Mode::Chinese,
            mode_scope: "global".into(),
            traditional: false,
            quick_phrases: true,
            emoji_suggest: true,
            date_time: true,
            calculator: true,
            toolbar: true,
            clipboard_history: false,
            email_completion: true,
            hotkey_emoji: "off".into(),
            hotkey_punct: "off".into(),
            hotkey_traditional: "off".into(),
            hotkey_mode: "off".into(),
            tray_icon: true,
            status_bar: false,
            status_bar_autohide: false,
            hint_on_toggle: true,
            hint_on_focus: false,
            hint_position: "top-left".into(),
        }
    }
}

/// 双拼方案。
pub const SHUANGPIN_SCHEMES: [&str; 7] = ["ziranma", "xiaohe", "microsoft", "sogou", "abc", "ziguang", "pinyinjiajia"];
pub const SHIFT_TOGGLES: [&str; 4] = ["both", "left", "right", "off"];
pub const SELECT_KEYS: [&str; 3] = ["off", "semicolon", "quote"];
pub const MODE_SCOPES: [&str; 2] = ["global", "app"];
pub const HINT_POSITIONS: [&str; 6] = ["top-left", "top", "top-right", "bottom-left", "bottom", "bottom-right"];
pub const HOTKEYS_EMOJI: [&str; 3] = ["off", "ctrl+shift+e", "ctrl+alt+e"];
pub const HOTKEYS_PUNCT: [&str; 2] = ["off", "ctrl+period"];
pub const HOTKEYS_TRADITIONAL: [&str; 3] = ["off", "ctrl+shift+f", "ctrl+shift+t"];
pub const HOTKEYS_MODE: [&str; 2] = ["off", "ctrl+space"];

/// 不在 `allowed` 里就换成 `fallback`。
fn pick(value: &mut String, allowed: &[&str], fallback: &str) {
    if !allowed.contains(&value.as_str()) {
        *value = fallback.into();
    }
}

impl Config {
    /// 把越界的值拉回范围，不认识的取值换成默认值。
    pub fn sanitized(mut self) -> Self {
        let d = Config::default();
        self.page_size = self.page_size.clamp(3, 10);
        self.font_size = self.font_size.clamp(12, 36);
        let family = self.font_family.trim();
        self.font_family = if family.is_empty() || family.len() > 64 {
            d.font_family.clone()
        } else {
            family.to_string()
        };
        self.lexicon = self.lexicon.trim().to_string();
        if self.lexicon.len() > 1024 {
            self.lexicon.clear();
        }
        self.fuzzy_groups &= 0xfff;
        self.page_keys &= 0xf;
        pick(&mut self.shuangpin_scheme, &SHUANGPIN_SCHEMES, &d.shuangpin_scheme);
        pick(&mut self.shift_toggle, &SHIFT_TOGGLES, &d.shift_toggle);
        pick(&mut self.select_keys, &SELECT_KEYS, &d.select_keys);
        pick(&mut self.mode_scope, &MODE_SCOPES, &d.mode_scope);
        pick(&mut self.hint_position, &HINT_POSITIONS, &d.hint_position);
        pick(&mut self.hotkey_emoji, &HOTKEYS_EMOJI, &d.hotkey_emoji);
        pick(&mut self.hotkey_punct, &HOTKEYS_PUNCT, &d.hotkey_punct);
        pick(&mut self.hotkey_traditional, &HOTKEYS_TRADITIONAL, &d.hotkey_traditional);
        pick(&mut self.hotkey_mode, &HOTKEYS_MODE, &d.hotkey_mode);
        self
    }
}

/// 一次按键。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub keysym: u32,
    pub modifiers: u32,
    /// 只问「这个键要不要」，不改变状态（TSF 的 OnTestKeyDown）。
    #[serde(default)]
    pub test: bool,
}

/// 一条快捷短语（设置程序用）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickPhrase {
    pub code: String,
    pub text: String,
    pub weight: u32,
}

/// 一份导入的词库。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dictionary {
    /// 文件名（不含 .txt），也是显示名。
    pub name: String,
    pub entries: usize,
}

/// 一个学到的词（设置程序的「学到的词」列表）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserWord {
    pub word: String,
    /// 拼音，空格分音节。
    pub pinyin: String,
    pub freq: f64,
}

/// DLL、状态栏、托盘发的命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// 中文 / 英文标点。
    TogglePunct,
    /// 简体 / 繁体。
    ToggleTraditional,
    /// 打开表情面板。
    EmojiPanel,
    /// 打开工具栏（和 v + 空格一样）。
    Toolbar,
    /// 打开设置窗口。
    OpenSettings,
    /// 显示 / 隐藏悬浮状态栏。
    ToggleStatusBar,
}

/// 请求。`op` 字段区分种类。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// 连上以后先说一声：DLL 带 `session: true`，服务进程为这个连接建输入会话；
    /// `app` 是应用的程序名（freetype.exe 这样，小写），按应用记中英文状态时用。
    Hello {
        #[serde(default)]
        session: bool,
        #[serde(default)]
        app: String,
    },
    /// 按键。`before` 是光标前的几个字（DLL 读得到时带上，空串表示光标在开头）。
    Key {
        key: Key,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
    },
    /// 鼠标点候选：当前页里的序号。
    Select { index: usize },
    /// 候选窗上的翻页按钮。
    Page { next: bool },
    /// 忘掉当前页里第 index 个候选（长按候选、右键菜单）。
    Forget { index: usize },
    /// 输入框失去焦点、应用要求结束组字：`commit` 时把编辑区原样上屏。
    Reset { commit: bool },
    /// 输入框得到 / 失去焦点。`before` 同 [`Request::Key`]。
    Focus {
        active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
    },
    /// 切换中英文。`shift` 是单按的哪个 Shift（left / right），按设置可能不切；
    /// 托盘、状态栏、任务栏按钮切时不带。
    ToggleMode {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shift: Option<String>,
    },
    Command { command: Command },
    /// 服务状态（版本、词库是否载入、模式）。
    Status,
    GetConfig,
    SetConfig { config: Config },
    ListQuickPhrases,
    /// 整份替换快捷短语。
    SetQuickPhrases { phrases: Vec<QuickPhrase> },
    /// 从水杉格式文件导入；`replace` 时清掉原有的。
    ImportQuickPhrases {
        path: String,
        #[serde(default)]
        replace: bool,
    },
    ExportQuickPhrases { path: String },
    ListDictionaries,
    /// 导入水杉格式词库文件，存为 `dicts\<name>.txt`。
    ImportDictionary { path: String, name: String },
    RemoveDictionary { name: String },
    /// 导出学到的用户词（水杉词库格式）。
    ExportUserDictionary { path: String },
    /// 学到的词（用户词库）。
    ListUserWords,
    /// 忘掉这些学到的词（用户词库和选词习惯都去掉）。
    ForgetWords { words: Vec<String> },
    /// 自己加一个词（进用户词库，马上能打出来）。拼音空着时按常用读音自动注音。
    AddUserWord {
        word: String,
        #[serde(default)]
        pinyin: String,
    },
    /// 清空学到的全部词和输入习惯。
    ClearLearned,
    /// 清空剪贴板历史。
    ClearClipboard,
    /// 重新读配置、词库和快捷短语。
    Reload,
    /// 存好用户词库后退出（安装、卸载前用）。
    Shutdown,
}

/// 候选窗的样子。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Panel {
    /// 一页候选，横排或竖排看 [`Frame::vertical`]。
    #[default]
    Normal,
    /// 展开的候选面板：[`Frame::columns`] 列，光标所在行标 1–6。
    Grid,
    /// 竖着一行一条（剪贴板、常用语）。
    List,
}

/// 一次按键之后 DLL 要做的事。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// 吃掉这个键（false：交给应用）。
    pub handled: bool,
    /// 要上屏的字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// 上屏后光标往回挪几个字（UTF-16），成对标点补全时停在中间。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub caret_back: usize,
    /// 编辑区（组字）文字；空串表示没有在组字。
    #[serde(default)]
    pub preedit: String,
    /// 编辑区光标，UTF-16 偏移。
    #[serde(default)]
    pub preedit_cursor: usize,
    /// 候选窗顶上的标题（工具栏、表情面板、邮箱补全的说明）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 这一页的候选（显示用：换行、Tab 已画成 ⏎、⇥）。
    #[serde(default)]
    pub candidates: Vec<String>,
    /// 候选前的标号；空表示 1、2、3……；某一项是空串表示不标。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// 高亮的候选（这一页里的序号）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<usize>,
    #[serde(default)]
    pub has_prev: bool,
    #[serde(default)]
    pub has_next: bool,
    #[serde(default)]
    pub panel: Panel,
    /// 展开面板一行几个。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub columns: usize,
    /// 候选能长按、右键忘掉（引擎的候选；工具栏、邮箱补全这些不能）。
    #[serde(default)]
    pub forgettable: bool,
    #[serde(default)]
    pub mode: Mode,
    /// 中文标点（状态显示用）。
    #[serde(default)]
    pub chinese_punct: bool,
    /// 输出繁体（状态显示用）。
    #[serde(default)]
    pub traditional: bool,
    /// 光标旁短暂显示的提示（切换中英文时的「中」「英」等）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// 提示在光标的哪边，见 [`Config::hint_position`]。
    #[serde(default)]
    pub hint_position: String,
    /// 候选窗字号（96 DPI 下的像素）和字体。
    #[serde(default)]
    pub font_size: u32,
    #[serde(default)]
    pub font_family: String,
    /// 候选竖排。
    #[serde(default)]
    pub vertical: bool,
    /// 要知道光标前的字（自动加空格打开了）：DLL 没在组字时每个键都带上 `before`。
    #[serde(default)]
    pub want_before: bool,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// 服务状态。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub protocol: u32,
    pub version: String,
    /// 词库已载入（载入前也能连，只是没有候选）。
    pub ready: bool,
    pub mode: Mode,
    #[serde(default)]
    pub chinese_punct: bool,
    #[serde(default)]
    pub traditional: bool,
    /// 用户数据目录。
    pub data_dir: String,
    /// 日志目录。
    #[serde(default)]
    pub log_dir: String,
    /// 正在用的主词库文件。
    #[serde(default)]
    pub lexicon: String,
    /// 载入失败的原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 导入结果。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub added: usize,
    pub skipped: usize,
    /// 最多前 20 条出错的行：「第 N 行：原因」。
    pub errors: Vec<String>,
    /// 补充说明（认出的格式、自动注音了几条）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 应答。成功时按请求种类填 `frame`、`status`、`config` 等之一。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Config>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_phrases: Option<Vec<QuickPhrase>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictionaries: Option<Vec<Dictionary>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_words: Option<Vec<UserWord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ImportReport>,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }

    pub fn with_frame(frame: Frame) -> Self {
        Self {
            frame: Some(frame),
            ..Self::ok()
        }
    }
}

/// 编码成一行（带 `\n`）。
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let mut line = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    line.push(b'\n');
    line
}

/// 解一行（可以带也可以不带结尾的 `\n`、`\r`）。
pub fn decode<'a, T: Deserialize<'a>>(line: &'a [u8]) -> Result<T, serde_json::Error> {
    let mut end = line.len();
    while end > 0 && matches!(line[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    serde_json::from_slice(&line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_uses_op_tag() {
        let request = Request::Key {
            key: Key {
                keysym: 'a' as u32,
                modifiers: 0,
                test: true,
            },
            before: None,
        };
        let line = encode(&request);
        assert!(line.ends_with(b"\n"));
        let text = String::from_utf8(line.clone()).unwrap();
        assert!(text.starts_with(r#"{"op":"key""#), "{text}");
        assert!(!text.contains("before"), "没有就不写：{text}");
        assert_eq!(decode::<Request>(&line).unwrap(), request);
    }

    #[test]
    fn missing_config_fields_take_defaults() {
        let config: Config =
            serde_json::from_str(r#"{"page_size": 50, "shuangpin_scheme": "x", "hint_position": "left"}"#).unwrap();
        let config = config.sanitized();
        assert_eq!(config.page_size, 10);
        assert_eq!(config.shuangpin_scheme, "xiaohe");
        assert_eq!(config.hint_position, "top-left");
        assert_eq!(config.font_family, DEFAULT_FONT);
        assert!(config.fullwidth_punct);
        assert_eq!(config.fuzzy_groups, fuzzy::DEFAULT);
    }

    #[test]
    fn unknown_choices_fall_back() {
        let config = Config {
            hotkey_emoji: "ctrl+x".into(),
            mode_scope: "window".into(),
            font_family: "  ".into(),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(config.hotkey_emoji, "off");
        assert_eq!(config.mode_scope, "global");
        assert_eq!(config.font_family, DEFAULT_FONT);
    }

    #[test]
    fn unit_requests_decode() {
        assert_eq!(
            decode::<Request>(br#"{"op":"toggle_mode"}"#).unwrap(),
            Request::ToggleMode { shift: None }
        );
        assert_eq!(
            decode::<Request>(b"{\"op\":\"hello\"}\r\n").unwrap(),
            Request::Hello {
                session: false,
                app: String::new()
            }
        );
        assert_eq!(
            decode::<Request>(br#"{"op":"command","command":"toggle_punct"}"#).unwrap(),
            Request::Command {
                command: Command::TogglePunct
            }
        );
    }

    #[test]
    fn pipe_name_has_sid() {
        assert_eq!(pipe_name("S-1-5-21-1"), r"\\.\pipe\FreeType-S-1-5-21-1");
    }
}
