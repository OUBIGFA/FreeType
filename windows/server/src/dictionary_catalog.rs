//! 导入词库的来源管理。元信息与词条放在同一个可原子替换的文本文件中，
//! 现有备份/恢复保留原文；外部输入法把这一行视为注释，不改变公开词库格式。
use std::{collections::HashSet, fs, path::PathBuf};
use serde::{Deserialize, Serialize};
use freetype_protocol::{Dictionary, DictionaryEntry, DictionaryPage};
use wowtype_core::msime::{self, DictEntry};
use crate::store::{dictionary_name, write_atomic, Store};

const HEADER: &str = "# FreeType-Dictionary: ";

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Metadata {
    pub id: String,
    pub enabled: bool,
    pub source: String,
    pub description: String,
}

impl Default for Metadata {
    fn default() -> Self { Self { id: String::new(), enabled: true, source: String::new(), description: String::new() } }
}

pub(crate) fn source_id(meta: &Metadata, name: &str) -> String {
    if meta.id.is_empty() { format!("legacy:{}", name.to_lowercase()) } else { meta.id.clone() }
}

pub(crate) fn metadata(text: &str) -> Result<Metadata, String> {
    let meta: Metadata = match text.lines().next().and_then(|line| line.strip_prefix(HEADER)) {
        Some(json) => serde_json::from_str(json).map_err(|error| format!("词库来源信息损坏：{error}")),
        None => Ok(Metadata::default()),
    }?;
    if meta.id.chars().count() > 160 || meta.source.chars().count() > 1024 || meta.description.chars().count() > 500
        || meta.id.contains('\0') || meta.source.contains('\0') || meta.description.contains('\0') {
        return Err("词库来源信息长度或内容无效".into());
    }
    Ok(meta)
}

impl Store {
    fn dictionary_path(&self, name: &str) -> Result<PathBuf, String> {
        if dictionary_name(name).as_deref() != Some(name) { return Err("词库名称无效".into()); }
        Ok(self.dir().join("dicts").join(format!("{name}.txt")))
    }

    pub(crate) fn dictionary_metadata(&self, name: &str) -> Result<Metadata, String> {
        match fs::read_to_string(self.dictionary_path(name)?) {
            Ok(text) => metadata(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let tick = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
                Ok(Metadata { id: format!("{:x}-{:x}", tick, std::process::id()), ..Metadata::default() })
            }
            Err(error) => Err(format!("读取词库失败：{error}")),
        }
    }

    pub(crate) fn write_dictionary(&self, name: &str, meta: &Metadata, entries: &[DictEntry]) -> Result<(), String> {
        let mut meta = meta.clone();
        meta.id = source_id(&meta, name);
        let text = format!("{HEADER}{}\n{}", serde_json::to_string(&meta).map_err(|e| e.to_string())?, msime::format_dictionary(entries));
        write_atomic(&self.dictionary_path(name)?, &text).map_err(|e| format!("保存词库失败：{e}"))
    }

    pub fn query_dictionary(&self, name: &str, query: &str, offset: usize, limit: usize) -> Result<DictionaryPage, String> {
        let text = fs::read_to_string(self.dictionary_path(name)?).map_err(|e| format!("读取词库失败：{e}"))?;
        let meta = metadata(&text)?;
        let parsed = msime::parse_dictionary(&text);
        let dictionary = Dictionary { name: name.into(), entries: parsed.items.len(), enabled: meta.enabled, source: meta.source, description: meta.description };
        let query = query.trim().to_lowercase();
        let code = |s: &str| s.chars().filter(|c| !c.is_whitespace() && *c != '\'').collect::<String>().to_lowercase();
        let reading = code(&query);
        let matches: Vec<_> = parsed.items.into_iter().filter(|e| query.is_empty() || e.word.to_lowercase().contains(&query) || code(&e.pinyin).contains(&reading)).collect();
        let total = matches.len();
        let offset = offset.min(total);
        let entries = matches.into_iter().skip(offset).take(limit.clamp(1, 200))
            .map(|e| DictionaryEntry { word: e.word, pinyin: e.pinyin, weight: e.weight }).collect();
        Ok(DictionaryPage { dictionary, total, offset, entries })
    }

    pub fn update_dictionary(&self, name: &str, new_name: &str, enabled: bool, description: &str) -> Result<(), String> {
        let old = self.dictionary_path(name)?;
        let new = self.dictionary_path(new_name)?;
        if description.chars().count() > 500 || description.contains('\0') { return Err("词库说明最多 500 字，不能包含空字符".into()); }
        let original = fs::read_to_string(&old).map_err(|e| format!("读取词库失败：{e}"))?;
        let mut meta = metadata(&original)?;
        meta.id = source_id(&meta, name);
        meta.enabled = enabled;
        meta.description = description.trim().into();
        if name != new_name {
            if !name.eq_ignore_ascii_case(new_name) && new.exists() { return Err("已有同名词库，请换一个名称".into()); }
            fs::rename(&old, &new).map_err(|e| format!("重命名词库失败：{e}"))?;
        }
        if let Err(error) = self.write_dictionary(new_name, &meta, &msime::parse_dictionary(&original).items) {
            if name != new_name { fs::rename(&new, &old).map_err(|rollback| format!("{error}；还原名称失败：{rollback}"))?; }
            return Err(error);
        }
        Ok(())
    }

    pub fn delete_dictionary_entries(&self, name: &str, selected: &[DictionaryEntry]) -> Result<usize, String> {
        let text = fs::read_to_string(self.dictionary_path(name)?).map_err(|e| format!("读取词库失败：{e}"))?;
        let meta = metadata(&text)?;
        let selected: HashSet<_> = selected.iter().map(|entry| (entry.pinyin.as_str(), entry.word.as_str())).collect();
        let mut entries = msime::parse_dictionary(&text).items;
        let before = entries.len();
        entries.retain(|entry| !selected.contains(&(entry.pinyin.as_str(), entry.word.as_str())));
        let removed = before - entries.len();
        self.write_dictionary(name, &meta, &entries)?;
        Ok(removed)
    }
}
