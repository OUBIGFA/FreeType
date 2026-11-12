//! FreeType 服务进程的命名管道消息。
//!
//! 输入法 DLL（每个应用进程里一份）和设置程序都连到同一个服务进程：管道是字节模式，
//! 每条请求、每条应答都是一行 JSON（UTF-8，`\n` 结尾）。一个连接里请求和应答一问一答。
//!
//! 管道名带当前用户的 SID：`\\.\pipe\FreeType-<SID>`，见 [`pipe_name`]。

use serde::{Deserialize, Serialize};

/// 协议版本：改了已有字段的含义时加一。服务进程在 `hello` 的应答里带上它。
pub const PROTOCOL_VERSION: u32 = 1;

/// 输入法文本服务的 COM 类 ID（注册后不能再改）。
pub const TEXT_SERVICE_CLSID: &str = "6AEA9A65-33E4-4FFA-94E3-399C2ADC4CF1";
/// 简体中文语言配置的 GUID（注册后不能再改）。
pub const PROFILE_GUID: &str = "2C829DC6-FA03-477E-85D0-C64E1C64B209";
/// 简体中文（中国）的 LANGID。
pub const LANGID_ZH_CN: u16 = 0x0804;
/// 显示在系统输入法列表里的名字。
pub const DISPLAY_NAME: &str = "FreeType";

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

/// 输入状态：中文（拼音）或英文（按键原样交给应用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Chinese,
    English,
}

/// 用户设置，存为 `%APPDATA%\FreeType\config.json`。缺的字段取默认值。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 一页几个候选（3–10）。
    pub page_size: u32,
    /// 候选横排。
    pub horizontal: bool,
    /// 模糊音（z/zh、c/ch、s/sh、an/ang、en/eng、in/ing 等）。
    pub fuzzy: bool,
    /// 双拼及方案：ziranma xiaohe microsoft sogou abc ziguang pinyinjiajia。
    pub shuangpin: bool,
    pub shuangpin_scheme: String,
    /// 输出繁体。
    pub traditional: bool,
    /// 中文状态下标点用全角（，。）。
    pub fullwidth_punct: bool,
    /// 打字时推荐 emoji。
    pub emoji_suggest: bool,
    /// riqi、shijian 给日期时间候选。
    pub date_time: bool,
    /// V 模式计算器。
    pub calculator: bool,
    /// 按错键纠错。
    pub typo_correction: bool,
    /// 快捷短语混入候选（编码完整对上时排第一）。
    pub quick_phrases: bool,
    /// 候选窗字号（像素，按 96 DPI 计）。
    pub font_size: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            page_size: 7,
            horizontal: true,
            fuzzy: false,
            shuangpin: false,
            shuangpin_scheme: "xiaohe".into(),
            traditional: false,
            fullwidth_punct: true,
            emoji_suggest: true,
            date_time: true,
            calculator: true,
            typo_correction: true,
            quick_phrases: true,
            font_size: 18,
        }
    }
}

impl Config {
    /// 把越界的值拉回范围，未知的双拼方案换成小鹤。
    pub fn sanitized(mut self) -> Self {
        self.page_size = self.page_size.clamp(3, 10);
        self.font_size = self.font_size.clamp(12, 36);
        const SCHEMES: [&str; 7] = [
            "ziranma",
            "xiaohe",
            "microsoft",
            "sogou",
            "abc",
            "ziguang",
            "pinyinjiajia",
        ];
        if !SCHEMES.contains(&self.shuangpin_scheme.as_str()) {
            self.shuangpin_scheme = "xiaohe".into();
        }
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

/// 请求。`op` 字段区分种类。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// 连上以后先说一声：DLL 带 `session: true`，服务进程为这个连接建输入会话。
    Hello {
        #[serde(default)]
        session: bool,
    },
    /// 按键。
    Key { key: Key },
    /// 鼠标点候选：当前页里的序号。
    Select { index: usize },
    /// 候选窗上的翻页按钮。
    Page { next: bool },
    /// 输入框失去焦点、应用要求结束组字：`commit` 时把编辑区原样上屏。
    Reset { commit: bool },
    /// 切换中英文（单按 Shift、托盘菜单）。
    ToggleMode,
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
    /// 重新读配置、词库和快捷短语。
    Reload,
    /// 存好用户词库后退出（安装、卸载前用）。
    Shutdown,
}

/// 一次按键之后 DLL 要做的事。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// 吃掉这个键（false：交给应用）。
    pub handled: bool,
    /// 要上屏的字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// 编辑区（组字）文字；空串表示没有在组字。
    #[serde(default)]
    pub preedit: String,
    /// 编辑区光标，UTF-16 偏移。
    #[serde(default)]
    pub preedit_cursor: usize,
    /// 这一页的候选（显示用：换行、Tab 已画成 ⏎、⇥）。
    #[serde(default)]
    pub candidates: Vec<String>,
    /// 高亮的候选（这一页里的序号）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<usize>,
    #[serde(default)]
    pub has_prev: bool,
    #[serde(default)]
    pub has_next: bool,
    #[serde(default)]
    pub mode: Mode,
    /// 候选窗短暂显示的提示（切换中英文时的「中」「英」）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// 候选窗字号（96 DPI 下的像素）。
    #[serde(default)]
    pub font_size: u32,
    /// 候选竖排。
    #[serde(default)]
    pub vertical: bool,
}

/// 服务状态。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub protocol: u32,
    pub version: String,
    /// 词库已载入（载入前也能连，只是没有候选）。
    pub ready: bool,
    pub mode: Mode,
    /// 用户数据目录。
    pub data_dir: String,
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
        };
        let line = encode(&request);
        assert!(line.ends_with(b"\n"));
        let text = String::from_utf8(line.clone()).unwrap();
        assert!(text.starts_with(r#"{"op":"key""#), "{text}");
        assert_eq!(decode::<Request>(&line).unwrap(), request);
    }

    #[test]
    fn missing_config_fields_take_defaults() {
        let config: Config = serde_json::from_str(r#"{"page_size": 50, "shuangpin_scheme": "x"}"#).unwrap();
        let config = config.sanitized();
        assert_eq!(config.page_size, 10);
        assert_eq!(config.shuangpin_scheme, "xiaohe");
        assert!(config.fullwidth_punct);
    }

    #[test]
    fn unit_requests_decode() {
        assert_eq!(decode::<Request>(br#"{"op":"toggle_mode"}"#).unwrap(), Request::ToggleMode);
        assert_eq!(
            decode::<Request>(b"{\"op\":\"hello\"}\r\n").unwrap(),
            Request::Hello { session: false }
        );
    }

    #[test]
    fn pipe_name_has_sid() {
        assert_eq!(pipe_name("S-1-5-21-1"), r"\\.\pipe\FreeType-S-1-5-21-1");
    }
}
