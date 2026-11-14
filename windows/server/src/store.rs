//! 用户数据目录里的文件：配置、快捷短语、导入的词库、用户词库和输入习惯。
//!
//! 目录固定为 `%APPDATA%\FreeType`（测试和开发用 `FREETYPE_DATA_DIR` 换掉）：
//!
//! | 文件 | 内容 |
//! | --- | --- |
//! | `config.json` | 设置 |
//! | `mode.json` | 全局模式下最后主动选择的中文/英文状态 |
//! | `quick_phrases.txt` | 快捷短语，水杉格式 `编码<Tab>短语<Tab>权重` |
//! | `common_phrases.json` | 工具栏常用语，JSON 数组保留多行内容 |
//! | `dicts\*.txt` | 导入的词库，水杉格式 `词语<Tab>拼音<Tab>权重` |
//! | `user.tsv`、`history.tsv` | 引擎学到的词和输入习惯 |
//! | `emoji-recent.txt` | 最近用过的表情，一行一个 |
//!
//! 悬浮状态栏的位置 `statusbar.txt` 由 [`crate::desktop`] 读写。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use freetype_protocol::{Config, Dictionary, ImportReport, Mode, QuickPhrase};
use wowtype_core::msime::{self, DictEntry, LineError, Parsed};

use crate::dictfmt;

/// 报告里最多列多少条出错的行。
const MAX_REPORTED_ERRORS: usize = 20;

pub struct Store {
    dir: PathBuf,
}

/// 词库名：去掉路径和扩展名，只留能做文件名的字符。
pub fn dictionary_name(name: &str) -> Option<String> {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control())
        .collect();
    let name = name.trim_end_matches(['.', ' ']).to_string();
    let reserved = {
        let upper = name.to_ascii_uppercase();
        let stem = upper.split('.').next().unwrap_or("");
        matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
    };
    (!name.is_empty() && name.chars().count() <= 64 && !reserved).then_some(name)
}

fn report(added: usize, errors: &[LineError]) -> ImportReport {
    ImportReport {
        added,
        skipped: errors.len(),
        errors: errors
            .iter()
            .take(MAX_REPORTED_ERRORS)
            .map(|e| {
                if e.line == 0 {
                    e.message.clone()
                } else {
                    format!("第 {} 行：{}", e.line, e.message)
                }
            })
            .collect(),
        note: None,
    }
}

/// 写到临时文件再改名，写一半断电不会留下坏文件。
pub(crate) fn write_atomic(path: &Path, data: &str) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let filename = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "缺少文件名"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // 导出目录属于用户，临时文件必须独占创建，不能占用已有的同名 .tmp 文件。
    let (tmp, mut file) = loop {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let mut name = std::ffi::OsString::from(".");
        name.push(filename);
        name.push(format!(".{}.{serial}.tmp", std::process::id()));
        let tmp = path.with_file_name(name);
        match fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = file.write_all(data.as_bytes()).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| fs::rename(&tmp, path));
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_file(&tmp) {
            return Err(io::Error::new(error.kind(), format!("{error}；清理临时文件 {} 失败：{cleanup}", tmp.display())));
        }
        return Err(error);
    }
    Ok(())
}

fn read_text(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    String::from_utf8(bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "文件不是 UTF-8 编码"))
}

/// 常用语完整校验后再保存；统一换行并去重，不丢掉短语内部的空白。
pub(crate) fn normalize_common_phrases(phrases: Vec<String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (i, text) in phrases.into_iter().enumerate() {
        let text = text.replace("\r\n", "\n");
        if text.trim().is_empty() || text.contains('\0') {
            return Err(format!("第 {} 条：常用语不能为空，也不能包含空字符", i + 1));
        }
        if !out.contains(&text) {
            out.push(text);
        }
    }
    Ok(out)
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn user_dict_path(&self) -> PathBuf {
        self.dir.join("user.tsv")
    }

    pub fn history_path(&self) -> PathBuf {
        self.dir.join("history.tsv")
    }

    fn config_path(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    fn quick_phrase_path(&self) -> PathBuf {
        self.dir.join("quick_phrases.txt")
    }

    fn dict_dir(&self) -> PathBuf {
        self.dir.join("dicts")
    }

    /// 读设置；没有文件或坏了用默认值（坏文件不覆盖，等用户在设置里保存）。
    pub fn load_config(&self) -> Config {
        fs::read_to_string(self.config_path())
            .ok()
            .and_then(|text| serde_json::from_str::<Config>(&text).ok())
            .unwrap_or_default()
            .sanitized()
    }

    pub fn save_config(&self, config: &Config) -> io::Result<()> {
        let text = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
        write_atomic(&self.config_path(), &(text + "\n"))
    }

    /// 运行状态单独保存，设置窗口整份写回配置时不会覆盖最近一次切换。
    pub fn load_mode(&self) -> Mode {
        fs::read_to_string(self.dir.join("mode.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_mode(&self, mode: Mode) -> io::Result<()> {
        let text = serde_json::to_string(&mode).map_err(io::Error::other)?;
        write_atomic(&self.dir.join("mode.json"), &(text + "\n"))
    }

    fn recent_emoji_path(&self) -> PathBuf {
        self.dir.join("emoji-recent.txt")
    }

    /// 最近用过的 emoji，最近的在前（一行一个）。
    pub fn load_recent_emoji(&self) -> Vec<String> {
        read_text(&self.recent_emoji_path())
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn save_recent_emoji(&self, emoji: &[String]) -> io::Result<()> {
        write_atomic(&self.recent_emoji_path(), &(emoji.join("\n") + "\n"))
    }

    pub fn load_quick_phrases(&self) -> Vec<msime::QuickPhrase> {
        read_text(&self.quick_phrase_path())
            .map(|text| msime::parse_quick_phrases(&text).items)
            .unwrap_or_default()
    }

    pub fn save_quick_phrases(&self, phrases: &[msime::QuickPhrase]) -> io::Result<()> {
        write_atomic(&self.quick_phrase_path(), &msime::format_quick_phrases(phrases))
    }

    pub fn list_quick_phrases(&self) -> Vec<QuickPhrase> {
        self.load_quick_phrases()
            .into_iter()
            .map(|p| QuickPhrase {
                code: p.code,
                text: p.text,
                weight: p.weight,
            })
            .collect()
    }

    /// 整份替换；编码不合法或短语为空的条目报错，一条都不写。
    pub fn set_quick_phrases(&self, phrases: &[QuickPhrase]) -> Result<(), String> {
        let mut out = Vec::new();
        for (i, p) in phrases.iter().enumerate() {
            let Some(code) = msime::normalize_code(&p.code) else {
                return Err(format!("第 {} 条：编码只能包含英文字母", i + 1));
            };
            if p.text.trim().is_empty() {
                return Err(format!("第 {} 条：短语不能为空", i + 1));
            }
            out.push(msime::QuickPhrase {
                code,
                text: p.text.replace("\r\n", "\n"),
                weight: p.weight,
            });
        }
        self.save_quick_phrases(&out).map_err(|e| format!("保存快捷短语失败：{e}"))
    }

    /// 导入快捷短语：水杉、Rime 格式，或搜狗自定义短语、微信输入法常用语导出的 `编码,位置=短语`。
    pub fn import_quick_phrases(&self, path: &Path, replace: bool) -> Result<ImportReport, String> {
        let bytes = fs::read(path).map_err(|e| format!("读取文件失败：{e}"))?;
        let parsed = dictfmt::parse_quick_phrases(&bytes);
        let mut phrases = if replace {
            Vec::new()
        } else {
            self.load_quick_phrases()
        };
        let added = msime::merge_quick_phrases(&mut phrases, parsed.items);
        self.save_quick_phrases(&phrases)
            .map_err(|e| format!("保存快捷短语失败：{e}"))?;
        Ok(report(added, &parsed.errors))
    }

    pub fn export_quick_phrases(&self, path: &Path) -> Result<usize, String> {
        let phrases = self.load_quick_phrases();
        fs::write(path, msime::format_quick_phrases(&phrases)).map_err(|e| format!("写入文件失败：{e}"))?;
        Ok(phrases.len())
    }

    pub fn load_common_phrases(&self) -> Result<Vec<String>, String> {
        let path = self.dir.join("common_phrases.json");
        let text = match read_text(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("读取常用语失败：{e}")),
        };
        let phrases = serde_json::from_str(&text).map_err(|e| format!("常用语文件格式错误：{e}"))?;
        normalize_common_phrases(phrases)
    }

    pub fn set_common_phrases(&self, phrases: Vec<String>) -> Result<(), String> {
        let phrases = normalize_common_phrases(phrases)?;
        let text = serde_json::to_string_pretty(&phrases).map_err(|e| e.to_string())?;
        write_atomic(&self.dir.join("common_phrases.json"), &(text + "\n"))
            .map_err(|e| format!("保存常用语失败：{e}"))
    }

    /// JSON 保留多行短语；纯文本按一行一条导入，空行不作短语。
    pub fn import_common_phrases(&self, path: &Path, replace: bool) -> Result<ImportReport, String> {
        let text = read_text(path).map_err(|e| format!("读取常用语失败：{e}"))?;
        let text = text.trim_start_matches('\u{feff}').trim();
        if text.is_empty() {
            return Err("文件里没有常用语".into());
        }
        let incoming = if text.starts_with(['{', '[']) {
            let data: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("常用语备份格式错误：{e}"))?;
            let phrases = if data.is_array() {
                data
            } else {
                if data["format"] != "freetype-common-phrases" || data["version"] != 1 {
                    return Err("不是支持的 FreeType 常用语备份".into());
                }
                data["phrases"].clone()
            };
            serde_json::from_value(phrases).map_err(|e| format!("常用语列表格式错误：{e}"))?
        } else {
            text.lines().filter(|line| !line.trim().is_empty()).map(str::to_string).collect()
        };
        let incoming = normalize_common_phrases(incoming)?;
        let mut phrases = if replace { Vec::new() } else { self.load_common_phrases()? };
        let before = phrases.len();
        for phrase in incoming {
            if !phrases.contains(&phrase) {
                phrases.push(phrase);
            }
        }
        let added = phrases.len() - before;
        self.set_common_phrases(phrases)?;
        Ok(ImportReport { added, ..ImportReport::default() })
    }

    pub fn export_common_phrases(&self, path: &Path) -> Result<usize, String> {
        let phrases = self.load_common_phrases()?;
        let text = serde_json::to_string_pretty(&serde_json::json!({
            "format": "freetype-common-phrases", "version": 1, "phrases": phrases,
        })).map_err(|e| e.to_string())?;
        write_atomic(path, &(text + "\n")).map_err(|e| format!("写入常用语备份失败：{e}"))?;
        Ok(phrases.len())
    }

    /// 导入词库（搜狗细胞词库、搜狗文本词库、水杉、Rime、只有词语的词表都认，见 [`dictfmt`]），
    /// 按水杉格式规整地写进 `dicts\<name>.txt`（同名覆盖）。`reading` 给只有词语的行注音。
    pub fn import_dictionary(
        &self,
        path: &Path,
        name: &str,
        reading: &dyn Fn(&[String]) -> Box<dyn dictfmt::Reading>,
    ) -> Result<ImportReport, String> {
        let name = dictionary_name(name).ok_or("词库名称不能为空，也不能是系统保留名")?;
        let bytes = fs::read(path).map_err(|e| format!("读取文件失败：{e}"))?;
        let imported = dictfmt::parse_dictionary(&bytes, reading);
        let mut report = self.save_dictionary(&name, imported.parsed)?;
        let mut note = format!("识别为{}", imported.format.name());
        if imported.annotated > 0 {
            note.push_str(&format!("；{} 条只有词语，已按常用读音自动注音（多音字可能不准）", imported.annotated));
        }
        report.note = Some(note);
        Ok(report)
    }

    /// 把条目写成 `dicts\<name>.txt`。
    fn save_dictionary(&self, name: &str, parsed: Parsed<DictEntry>) -> Result<ImportReport, String> {
        if parsed.items.is_empty() {
            let mut message = String::from("文件里没有可用的词条");
            if let Some(first) = parsed.errors.first() {
                message.push_str(&format!("（第 {} 行：{}）", first.line, first.message));
            }
            return Err(message);
        }
        write_atomic(
            &self.dict_dir().join(format!("{name}.txt")),
            &msime::format_dictionary(&parsed.items),
        )
        .map_err(|e| format!("保存词库失败：{e}"))?;
        Ok(report(parsed.items.len(), &parsed.errors))
    }

    pub fn remove_dictionary(&self, name: &str) -> Result<(), String> {
        let name = dictionary_name(name).ok_or("词库名称无效")?;
        fs::remove_file(self.dict_dir().join(format!("{name}.txt"))).map_err(|e| format!("删除词库失败：{e}"))
    }

    /// 全部导入的词库，按名字排。
    pub fn load_dictionaries(&self) -> Vec<(String, Vec<msime::DictEntry>)> {
        let Ok(dir) = fs::read_dir(self.dict_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<(String, Vec<msime::DictEntry>)> = dir
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                let text = read_text(&path).ok()?;
                Some((name, msime::parse_dictionary(&text).items))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn list_dictionaries(&self) -> Vec<Dictionary> {
        self.load_dictionaries()
            .into_iter()
            .map(|(name, entries)| Dictionary {
                name,
                entries: entries.len(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("freetype-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::new(dir)
    }

    #[test]
    fn config_defaults_and_round_trip() {
        let store = temp_store("config");
        assert_eq!(store.load_config(), Config::default());
        let mut config = Config::default();
        config.page_size = 9;
        store.save_config(&config).unwrap();
        assert_eq!(store.load_config().page_size, 9);
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn atomic_export_preserves_existing_sibling_files() {
        let store = temp_store("atomic-siblings");
        fs::create_dir_all(store.dir()).unwrap();
        let target = store.dir().join("backup.json");
        let sibling = store.dir().join("backup.tmp");
        fs::write(&target, "old backup").unwrap();
        fs::write(&sibling, "unrelated user file").unwrap();

        write_atomic(&target, "new backup").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new backup");
        assert_eq!(fs::read_to_string(&sibling).unwrap(), "unrelated user file");
        assert_eq!(fs::read_dir(store.dir()).unwrap().count(), 2);
    }

    #[test]
    fn failed_atomic_export_preserves_target_and_cleans_only_its_scratch() {
        let store = temp_store("atomic-failure");
        let target = store.dir().join("backup.json");
        fs::create_dir_all(&target).unwrap();
        let sibling = store.dir().join("backup.tmp");
        fs::write(&sibling, "unrelated user file").unwrap();

        assert!(write_atomic(&target, "new backup").is_err());

        assert!(target.is_dir());
        assert_eq!(fs::read_to_string(&sibling).unwrap(), "unrelated user file");
        assert_eq!(fs::read_dir(store.dir()).unwrap().count(), 2);
    }

    #[test]
    fn quick_phrase_import_merges_and_exports_msime_format() {
        let store = temp_store("qp");
        let file = store.dir().with_extension("in.txt");
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(&file, "\u{feff}dz\t四川省\\n宜宾市\t20\r\nbad1\tx\r\n").unwrap();
        let report = store.import_quick_phrases(&file, false).unwrap();
        assert_eq!((report.added, report.skipped), (1, 1));
        assert_eq!(report.errors, vec!["第 2 行：编码只能包含英文字母，短语不能为空"]);
        assert_eq!(store.list_quick_phrases()[0].text, "四川省\n宜宾市");
        let again = store.import_quick_phrases(&file, false).unwrap();
        assert_eq!(again.added, 0, "同一条不重复加");
        let out = store.dir().with_extension("out.txt");
        assert_eq!(store.export_quick_phrases(&out).unwrap(), 1);
        assert_eq!(fs::read_to_string(&out).unwrap(), "dz\t四川省\\n宜宾市\t20\n");
        for path in [file, out] {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn dictionary_import_list_remove() {
        let store = temp_store("dict");
        fs::create_dir_all(store.dir()).unwrap();
        let file = store.dir().join("src.txt");
        fs::write(&file, "# 人名\n韵酒\tyun jiu\t500\n坏\n").unwrap();
        let report = store.import_dictionary(&file, "我的词库", &|_| Box::new(crate::dictfmt::LexiconReading::default())).unwrap();
        assert_eq!((report.added, report.skipped), (1, 1));
        assert_eq!(
            store.list_dictionaries(),
            vec![Dictionary {
                name: "我的词库".into(),
                entries: 1
            }]
        );
        assert!(store.import_dictionary(&file, "CON", &|_| Box::new(crate::dictfmt::LexiconReading::default())).is_err());
        store.remove_dictionary("我的词库").unwrap();
        assert!(store.list_dictionaries().is_empty());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn dictionary_names_are_safe_file_names() {
        assert_eq!(dictionary_name(" a/b:c. ").as_deref(), Some("abc"));
        assert_eq!(dictionary_name("com1"), None);
        assert_eq!(dictionary_name(""), None);
    }
}
