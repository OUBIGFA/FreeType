//! 完整设置备份。只读写应用拥有的数据文件；学习记录直接复用核心的备份格式。
//!
//! 备份里的数据分成几类（[`BackupPart`]）。导出时可以只选几类：没选的类别不写进文件。
//! 导入时文件里有哪几类就替换哪几类，没有的类别本机保持不变。云端同步用的快照总是全部类别。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use freetype_protocol::{BackupPart, Config, ImportReport, Mode, QuickPhrase};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wowtype_core::{msime, BigramLm, Engine, MemoryLexicon};

use crate::store::{dictionary_name, normalize_common_phrases, write_atomic, Store};

const MAIN_LEXICON: &str = "main-lexicon.tsv";
const PROFILE_FILES: [&str; 9] = [
    "config.json",
    "mode.json",
    "quick_phrases.txt",
    "common_phrases.json",
    "user.tsv",
    "history.tsv",
    "emoji-recent.txt",
    "statusbar.txt",
    MAIN_LEXICON,
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupDictionary {
    name: String,
    text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsBackup {
    format: String,
    version: u32,
    pub config: Config,
    pub mode: Mode,
    quick_phrases: Vec<QuickPhrase>,
    pub common_phrases: Vec<String>,
    dictionaries: Vec<BackupDictionary>,
    pub learning: Value,
    pub recent_emoji: Vec<String>,
    status_bar_position: Option<[i32; 2]>,
    // 主词库用内容迁移；config.lexicon 不保存来源机器的绝对路径。
    pub main_lexicon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_records: Option<Value>,
    // 本地一键备份由设置程序附加账号和计划；服务只接受它，不恢复或上传连接配置。
    #[serde(default, skip_serializing, rename = "sync_connection")]
    _sync_connection: Option<Value>,
    // 云端平铺备份内嵌设备索引；它不属于输入数据，也不参与再次导出。
    #[serde(default, skip_serializing, rename = "sync_metadata")]
    _sync_metadata: Option<Value>,
}

/// 校验过的备份。`data` 里没有的类别是空值，用之前先问 [`PreparedBackup::has`]。
pub(crate) struct PreparedBackup {
    pub data: SettingsBackup,
    learned: Engine,
    /// 备份里有的类别。
    parts: Vec<BackupPart>,
}

/// 一类数据在备份文件里占哪些字段：（必有的，可以没有的）。
/// 文件里出现了其中任何一个字段就算有这一类，这时必有的字段都要在。
fn part_fields(part: BackupPart) -> (&'static [&'static str], &'static [&'static str]) {
    match part {
        BackupPart::Settings => (&["config", "mode"], &["status_bar_position"]),
        BackupPart::Dictionaries => (&["dictionaries"], &["main_lexicon"]),
        BackupPart::QuickPhrases => (&["quick_phrases"], &[]),
        BackupPart::CommonPhrases => (&["common_phrases"], &[]),
        BackupPart::Learning => (&["learning", "recent_emoji"], &[]),
    }
}

/// 一类数据在数据目录里是哪些文件（导入的词库 `dicts\*.txt` 另算）。
/// 自定义主词库的位置记在 `config.json` 里，所以词库这一类也要改它。
fn part_files(part: BackupPart) -> &'static [&'static str] {
    match part {
        BackupPart::Settings => &["config.json", "mode.json", "statusbar.txt"],
        BackupPart::Dictionaries => &["config.json", MAIN_LEXICON],
        BackupPart::QuickPhrases => &["quick_phrases.txt"],
        BackupPart::CommonPhrases => &["common_phrases.json"],
        BackupPart::Learning => &["user.tsv", "history.tsv", "emoji-recent.txt"],
    }
}

fn part_name(part: BackupPart) -> &'static str {
    match part {
        BackupPart::Settings => "设置",
        BackupPart::Dictionaries => "词库",
        BackupPart::QuickPhrases => "快捷短语",
        BackupPart::CommonPhrases => "常用语",
        BackupPart::Learning => "学习记录",
    }
}

/// 认出备份文件里有哪几类数据。没有的类别补上空值，好按同一个结构读。
fn present_parts(value: &mut Value) -> Result<Vec<BackupPart>, String> {
    let Some(object) = value.as_object_mut() else {
        return Err("全部设置备份格式不正确：不是 JSON 对象".into());
    };
    let mut parts = Vec::new();
    for part in BackupPart::ALL {
        let (required, optional) = part_fields(part);
        if !required.iter().chain(optional).any(|key| object.contains_key(*key)) {
            for key in required {
                let empty = match *key {
                    "config" => serde_json::to_value(Config::default()).map_err(|error| error.to_string())?,
                    "mode" => serde_json::to_value(Mode::default()).map_err(|error| error.to_string())?,
                    "learning" => Value::Null,
                    _ => Value::Array(Vec::new()),
                };
                object.insert((*key).to_string(), empty);
            }
            continue;
        }
        if let Some(missing) = required.iter().find(|key| !object.contains_key(**key)) {
            return Err(format!("全部设置备份不完整：有{}，但缺少 {missing}", part_name(part)));
        }
        parts.push(part);
    }
    Ok(parts)
}

fn optional_text(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取 {} 失败：{error}", path.display())),
    }
}

fn dictionary_paths(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("读取词库目录失败：{error}")),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// 把 `parts` 这几类数据写成备份文件。
pub(crate) fn export(
    store: &Store,
    config: &Config,
    mode: Mode,
    recent_emoji: &[String],
    engine: &Engine,
    path: &Path,
    parts: &[BackupPart],
) -> Result<ImportReport, String> {
    let (text, report) = snapshot_of(store, config, mode, recent_emoji, engine, parts)?;
    write_atomic(path, &text).map_err(|error| format!("写入全部设置备份失败：{error}"))?;
    Ok(report)
}

/// 全部数据的快照。同一份序列化用于导出和条件恢复比较，避免下载期间的新编辑被覆盖。
pub(crate) fn snapshot(
    store: &Store, config: &Config, mode: Mode, recent_emoji: &[String], engine: &Engine,
) -> Result<(String, ImportReport), String> {
    snapshot_of(store, config, mode, recent_emoji, engine, &BackupPart::ALL)
}

/// 只读取选中的类别：没选的数据不读、不校验，也不会进文件。
fn snapshot_of(
    store: &Store, config: &Config, mode: Mode, recent_emoji: &[String], engine: &Engine, parts: &[BackupPart],
) -> Result<(String, ImportReport), String> {
    let has = |part: BackupPart| parts.contains(&part);
    let mut quick_phrases = Vec::new();
    if has(BackupPart::QuickPhrases) {
        let quick = optional_text(&store.dir().join("quick_phrases.txt"))?.unwrap_or_default();
        let parsed = msime::parse_quick_phrases(&quick);
        if !parsed.errors.is_empty() {
            return Err("快捷短语文件有错误，请先修正后再备份".into());
        }
        quick_phrases = parsed
            .items
            .into_iter()
            .map(|p| QuickPhrase {
                code: p.code,
                text: p.text,
                weight: p.weight,
            })
            .collect();
    }
    let mut dictionaries = Vec::new();
    let mut main_lexicon = None;
    if has(BackupPart::Dictionaries) {
        for file in dictionary_paths(&store.dir().join("dicts"))? {
            let name = file
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or("词库文件名不是有效文字")?
                .to_string();
            let text = fs::read_to_string(&file).map_err(|error| format!("读取词库失败：{error}"))?;
            dictionaries.push(BackupDictionary { name, text });
        }
        if !config.lexicon.is_empty() {
            main_lexicon =
                Some(fs::read_to_string(&config.lexicon).map_err(|error| format!("读取自定义主词库失败：{error}"))?);
        }
    }
    let mut config = config.clone();
    config.lexicon.clear();
    config.candidate_diagnostics = false;
    let mut status_bar_position = None;
    if has(BackupPart::Settings) {
        status_bar_position = optional_text(&store.dir().join("statusbar.txt"))?
            .map(|text| {
                let (x, y) = text.trim().split_once(',').ok_or("悬浮状态栏位置格式不正确")?;
                Ok::<_, String>([
                    x.trim().parse().map_err(|_| "悬浮状态栏横坐标不正确")?,
                    y.trim().parse().map_err(|_| "悬浮状态栏纵坐标不正确")?,
                ])
            })
            .transpose()?;
    }
    let (mut learning, mut recent) = (Value::Null, Vec::new());
    if has(BackupPart::Learning) {
        learning = serde_json::from_str(
            &engine
                .export_learning()
                .map_err(|error| format!("读取学习记录失败：{error:#}"))?,
        )
        .map_err(|error| error.to_string())?;
        recent = recent_emoji.to_vec();
    }
    let data = SettingsBackup {
        format: "freetype-settings".into(),
        version: 1,
        config,
        mode,
        quick_phrases,
        common_phrases: if has(BackupPart::CommonPhrases) { store.load_common_phrases()? } else { Vec::new() },
        dictionaries,
        learning,
        recent_emoji: recent,
        status_bar_position,
        main_lexicon,
        sync_records: None,
        _sync_connection: None,
        _sync_metadata: None,
    };
    let prepared = prepare(data, BackupPart::ALL.into_iter().filter(|part| has(*part)).collect())?;
    Ok((prepared.text()?, prepared.report()))
}

/// 首次同步和服务重启后，使用数据文件的时间，不能使用本次导出文件的时间。
pub(crate) fn modified_unix_ms(store: &Store, config: &Config) -> Result<u64, String> {
    let mut paths: Vec<PathBuf> = PROFILE_FILES.iter().map(|name| store.dir().join(name)).collect();
    paths.push(store.dir().join("dicts"));
    paths.extend(dictionary_paths(&store.dir().join("dicts"))?);
    if !config.lexicon.is_empty() { paths.push(PathBuf::from(&config.lexicon)); }
    let mut latest = 0;
    for path in paths {
        match fs::metadata(&path) {
            Ok(meta) => {
                let stamp = meta.modified().map_err(|e| e.to_string())?
                    .duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                latest = latest.max(stamp);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("读取数据修改时间失败：{e}")),
        }
    }
    Ok(latest)
}

pub(crate) fn read(path: &Path) -> Result<PreparedBackup, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("读取全部设置备份失败：{error}"))?;
    from_text(&text)
}

/// 忽略内嵌的 `sync_records` 只校验并解析备份正文，用于同步记录不一致时的主动整份恢复。
pub(crate) fn read_ignoring_records(path: &Path) -> Result<PreparedBackup, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("读取全部设置备份失败：{error}"))?;
    let mut value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("全部设置备份格式不正确：{error}"))?;
    if let Some(object) = value.as_object_mut() {
        object.remove("sync_records");
    }
    let parts = present_parts(&mut value)?;
    let data = serde_json::from_value(value).map_err(|error| format!("全部设置备份格式不正确：{error}"))?;
    prepare(data, parts)
}

pub(crate) fn from_text(text: &str) -> Result<PreparedBackup, String> {
    let mut value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("全部设置备份格式不正确：{error}"))?;
    let parts = present_parts(&mut value)?;
    let data = serde_json::from_value(value).map_err(|error| format!("全部设置备份格式不正确：{error}"))?;
    prepare(data, parts)
}

/// 校验整份备份。没有的类别是空值，下面对各类的检查对空值都成立；学习记录没有就不读。
fn prepare(mut data: SettingsBackup, parts: Vec<BackupPart>) -> Result<PreparedBackup, String> {
    if data.sync_records.is_some() {
        crate::sync_merge::validate_snapshot(&serde_json::to_value(&data).map_err(|e| e.to_string())?)?;
    }
    if data.format != "freetype-settings" || data.version != 1 {
        return Err("不是支持的 FreeType 全部设置备份".into());
    }
    if !data.config.lexicon.is_empty() || data.config.clone().sanitized() != data.config {
        return Err("备份中的设置取值不正确".into());
    }
    for (i, phrase) in data.quick_phrases.iter_mut().enumerate() {
        phrase.code =
            msime::normalize_code(&phrase.code).ok_or_else(|| format!("第 {} 条快捷短语编码不正确", i + 1))?;
        if phrase.text.trim().is_empty() || phrase.text.contains('\0') {
            return Err(format!("第 {} 条快捷短语为空或包含空字符", i + 1));
        }
        phrase.text = phrase.text.replace("\r\n", "\n");
    }
    data.common_phrases = normalize_common_phrases(data.common_phrases)?;
    let mut names = HashSet::new();
    for dict in &data.dictionaries {
        if dictionary_name(&dict.name).as_deref() != Some(&dict.name) || !names.insert(dict.name.to_lowercase()) {
            return Err("备份中的词库名称不正确或重复".into());
        }
        let parsed = msime::parse_dictionary(&dict.text);
        crate::dictionary_catalog::metadata(&dict.text)?;
        if !parsed.errors.is_empty() || (parsed.items.is_empty() && !dict.text.starts_with("# FreeType-Dictionary: ")) {
            return Err(format!("词库「{}」有无效词条", dict.name));
        }
    }
    if data
        .recent_emoji
        .iter()
        .any(|text| text.trim().is_empty() || text.contains(['\r', '\n', '\t', '\0']))
    {
        return Err("最近表情的内容不正确".into());
    }
    if let Some(text) = &data.main_lexicon {
        if text.trim().is_empty() {
            return Err("备份中的主词库为空".into());
        }
        MemoryLexicon::validate_tsv_str(text).map_err(|error| format!("备份中的主词库不正确：{error:#}"))?;
    }
    // 用真实核心校验整份学习记录，并预先生成恢复后的数据；不改动运行中的引擎。
    let learned = Engine::new(Arc::new(MemoryLexicon::new()), Arc::new(BigramLm::default()));
    if parts.contains(&BackupPart::Learning) {
        learned
            .import_learning(&data.learning.to_string())
            .map_err(|error| format!("备份中的学习记录不正确：{error:#}"))?;
    }
    Ok(PreparedBackup { data, learned, parts })
}

/// 合并恢复时各类数据新增了多少。
#[derive(Default)]
pub(crate) struct MergeCounts {
    pub quick_phrases: usize,
    pub common_phrases: usize,
    pub dictionaries: usize,
}

impl PreparedBackup {
    /// 备份里有没有这一类数据。
    pub fn has(&self, part: BackupPart) -> bool {
        self.parts.contains(&part)
    }

    /// 写成备份文件的内容：只写有的类别。学习记录固定压成单行紧凑 JSON，避免词条和搭配分行铺满文件。
    fn text(&self) -> Result<String, String> {
        let compact_learning = if self.has(BackupPart::Learning) {
            Some(serde_json::to_string(&self.data.learning).map_err(|error| error.to_string())?)
        } else {
            None
        };
        let mut pretty = if BackupPart::ALL.into_iter().all(|part| self.has(part)) {
            let mut value = serde_json::to_value(&self.data).map_err(|error| error.to_string())?;
            if let Some(object) = value.as_object_mut() {
                object.insert("learning".into(), Value::Null);
            }
            let shell: SettingsBackup = serde_json::from_value(value).map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&shell).map_err(|error| error.to_string())?
        } else {
            let mut value = serde_json::to_value(&self.data).map_err(|error| error.to_string())?;
            if let Some(object) = value.as_object_mut() {
                if compact_learning.is_some() {
                    object.insert("learning".into(), Value::Null);
                }
                for part in BackupPart::ALL.into_iter().filter(|part| !self.has(*part)) {
                    let (required, optional) = part_fields(part);
                    for key in required.iter().chain(optional) {
                        object.remove(*key);
                    }
                }
            }
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        };
        if let Some(learning) = compact_learning {
            pretty = pretty.replacen("\n  \"learning\": null", &format!("\n  \"learning\": {learning}"), 1);
        }
        Ok(pretty + "\n")
    }

    /// 合并恢复用的设置：以本机为底，只采纳备份里和出厂默认值不同的项
    /// （那台设备上用户改过的）；备份里还是默认值的项不动本机。
    /// 主词库路径是各设备自己的，备份里总是空的，所以不会被改。
    pub fn merged_config(&self, local: &Config) -> Config {
        let values = (
            serde_json::to_value(local),
            serde_json::to_value(&self.data.config),
            serde_json::to_value(Config::default()),
        );
        let (Ok(Value::Object(mut merged)), Ok(Value::Object(remote)), Ok(Value::Object(default))) = values else {
            return local.clone();
        };
        for (key, value) in remote {
            if default.get(&key) != Some(&value) {
                merged.insert(key, value);
            }
        }
        serde_json::from_value::<Config>(Value::Object(merged)).map_or_else(|_| local.clone(), Config::sanitized)
    }

    /// 把备份里的快捷短语、常用语、导入的词库并进本机的文件：只添加，不删除。
    /// 同名词库保留本机的；同一条快捷短语（编码和内容都相同）取备份里的权重。
    pub fn merge_files(&self, store: &Store) -> Result<MergeCounts, String> {
        let mut counts = MergeCounts::default();
        let mut phrases = store.load_quick_phrases();
        let before = phrases.clone();
        let remote = self.data.quick_phrases.iter().map(|p| msime::QuickPhrase {
            code: p.code.clone(),
            text: p.text.clone(),
            weight: p.weight,
        });
        counts.quick_phrases = msime::merge_quick_phrases(&mut phrases, remote.collect());
        if phrases != before {
            store.save_quick_phrases(&phrases).map_err(|error| format!("保存快捷短语失败：{error}"))?;
        }
        let mut common = store.load_common_phrases()?;
        let before = common.len();
        common.extend(self.data.common_phrases.iter().cloned());
        let common = normalize_common_phrases(common)?;
        counts.common_phrases = common.len() - before;
        if counts.common_phrases > 0 {
            store.set_common_phrases(common)?;
        }
        let dicts = store.dir().join("dicts");
        let existing: HashSet<String> = dictionary_paths(&dicts)?
            .iter()
            .filter_map(|path| path.file_stem().and_then(|name| name.to_str()).map(str::to_lowercase))
            .collect();
        for dict in &self.data.dictionaries {
            if existing.contains(&dict.name.to_lowercase()) {
                continue;
            }
            write_atomic(&dicts.join(format!("{}.txt", dict.name)), &dict.text)
                .map_err(|error| format!("写入词库「{}」失败：{error}", dict.name))?;
            counts.dictionaries += 1;
        }
        Ok(counts)
    }

    /// 恢复以后的设置。`current` 是本机现在的设置：备份里没有的类别沿用它。
    /// 设置页的选项和主词库的位置是两类数据，各看各的。
    pub fn restored_config(&self, store: &Store, current: &Config) -> Config {
        let mut config = if self.has(BackupPart::Settings) { self.data.config.clone() } else { current.clone() };
        // 导入任何备份都不能替用户开启内容记录；当前已主动开启的状态只留本机。
        config.candidate_diagnostics = current.candidate_diagnostics;
        config.lexicon = if !self.has(BackupPart::Dictionaries) {
            current.lexicon.clone()
        } else if self.data.main_lexicon.is_some() {
            store.dir().join(MAIN_LEXICON).to_string_lossy().into()
        } else {
            String::new()
        };
        config
    }

    /// 备份里有什么（给用户看）：有的类别和条数；不是全部类别时写明不含哪些。
    pub fn report(&self) -> ImportReport {
        let words = self.learned.user_words().len();
        let (mut added, mut included, mut excluded) = (0, Vec::new(), Vec::new());
        for part in BackupPart::ALL {
            if !self.has(part) {
                excluded.push(part_name(part));
                continue;
            }
            included.push(match part {
                BackupPart::Settings => "全部设置".to_string(),
                BackupPart::Dictionaries => format!("{} 份词库", self.data.dictionaries.len()),
                BackupPart::QuickPhrases => {
                    added += self.data.quick_phrases.len();
                    format!("{} 条快捷短语", self.data.quick_phrases.len())
                }
                BackupPart::CommonPhrases => {
                    added += self.data.common_phrases.len();
                    format!("{} 条常用语", self.data.common_phrases.len())
                }
                BackupPart::Learning => {
                    added += words;
                    format!("{words} 条学到的词及使用记录")
                }
            });
        }
        let mut note = if included.is_empty() { "没有输入法数据".to_string() } else { included.join("、") };
        if !excluded.is_empty() && !included.is_empty() {
            note.push_str(&format!("；不含{}", excluded.join("、")));
        }
        ImportReport {
            added,
            note: Some(note),
            ..ImportReport::default()
        }
    }

    /// 先生成新文件，再逐一替换应用拥有的文件；失败时还原已经移动的旧文件。
    /// 只动备份里有的类别对应的文件。`current` 是本机现在的设置。
    pub fn restore_files(&self, store: &Store, current: &Config) -> Result<Option<String>, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let scratch = store
            .dir()
            .join("_temp")
            .join(format!("restore-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&scratch).map_err(|error| format!("准备恢复目录失败：{error}"))?;
        let fresh = Store::new(scratch.join("new"));
        let old = scratch.join("old");
        let result = self
            .stage(&fresh, store, current)
            .and_then(|()| self.replace_files(store, &fresh, &old));
        match result {
            Ok(()) => Ok(fs::remove_dir_all(&scratch)
                .err()
                .map(|error| format!("数据已恢复，临时备份未清理（{}）：{error}", scratch.display()))),
            Err(error) => {
                // 回滚失败时保留现场和旧数据，错误中给出位置，不把半份恢复报告成成功。
                if old.exists() {
                    return Err(format!("{error}；恢复文件保存在 {}", scratch.display()));
                }
                match fs::remove_dir_all(&scratch) {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(format!("{error}；临时文件未清理（{}）：{cleanup}", scratch.display())),
                }
            }
        }
    }

    /// 把备份里有的类别写成新文件，放在 `fresh` 里等着替换。
    fn stage(&self, fresh: &Store, store: &Store, current: &Config) -> Result<(), String> {
        let data = &self.data;
        let write = |path: PathBuf, text: String| {
            write_atomic(&path, &text).map_err(|error| format!("准备恢复文件失败：{error}"))
        };
        if self.has(BackupPart::Settings) || self.has(BackupPart::Dictionaries) {
            fresh
                .save_config(&self.restored_config(store, current))
                .map_err(|error| error.to_string())?;
        }
        if self.has(BackupPart::Settings) {
            fresh.save_mode(data.mode).map_err(|error| error.to_string())?;
            if let Some([x, y]) = data.status_bar_position {
                write(fresh.dir().join("statusbar.txt"), format!("{x},{y}\n"))?;
            }
        }
        if self.has(BackupPart::QuickPhrases) {
            let phrases: Vec<_> = data
                .quick_phrases
                .iter()
                .map(|p| msime::QuickPhrase {
                    code: p.code.clone(),
                    text: p.text.clone(),
                    weight: p.weight,
                })
                .collect();
            fresh.save_quick_phrases(&phrases).map_err(|error| error.to_string())?;
        }
        if self.has(BackupPart::CommonPhrases) {
            write(
                fresh.dir().join("common_phrases.json"),
                serde_json::to_string_pretty(&data.common_phrases).map_err(|error| error.to_string())? + "\n",
            )?;
        }
        if self.has(BackupPart::Learning) {
            fresh
                .save_recent_emoji(&data.recent_emoji)
                .map_err(|error| error.to_string())?;
            self.learned
                .save_user_dict(fresh.user_dict_path())
                .map_err(|error| error.to_string())?;
            self.learned
                .save_history(fresh.history_path())
                .map_err(|error| error.to_string())?;
        }
        if self.has(BackupPart::Dictionaries) {
            for dict in &data.dictionaries {
                write(
                    fresh.dir().join("dicts").join(format!("{}.txt", dict.name)),
                    dict.text.clone(),
                )?;
            }
            if let Some(text) = &data.main_lexicon {
                write(fresh.dir().join(MAIN_LEXICON), text.clone())?;
            }
        }
        if let Some(records) = &data.sync_records {
            write(fresh.dir().join("sync-records.json"), records.to_string())?;
        }
        Ok(())
    }

    fn replace_files(&self, store: &Store, fresh: &Store, old: &Path) -> Result<(), String> {
        // Windows 文件名不区分大小写。每个文件只替换一次，分别保留原名称和备份名称。
        // 只列备份里有的类别的文件：别的类别的文件留在原处不动。
        let mut paths: BTreeMap<String, (PathBuf, PathBuf)> = BackupPart::ALL
            .into_iter()
            .filter(|part| self.has(*part))
            .flat_map(part_files)
            .map(|name| (name.to_string(), (PathBuf::from(name), PathBuf::from(name))))
            .collect();
        if self.data.sync_records.is_some() {
            paths.insert("sync-records.json".into(), (PathBuf::from("sync-records.json"), PathBuf::from("sync-records.json")));
        }
        if self.has(BackupPart::Dictionaries) {
            for path in dictionary_paths(&store.dir().join("dicts"))? {
                let relative = path
                    .strip_prefix(store.dir())
                    .map_err(|error| error.to_string())?
                    .to_path_buf();
                paths.insert(relative.to_string_lossy().to_lowercase(), (relative.clone(), relative));
            }
            for dict in &self.data.dictionaries {
                let relative = PathBuf::from("dicts").join(format!("{}.txt", dict.name));
                let pair = paths
                    .entry(relative.to_string_lossy().to_lowercase())
                    .or_insert((relative.clone(), relative.clone()));
                pair.1 = relative;
            }
        }
        let root = fs::canonicalize(store.dir()).map_err(|error| format!("读取数据目录失败：{error}"))?;
        for (relative, _) in paths.values() {
            let path = store.dir().join(relative);
            let parent = path.parent().unwrap();
            if parent.exists()
                && !fs::canonicalize(parent)
                    .map_err(|error| error.to_string())?
                    .starts_with(&root)
            {
                return Err(format!("恢复目标超出应用数据目录：{}", path.display()));
            }
            match fs::symlink_metadata(&path) {
                Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                    return Err(format!("恢复目标不是应用的数据文件：{}", path.display()))
                }
                Ok(_) => {
                    if !fs::canonicalize(&path)
                        .map_err(|error| error.to_string())?
                        .starts_with(&root)
                    {
                        return Err(format!("恢复目标超出应用数据目录：{}", path.display()));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(format!("检查恢复目标失败：{error}")),
            }
        }
        let mut moved = Vec::new();
        let result = (|| -> io::Result<()> {
            for (relative, destination) in paths.into_values() {
                let current = store.dir().join(&relative);
                let previous = old.join(&relative);
                let incoming = fresh.dir().join(&destination);
                let target = store.dir().join(&destination);
                fs::create_dir_all(previous.parent().unwrap())?;
                fs::create_dir_all(current.parent().unwrap())?;
                if current.exists() {
                    fs::rename(&current, &previous)?;
                }
                moved.push((relative, destination));
                if incoming.exists() {
                    fs::rename(incoming, target)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            for (relative, destination) in moved.into_iter().rev() {
                let current = store.dir().join(&destination);
                let previous = old.join(&relative);
                let incoming = fresh.dir().join(&destination);
                if current.exists() {
                    fs::create_dir_all(incoming.parent().unwrap())
                        .map_err(|rollback| format!("恢复失败：{error}；回滚失败：{rollback}"))?;
                    fs::rename(&current, &incoming)
                        .map_err(|rollback| format!("恢复失败：{error}；回滚失败：{rollback}"))?;
                }
                if previous.exists() {
                    fs::rename(previous, store.dir().join(&relative))
                        .map_err(|rollback| format!("恢复失败：{error}；回滚失败：{rollback}"))?;
                }
            }
            if old.exists() {
                fs::remove_dir_all(old).map_err(|cleanup| format!("已还原原有文件，但临时目录未清理：{cleanup}"))?;
            }
            return Err(format!("恢复失败，已还原原有文件：{error}"));
        }
        Ok(())
    }
}
