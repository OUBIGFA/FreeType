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
    /// 同一次物理按键的自动重复；不属于应用快捷键修饰键。
    pub const REPEAT: u32 = 1 << 5;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_position_defaults_below_and_rejects_unknown_values() {
        let old: Config = serde_json::from_str("{}").unwrap();
        assert!(!old.sentence_editor);
        assert_eq!(old.sentence_editor_position, "below");
        let above: Config =
            serde_json::from_str(r#"{"sentence_editor":true,"sentence_editor_position":"above"}"#)
                .unwrap();
        assert_eq!(above.sanitized().sentence_editor_position, "above");
        let unknown: Config =
            serde_json::from_str(r#"{"sentence_editor_position":"sideways"}"#).unwrap();
        assert_eq!(unknown.sanitized().sentence_editor_position, "below");
    }

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
        let config: Config = serde_json::from_str(
            r#"{"page_size": 50, "shuangpin_scheme": "x", "hint_position": "left"}"#,
        )
        .unwrap();
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
            assert!(saved["custom_brand_icon"]
                .as_str()
                .is_some_and(|data| !data.is_empty()));
        }
        let unknown: Config =
            serde_json::from_value(serde_json::json!({"brand_icon":"other"})).unwrap();
        assert_eq!(
            serde_json::to_value(unknown.sanitized()).unwrap()["brand_icon"],
            "default"
        );
        let oversized: Config = serde_json::from_value(serde_json::json!({
            "brand_icon":"custom", "custom_brand_icon":"A".repeat(512 * 1024 + 1)
        }))
        .unwrap();
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
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.auto_update);
    }

    #[test]
    fn showing_settings_after_update_defaults_off_and_round_trips() {
        let old: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(
            serde_json::to_value(old).unwrap()["show_settings_after_update"],
            false
        );
        let enabled: Config =
            serde_json::from_str(r#"{"show_settings_after_update":true}"#).unwrap();
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&enabled).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap()["show_settings_after_update"],
            true
        );
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
        let sogou = Config {
            candidate_style: "sogou".into(),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(sogou.candidate_style, "sogou");
        let custom = Config {
            candidate_style: "custom".into(),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(custom.candidate_style, "custom");
        let arrow = Config {
            arrow_keys: "other".into(),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(arrow.arrow_keys, "candidates");
        let caret = Config {
            arrow_keys: "caret".into(),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(caret.arrow_keys, "caret");
        let fuzzy = Config {
            fuzzy_groups: fuzzy::AN_AI | fuzzy::ENG_ONG | (1 << 20),
            digit_punct_fullwidth: digit_punct::DUNHAO | digit_punct::COLON | (1 << 25),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(fuzzy.fuzzy_groups, fuzzy::AN_AI | fuzzy::ENG_ONG);
        assert_eq!(
            fuzzy.digit_punct_fullwidth,
            digit_punct::DUNHAO | digit_punct::COLON
        );
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
                AppPreset {
                    app: "code.exe".into(),
                    mode: Mode::English
                },
                AppPreset {
                    app: "wechat.exe".into(),
                    mode: Mode::Chinese
                },
            ],
            "统一成小写文件名，重复的只留第一条，写不成文件名的丢掉"
        );
        let old: Config = serde_json::from_str("{}").unwrap();
        assert!(
            !old.app_presets && old.app_preset_rules.is_empty(),
            "旧配置不能自动开启"
        );
    }

    #[test]
    fn unit_requests_decode() {
        assert_eq!(
            decode::<Request>(br#"{"op":"caps_lock","on":true}"#).unwrap(),
            Request::CapsLock { on: true }
        );
        assert_eq!(
            decode::<Request>(br#"{"op":"list_apps"}"#).unwrap(),
            Request::ListApps
        );
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
        assert_eq!(
            decode::<Request>(br#"{"op":"reset_sync_records"}"#).unwrap(),
            Request::ResetSyncRecords
        );
        assert_eq!(
            decode::<Request>(
                br#"{"op":"import_sync_settings","path":"a.json","expected_path":"b.json"}"#
            )
            .unwrap(),
            Request::ImportSyncSettings {
                path: "a.json".into(),
                expected_path: "b.json".into(),
                ignore_records: false,
            }
        );
        assert_eq!(
            decode::<Request>(br#"{"op":"import_sync_settings","path":"a.json","expected_path":"b.json","ignore_records":true}"#).unwrap(),
            Request::ImportSyncSettings {
                path: "a.json".into(),
                expected_path: "b.json".into(),
                ignore_records: true,
            }
        );
    }

    #[test]
    fn pipe_name_has_sid() {
        assert_eq!(pipe_name("S-1-5-21-1"), r"\\.\pipe\FreeType-S-1-5-21-1");
    }

    #[test]
    fn custom_candidate_skin_parses_normalizes_and_keeps_old_defaults() {
        assert_eq!(parse_hex_rgb(" #1A2b3C "), Some((0x1a, 0x2b, 0x3c)));
        assert_eq!(parse_hex_rgb("abc"), Some((0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex_rgb("#12345"), None);
        assert_eq!(parse_hex_rgb("xyz"), None);
        assert_eq!(normalize_hex_color(" #0E8549 "), Some("#0e8549".into()));

        let old: Config = serde_json::from_str(r#"{"candidate_style":"blue"}"#).unwrap();
        assert_eq!(old.candidate_skin, CandidateSkin::default());

        let dirty = serde_json::from_value::<Config>(serde_json::json!({
            "candidate_style": "custom",
            "candidate_skin": {
                "background": " #FBF8F1 ",
                "border": "invalid",
                "text": "123",
                "muted": "#6B5C63",
                "accent": "#C44569",
                "accent_text": "bad-hex",
                "accent_fill": false
            }
        }))
        .unwrap()
        .sanitized();
        assert_eq!(dirty.candidate_style, "custom");
        assert_eq!(dirty.candidate_skin.background, "#fbf8f1");
        assert_eq!(dirty.candidate_skin.border, CandidateSkin::default().border);
        assert_eq!(dirty.candidate_skin.text, "#112233");
        assert_eq!(dirty.candidate_skin.muted, "#6b5c63");
        assert_eq!(dirty.candidate_skin.accent, "#c44569");
        assert_eq!(
            dirty.candidate_skin.accent_text,
            CandidateSkin::default().accent_text
        );
        assert!(!dirty.candidate_skin.accent_fill);

        let restored: Config = decode(&encode(&dirty)).unwrap();
        assert_eq!(restored, dirty);
    }
}

mod config;
pub use config::{
    normalize_app_name, normalize_hex_color, parse_hex_rgb, AppPreset, AppWindow, CandidateSkin,
    Config, DefaultMode, ARROW_KEYS, CANDIDATE_STYLES, HINT_POSITIONS, HOTKEYS_EMOJI, HOTKEYS_MODE,
    HOTKEYS_PUNCT, HOTKEYS_TRADITIONAL, MAX_APP_PRESETS, MODE_SCOPES, SELECT_KEYS, SHIFT_TOGGLES,
    SHUANGPIN_SCHEMES,
};
mod request;
pub use request::{
    BackupPart, CandidateAction, Command, Dictionary, DictionaryEntry, DictionaryPage, Key,
    QuickPhrase, Request, SyncMergePreview, TypingDay, UserWord,
};
mod frame;
use frame::*;
pub use frame::{Frame, PairEdit, Panel, SentenceBlock, SentenceView};
mod response;
pub use response::{
    decode, encode, BrandIcon, CandidateDiagnostics, DictionaryUpdateStatus, EnhancementStatus,
    ImportReport, Response, Status,
};
