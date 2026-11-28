use freetype_protocol::Request;
use freetype_server::{host::Host, store::Store};
use std::sync::Arc;
use wowtype_core::{BigramLm, Engine, MemoryLexicon};

fn host(name: &str) -> Host {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "freetype-sync-merge-{}-{name}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let mut host = Host::new(Store::new(path));
    host.install_engine(Ok(Engine::new(
        Arc::new(MemoryLexicon::new()),
        Arc::new(BigramLm::default()),
    )));
    host
}

#[test]
fn source_entries_and_config_fields_merge_independently_and_local_app_rules_stay_local() {
    let (mut a, mut b) = (host("source-a"), host("source-b"));
    let (af, bf) = (
        a.data_dir().join("export.json"),
        b.data_dir().join("export.json"),
    );
    let input = a.data_dir().join("terms.txt");
    std::fs::write(&input, "甲词\tjia ci\t100\n乙词\tyi ci\t90\n").unwrap();
    call(
        &mut a,
        serde_json::json!({"op":"import_dictionary","path":input,"name":"专业词"}),
    );
    let mut config = a.config().clone();
    config.font_size = 24;
    config.vim_mode_apps = vec!["code.exe".into()];
    config.custom_brand_icon = "saved-image".into();
    config.brand_icon = "custom".into();
    assert!(a.handle(0, Request::SetConfig { config }).ok);
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":af}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":bf}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"merge_sync_settings","path":af,"expected_path":bf}),
    );
    assert_eq!(b.config().font_size, 24);
    assert!(b.config().vim_mode_apps.is_empty());
    assert_eq!(b.config().custom_brand_icon, "saved-image");
    // Delete one entry on A while B adds another entry to the same source.
    call(
        &mut a,
        serde_json::json!({"op":"delete_dictionary_entries","name":"专业词","entries":[{"word":"甲词","pinyin":"jia ci"}]}),
    );
    call(
        &mut a,
        serde_json::json!({"op":"update_dictionary","name":"专业词","new_name":"新名称","enabled":true,"description":"更清楚的来源说明"}),
    );
    let input_b = b.data_dir().join("terms.txt");
    std::fs::write(
        &input_b,
        "甲词\tjia ci\t100\n乙词\tyi ci\t90\n丙词\tbing ci\t80\n",
    )
    .unwrap();
    call(
        &mut b,
        serde_json::json!({"op":"import_dictionary","path":input_b,"name":"专业词"}),
    );
    let mut config = a.config().clone();
    config.font_size = freetype_protocol::Config::default().font_size;
    config.custom_brand_icon.clear();
    config.brand_icon = "default".into();
    assert!(a.handle(0, Request::SetConfig { config }).ok);
    let mut config = b.config().clone();
    config.font_family = "SimSun".into();
    assert!(b.handle(0, Request::SetConfig { config }).ok);
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":af}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":bf}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"merge_sync_settings","path":af,"expected_path":bf}),
    );
    let page = call(&mut b, serde_json::json!({"op":"query_dictionary","name":"新名称","query":"","offset":0,"limit":50})).dictionary_page.unwrap();
    let words: Vec<_> = page.entries.into_iter().map(|entry| entry.word).collect();
    assert!(!words.contains(&"甲词".into()));
    assert!(words.contains(&"乙词".into()) && words.contains(&"丙词".into()));
    assert_eq!(
        b.config().font_size,
        freetype_protocol::Config::default().font_size
    );
    assert_eq!(b.config().font_family, "SimSun");
    assert!(
        b.config().custom_brand_icon.is_empty(),
        "清空自定义图标也需要同步，不能因空字段省略而保留旧图像"
    );
}

#[test]
fn merging_rejects_changed_local_data_and_mismatched_record_payload_without_partial_restore() {
    let (mut a, mut b) = (host("guard-a"), host("guard-b"));
    let (af, bf) = (
        a.data_dir().join("export.json"),
        b.data_dir().join("export.json"),
    );
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":af}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":bf}),
    );
    b.handle(
        0,
        Request::SetCommonPhrases {
            phrases: vec!["新的本机内容".into()],
        },
    );
    let response = b.handle(
        0,
        serde_json::from_value(
            serde_json::json!({"op":"merge_sync_settings","path":af,"expected_path":bf}),
        )
        .unwrap(),
    );
    assert!(!response.ok);
    assert_eq!(
        b.handle(0, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["新的本机内容"]
    );
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&af).unwrap()).unwrap();
    snapshot["common_phrases"] = serde_json::json!(["未记入同步记录的内容"]);
    std::fs::write(&af, snapshot.to_string()).unwrap();
    let response = b.handle(
        0,
        Request::ImportSettings {
            path: af.display().to_string(),
        },
    );
    assert!(!response.ok);
    assert_eq!(
        b.handle(0, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["新的本机内容"]
    );
}

fn call(host: &mut Host, value: serde_json::Value) -> freetype_protocol::Response {
    let response = host.handle(0, serde_json::from_value(value).expect("逐项合并同步协议"));
    assert!(response.ok, "{:?}", response.error);
    response
}

#[test]
fn independent_additions_and_deletions_merge_without_resurrecting_old_snapshots() {
    let (mut a, mut b) = (host("a"), host("b"));
    let a_file = a.data_dir().join("snapshot.json");
    let b_file = b.data_dir().join("snapshot.json");
    let old = a.data_dir().join("old.json");
    assert!(
        a.handle(
            0,
            Request::SetCommonPhrases {
                phrases: vec!["旧条目".into()]
            }
        )
        .ok
    );
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":a_file}),
    );
    std::fs::copy(&a_file, &old).unwrap();
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":b_file}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"merge_sync_settings","path":a_file,"expected_path":b_file}),
    );
    assert!(
        a.handle(
            0,
            Request::SetCommonPhrases {
                phrases: vec!["甲新增".into()]
            }
        )
        .ok
    );
    assert!(
        b.handle(
            0,
            Request::SetCommonPhrases {
                phrases: vec!["旧条目".into(), "乙新增".into()]
            }
        )
        .ok
    );
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":a_file}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":b_file}),
    );
    let preview = call(
        &mut b,
        serde_json::json!({"op":"preview_sync_merge","path":a_file}),
    );
    let report = serde_json::to_value(preview).unwrap();
    assert!(report["sync_merge"]["deleted"].as_u64().unwrap() >= 1);
    call(
        &mut b,
        serde_json::json!({"op":"merge_sync_settings","path":a_file,"expected_path":b_file}),
    );
    let mut phrases = b
        .handle(0, Request::ListCommonPhrases)
        .common_phrases
        .unwrap();
    phrases.sort();
    assert_eq!(phrases, ["乙新增", "甲新增"]);
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":b_file}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"merge_sync_settings","path":old,"expected_path":b_file}),
    );
    assert!(!b
        .handle(0, Request::ListCommonPhrases)
        .common_phrases
        .unwrap()
        .contains(&"旧条目".into()));
    // The legacy complete-replacement operation remains separately available.
    assert!(
        b.handle(
            0,
            Request::ImportSettings {
                path: old.display().to_string()
            }
        )
        .ok
    );
    assert_eq!(
        b.handle(0, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["旧条目"]
    );
    // An explicit full restore is a new user decision, not a stale archive replay.
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":b_file}),
    );
    call(
        &mut a,
        serde_json::json!({"op":"merge_sync_settings","path":b_file,"expected_path":a_file}),
    );
    assert!(
        a.handle(0, Request::ListCommonPhrases)
            .common_phrases
            .unwrap()
            .contains(&"旧条目".into()),
        "主动整份恢复后的词条应作为新修改同步，而不是立即被旧删除记录撤掉"
    );
}

#[test]
fn candidate_style_and_skin_sync_across_devices() {
    let (mut a, mut b) = (host("skin-a"), host("skin-b"));
    let (af, bf) = (
        a.data_dir().join("export.json"),
        b.data_dir().join("export.json"),
    );
    let mut config = a.config().clone();
    config.candidate_style = "custom".into();
    config.candidate_skin = freetype_protocol::CandidateSkin {
        background: "#1e1e24".into(),
        border: "#383844".into(),
        text: "#ebebf0".into(),
        muted: "#9595a6".into(),
        accent: "#6366f1".into(),
        accent_text: "#ffffff".into(),
        accent_fill: false,
    };
    assert!(
        a.handle(
            0,
            Request::SetConfig {
                config: config.clone()
            }
        )
        .ok
    );
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":af}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":bf}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"merge_sync_settings","path":af,"expected_path":bf}),
    );
    assert_eq!(b.config().candidate_style, "custom");
    assert_eq!(b.config().candidate_skin, config.candidate_skin);
    let _ = std::fs::remove_dir_all(a.data_dir());
    let _ = std::fs::remove_dir_all(b.data_dir());
}

#[test]
fn older_snapshot_missing_new_config_fields_merges_without_mismatch() {
    let (mut a, mut b) = (host("legacy-a"), host("legacy-b"));
    let (af, bf) = (
        a.data_dir().join("export.json"),
        b.data_dir().join("export.json"),
    );

    let mut cfg_b = b.config().clone();
    cfg_b.candidate_style = "custom".into();
    cfg_b.candidate_skin.background = "#1e1e24".into();
    assert!(
        b.handle(
            0,
            Request::SetConfig {
                config: cfg_b.clone()
            }
        )
        .ok
    );

    let mut cfg_a = a.config().clone();
    cfg_a.font_size = 22;
    assert!(a.handle(0, Request::SetConfig { config: cfg_a }).ok);
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":af}),
    );

    // 模拟旧版客户端导出的快照：既没有 candidate_skin 配置字段，也没有对应的 sync_records 条目。
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&af).unwrap()).unwrap();
    legacy["config"]
        .as_object_mut()
        .unwrap()
        .remove("candidate_skin");
    legacy["sync_records"]["records"]
        .as_object_mut()
        .unwrap()
        .remove(r#"["config","candidate_skin"]"#);
    std::fs::write(&af, legacy.to_string()).unwrap();

    let preview = call(
        &mut b,
        serde_json::json!({"op":"preview_sync_merge","path":af}),
    );
    assert!(preview.ok);
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":bf}),
    );
    call(
        &mut b,
        serde_json::json!({"op":"merge_sync_settings","path":af,"expected_path":bf}),
    );
    assert_eq!(b.config().font_size, 22);
    assert_eq!(
        b.config().candidate_skin.background,
        "#1e1e24",
        "旧快照缺失的新增设置字段不应覆盖本机已有定制"
    );

    // 非默认字段的正文篡改仍必须被校验拦截。
    let mut tampered = legacy;
    tampered["config"]["font_size"] = serde_json::json!(28);
    std::fs::write(&af, tampered.to_string()).unwrap();
    let bad = b.handle(
        0,
        serde_json::from_value(serde_json::json!({"op":"preview_sync_merge","path":af})).unwrap(),
    );
    assert!(!bad.ok);
    assert!(bad
        .error
        .as_deref()
        .unwrap_or("")
        .contains("同步记录与备份正文不一致"));

    let _ = std::fs::remove_dir_all(a.data_dir());
    let _ = std::fs::remove_dir_all(b.data_dir());
}

#[test]
fn reset_sync_records_and_ignore_records_restore_recover_from_corrupt_records() {
    let (mut a, mut b) = (host("recover-a"), host("recover-b"));
    let (af, bf) = (
        a.data_dir().join("export.json"),
        b.data_dir().join("export.json"),
    );
    assert!(
        a.handle(
            0,
            Request::SetCommonPhrases {
                phrases: vec!["来自远端".into()]
            }
        )
        .ok
    );
    call(
        &mut a,
        serde_json::json!({"op":"export_sync_settings","path":af}),
    );

    // 破坏本机 sync-records.json 后，正常导出与合并失败；调用 reset_sync_records 后恢复。
    std::fs::write(b.data_dir().join("sync-records.json"), "{corrupt").unwrap();
    assert!(
        !b.handle(
            0,
            Request::ExportSyncSettings {
                path: bf.display().to_string()
            }
        )
        .ok
    );
    assert!(b.handle(0, Request::ResetSyncRecords).ok);
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":bf}),
    );

    // 破坏远端快照中的 sync_records，正常合并和带校验恢复拒绝，而 ignore_records=true 允许按正文整份恢复。
    let mut bad_remote: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&af).unwrap()).unwrap();
    bad_remote["common_phrases"] = serde_json::json!(["正文已改但记录未改"]);
    std::fs::write(&af, bad_remote.to_string()).unwrap();
    let normal = b.handle(
        0,
        Request::ImportSyncSettings {
            path: af.display().to_string(),
            expected_path: bf.display().to_string(),
            ignore_records: false,
        },
    );
    assert!(!normal.ok);

    // 即便本机 sync-records.json 也损坏，按正文恢复后仍应重建出干净记录。
    std::fs::write(b.data_dir().join("sync-records.json"), "{corrupt").unwrap();
    let forced = b.handle(
        0,
        Request::ImportSyncSettings {
            path: af.display().to_string(),
            expected_path: bf.display().to_string(),
            ignore_records: true,
        },
    );
    assert!(forced.ok, "{:?}", forced.error);
    assert_eq!(
        b.handle(0, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["正文已改但记录未改"]
    );
    call(
        &mut b,
        serde_json::json!({"op":"export_sync_settings","path":bf}),
    );

    let _ = std::fs::remove_dir_all(a.data_dir());
    let _ = std::fs::remove_dir_all(b.data_dir());
}
