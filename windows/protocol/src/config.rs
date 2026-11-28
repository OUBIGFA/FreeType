//! 用户设置 Config：字段、默认值、取值白名单与规范化。
use super::*;

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

/// 自定义候选窗皮肤颜色（十六进制 `#rrggbb`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandidateSkin {
    pub background: String,
    pub border: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub accent_text: String,
    pub accent_fill: bool,
}

impl Default for CandidateSkin {
    fn default() -> Self {
        Self {
            background: "#ffffff".into(),
            border: "#d6d6d6".into(),
            text: "#1f1f1f".into(),
            muted: "#616161".into(),
            accent: "#2b6ce6".into(),
            accent_text: "#ffffff".into(),
            accent_fill: true,
        }
    }
}

/// 解析十六进制 RGB 颜色（支持 `#rrggbb`、`rrggbb`、`#rgb`、`rgb`）。
pub fn parse_hex_rgb(raw: &str) -> Option<(u8, u8, u8)> {
    let trimmed = raw.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b))
        }
        3 => {
            let nibble = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| v * 17);
            Some((
                nibble(&hex[0..1])?,
                nibble(&hex[1..2])?,
                nibble(&hex[2..3])?,
            ))
        }
        _ => None,
    }
}

/// 规范化十六进制 RGB 颜色为小写 `#rrggbb`；无效输入返回 `None`。
pub fn normalize_hex_color(raw: &str) -> Option<String> {
    let (r, g, b) = parse_hex_rgb(raw)?;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

impl CandidateSkin {
    /// 将无效颜色回退为默认值，并统一格式化为小写 `#rrggbb`。
    pub fn sanitized(mut self) -> Self {
        let d = Self::default();
        self.background = normalize_hex_color(&self.background).unwrap_or(d.background);
        self.border = normalize_hex_color(&self.border).unwrap_or(d.border);
        self.text = normalize_hex_color(&self.text).unwrap_or(d.text);
        self.muted = normalize_hex_color(&self.muted).unwrap_or(d.muted);
        self.accent = normalize_hex_color(&self.accent).unwrap_or(d.accent);
        self.accent_text = normalize_hex_color(&self.accent_text).unwrap_or(d.accent_text);
        self
    }
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
    /// 选中候选的样式：pink 玫瑰粉、blue 蓝、green 微信绿、sogou 无底色块、custom 自定义配色。
    pub candidate_style: String,
    /// 自定义候选窗皮肤颜色；切换内置样式后仍保留。
    pub candidate_skin: CandidateSkin,
    /// 窗口、托盘及系统输入指示器的品牌图标：default、white、custom。
    pub brand_icon: String,
    /// 自定义图像转为 256 px PNG 后以 Base64 保存，让备份和同步不依赖本机路径。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_brand_icon: String,
    /// 鼠标悬停候选窗时允许滚轮翻页。
    pub candidate_wheel: bool,
    /// 行内预编辑：segmented、raw、off；候选窗预编辑独立控制。
    pub inline_preedit: String,
    pub candidate_preedit: bool,
    /// 独立显示整句编辑区；旧配置缺字段时也默认关闭。
    pub sentence_editor: bool,
    /// below（默认）或 above；空间不足时收起，不挪动候选窗。
    pub sentence_editor_position: String,
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
    /// 自定义主词库（`拼音<Tab>词<Tab>词频`）的路径；为空或无法载入时使用安装包的默认词库。
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
    /// 习惯连打左右两半时，补全后紧接着按下的右半标点不再重复插入（默认关）。
    pub auto_pair_absorb_closing: bool,
    /// 每七天检查并下载词库更新包（默认关）；下载校验后在下次服务启动时生效。
    pub dictionary_auto_update: bool,
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
    /// 仅这些程序在空闲的单次 Esc 时切英文；按键继续交给宿主。
    pub vim_mode_apps: Vec<String>,
    pub english_correction: bool,
    pub english_space: bool,
    pub english_gloss: bool,
    pub hotkey_english: String,
    pub hotkey_radical: String,

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
    /// 至少四个完整音节后补一个词库长词，默认开启，与上屏联想独立。
    pub long_word_completion: bool,
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
    /// 本机候选诊断，默认关闭；含正文，不随备份或同步启用。
    pub candidate_diagnostics: bool,
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
            candidate_skin: CandidateSkin::default(),
            brand_icon: "default".into(),
            custom_brand_icon: String::new(),
            candidate_wheel: false,
            inline_preedit: "segmented".into(),
            candidate_preedit: true,
            sentence_editor: false,
            sentence_editor_position: "below".into(),
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
            auto_pair_absorb_closing: false,
            dictionary_auto_update: false,
            autospace: true,
            autospace_letters: true,
            autospace_digits: true,
            default_mode: DefaultMode::Chinese,
            mode_scope: "global".into(),
            traditional: false,
            app_presets: false,
            app_preset_rules: Vec::new(),
            vim_mode_apps: Vec::new(),
            english_correction: true,
            english_space: true,
            english_gloss: true,
            hotkey_english: "off".into(),
            hotkey_radical: "off".into(),
            quick_phrases: true,
            emoji_suggest: true,
            date_time: true,
            calculator: true,
            toolbar: true,
            clipboard_history: false,
            email_completion: true,
            long_word_completion: true,
            predict_next: false,
            predict_digits: false,
            predict_space: true,
            predict_double_space: true,
            predict_arrows: true,
            typing_stats: false,
            candidate_diagnostics: false,
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
pub const SHUANGPIN_SCHEMES: [&str; 7] = [
    "ziranma",
    "xiaohe",
    "microsoft",
    "sogou",
    "abc",
    "ziguang",
    "pinyinjiajia",
];

pub const CANDIDATE_STYLES: [&str; 5] = ["pink", "blue", "green", "sogou", "custom"];

pub const SHIFT_TOGGLES: [&str; 4] = ["both", "left", "right", "off"];

pub const SELECT_KEYS: [&str; 3] = ["off", "semicolon", "quote"];

pub const ARROW_KEYS: [&str; 2] = ["candidates", "caret"];

pub const MODE_SCOPES: [&str; 2] = ["global", "app"];

pub const HINT_POSITIONS: [&str; 6] = [
    "top-left",
    "top",
    "top-right",
    "bottom-left",
    "bottom",
    "bottom-right",
];

pub const HOTKEYS_EMOJI: [&str; 3] = ["off", "ctrl+shift+e", "ctrl+alt+e"];

pub const HOTKEYS_PUNCT: [&str; 2] = ["off", "ctrl+period"];

pub const HOTKEYS_TRADITIONAL: [&str; 3] = ["off", "ctrl+shift+f", "ctrl+shift+t"];

pub const HOTKEYS_MODE: [&str; 2] = ["off", "ctrl+space"];

/// 指定软件的规则最多几条（设置里手工维护的列表，不需要更多）。
pub const MAX_APP_PRESETS: usize = 200;

/// 规则里的程序名统一成小写文件名：去掉路径，没写扩展名的补上 `.exe`。写不成文件名的返回 None。
pub fn normalize_app_name(name: &str) -> Option<String> {
    let name = name.trim().trim_matches('"');
    let name = name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(name)
        .trim()
        .to_lowercase();
    if name.is_empty()
        || name.len() > 260
        || name
            .chars()
            .any(|c| c.is_control() || r#"<>:"|?*"#.contains(c))
    {
        return None;
    }
    Some(if name.ends_with(".exe") {
        name
    } else {
        name + ".exe"
    })
}

/// 不在 `allowed` 里就换成 `fallback`。
pub(super) fn pick(value: &mut String, allowed: &[&str], fallback: &str) {
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
        if self.brand_icon == "custom" {
            self.custom_brand_icon.hash(&mut hash);
        }
        hash.finish()
    }

    /// 把越界的值拉回范围，不认识的取值换成默认值。
    pub fn sanitized(mut self) -> Self {
        let d = Config::default();
        self.page_size = self.page_size.clamp(3, 10);
        self.font_size = self.font_size.clamp(12, 36);
        if self.preedit_size != 0 {
            self.preedit_size = self.preedit_size.clamp(10, 36);
        }
        pick(
            &mut self.inline_preedit,
            &["segmented", "spaces", "raw", "off"],
            &d.inline_preedit,
        );
        pick(
            &mut self.sentence_editor_position,
            &["below", "above"],
            &d.sentence_editor_position,
        );
        self.font_fallbacks = self
            .font_fallbacks
            .split([',', '，'])
            .map(str::trim)
            .filter(|name| {
                !name.is_empty() && name.len() <= 64 && !name.chars().any(char::is_control)
            })
            .take(6)
            .collect::<Vec<_>>()
            .join(",");
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
        pick(
            &mut self.pin_badge,
            &["off", "paperclip", "pushpin", "dot"],
            &d.pin_badge,
        );
        if ![0, 30, 90, 180, 365].contains(&self.stats_retention) {
            self.stats_retention = d.stats_retention;
        }
        self.page_keys &= 0xf;
        self.digit_punct_fullwidth &= digit_punct::ALL;
        pick(
            &mut self.candidate_style,
            &CANDIDATE_STYLES,
            &d.candidate_style,
        );
        self.candidate_skin = self.candidate_skin.sanitized();
        pick(
            &mut self.brand_icon,
            &["default", "white", "custom"],
            &d.brand_icon,
        );
        // 显示端只导入 256 px PNG；限制配置体积，保留未选中的自定义图标供再次选择。
        if self.custom_brand_icon.len() > 512 * 1024 {
            self.custom_brand_icon.clear();
        }
        if self.brand_icon == "custom" && self.custom_brand_icon.is_empty() {
            self.brand_icon = d.brand_icon.clone();
        }
        pick(
            &mut self.shuangpin_scheme,
            &SHUANGPIN_SCHEMES,
            &d.shuangpin_scheme,
        );
        pick(&mut self.shift_toggle, &SHIFT_TOGGLES, &d.shift_toggle);
        pick(&mut self.select_keys, &SELECT_KEYS, &d.select_keys);
        pick(&mut self.arrow_keys, &ARROW_KEYS, &d.arrow_keys);
        pick(&mut self.mode_scope, &MODE_SCOPES, &d.mode_scope);
        pick(&mut self.hint_position, &HINT_POSITIONS, &d.hint_position);
        pick(&mut self.hotkey_emoji, &HOTKEYS_EMOJI, &d.hotkey_emoji);
        pick(
            &mut self.hotkey_english,
            &["off", "ctrl+shift+y", "ctrl+alt+y"],
            &d.hotkey_english,
        );
        pick(
            &mut self.hotkey_radical,
            &["off", "ctrl+shift+u", "ctrl+alt+u"],
            &d.hotkey_radical,
        );
        pick(&mut self.hotkey_punct, &HOTKEYS_PUNCT, &d.hotkey_punct);
        pick(
            &mut self.hotkey_traditional,
            &HOTKEYS_TRADITIONAL,
            &d.hotkey_traditional,
        );
        pick(&mut self.hotkey_mode, &HOTKEYS_MODE, &d.hotkey_mode);
        // 同一个程序只留最先的一条：匹配时不分大小写，重复的规则没有意义
        let mut rules: Vec<AppPreset> = Vec::new();
        for rule in std::mem::take(&mut self.app_preset_rules) {
            let Some(app) = normalize_app_name(&rule.app) else {
                continue;
            };
            if rules.len() < MAX_APP_PRESETS && rules.iter().all(|r| r.app != app) {
                rules.push(AppPreset {
                    app,
                    mode: rule.mode,
                });
            }
        }
        self.app_preset_rules = rules;
        self.vim_mode_apps = self
            .vim_mode_apps
            .iter()
            .filter_map(|name| normalize_app_name(name))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
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
