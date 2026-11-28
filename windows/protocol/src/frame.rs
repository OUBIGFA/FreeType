//! 输入法 DLL 绘制与上屏用的画面 Frame 及整句编辑区。
use super::*;

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
pub enum PairEdit {
    Skip,
    Erase,
}

/// 一次按键之后 DLL 要做的事。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    #[serde(default)]
    pub enhancement_pending: bool,
    /// Optional sentence editing metadata; offsets refer to the original keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentence: Option<SentenceView>,
    #[serde(default)]
    pub sentence_editor_above: bool,
    /// 显示注音/释义，与实际候选及上屏内容分离。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<String>,
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
    /// 设置「连打两半时不重复右标点」：补全的左移键发出前按下的右半标点只跳过。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pair_absorb_closing: bool,
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
    /// 双击选词时用完整联想词替换 Windows 宿主保留的第一个半角空格。
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
    /// 自定义皮肤颜色；仅在 `candidate_style` 为 `custom` 时由服务端下发。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_skin: Option<CandidateSkin>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub candidate_wheel: bool,
    /// 候选竖排。
    #[serde(default)]
    pub vertical: bool,
    /// 要知道光标前的字（自动加空格打开了）：DLL 没在组字时每个键都带上 `before`。
    #[serde(default)]
    pub want_before: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SentenceBlock {
    pub from: usize,
    pub to: usize,
    pub text: String,
    pub confirmed: bool,
    pub corrected: bool,
    pub focused: bool,
    #[serde(default)]
    pub uncertain: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SentenceView {
    pub text: String,
    pub raw: String,
    pub cursor: Option<usize>,
    pub blocks: Vec<SentenceBlock>,
    pub alternatives: Vec<String>,
    pub page: usize,
    pub has_prev: bool,
    pub has_next: bool,
}

pub(super) fn is_zero(n: &usize) -> bool {
    *n == 0
}
