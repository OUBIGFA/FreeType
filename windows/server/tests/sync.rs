use std::path::{Path, PathBuf};
use std::sync::Arc;
use freetype_protocol::{Config, Request, Response};
use freetype_server::{host::Host, store::Store};
use wowtype_core::{BigramLm, Engine, MemoryLexicon};

fn test_host(name: &str) -> (Host, PathBuf) {
    let dir = std::env::temp_dir().join(format!("sync-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut host = Host::new(Store::new(dir.clone()));
    host.install_engine(Ok(Engine::new(Arc::new(MemoryLexicon::new()), Arc::new(BigramLm::default()))));
    (host, dir)
}

fn request(host: &mut Host, value: serde_json::Value) -> Response {
    host.handle(0, serde_json::from_value(value).expect("同步请求必须受协议支持"))
}

fn export(host: &mut Host, path: &Path) -> Response {
    request(host, serde_json::json!({"op":"export_sync_settings","path":path}))
}

#[test]
fn sync_export_reports_modification_time_without_turning_reads_into_edits() {
    let (mut host, dir) = test_host("clock");
    let path = dir.join("export.json");
    assert!(host.handle(0, Request::SetCommonPhrases { phrases: vec!["同步测试".into()] }).ok);
    let first = export(&mut host, &path);
    assert!(first.ok, "{:?}", first.error);
    let stamp = serde_json::to_value(&first).unwrap()["modified_unix_ms"].as_u64().unwrap();
    assert!(stamp > 0);
    let data = std::fs::read(&path).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let again = export(&mut host, &path);
    assert_eq!(serde_json::to_value(again).unwrap()["modified_unix_ms"], stamp);
    assert_eq!(std::fs::read(path).unwrap(), data);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sync_restore_rejects_changed_local_data_then_replaces_with_remote_defaults_and_deletions() {
    let (mut local, dir) = test_host("restore");
    let (mut remote, remote_dir) = test_host("remote");
    let expected = dir.join("expected.json");
    let incoming = dir.join("incoming.json");
    assert!(local.handle(0, Request::SetConfig { config: Config { font_size: 24, ..Config::default() } }).ok);
    assert!(local.handle(0, Request::SetCommonPhrases { phrases: vec!["本机旧内容".into()] }).ok);
    assert!(export(&mut local, &expected).ok);
    assert!(export(&mut remote, &incoming).ok);
    assert!(local.handle(0, Request::SetCommonPhrases { phrases: vec!["下载期间新增".into()] }).ok);
    let import = serde_json::json!({"op":"import_sync_settings","path":incoming,"expected_path":expected});
    let rejected = request(&mut local, import.clone());
    assert!(!rejected.ok, "不能覆盖下载期间的新改动");
    assert_eq!(local.handle(0, Request::ListCommonPhrases).common_phrases.unwrap(), ["下载期间新增"]);
    assert!(export(&mut local, &expected).ok);
    let restored = request(&mut local, import);
    assert!(restored.ok, "{:?}", restored.error);
    assert_eq!(restored.config.unwrap().font_size, Config::default().font_size);
    assert!(restored.common_phrases.unwrap().is_empty(), "较新快照的删除必须生效，不能重新合并回来");
    let result = dir.join("result.json");
    assert!(export(&mut local, &result).ok);
    let mut actual: serde_json::Value = serde_json::from_slice(&std::fs::read(result).unwrap()).unwrap();
    let mut source: serde_json::Value = serde_json::from_slice(&std::fs::read(incoming).unwrap()).unwrap();
    let actual_records = actual.as_object_mut().unwrap().remove("sync_records");
    let source_records = source.as_object_mut().unwrap().remove("sync_records");
    assert_eq!(actual, source, "整份恢复必须还原全部正文与默认值");
    assert_ne!(actual_records, source_records, "主动恢复使用本机的新编辑版本，而非复用旧快照版本");
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(remote_dir).unwrap();
}

#[test]
fn sync_restore_waits_for_composition_to_finish() {
    let (mut host, dir) = test_host("composition");
    host.handle(1, Request::Hello { session: true, app: "notepad.exe".into() });
    let snapshot = dir.join("snapshot.json");
    assert!(export(&mut host, &snapshot).ok);
    host.handle(1, Request::Key { key: freetype_protocol::Key { keysym: 'n' as u32, modifiers: 0, test: false }, before: None });
    let restore = serde_json::json!({"op":"import_sync_settings","path":snapshot,"expected_path":snapshot});
    let result = request(&mut host, restore.clone());
    assert!(!result.ok, "组字时不能清掉编辑区");
    assert!(result.error.unwrap().contains("正在输入"));
    host.handle(1, Request::Reset { commit: false });
    assert!(request(&mut host, restore).ok);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn backup_accepts_connection_and_embedded_sync_metadata_without_reexporting_them() {
    let (mut host, dir) = test_host("account-backup");
    let file = dir.join("local.json");
    assert!(export(&mut host, &file).ok);
    let mut data: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    data["sync_connection"] = serde_json::json!({"provider":"webdav", "webdav_user":"sample-account"});
    data["sync_metadata"] = serde_json::json!({"device_id":"0123456789abcdef0123456789abcdef", "hostname":"办公电脑", "revision":"sync-20261117143005-办公电脑"});
    std::fs::write(&file, serde_json::to_vec(&data).unwrap()).unwrap();
    let result = request(&mut host, serde_json::json!({"op":"inspect_settings_backup","path":file}));
    assert!(result.ok, "{:?}", result.error);
    let result = request(&mut host, serde_json::json!({"op":"import_settings","path":file}));
    assert!(result.ok, "{:?}", result.error);
    assert!(export(&mut host, &file).ok);
    let cloud: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert!(cloud.get("sync_connection").is_none());
    assert!(cloud.get("sync_metadata").is_none());
    std::fs::remove_dir_all(dir).unwrap();
}
