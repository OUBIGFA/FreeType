//! 同步按记录保存版本和删除标记。完整备份仍可独立恢复；没有版本的旧备份只合并新增。
//! 版本比较不依赖上传顺序，重放同一快照不累加学习次数，也不复活删除项。
use std::{collections::BTreeMap, fs};
use freetype_protocol::{Config, SyncMergePreview};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wowtype_core::msime;
use crate::{dictionary_catalog::{metadata, source_id, Metadata}, store::{write_atomic, Store}};

const JOURNAL: &str = "sync-records.json";
const MAX_RECORDS: usize = 2_000_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    modified: u64,
    origin: String,
    // None is a retained deletion marker; it must survive a merge with old archives.
    value: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    records: BTreeMap<String, Record>,
}

fn key(parts: &[&str]) -> String { serde_json::to_string(parts).expect("字符串数组可序列化") }
fn local_config(name: &str) -> bool { matches!(name, "candidate_diagnostics" | "lexicon" | "default_mode" | "mode_scope" | "app_presets" | "app_preset_rules" | "vim_mode_apps") }
fn error(error: impl std::fmt::Display) -> String { error.to_string() }

fn config_defaults() -> Result<Value, String> {
    let mut defaults = serde_json::to_value(Config::default()).map_err(error)?;
    // 普通配置省略空图像；同步需要显式的空值，才能传播“恢复默认图标”。
    defaults["custom_brand_icon"] = json!("");
    Ok(defaults)
}

/// 字典的来源设置与每个词条分别比较，两个设备编辑同一份导入词库也能保留独立新增。
fn rows(snapshot: &Value) -> Result<BTreeMap<String, Value>, String> {
    let mut rows = BTreeMap::new();
    let mut config = snapshot["config"].as_object().ok_or("缺少设置")?.clone();
    config.entry("custom_brand_icon").or_insert(json!(""));
    for (name, value) in &config {
        if !local_config(name) { rows.insert(key(&["config", name]), value.clone()); }
    }
    for phrase in snapshot["quick_phrases"].as_array().ok_or("缺少快捷短语")? {
        rows.insert(key(&["quick", phrase["code"].as_str().ok_or("短语编码无效")?, phrase["text"].as_str().ok_or("短语正文无效")?]), phrase.clone());
    }
    for phrase in snapshot["common_phrases"].as_array().ok_or("缺少常用语")? {
        rows.insert(key(&["common", phrase.as_str().ok_or("常用语无效")?]), phrase.clone());
    }
    for dict in snapshot["dictionaries"].as_array().ok_or("缺少词库")? {
        let name = dict["name"].as_str().ok_or("词库名称无效")?;
        let text = dict["text"].as_str().ok_or("词库内容无效")?;
        let mut meta = metadata(text)?;
        meta.id = source_id(&meta, name);
        let id = meta.id.clone();
        if rows.insert(key(&["dictionary", &id]), json!({"name":name,"metadata":meta})).is_some() { return Err("备份中存在重复的词库来源标识".into()); }
        for entry in msime::parse_dictionary(text).items {
            rows.insert(key(&["entry", &id, &entry.pinyin, &entry.word]), json!(entry.weight));
        }
    }
    for (domain, indices) in [("words", &[1][..]), ("history", &[0][..]), ("pairs", &[0, 1][..])] {
        for row in snapshot["learning"][domain].as_array().ok_or("学习记录不完整")? {
            let mut parts = vec![domain];
            for &i in indices { parts.push(row[i].as_str().ok_or("学习记录标识无效")?); }
            rows.insert(key(&parts), row.clone());
        }
    }
    // 最近表情按列表整体更新，避免并集合并把已清理的顺序变成字典序。
    for domain in ["boundaries","pending"] {
        if let Some(records)=snapshot["learning"][domain].as_object() {
            for (reading,value) in records {rows.insert(key(&[domain,reading]),value.clone());}
        }
    }
    rows.insert(key(&["recent"]), snapshot["recent_emoji"].clone());
    if rows.len() > MAX_RECORDS { return Err("同步记录过多".into()); }
    Ok(rows)
}

fn read_journal(store: &Store) -> Result<Journal, String> {
    match fs::read_to_string(store.dir().join(JOURNAL)) {
        Ok(text) => { let journal = serde_json::from_str(&text).map_err(error)?; validate(&journal)?; Ok(journal) }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Journal { version: 1, ..Journal::default() }),
        Err(e) => Err(format!("读取同步记录失败：{e}")),
    }
}

fn validate(journal: &Journal) -> Result<(), String> {
    if journal.version != 1 || journal.records.len() > MAX_RECORDS { return Err("同步记录版本或数量不正确".into()); }
    let defaults = config_defaults()?;
    for (name, record) in &journal.records {
        let parts: Vec<String> = serde_json::from_str(name).map_err(error)?;
        let valid = match parts.first().map(String::as_str) {
            Some("config") => parts.len() == 2 && defaults.get(&parts[1]).is_some() && !local_config(&parts[1]),
            Some("dictionary" | "common" | "words" | "history" | "boundaries" | "pending") => parts.len() == 2,
            Some("quick" | "pairs") => parts.len() == 3,
            Some("entry") => parts.len() == 4,
            Some("recent") => parts.len() == 1,
            _ => false,
        };
        if !valid || record.origin.len() != 32 || !record.origin.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("同步记录标识不正确".into());
        }
    }
    Ok(())
}

/// 导出时将当前内存快照与上次已知记录比较；未变化的值保持版本，不把下载时间算作编辑。
pub(crate) fn attach(store: &Store, snapshot: &mut Value, initial_stamp: u64, user_restore: bool) -> Result<(), String> {
    let current = rows(snapshot)?;
    let mut journal = read_journal(store)?;
    let device_path = store.dir().join("sync-record-device.txt");
    let device = match fs::read_to_string(&device_path) {
        Ok(text) if text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()) => text,
        Ok(_) => return Err("同步设备记录损坏".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // 本机标识不编码路径或账号；纳秒时钟和进程组成持久唯一值。
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(error)?.as_nanos();
            let text = format!("{:032x}", nanos ^ ((std::process::id() as u128) << 96));
            write_atomic(&device_path, &text).map_err(error)?;
            text
        }
        Err(e) => return Err(format!("读取同步设备记录失败：{e}")),
    };
    let first = journal.records.is_empty();
    let stamp = if first { initial_stamp.max(1) } else { wowtype_core::now_ms().max(journal.records.values().map(|r| r.modified).max().unwrap_or(0).saturating_add(1)) };
    let defaults = config_defaults()?;
    for (name, value) in &current {
        if journal.records.get(name).and_then(|record| record.value.as_ref()) == Some(value) { continue; }
        let parts: Vec<String> = serde_json::from_str(name).map_err(error)?;
        let default = first && !user_restore && parts.first().map(String::as_str) == Some("config") && defaults.get(&parts[1]) == Some(value);
        let learned_at = match parts[0].as_str() {
            "history" => value.get(2).and_then(Value::as_u64),
            "pairs" => value.get(3).and_then(Value::as_u64),
            "words" => current.get(&key(&["history", &parts[1]])).and_then(|history| history.get(2)).and_then(Value::as_u64),
            _ => None,
        };
        // 手动恢复是用户此刻做的新选择；正常学习沿用实际选词时间。
        let modified = if default { 0 } else if user_restore { stamp } else { learned_at.filter(|time| *time > 0).unwrap_or(stamp) };
        journal.records.insert(name.clone(), Record { modified, origin: device.clone(), value: Some(value.clone()) });
    }
    for (name, record) in &mut journal.records {
        if !current.contains_key(name) && record.value.is_some() {
            *record = Record { modified: stamp, origin: device.clone(), value: None };
        }
    }
    validate(&journal)?;
    let text = serde_json::to_string(&journal).map_err(error)?;
    if fs::read_to_string(store.dir().join(JOURNAL)).ok().as_deref() != Some(text.as_str()) {
        write_atomic(&store.dir().join(JOURNAL), &text).map_err(error)?;
    }
    snapshot["sync_records"] = serde_json::to_value(journal).map_err(error)?;
    Ok(())
}

/// 清理已损坏或不一致的本机同步记录，并按当前数据重建基线。
pub(crate) fn reset(store: &Store, snapshot: &mut Value, stamp: u64) -> Result<(), String> {
    let journal_path = store.dir().join(JOURNAL);
    match fs::remove_file(&journal_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("清理同步记录失败：{e}")),
    }
    let device_path = store.dir().join("sync-record-device.txt");
    if let Ok(text) = fs::read_to_string(&device_path) {
        if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            let _ = fs::remove_file(&device_path);
        }
    }
    attach(store, snapshot, stamp, true)
}

fn is_schema_default_config_entry(key_json: &str, value: &Value, defaults: &Value) -> bool {
    let Ok(parts) = serde_json::from_str::<Vec<String>>(key_json) else {
        return false;
    };
    parts.len() == 2
        && parts[0] == "config"
        && !local_config(&parts[1])
        && defaults.get(&parts[1]) == Some(value)
}

fn journal_from_snapshot(snapshot: &Value) -> Result<Journal, String> {
    let actual = rows(snapshot)?;
    if let Some(value) = snapshot.get("sync_records").filter(|v| !v.is_null()) {
        let journal: Journal = serde_json::from_value(value.clone()).map_err(error)?;
        validate(&journal)?;
        let defaults = config_defaults()?;
        let live: BTreeMap<_, _> = journal.records.iter().filter_map(|(k, r)| r.value.as_ref().map(|v| (k.clone(), v.clone()))).collect();
        let all_live_match = live.iter().all(|(k, v)| actual.get(k) == Some(v));
        // 旧版快照在反序列化为新版 Config 时会补入新增设置项的默认值；
        // 仅允许「记录里从未出现且等于默认值的设置字段」通过校验，仍拦截正文篡改与删除冲突。
        let all_actual_match = actual.iter().all(|(k, v)| {
            live.get(k) == Some(v)
                || (!journal.records.contains_key(k) && is_schema_default_config_entry(k, v, &defaults))
        });
        if !all_live_match || !all_actual_match {
            return Err("同步记录与备份正文不一致".into());
        }
        Ok(journal)
    } else {
        // Older snapshots contain no deletions or per-record clock. They can fill missing
        // content, but cannot overwrite versioned edits or deletion markers.
        Ok(Journal { version: 1, records: actual.into_iter().map(|(k, value)| (k, Record { modified: 0, origin: "0".repeat(32), value: Some(value) })).collect() })
    }
}

pub(crate) fn validate_snapshot(snapshot: &Value) -> Result<(), String> {
    journal_from_snapshot(snapshot).map(|_| ())
}

fn label(name: &str, local: &Journal, remote: &Journal) -> String {
    let mut parts: Vec<String> = serde_json::from_str(name).unwrap_or_default();
    if parts.len() >= 2 && matches!(parts[0].as_str(), "dictionary" | "entry") {
        let dictionary = key(&["dictionary", &parts[1]]);
        if let Some(value) = remote.records.get(&dictionary).and_then(|r| r.value.as_ref()).or_else(|| local.records.get(&dictionary).and_then(|r| r.value.as_ref())) {
            if let Some(name) = value["name"].as_str() { parts[1] = name.into(); }
        }
    }
    let title = match parts.first().map(String::as_str) { Some("config") => "设置", Some("dictionary") => "词库", Some("entry") => "导入词", Some("words" | "history" | "pairs") => "学习", Some("quick") => "短语", Some("common") => "常用语", _ => "最近表情" };
    format!("{title}：{}", parts.into_iter().skip(1).collect::<Vec<_>>().join(" / "))
}

pub(crate) fn merge(local: &Value, remote: &Value) -> Result<(Value, SyncMergePreview), String> {
    let mut journal = journal_from_snapshot(local)?;
    let incoming = journal_from_snapshot(remote)?;
    let mut preview = SyncMergePreview::default();
    for (name, record) in &incoming.records {
        let old = journal.records.get(name);
        let incoming_order = (record.modified, record.value.is_none(), &record.origin);
        let wins = old.is_none_or(|old| incoming_order > (old.modified, old.value.is_none(), &old.origin));
        let differs = old.is_none_or(|old| old.value != record.value);
        if differs && old.is_some_and(|old| old.modified != 0 && record.modified != 0) { preview.conflicts += 1; }
        if wins {
            preview.versions += 1;
            if differs {
                let action = if record.value.is_none() { preview.deleted += 1; "删除" }
                else if old.and_then(|r| r.value.as_ref()).is_none() { preview.added += 1; "新增" }
                else { preview.updated += 1; "更新" };
                if preview.details.len() < 100 { preview.details.push(format!("{action} {}", label(name, &journal, &incoming))); }
            }
            journal.records.insert(name.clone(), record.clone());
        }
    }
    let mut result = local.clone();
    result["quick_phrases"] = json!([]); result["common_phrases"] = json!([]); result["dictionaries"] = json!([]);
    for name in ["words", "history", "pairs"] { result["learning"][name] = json!([]); }
    result["learning"]["boundaries"]=json!({});
    result["learning"]["pending"]=json!({});
    let mut dictionaries: BTreeMap<String, (String, Metadata, Vec<msime::DictEntry>)> = BTreeMap::new();
    for (name, record) in &journal.records {
        let parts: Vec<String> = serde_json::from_str(name).map_err(error)?;
        if parts[0] == "dictionary" {
            if let Some(value) = &record.value {
                dictionaries.insert(parts[1].clone(), (value["name"].as_str().ok_or("来源名称无效")?.into(), serde_json::from_value(value["metadata"].clone()).map_err(error)?, Vec::new()));
            }
        }
    }
    for (name, record) in &journal.records {
        let Some(value) = &record.value else { continue; };
        let parts: Vec<String> = serde_json::from_str(name).map_err(error)?;
        match parts[0].as_str() {
            "config" => result["config"][&parts[1]] = value.clone(),
            "quick" => result["quick_phrases"].as_array_mut().unwrap().push(value.clone()),
            "common" => result["common_phrases"].as_array_mut().unwrap().push(value.clone()),
            "words" | "history" | "pairs" => result["learning"][&parts[0]].as_array_mut().unwrap().push(value.clone()),
            "boundaries" => result["learning"]["boundaries"][&parts[1]]=value.clone(),
            "pending" => result["learning"]["pending"][&parts[1]]=value.clone(),
            "recent" => result["recent_emoji"] = value.clone(),
            "entry" => if let Some((_, _, entries)) = dictionaries.get_mut(&parts[1]) {
                let weight = value.as_u64().filter(|v| *v <= u32::MAX as u64).ok_or("同步词频无效")? as u32;
                entries.push(msime::DictEntry { pinyin: parts[2].clone(), word: parts[3].clone(), weight });
            },
            _ => {}
        }
    }
    let mut used_names = std::collections::HashSet::new();
    for (id, (mut name, meta, entries)) in dictionaries {
        // 独立导入的同名来源不互相覆盖；同一来源改名继续按稳定 id 合并词条。
        if !used_names.insert(name.to_lowercase()) {
            let suffix: String = id.chars().rev().take(8).collect();
            name = format!("{}-{suffix}", name.chars().take(48).collect::<String>());
            if !used_names.insert(name.to_lowercase()) { return Err("合并后的词库名称冲突，请先改名".into()); }
        }
        let text = format!("# FreeType-Dictionary: {}\n{}", serde_json::to_string(&meta).map_err(error)?, msime::format_dictionary(&entries));
        result["dictionaries"].as_array_mut().unwrap().push(json!({"name":name,"text":text}));
    }
    // A deleted source wins over its children; retain their deletion information so old
    // snapshots cannot restore its entries independently of the source.
    let active = rows(&result)?;
    for (name, record) in &mut journal.records {
        if let Some(value) = active.get(name) { record.value = Some(value.clone()); }
        else if record.value.is_some() { record.value = None; }
    }
    result["sync_records"] = serde_json::to_value(journal).map_err(error)?;
    Ok((result, preview))
}

pub(crate) fn without_records(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() { object.remove("sync_records"); object.remove("sync_metadata"); }
    value
}
