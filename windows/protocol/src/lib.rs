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
/// 语言栏监视窗口和只读刷新消息；消息不携带图像或配置，接收方从本用户服务读取。
pub const TSF_BRAND_MONITOR_CLASS: &str = "FreeTypeBrandIconMonitorV1";
pub const BRAND_ICON_CHANGED_MESSAGE: &str = "FreeType.BrandIconChanged.v1";

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
    pub const F1: u32 = 0xffbe;
    /// Windows 宿主确认的双击空格；键码需与服务进程保持一致。
    pub const SPACE_DOUBLE: u32 = 0x11_0001;
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
    pub const AN_AI: u32 = 4096;
    pub const HUANG_WANG: u32 = 8192;
    pub const UN_IONG: u32 = 16384;
    pub const ENG_ONG: u32 = 32768;
    /// 打开模糊音时默认的几组（和 WowType 一样）。
    pub const DEFAULT: u32 = Z_ZH | C_CH | S_SH | AN_ANG | EN_ENG | IN_ING | IAN_IANG | UAN_UANG;
}

/// 数字后仍保持全角的标点（[`Config::digit_punct_fullwidth`] 的位掩码）。
pub mod digit_punct {
    /// 顿号 `\` -> `、`
    pub const DUNHAO: u32 = 1 << 0;
    /// 冒号 `:` -> `：`
    pub const COLON: u32 = 1 << 1;
    /// 逗号 `,` -> `，`
    pub const COMMA: u32 = 1 << 2;
    /// 句号 `.` -> `。`
    pub const PERIOD: u32 = 1 << 3;
    /// 分号 `;` -> `；`
    pub const SEMICOLON: u32 = 1 << 4;
    /// 问号 `?` -> `？`
    pub const QUESTION: u32 = 1 << 5;
    /// 感叹号 `!` -> `！`
    pub const EXCLAMATION: u32 = 1 << 6;
    /// 双引号 `"` -> `“ ”`（或直角引号 `「 」`）
    pub const DOUBLE_QUOTE: u32 = 1 << 7;
    /// 单引号 `'` -> `‘ ’`
    pub const SINGLE_QUOTE: u32 = 1 << 8;
    /// 圆括号 `( )` -> `（ ）`
    pub const PAREN: u32 = 1 << 9;
    /// 方括号 `[ ]` -> `【 】`
    pub const BRACKET: u32 = 1 << 10;
    /// 书名号 `< >` -> `《 》`
    pub const BOOK_TITLE: u32 = 1 << 11;
    /// 直角引号 `{ }` -> `「 」`
    pub const CORNER_QUOTE: u32 = 1 << 12;
    /// 破折号 `_` -> `——`
    pub const DASH: u32 = 1 << 13;
    /// 省略号 `^` -> `……`
    pub const ELLIPSIS: u32 = 1 << 14;
    /// 波浪号 `~` -> `～`
    pub const TILDE: u32 = 1 << 15;
    /// 间隔号 `` ` `` -> `·`
    pub const MIDDLE_DOT: u32 = 1 << 16;
    /// 人民币 `$` -> `￥`
    pub const YUAN: u32 = 1 << 17;

    /// 全部有效位掩码。
    pub const ALL: u32 = (1 << 18) - 1;
    /// 默认勾选：顿号「、」。
    pub const DEFAULT: u32 = DUNHAO;

    /// ASCII 标点对应的位掩码（不在列表里的返回 0）。
    pub fn bit_of(ascii: char) -> u32 {
        match ascii {
            '\\' => DUNHAO,
            ':' => COLON,
            ',' => COMMA,
            '.' => PERIOD,
            ';' => SEMICOLON,
            '?' => QUESTION,
            '!' => EXCLAMATION,
            '"' => DOUBLE_QUOTE,
            '\'' => SINGLE_QUOTE,
            '(' | ')' => PAREN,
            '[' | ']' => BRACKET,
            '<' | '>' => BOOK_TITLE,
            '{' | '}' => CORNER_QUOTE,
            '_' => DASH,
            '^' => ELLIPSIS,
            '~' => TILDE,
            '`' => MIDDLE_DOT,
            '$' => YUAN,
            _ => 0,
        }
    }
}

/// 输入状态：中文（拼音）或英文（按键原样交给应用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Chinese,
    English,
}

/// 默认状态和正在输入的状态分开：Global 是记忆策略，不能作为中/英文状态发给 DLL。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultMode {
    #[default]
    Chinese,
    English,
    Global,
}

impl DefaultMode {
    /// 把启动策略解析为可交给输入会话的中文或英文状态。
    pub fn resolve(self, remembered: Mode) -> Mode {
        match self {
            Self::Chinese => Mode::Chinese,
            Self::English => Mode::English,
            Self::Global => remembered,
        }
    }
}

/// 指定软件的预设输入状态：焦点从别的程序切进这个程序时切换一次。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPreset {
    /// 程序文件名，小写带扩展名（code.exe）。
    pub app: String,
    pub mode: Mode,
}

/// 一个开着窗口的程序（设置里添加 [`AppPreset`] 时从中挑）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppWindow {
    /// 程序文件名，小写带扩展名。
    pub app: String,
    /// 它的一个窗口标题（帮用户认出是哪个软件）。
    pub title: String,
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
    /// 选中候选的样子：pink 品牌粉、blue 蓝、green 微信绿、sogou 不画底色块只换字色。
    pub candidate_style: String,
    /// 运行中窗口和托盘的品牌图标：default、white、custom；不改安装和注册资源。
    pub brand_icon: String,
    /// 自定义图像转为 256 px PNG 后以 Base64 保存，让备份和同步不依赖本机路径。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_brand_icon: String,
    /// 鼠标悬停候选窗时允许滚轮翻页。
    pub candidate_wheel: bool,
    /// 行内预编辑：segmented、raw、off；候选窗预编辑独立控制。
    pub inline_preedit: String,
    pub candidate_preedit: bool,
    /// 0 按候选字号自动缩放。
    pub preedit_size: u32,
    pub candidate_follow: bool,
    pub candidate_single_line: bool,
    /// 逗号分隔的字体族名，按顺序供 DirectWrite 补字。
    pub font_fallbacks: String,

    // —— 拼音 ——
    /// 模糊音总开关；打开时用 [`Config::fuzzy_groups`] 里的几组。
    pub fuzzy: bool,
    pub fuzzy_groups: u32,
    /// 双拼及方案：ziranma xiaohe microsoft sogou abc ziguang pinyinjiajia。
    pub shuangpin: bool,
    pub shuangpin_scheme: String,
    /// 双拼时编辑区直接显示按下的键（如 `nihc`），不展开成全拼；默认展开。
    pub shuangpin_show_keys: bool,
    /// 按错键纠错。
    pub typo_correction: bool,
    /// 乱序、邻键、漏字、多字。旧设置缺字段时保持全部开启。
    pub typo_kinds: u8,
    pub typo_mark: bool,
    /// 固定排位的显示角标：off、paperclip、pushpin、dot。
    pub pin_badge: String,
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
    /// 左右方向键用途：`candidates` 移候选；`caret` 移拼音里的光标（Home、End、Delete 同管光标，
    /// 上下键改为选词，横排也不再用下键展开面板）。
    pub arrow_keys: String,
    /// 小键盘的点直接上屏半角「.」；打字途中按小键盘的点进入网址、邮箱输入。
    pub numpad_dot: bool,

    // —— 中英文与标点 ——
    /// 中文状态下标点用中文（，。）；关掉是英文标点。
    pub fullwidth_punct: bool,
    /// 中文标点下，数字后面的标点保持半角（如 3.14、3,000）。
    pub digit_halfwidth_punct: bool,
    /// 数字标点保持半角时，允许勾选的例外标点仍输出全角。
    pub digit_punct_exceptions: bool,
    /// 数字后面仍保持全角的例外标点位掩码（[`digit_punct`]），默认勾选顿号「、」。
    pub digit_punct_fullwidth: u32,
    /// 中文标点下，双引号改用直角引号「」替代默认弯引号“ ”。
    pub corner_quotes: bool,
    /// 成对标点自动补全：打（ “ 《 【 时补上另一半，光标放中间。
    pub auto_pair: bool,
    /// 中英文之间自动加空格（总开关，默认开）；字母、数字两种分别控制。
    pub autospace: bool,
    pub autospace_letters: bool,
    pub autospace_digits: bool,
    /// 启动、切到新应用时的状态；global 沿用最后主动切换的状态，所有应用共用并存盘。
    pub default_mode: DefaultMode,
    /// 中英文状态：global 全局统一，app 每个应用各记各的；default_mode 为 global 时忽略此项。
    pub mode_scope: String,
    /// 输出繁体。
    pub traditional: bool,
    /// 指定软件自动切换中英文（总开关）：只在焦点从别的程序切进规则里的程序时切换一次，
    /// 软件内部可手动切换；离开后恢复基础状态，不覆盖普通程序或全局记忆。
    pub app_presets: bool,
    /// 各程序切进来时的状态，见 [`Config::app_presets`]。
    pub app_preset_rules: Vec<AppPreset>,

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
    /// 上屏联想：一个词上屏以后，候选窗接着给出常跟在它后面的词（默认关）。
    pub predict_next: bool,
    /// 上屏联想时数字键选联想词（默认关，数字照常输入）。
    pub predict_digits: bool,
    /// 上屏联想时允许用空格选当前高亮的联想词（默认开）。
    pub predict_space: bool,
    /// 默认开；仅在允许空格选词时生效，单次空格照常交给应用。
    pub predict_double_space: bool,
    /// 上屏联想时左右方向键移动候选高亮（默认开，独立于组字时的方向键设置）。
    pub predict_arrows: bool,
    /// 只在本机统计每日聚合数字，默认关闭。
    pub typing_stats: bool,
    /// 0 表示永久；其余是保留天数。
    pub stats_retention: u32,

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
    pub bar_scale: u32,
    pub bar_icon_size: u32,
    pub bar_hide_seconds: u32,
    pub bar_shadow: bool,
    /// 标点、简繁、表情、设置，位 0–3；中英切换始终显示。
    pub bar_parts: u8,
    /// 切换中英、标点、简繁时在光标旁提示。
    pub hint_on_toggle: bool,
    /// 换到另一个输入框时在光标旁提示当前中英文状态。
    pub hint_on_focus: bool,
    /// 提示在光标的哪边：top-left top top-right bottom-left bottom bottom-right。
    pub hint_position: String,
    /// 开启后由托盘程序后台下载更新，下一次启动时交给完整安装器；默认关闭。
    pub auto_update: bool,
    /// 安装或更新完成后自动打开设置页；默认关闭，主动打开设置不受影响。
    pub show_settings_after_update: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            page_size: 7,
            horizontal: true,
            font_size: 18,
            font_family: DEFAULT_FONT.into(),
            candidate_style: "pink".into(),
            brand_icon: "default".into(),
            custom_brand_icon: String::new(),
            candidate_wheel: false,
            inline_preedit: "segmented".into(),
            candidate_preedit: true,
            preedit_size: 0,
            candidate_follow: true,
            candidate_single_line: true,
            font_fallbacks: String::new(),
            fuzzy: false,
            fuzzy_groups: fuzzy::DEFAULT,
            shuangpin: false,
            shuangpin_scheme: "xiaohe".into(),
            shuangpin_show_keys: false,
            typo_correction: true,
            typo_kinds: 0xf,
            typo_mark: false,
            pin_badge: "paperclip".into(),
            rare_chars: false,
            lexicon: String::new(),
            shift_toggle: "both".into(),
            page_keys: page_keys::MINUS,
            select_keys: "off".into(),
            pick_char: true,
            arrow_keys: "candidates".into(),
            numpad_dot: true,
            fullwidth_punct: true,
            digit_halfwidth_punct: true,
            digit_punct_exceptions: true,
            digit_punct_fullwidth: digit_punct::DEFAULT,
            corner_quotes: false,
            auto_pair: true,
            autospace: true,
            autospace_letters: true,
            autospace_digits: true,
            default_mode: DefaultMode::Chinese,
            mode_scope: "global".into(),
            traditional: false,
            app_presets: false,
            app_preset_rules: Vec::new(),
            quick_phrases: true,
            emoji_suggest: true,
            date_time: true,
            calculator: true,
            toolbar: true,
            clipboard_history: false,
            email_completion: true,
            predict_next: false,
            predict_digits: false,
            predict_space: true,
            predict_double_space: true,
            predict_arrows: true,
            typing_stats: false,
            stats_retention: 365,
            hotkey_emoji: "off".into(),
            hotkey_punct: "off".into(),
            hotkey_traditional: "off".into(),
            hotkey_mode: "off".into(),
            tray_icon: true,
            status_bar: false,
            status_bar_autohide: false,
            bar_scale: 100,
            bar_icon_size: 15,
            bar_hide_seconds: 4,
            bar_shadow: true,
            bar_parts: 0b1011,
            hint_on_toggle: true,
            hint_on_focus: false,
            hint_position: "top-left".into(),
            auto_update: false,
            show_settings_after_update: false,
        }
    }
}

/// 双拼方案。
pub const SHUANGPIN_SCHEMES: [&str; 7] = ["ziranma", "xiaohe", "microsoft", "sogou", "abc", "ziguang", "pinyinjiajia"];
pub const CANDIDATE_STYLES: [&str; 4] = ["pink", "blue", "green", "sogou"];
pub const SHIFT_TOGGLES: [&str; 4] = ["both", "left", "right", "off"];
pub const SELECT_KEYS: [&str; 3] = ["off", "semicolon", "quote"];
pub const ARROW_KEYS: [&str; 2] = ["candidates", "caret"];
pub const MODE_SCOPES: [&str; 2] = ["global", "app"];
pub const HINT_POSITIONS: [&str; 6] = ["top-left", "top", "top-right", "bottom-left", "bottom", "bottom-right"];
pub const HOTKEYS_EMOJI: [&str; 3] = ["off", "ctrl+shift+e", "ctrl+alt+e"];
pub const HOTKEYS_PUNCT: [&str; 2] = ["off", "ctrl+period"];
pub const HOTKEYS_TRADITIONAL: [&str; 3] = ["off", "ctrl+shift+f", "ctrl+shift+t"];
pub const HOTKEYS_MODE: [&str; 2] = ["off", "ctrl+space"];
/// 指定软件的规则最多几条（设置里手工维护的列表，不需要更多）。
pub const MAX_APP_PRESETS: usize = 200;

/// 规则里的程序名统一成小写文件名：去掉路径，没写扩展名的补上 `.exe`。写不成文件名的返回 None。
pub fn normalize_app_name(name: &str) -> Option<String> {
    let name = name.trim().trim_matches('"');
    let name = name.rsplit(['\\', '/']).next().unwrap_or(name).trim().to_lowercase();
    if name.is_empty() || name.len() > 260 || name.chars().any(|c| c.is_control() || r#"<>:"|?*"#.contains(c)) {
        return None;
    }
    Some(if name.ends_with(".exe") { name } else { name + ".exe" })
}

/// 不在 `allowed` 里就换成 `fallback`。
fn pick(value: &mut String, allowed: &[&str], fallback: &str) {
    if !allowed.contains(&value.as_str()) {
        *value = fallback.into();
    }
}

impl Config {
    /// 只用来判断图像是否变化，不是签名。未选中的自定义图像不影响当前图标缓存。
    pub fn brand_icon_revision(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.brand_icon.hash(&mut hash);
        if self.brand_icon == "custom" { self.custom_brand_icon.hash(&mut hash); }
        hash.finish()
    }

    /// 把越界的值拉回范围，不认识的取值换成默认值。
    pub fn sanitized(mut self) -> Self {
        let d = Config::default();
        self.page_size = self.page_size.clamp(3, 10);
        self.font_size = self.font_size.clamp(12, 36);
        if self.preedit_size != 0 { self.preedit_size = self.preedit_size.clamp(10, 36); }
        pick(&mut self.inline_preedit, &["segmented", "raw", "off"], &d.inline_preedit);
        self.font_fallbacks = self.font_fallbacks.split([',', '，']).map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 64 && !name.chars().any(char::is_control))
            .take(6).collect::<Vec<_>>().join(",");
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
        self.fuzzy_groups &= 0xffff;
        self.bar_scale = self.bar_scale.clamp(75, 150);
        self.bar_icon_size = self.bar_icon_size.clamp(12, 23);
        self.bar_hide_seconds = self.bar_hide_seconds.clamp(1, 60);
        self.bar_parts &= 0xf;
        self.typo_kinds &= 0xf;
        pick(&mut self.pin_badge, &["off", "paperclip", "pushpin", "dot"], &d.pin_badge);
        if ![0, 30, 90, 180, 365].contains(&self.stats_retention) { self.stats_retention = d.stats_retention; }
        self.page_keys &= 0xf;
        self.digit_punct_fullwidth &= digit_punct::ALL;
        pick(&mut self.candidate_style, &CANDIDATE_STYLES, &d.candidate_style);
        pick(&mut self.brand_icon, &["default", "white", "custom"], &d.brand_icon);
        // 显示端只导入 256 px PNG；限制配置体积，保留未选中的自定义图标供再次选择。
        if self.custom_brand_icon.len() > 512 * 1024 { self.custom_brand_icon.clear(); }
        if self.brand_icon == "custom" && self.custom_brand_icon.is_empty() { self.brand_icon = d.brand_icon.clone(); }
        pick(&mut self.shuangpin_scheme, &SHUANGPIN_SCHEMES, &d.shuangpin_scheme);
        pick(&mut self.shift_toggle, &SHIFT_TOGGLES, &d.shift_toggle);
        pick(&mut self.select_keys, &SELECT_KEYS, &d.select_keys);
        pick(&mut self.arrow_keys, &ARROW_KEYS, &d.arrow_keys);
        pick(&mut self.mode_scope, &MODE_SCOPES, &d.mode_scope);
        pick(&mut self.hint_position, &HINT_POSITIONS, &d.hint_position);
        pick(&mut self.hotkey_emoji, &HOTKEYS_EMOJI, &d.hotkey_emoji);
        pick(&mut self.hotkey_punct, &HOTKEYS_PUNCT, &d.hotkey_punct);
        pick(&mut self.hotkey_traditional, &HOTKEYS_TRADITIONAL, &d.hotkey_traditional);
        pick(&mut self.hotkey_mode, &HOTKEYS_MODE, &d.hotkey_mode);
        // 同一个程序只留最先的一条：匹配时不分大小写，重复的规则没有意义
        let mut rules: Vec<AppPreset> = Vec::new();
        for rule in std::mem::take(&mut self.app_preset_rules) {
            let Some(app) = normalize_app_name(&rule.app) else {
                continue;
            };
            if rules.len() < MAX_APP_PRESETS && rules.iter().all(|r| r.app != app) {
                rules.push(AppPreset { app, mode: rule.mode });
            }
        }
        self.app_preset_rules = rules;
        self
    }

    /// 数字后面的这个 ASCII 标点要不要按中文全角标点输出。
    pub fn allows_fullwidth_after_digit(&self, ascii: char) -> bool {
        if !self.digit_halfwidth_punct {
            return true;
        }
        if !self.digit_punct_exceptions {
            return false;
        }
        let bit = digit_punct::bit_of(ascii);
        bit != 0 && (self.digit_punct_fullwidth & bit) != 0
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
    /// 设置列表中的上屏格式；word 仍是学习、删除与备份使用的原始标识。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display: String,
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

/// 全部设置备份里的一类数据。导出时可以只选其中几类；导入时只替换备份里有的类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupPart {
    /// 设置页里的各个选项、全局的中英文状态、悬浮状态栏的位置。
    Settings,
    /// 导入的词库和自定义主词库。
    Dictionaries,
    QuickPhrases,
    CommonPhrases,
    /// 学到的词、输入习惯、最近用过的表情。
    Learning,
}

impl BackupPart {
    /// 全部类别，按给用户看的顺序。
    pub const ALL: [BackupPart; 5] = [
        BackupPart::Settings,
        BackupPart::Dictionaries,
        BackupPart::QuickPhrases,
        BackupPart::CommonPhrases,
        BackupPart::Learning,
    ];
}

/// 一天的统计只有数字，键位是 Windows 扫描码，不含按键顺序。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypingDay {
    pub characters: [u64; 5], // 汉字、拉丁字母、数字、标点、其他
    pub active_ms: u64,
    pub hours: [u64; 24],
    pub keys: std::collections::BTreeMap<u16, u64>,
    pub candidates: [u64; 11], // 第 1–10 位、第 11 位以后
}

/// 由已完成绘制的候选页发出的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CandidateAction {
    Select { index: usize },
    Page { next: bool },
    Forget { index: usize },
    Pin { index: usize, place: u8 }, // 0 = 取消固定
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
    /// 实际按键的扫描码只用来累加按键统计（预判与非 TSF 客户端仍可发 Key）。
    PhysicalKey {
        key: Key,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        scan: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shown_revision: Option<u64>,
    },
    CandidateAction { revision: u64, action: CandidateAction },
    ListTypingStats,
    ClearTypingStats,
    /// Windows 宿主识别敏感输入框时，不保留编辑区、上下文和联想。
    SensitiveFocus { active: bool },
    /// TSF 已核对紧接的退格确实删除了刚上屏的字符。
    UndoCommitLearning,
    /// TSF 已验证右标点存在，先把当前输入候选上屏，不再追加一个右符号。
    ClosePair,
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
    /// 大写锁定开了 / 关了（DLL 在按下 Caps Lock 和输入框得到焦点时报）。
    /// 开着时一律按英文输入，关掉后回到原来的中英文状态。
    CapsLock { on: bool },
    /// 服务状态（版本、词库是否载入、模式）。
    Status,
    /// 现在开着窗口的程序（设置里添加指定软件时挑）。
    ListApps,
    GetConfig,
    /// 所有输入宿主可读取已公开显示的图像；不暴露整份设置，不建立输入会话。
    GetBrandIcon {
        #[serde(default)]
        known_revision: Option<u64>,
    },
    /// 仅设置程序可用；None 移除外部模型并回退随包版本。
    SetNeuralModel { path: Option<String> },
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
    /// 工具栏里的独立常用语，不使用拼音编码。
    ListCommonPhrases,
    SetCommonPhrases { phrases: Vec<String> },
    ImportCommonPhrases {
        path: String,
        #[serde(default)]
        replace: bool,
    },
    ExportCommonPhrases { path: String },
    ListDictionaries,
    /// 导入水杉格式词库文件，存为 `dicts\<name>.txt`。
    ImportDictionary { path: String, name: String },
    RemoveDictionary { name: String },
    /// 导出学到的用户词（水杉词库格式）。
    ExportUserDictionary { path: String },
    /// 完整学习备份（JSON）：词条、次数、时间和搭配。
    ExportLearning { path: String },
    /// 合并完整学习备份，不清空现有记录，也不重复累加次数。
    ImportLearning { path: String },
    /// 全部设置和用户数据的版本化备份；校验预览不改动当前数据。
    /// `parts` 是要导出的类别，不写就是全部。
    ExportSettings {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parts: Option<Vec<BackupPart>>,
    },
    /// 自动同步的完整快照及数据修改时间；导出本身不算修改。
    ExportSyncSettings { path: String },
    /// 仅当当前数据仍与 expected_path 一致且没有组字时整份恢复。
    ImportSyncSettings { path: String, expected_path: String },
    InspectSettingsBackup { path: String },
    ImportSettings { path: String },
    /// 把一份全部设置备份合并进当前数据（数据同步的恢复）：不删除、不替换本机已有的内容。
    MergeSettings { path: String },
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

/// TSF 自身验证并执行的文档动作，不携带实际正文。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairEdit { Skip, Erase }

/// 一次按键之后 DLL 要做的事。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// 吃掉这个键（false：交给应用）。
    pub handled: bool,
    /// 要上屏的字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// 刚提交的内容有可撤销的学习记录；空按键预判不发布此标识。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub undo_learning: bool,
    /// 本次上屏的候选全局序号，从 1 起；0 为不是候选提交。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub selected_rank: usize,
    /// 上屏后光标往回挪几个字（UTF-16），成对标点补全时停在中间。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub caret_back: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pair_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair_edit: Option<PairEdit>,
    /// 编辑区（组字）文字；空串表示没有在组字。
    #[serde(default)]
    pub preedit: String,
    /// 编辑区光标，UTF-16 偏移。
    #[serde(default)]
    pub preedit_cursor: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_preedit: Option<String>,
    #[serde(default)]
    pub inline_cursor: usize,
    /// 布尔反义保持旧协议缺省值仍显示并跟随。
    #[serde(default)]
    pub hide_candidate_preedit: bool,
    #[serde(default)]
    pub preedit_size: u32,
    #[serde(default)]
    pub lock_candidate_position: bool,
    #[serde(default)]
    pub wrap_candidates: bool,
    #[serde(default)]
    pub font_fallbacks: String,
    /// 候选窗顶上的标题（工具栏、表情面板、邮箱补全的说明）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 这一页的候选（显示用：换行、Tab 已画成 ⏎、⇥）。
    #[serde(default)]
    pub candidates: Vec<String>,
    /// 与候选一一对应的纯显示角标，不改变选择或上屏的文本。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pin_slots: Vec<Option<u8>>,
    #[serde(default)]
    pub pin_limit: u8,
    #[serde(default)]
    pub page_revision: u64,
    /// 候选前的标号；空表示 1–9、0（第 10 个）；某一项是空串表示不标。
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
    /// 这些候选是上屏联想：没有在组字，不认的键照常交给应用，一会儿不选就收起来。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub predict: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub predict_double_space: bool,
    /// 双击选词时替换刚由应用插入且经 TSF 核对过的一个半角空格。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace_space: bool,
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
    /// 候选强调色，见 [`Config::candidate_style`]。
    #[serde(default)]
    pub candidate_style: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub candidate_wheel: bool,
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
    #[serde(default)]
    pub neural_model: String,
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

/// 语言栏图像。内置外观由 DLL 的完整源图生成原生尺寸；自定义内容沿用导入时的 PNG。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrandIcon {
    pub revision: u64,
    pub appearance: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub custom_png: String,
}

/// 应答。成功时按请求种类填 `frame`、`status`、`config` 等之一。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Config>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand_icon: Option<BrandIcon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuangpin_example: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_phrases: Option<Vec<QuickPhrase>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub common_phrases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictionaries: Option<Vec<Dictionary>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_words: Option<Vec<UserWord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ImportReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<Vec<AppWindow>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typing_stats: Option<std::collections::BTreeMap<String, TypingDay>>,
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
        assert!(config.digit_halfwidth_punct);
        assert!(config.digit_punct_exceptions);
        assert_eq!(config.digit_punct_fullwidth, digit_punct::DEFAULT);
        assert!(!config.corner_quotes);
        assert_eq!(config.fuzzy_groups, fuzzy::DEFAULT);
    }

    #[test]
    fn brand_icon_settings_round_trip_and_keep_old_defaults() {
        let old: Config = serde_json::from_str(r#"{"tray_icon":true}"#).unwrap();
        assert_eq!(serde_json::to_value(old).unwrap()["brand_icon"], "default");
        for variant in ["default", "white", "custom"] {
            let config: Config = serde_json::from_value(serde_json::json!({
                "brand_icon": variant,
                "custom_brand_icon": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg=="
            })).unwrap();
            let saved = serde_json::to_value(config.sanitized()).unwrap();
            assert_eq!(saved["brand_icon"], variant);
            assert!(saved["custom_brand_icon"].as_str().is_some_and(|data| !data.is_empty()));
        }
        let unknown: Config = serde_json::from_value(serde_json::json!({"brand_icon":"other"})).unwrap();
        assert_eq!(serde_json::to_value(unknown.sanitized()).unwrap()["brand_icon"], "default");
        let oversized: Config = serde_json::from_value(serde_json::json!({
            "brand_icon":"custom", "custom_brand_icon":"A".repeat(512 * 1024 + 1)
        })).unwrap();
        let saved = serde_json::to_value(oversized.sanitized()).unwrap();
        assert_eq!(saved["brand_icon"], "default");
        assert!(saved.get("custom_brand_icon").is_none());
    }

    #[test]
    fn prediction_defaults_fill_missing_fields_and_preserve_explicit_preferences() {
        let defaults: Config = serde_json::from_str("{}").unwrap();
        assert!(!defaults.predict_next);
        let old: Config = serde_json::from_str(r#"{"predict_next":true}"#).unwrap();
        assert!(!old.predict_digits);
        assert!(old.predict_space && old.predict_double_space && old.predict_arrows);
        let explicit = Config {
            predict_digits: true,
            predict_space: false,
            predict_double_space: false,
            predict_arrows: false,
            ..old
        };
        let restored: Config = decode(&encode(&explicit)).unwrap();
        assert_eq!(restored, explicit, "不能用默认值覆盖已保存的选词键偏好");
    }

    #[test]
    fn automatic_updates_default_off_and_round_trip_in_config() {
        let mut config: Config = serde_json::from_str("{}").unwrap();
        assert!(!config.auto_update, "旧配置不能自动开启软件更新");
        config.auto_update = true;
        let restored: Config = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.auto_update);
    }

    #[test]
    fn showing_settings_after_update_defaults_off_and_round_trips() {
        let old: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(serde_json::to_value(old).unwrap()["show_settings_after_update"], false);
        let enabled: Config = serde_json::from_str(r#"{"show_settings_after_update":true}"#).unwrap();
        let restored: Config = serde_json::from_str(&serde_json::to_string(&enabled).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap()["show_settings_after_update"], true);
    }

    #[test]
    fn unknown_choices_fall_back() {
        let config = Config {
            hotkey_emoji: "ctrl+x".into(),
            mode_scope: "window".into(),
            font_family: "  ".into(),
            candidate_style: "purple".into(),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(config.hotkey_emoji, "off");
        assert_eq!(config.mode_scope, "global");
        assert_eq!(config.font_family, DEFAULT_FONT);
        assert_eq!(config.candidate_style, "pink");
        let sogou = Config { candidate_style: "sogou".into(), ..Config::default() }.sanitized();
        assert_eq!(sogou.candidate_style, "sogou");
        let arrow = Config { arrow_keys: "other".into(), ..Config::default() }.sanitized();
        assert_eq!(arrow.arrow_keys, "candidates");
        let caret = Config { arrow_keys: "caret".into(), ..Config::default() }.sanitized();
        assert_eq!(caret.arrow_keys, "caret");
        let fuzzy = Config {
            fuzzy_groups: fuzzy::AN_AI | fuzzy::ENG_ONG | (1 << 20),
            digit_punct_fullwidth: digit_punct::DUNHAO | digit_punct::COLON | (1 << 25),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(fuzzy.fuzzy_groups, fuzzy::AN_AI | fuzzy::ENG_ONG);
        assert_eq!(fuzzy.digit_punct_fullwidth, digit_punct::DUNHAO | digit_punct::COLON);
    }

    #[test]
    fn app_preset_rules_are_normalized_and_deduplicated() {
        let config: Config = serde_json::from_str(
            r#"{"app_presets": true, "app_preset_rules": [
                {"app": " Code.EXE ", "mode": "english"},
                {"app": "C:\\Program Files\\Tencent\\WeChat.exe", "mode": "chinese"},
                {"app": "code", "mode": "chinese"},
                {"app": "bad|name.exe", "mode": "english"},
                {"app": "  ", "mode": "english"}
            ]}"#,
        )
        .unwrap();
        let config = config.sanitized();
        assert!(config.app_presets);
        assert_eq!(
            config.app_preset_rules,
            vec![
                AppPreset { app: "code.exe".into(), mode: Mode::English },
                AppPreset { app: "wechat.exe".into(), mode: Mode::Chinese },
            ],
            "统一成小写文件名，重复的只留第一条，写不成文件名的丢掉"
        );
        let old: Config = serde_json::from_str("{}").unwrap();
        assert!(!old.app_presets && old.app_preset_rules.is_empty(), "旧配置不能自动开启");
    }

    #[test]
    fn unit_requests_decode() {
        assert_eq!(
            decode::<Request>(br#"{"op":"caps_lock","on":true}"#).unwrap(),
            Request::CapsLock { on: true }
        );
        assert_eq!(decode::<Request>(br#"{"op":"list_apps"}"#).unwrap(), Request::ListApps);
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
