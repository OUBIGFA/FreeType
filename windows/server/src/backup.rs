//! 完整设置备份。只读写应用拥有的数据文件；学习记录直接复用核心的备份格式。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use freetype_protocol::{Config, ImportReport, Mode, QuickPhrase};
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
}

pub(crate) struct PreparedBackup {
    pub data: SettingsBackup,
    learned: Engine,
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

pub(crate) fn export(
    store: &Store,
    config: &Config,
    mode: Mode,
    recent_emoji: &[String],
    engine: &Engine,
    path: &Path,
) -> Result<ImportReport, String> {
    let quick = optional_text(&store.dir().join("quick_phrases.txt"))?.unwrap_or_default();
    let parsed = msime::parse_quick_phrases(&quick);
    if !parsed.errors.is_empty() {
        return Err("快捷短语文件有错误，请先修正后再备份".into());
    }
    let quick_phrases = parsed
        .items
        .into_iter()
        .map(|p| QuickPhrase {
            code: p.code,
            text: p.text,
            weight: p.weight,
        })
        .collect();
    let mut dictionaries = Vec::new();
    for file in dictionary_paths(&store.dir().join("dicts"))? {
        let name = file
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or("词库文件名不是有效文字")?
            .to_string();
        let text = fs::read_to_string(&file).map_err(|error| format!("读取词库失败：{error}"))?;
        dictionaries.push(BackupDictionary { name, text });
    }
    let main_lexicon = if config.lexicon.is_empty() {
        None
    } else {
        Some(fs::read_to_string(&config.lexicon).map_err(|error| format!("读取自定义主词库失败：{error}"))?)
    };
    let mut config = config.clone();
    config.lexicon.clear();
    let status_bar_position = optional_text(&store.dir().join("statusbar.txt"))?
        .map(|text| {
            let (x, y) = text.trim().split_once(',').ok_or("悬浮状态栏位置格式不正确")?;
            Ok::<_, String>([
                x.trim().parse().map_err(|_| "悬浮状态栏横坐标不正确")?,
                y.trim().parse().map_err(|_| "悬浮状态栏纵坐标不正确")?,
            ])
        })
        .transpose()?;
    let learning = serde_json::from_str(
        &engine
            .export_learning()
            .map_err(|error| format!("读取学习记录失败：{error:#}"))?,
    )
    .map_err(|error| error.to_string())?;
    let data = SettingsBackup {
        format: "freetype-settings".into(),
        version: 1,
        config,
        mode,
        quick_phrases,
        common_phrases: store.load_common_phrases()?,
        dictionaries,
        learning,
        recent_emoji: recent_emoji.to_vec(),
        status_bar_position,
        main_lexicon,
    };
    let prepared = prepare(data)?;
    let text = serde_json::to_string_pretty(&prepared.data).map_err(|error| error.to_string())? + "\n";
    write_atomic(path, &text).map_err(|error| format!("写入全部设置备份失败：{error}"))?;
    Ok(prepared.report())
}

pub(crate) fn read(path: &Path) -> Result<PreparedBackup, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("读取全部设置备份失败：{error}"))?;
    let data = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("全部设置备份格式不正确：{error}"))?;
    prepare(data)
}

fn prepare(mut data: SettingsBackup) -> Result<PreparedBackup, String> {
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
        if !parsed.errors.is_empty() || parsed.items.is_empty() {
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
        MemoryLexicon::from_tsv_str(text).map_err(|error| format!("备份中的主词库不正确：{error:#}"))?;
    }
    // 用真实核心校验整份学习记录，并预先生成恢复后的数据；不改动运行中的引擎。
    let learned = Engine::new(Arc::new(MemoryLexicon::new()), Arc::new(BigramLm::default()));
    learned
        .import_learning(&data.learning.to_string())
        .map_err(|error| format!("备份中的学习记录不正确：{error:#}"))?;
    Ok(PreparedBackup { data, learned })
}

impl PreparedBackup {
    pub fn restored_config(&self, store: &Store) -> Config {
        let mut config = self.data.config.clone();
        if self.data.main_lexicon.is_some() {
            config.lexicon = store.dir().join(MAIN_LEXICON).to_string_lossy().into();
        }
        config
    }

    pub fn report(&self) -> ImportReport {
        let words = self.learned.user_words().len();
        ImportReport {
            added: self.data.quick_phrases.len() + self.data.common_phrases.len() + words,
            note: Some(format!(
                "全部设置、{} 份词库、{} 条快捷短语、{} 条常用语、{} 条学到的词及使用记录",
                self.data.dictionaries.len(),
                self.data.quick_phrases.len(),
                self.data.common_phrases.len(),
                words
            )),
            ..ImportReport::default()
        }
    }

    /// 先生成新文件，再逐一替换应用拥有的文件；失败时还原已经移动的旧文件。
    pub fn restore_files(&self, store: &Store) -> Result<Option<String>, String> {
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
            .stage(&fresh, store)
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

    fn stage(&self, fresh: &Store, store: &Store) -> Result<(), String> {
        let data = &self.data;
        let phrases: Vec<_> = data
            .quick_phrases
            .iter()
            .map(|p| msime::QuickPhrase {
                code: p.code.clone(),
                text: p.text.clone(),
                weight: p.weight,
            })
            .collect();
        let write = |path: PathBuf, text: String| {
            write_atomic(&path, &text).map_err(|error| format!("准备恢复文件失败：{error}"))
        };
        fresh
            .save_config(&self.restored_config(store))
            .map_err(|error| error.to_string())?;
        fresh.save_mode(data.mode).map_err(|error| error.to_string())?;
        fresh.save_quick_phrases(&phrases).map_err(|error| error.to_string())?;
        write(
            fresh.dir().join("common_phrases.json"),
            serde_json::to_string_pretty(&data.common_phrases).map_err(|error| error.to_string())? + "\n",
        )?;
        fresh
            .save_recent_emoji(&data.recent_emoji)
            .map_err(|error| error.to_string())?;
        self.learned
            .save_user_dict(fresh.user_dict_path())
            .map_err(|error| error.to_string())?;
        self.learned
            .save_history(fresh.history_path())
            .map_err(|error| error.to_string())?;
        for dict in &data.dictionaries {
            write(
                fresh.dir().join("dicts").join(format!("{}.txt", dict.name)),
                dict.text.clone(),
            )?;
        }
        if let Some(text) = &data.main_lexicon {
            write(fresh.dir().join(MAIN_LEXICON), text.clone())?;
        }
        if let Some([x, y]) = data.status_bar_position {
            write(fresh.dir().join("statusbar.txt"), format!("{x},{y}\n"))?;
        }
        Ok(())
    }

    fn replace_files(&self, store: &Store, fresh: &Store, old: &Path) -> Result<(), String> {
        // Windows 文件名不区分大小写。每个文件只替换一次，分别保留原名称和备份名称。
        let mut paths: BTreeMap<String, (PathBuf, PathBuf)> = PROFILE_FILES
            .iter()
            .map(|name| (name.to_string(), (PathBuf::from(name), PathBuf::from(name))))
            .collect();
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
