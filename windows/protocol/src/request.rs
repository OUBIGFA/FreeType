//! 请求：按键、候选操作、命令和设置程序的管理请求。
use super::*;

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
    #[serde(default = "dictionary_enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub description: String,
}

pub(super) fn dictionary_enabled_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryEntry {
    pub word: String,
    pub pinyin: String,
    #[serde(default)]
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryPage {
    pub dictionary: Dictionary,
    pub total: usize,
    pub offset: usize,
    pub entries: Vec<DictionaryEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncMergePreview {
    pub versions: usize,
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub conflicts: usize,
    pub details: Vec<String>,
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
    EnglishCandidates,
    RadicalLookup,
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
    SentenceFocus { at: usize },
    SentenceCaret { at: usize },
    SentencePage { next: bool },
    SentenceChoose { index: usize },
    SentenceResize { expand: bool },
    SentenceLeave,
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
    CandidateAction {
        revision: u64,
        action: CandidateAction,
    },
    ListTypingStats,
    ClearTypingStats,
    GetCandidateDiagnostics,
    ClearCandidateDiagnostics,
    ExportCandidateDiagnostics {
        path: String,
        #[serde(default)]
        evaluation: bool,
    },
    /// Windows 宿主识别敏感输入框时，不保留编辑区、上下文和联想。
    SensitiveFocus {
        active: bool,
    },
    /// TSF 已核对紧接的退格确实删除了刚上屏的字符。
    UndoCommitLearning,
    /// TSF 已验证右标点存在，先把当前输入候选上屏，不再追加一个右符号。
    ClosePair,
    /// 鼠标点候选：当前页里的序号。
    Select {
        index: usize,
    },
    /// 候选窗上的翻页按钮。
    Page {
        next: bool,
    },
    /// 忘掉当前页里第 index 个候选（长按候选、右键菜单）。
    Forget {
        index: usize,
    },
    /// 输入框失去焦点、应用要求结束组字：`commit` 时把编辑区原样上屏。
    Reset {
        commit: bool,
    },
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
    Command {
        command: Command,
    },
    /// 大写锁定开了 / 关了（DLL 在按下 Caps Lock 和输入框得到焦点时报）。
    /// 开着时一律按英文输入，关掉后回到原来的中英文状态。
    CapsLock {
        on: bool,
    },
    /// 服务状态（版本、词库是否载入、模式）。
    Status,
    EnhancementStatus,
    SetEnhancement {
        enabled: bool,
    },
    RefreshEnhancement {
        revision: u64,
    },
    /// 词库更新包的本机状态（设置程序用）。
    DictionaryUpdateStatus,
    /// 设置程序已把校验过的更新包解到 `data-updates/<version>`：登记为下次启动生效。
    /// `automatic` 记下是否由自动更新登记，关闭自动更新时只撤销这类安排。
    StageDictionaryUpdate {
        version: String,
        #[serde(default)]
        automatic: bool,
    },
    /// 下次启动换回上一份词库（上一份为空表示随安装包的词库）。
    RestorePreviousDictionary,
    /// 撤销尚未生效的更新；`automatic_only` 时只撤销自动登记的。
    CancelPendingDictionary {
        #[serde(default)]
        automatic_only: bool,
    },
    /// 现在开着窗口的程序（设置里添加指定软件时挑）。
    ListApps,
    GetConfig,
    /// 所有输入宿主可读取已公开显示的图像；不暴露整份设置，不建立输入会话。
    GetBrandIcon {
        #[serde(default)]
        known_revision: Option<u64>,
    },
    /// 仅设置程序可用；None 移除外部模型并回退随包版本。
    SetNeuralModel {
        path: Option<String>,
    },
    SetConfig {
        config: Config,
    },
    ListQuickPhrases,
    /// 整份替换快捷短语。
    SetQuickPhrases {
        phrases: Vec<QuickPhrase>,
    },
    /// 从水杉格式文件导入；`replace` 时清掉原有的。
    ImportQuickPhrases {
        path: String,
        #[serde(default)]
        replace: bool,
    },
    ExportQuickPhrases {
        path: String,
    },
    /// 工具栏里的独立常用语，不使用拼音编码。
    ListCommonPhrases,
    SetCommonPhrases {
        phrases: Vec<String>,
    },
    ImportCommonPhrases {
        path: String,
        #[serde(default)]
        replace: bool,
    },
    ExportCommonPhrases {
        path: String,
    },
    ListDictionaries,
    /// 导入水杉格式词库文件，存为 `dicts\<name>.txt`。
    ImportDictionary {
        path: String,
        name: String,
    },
    RemoveDictionary {
        name: String,
    },
    QueryDictionary {
        name: String,
        query: String,
        offset: usize,
        limit: usize,
    },
    UpdateDictionary {
        name: String,
        new_name: String,
        enabled: bool,
        description: String,
    },
    DeleteDictionaryEntries {
        name: String,
        entries: Vec<DictionaryEntry>,
    },
    /// 导出学到的用户词（水杉词库格式）。
    ExportUserDictionary {
        path: String,
    },
    /// 完整学习备份（JSON）：词条、次数、时间和搭配。
    ExportLearning {
        path: String,
    },
    /// 合并完整学习备份，不清空现有记录，也不重复累加次数。
    ImportLearning {
        path: String,
    },
    /// 全部设置和用户数据的版本化备份；校验预览不改动当前数据。
    /// `parts` 是要导出的类别，不写就是全部。
    ExportSettings {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parts: Option<Vec<BackupPart>>,
    },
    /// 自动同步的完整快照及数据修改时间；导出本身不算修改。
    ExportSyncSettings {
        path: String,
    },
    /// 仅当当前数据仍与 expected_path 一致且没有组字时整份恢复。
    ImportSyncSettings {
        path: String,
        expected_path: String,
        #[serde(default)]
        ignore_records: bool,
    },
    /// 重建本机同步变更记录，用于同步记录损坏或与正文不一致时恢复。
    ResetSyncRecords,
    InspectSettingsBackup {
        path: String,
    },
    ImportSettings {
        path: String,
    },
    PreviewSyncMerge {
        path: String,
    },
    MergeSyncSettings {
        path: String,
        expected_path: String,
    },
    /// 把一份全部设置备份合并进当前数据（数据同步的恢复）：不删除、不替换本机已有的内容。
    MergeSettings {
        path: String,
    },
    /// 学到的词（用户词库）。
    ListUserWords,
    /// 忘掉这些学到的词（用户词库和选词习惯都去掉）。
    ForgetWords {
        words: Vec<String>,
    },
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
