//! 应答与状态：服务状态、导入报告、品牌图标、词库更新与增强模型状态，以及一行一条的编码。
use super::*;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lexicon_warning: Option<String>,
    #[serde(default)]
    pub lexicon_loading: bool,
    /// 距上次实际按键或选词的整秒数；启动后还没打过字时为空。
    /// 托盘的自动同步据此避开打字时段。只发给可管理的客户端，受限应用拿不到输入节奏。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_idle_seconds: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enhancement: Option<EnhancementStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictionary_update: Option<DictionaryUpdateStatus>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_diagnostics: Option<CandidateDiagnostics>,
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
    pub dictionary_page: Option<DictionaryPage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_merge: Option<SyncMergePreview>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sync_format: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_words: Option<Vec<UserWord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ImportReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<Vec<AppWindow>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typing_stats: Option<std::collections::BTreeMap<String, TypingDay>>,
}

/// 词库更新包状态。版本为空串表示随安装包的词库。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DictionaryUpdateStatus {
    /// 更新包所在目录（`%LOCALAPPDATA%\FreeType\data-updates`）；设置程序解包到这里。
    pub directory: String,
    /// 本次服务启动实际载入的词库版本。
    pub active: String,
    /// 随安装包词库的数据版本。
    pub bundled: String,
    pub current: String,
    pub previous: String,
    /// 下次服务启动时生效的版本。
    pub pending: String,
    pub pending_automatic: bool,
    /// 最近一次未能生效的版本及原因。
    pub failed_version: String,
    pub failed_reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EnhancementStatus {
    pub enabled: bool,
    pub state: String,
    pub error: String,
}

/// 后台记录状态；不含输入内容，只在用户打开诊断入口时请求。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CandidateDiagnostics {
    pub bytes: u64,
    pub dropped: u64,
    pub error: String,
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
