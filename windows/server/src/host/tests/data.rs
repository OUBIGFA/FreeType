//! 学习记录、设置备份、词库与快捷短语、导入导出和同步合并。
use super::*;

#[test]
fn sync_work_runs_beside_typing_and_merges_apply_only_if_local_data_is_unchanged() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let mut a = host("sync-beside-a");
    let mut b = host("sync-beside-b");
    let (af, bf) = (a.data_dir().join("a.json"), b.data_dir().join("b.json"));
    let path = |path: &std::path::Path| path.display().to_string();
    assert!(
        a.handle(
            1,
            Request::SetCommonPhrases {
                phrases: vec!["来自另一台".into()]
            }
        )
        .ok
    );
    assert!(
        a.handle(1, Request::ExportSyncSettings { path: path(&af) })
            .ok
    );
    let (apply_tx, apply_rx) = channel();
    b.set_apply_channel(apply_tx);

    // 占住后台线程，相当于开机时慢到数秒的同步计算：按键照常出候选，同步应答在后面排队。
    let (release, blocked) = channel::<()>();
    b.tasks.submit(
        Box::new(move || {
            let _ = blocked.recv();
            Step::Done
        }),
        None,
    );
    let (reply, export) = channel();
    b.dispatch(1, Request::ExportSyncSettings { path: path(&bf) }, reply);
    assert_eq!(
        typing(&mut b, "nihao")
            .candidates
            .first()
            .map(String::as_str),
        Some("你好")
    );
    assert!(export.try_recv().is_err(), "同步导出不应在引擎线程里完成");
    release.send(()).unwrap();
    assert!(export.recv_timeout(Duration::from_secs(10)).unwrap().ok);
    assert!(b.handle(1, Request::Reset { commit: false }).ok);

    // 合并在后台比较，回到引擎线程应用后才应答。
    let (reply, merged) = channel();
    b.dispatch(
        1,
        Request::MergeSyncSettings {
            path: path(&af),
            expected_path: path(&bf),
        },
        reply,
    );
    let apply = apply_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(merged.try_recv().is_err());
    apply(&mut b);
    let response = merged.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(
        b.handle(1, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["来自另一台"]
    );

    // 后台比较期间本机又改了：放弃这次合并，保留刚做的修改，由同步重新比较。
    assert!(
        a.handle(
            1,
            Request::SetCommonPhrases {
                phrases: vec!["另一台又改了".into()]
            }
        )
        .ok
    );
    assert!(
        a.handle(1, Request::ExportSyncSettings { path: path(&af) })
            .ok
    );
    let (reply, export) = channel();
    b.dispatch(1, Request::ExportSyncSettings { path: path(&bf) }, reply);
    assert!(export.recv_timeout(Duration::from_secs(10)).unwrap().ok);
    let (reply, merged) = channel();
    b.dispatch(
        1,
        Request::MergeSyncSettings {
            path: path(&af),
            expected_path: path(&bf),
        },
        reply,
    );
    let apply = apply_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        b.handle(
            1,
            Request::SetCommonPhrases {
                phrases: vec!["本机刚改的".into()]
            }
        )
        .ok
    );
    apply(&mut b);
    let response = merged.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!response.ok);
    assert_eq!(
        b.handle(1, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["本机刚改的"]
    );

    // 手动重置同步记录排在后台登记之后，回到引擎线程执行，不会互相等待。
    let (reply, reset) = channel();
    b.dispatch(1, Request::ResetSyncRecords, reply);
    apply_rx.recv_timeout(Duration::from_secs(10)).unwrap()(&mut b);
    assert!(reset.recv_timeout(Duration::from_secs(10)).unwrap().ok);
    cleanup(&a);
    cleanup(&b);
}

#[test]
fn dictionary_catalog_controls_sources_queries_pages_and_removes_only_selected_entries() {
    let mut host = host("dictionary-catalog");
    let path = host.data_dir().join("source.txt");
    std::fs::create_dir_all(host.data_dir()).unwrap();
    std::fs::write(&path, "韵酒\tyun jiu\t10000\n韵茶\tyun cha\t9000\n").unwrap();
    for name in ["甲", "乙"] {
        assert!(
            host.handle(
                1,
                Request::ImportDictionary {
                    path: path.display().to_string(),
                    name: name.into()
                }
            )
            .ok
        );
    }
    let call = |host: &mut Host, value| {
        let request: Request = serde_json::from_value(value).expect("词库管理协议应可用");
        let response = host.handle(1, request);
        assert!(response.ok, "{:?}", response.error);
        serde_json::to_value(response).unwrap()
    };
    let page = call(
        &mut host,
        serde_json::json!({"op":"query_dictionary","name":"甲","query":"韵","offset":0,"limit":1}),
    );
    assert_eq!(page["dictionary_page"]["total"], 2);
    assert_eq!(
        page["dictionary_page"]["entries"].as_array().unwrap().len(),
        1
    );
    for name in ["甲", "乙"] {
        call(
            &mut host,
            serde_json::json!({"op":"update_dictionary","name":name,"new_name":name,"enabled":false,"description":"本地专业词库"}),
        );
        if name == "甲" {
            assert!(host
                .engine_ref()
                .unwrap()
                .input("yunjiu")
                .iter()
                .any(|c| c.text == "韵酒"));
        }
    }
    assert!(!host
        .engine_ref()
        .unwrap()
        .input("yunjiu")
        .iter()
        .any(|c| c.text == "韵酒"));
    call(
        &mut host,
        serde_json::json!({"op":"update_dictionary","name":"甲","new_name":"饮品","enabled":true,"description":"茶与酒"}),
    );
    assert!(host
        .engine_ref()
        .unwrap()
        .input("yunjiu")
        .iter()
        .any(|c| c.text == "韵酒"));
    call(
        &mut host,
        serde_json::json!({"op":"delete_dictionary_entries","name":"饮品","entries":[{"word":"韵酒","pinyin":"yun jiu"}]}),
    );
    assert!(!host
        .engine_ref()
        .unwrap()
        .input("yunjiu")
        .iter()
        .any(|c| c.text == "韵酒"));
    assert!(host
        .engine_ref()
        .unwrap()
        .input("yuncha")
        .iter()
        .any(|c| c.text == "韵茶"));
    let listed = call(&mut host, serde_json::json!({"op":"list_dictionaries"}));
    let renamed = listed["dictionaries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "饮品")
        .unwrap();
    assert_eq!(renamed["description"], "茶与酒");
    assert_eq!(renamed["enabled"], true);
    assert_eq!(
        Store::new(host.data_dir().to_path_buf())
            .load_dictionaries()
            .len(),
        1,
        "停用状态需在重启后保留"
    );
    cleanup(&host);
}

#[test]
fn learning_backup_restores_unsaved_words_history_and_pairs_after_restart() {
    let mut source = learning_host("learning-source");
    let engine = source.engine.as_ref().unwrap();
    for _ in 0..3 {
        engine.record_selection_with_pinyin("阿特", Some("a te"));
    }
    engine.set_context("阿特");
    engine.input("nihao");
    engine.record_selection_with_pinyin("你好", Some("ni hao"));
    assert!(engine.pair_count() > 0);
    assert!(
        !source.store.user_dict_path().exists(),
        "导出必须包含尚未定时存盘的学习"
    );
    let backup = source.data_dir().join("学习备份.json");
    let response = source.handle(1, learning_request("export_learning", &backup));
    assert!(response.ok, "{:?}", response.error);
    let bytes = std::fs::read(&backup).unwrap();
    let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(data["format"], "freetype-learning");
    assert_eq!(data["version"], 1);

    let mut restored = learning_host("learning-restored");
    let response = restored.handle(1, learning_request("import_learning", &backup));
    assert!(response.ok, "{:?}", response.error);
    assert!(response
        .user_words
        .unwrap()
        .iter()
        .any(|word| word.word == "阿特"));
    let restarted = Engine::new(
        std::sync::Arc::new(wowtype_core::MemoryLexicon::new()),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    restarted
        .load_user_dict(restored.store.user_dict_path())
        .unwrap();
    restarted
        .load_history(restored.store.history_path())
        .unwrap();
    assert_eq!(restarted.history_count("阿特"), 3);
    assert_eq!(restarted.history_count("你好"), 1);
    assert!(restarted.pair_count() > 0);
    assert!(restarted
        .input("ate")
        .iter()
        .any(|candidate| candidate.text == "阿特"));
    cleanup(&source);
    cleanup(&restored);
}

#[test]
fn learning_backup_merge_preserves_current_learning_and_is_idempotent() {
    let mut target = learning_host("learning-merge");
    let engine = target.engine.as_ref().unwrap();
    for _ in 0..5 {
        engine.record_selection_with_pinyin("你好", Some("ni hao"));
    }
    for _ in 0..3 {
        engine.record_selection_with_pinyin("霜频阁", Some("shuang pin ge"));
    }
    assert!(engine
        .user_words()
        .iter()
        .any(|(_, word, _)| word == "霜频阁"));
    let backup = target.data_dir().join("backup.json");
    std::fs::write(
        &backup,
        serde_json::to_vec(&serde_json::json!({
            "format": "freetype-learning", "version": 1,
            "words": [["a te", "阿特", 3000.0]],
            "history": [["阿特", 3, 1234], ["你好", 1, 1234]],
            "pairs": [["阿特", "你好", 2, 1234]]
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        target
            .handle(1, learning_request("import_learning", &backup))
            .ok
    );
    let first = std::fs::read(target.store.history_path()).unwrap();
    assert!(
        target
            .handle(1, learning_request("import_learning", &backup))
            .ok
    );
    assert_eq!(std::fs::read(target.store.history_path()).unwrap(), first);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!(
        engine.history_count("你好"),
        5,
        "旧备份不能覆盖较新的使用记录"
    );
    assert_eq!(engine.history_count("阿特"), 3, "重复导入不能累加次数");
    assert!(engine
        .user_words()
        .iter()
        .any(|(_, word, _)| word == "霜频阁"));
    assert_eq!(engine.pair_count(), 1);
    cleanup(&target);
}

#[test]
fn learning_backup_rejects_invalid_records_without_partial_import() {
    let mut target = learning_host("learning-invalid");
    target
        .engine
        .as_ref()
        .unwrap()
        .record_selection_with_pinyin("你好", Some("ni hao"));
    let backup = target.data_dir().join("invalid.json");
    let valid = serde_json::json!({
        "format": "freetype-learning", "version": 1,
        "words": [["a te", "阿特", 3000.0]],
        "history": [["阿特", 3, 1234]], "pairs": []
    });
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("format", serde_json::json!("other-app")),
        (
            "words",
            serde_json::json!([["a te", "阿特", 3000], ["bad-code!", "坏词", 3]]),
        ),
        ("history", serde_json::json!([["阿特", -1, 1234]])),
        ("pairs", serde_json::json!([["你好", "", 1, 1234]])),
        ("words", serde_json::Value::Null),
    ] {
        let mut data = valid.clone();
        data[field] = value;
        std::fs::write(&backup, serde_json::to_vec(&data).unwrap()).unwrap();
        let response = target.handle(1, learning_request("import_learning", &backup));
        assert!(!response.ok, "损坏的 {field} 必须报错");
        let engine = target.engine.as_ref().unwrap();
        assert_eq!(engine.history_count("你好"), 1);
        assert_eq!(engine.history_count("阿特"), 0, "校验完全部记录后才能导入");
        assert!(!engine
            .user_words()
            .iter()
            .any(|(_, word, _)| word == "阿特"));
    }
    cleanup(&target);
}

#[test]
fn learning_backup_reports_io_failures() {
    let mut target = learning_host("learning-io");
    let missing = target.data_dir().join("missing.json");
    assert!(
        !target
            .handle(1, learning_request("import_learning", &missing))
            .ok
    );
    let directory = target.data_dir().to_path_buf();
    assert!(
        !target
            .handle(1, learning_request("export_learning", &directory))
            .ok
    );
    let backup = target.data_dir().join("backup.json");
    target
        .engine
        .as_ref()
        .unwrap()
        .record_selection_with_pinyin("阿特", Some("a te"));
    assert!(
        target
            .handle(1, learning_request("export_learning", &backup))
            .ok
    );
    std::fs::create_dir(target.store.history_path()).unwrap();
    let response = target.handle(1, learning_request("import_learning", &backup));
    assert!(!response.ok, "无法持久化时不能声称恢复成功");
    assert!(target.dirty, "保存失败后仍需重试存盘");
    cleanup(&target);
}

#[test]
fn key_test_does_not_learn_or_forget_before_the_key_is_accepted() {
    let mut host = host("test-learning");
    typing(&mut host, "nihao");
    let before = host.engine.as_ref().unwrap().export_learning().unwrap();
    let preview = Key {
        keysym: 0x20,
        test: true,
        ..Key::default()
    };
    for _ in 0..3 {
        assert!(
            host.handle(
                1,
                Request::Key {
                    key: preview,
                    before: None
                }
            )
            .frame
            .unwrap()
            .handled
        );
        assert_eq!(
            host.engine.as_ref().unwrap().export_learning().unwrap(),
            before,
            "反复预判不能提前学习"
        );
    }
    let result = press(&mut host, 0x20);
    assert_eq!(result.commit.as_deref(), Some("你好"));
    let learned = host.engine.as_ref().unwrap().export_learning().unwrap();
    assert_ne!(learned, before, "真正接受按键后才学习");
    let records: serde_json::Value = serde_json::from_str(&learned).unwrap();
    assert_eq!(
        records["history"][0][1], 1,
        "重复预判之后一次上屏仍然只学习一次"
    );

    typing(&mut host, "nihao");
    let preview = Key {
        keysym: keysym::DELETE,
        modifiers: modifier::CTRL,
        test: true,
    };
    assert!(
        host.handle(
            1,
            Request::Key {
                key: preview,
                before: None
            }
        )
        .frame
        .unwrap()
        .handled
    );
    assert_eq!(
        host.engine.as_ref().unwrap().export_learning().unwrap(),
        learned,
        "预判删除键不能删除用户记录"
    );
    key_with(&mut host, 1, keysym::DELETE, modifier::CTRL);
    assert_ne!(
        host.engine.as_ref().unwrap().export_learning().unwrap(),
        learned
    );
    cleanup(&host);
}

#[test]
fn quick_phrases_and_dictionaries_change_candidates() {
    let mut host = host("user-data");
    let file = host.data_dir().with_extension("qp.txt");
    std::fs::write(&file, "yyjq\t韵酒\n").unwrap();
    let response = host.handle(
        1,
        Request::ImportQuickPhrases {
            path: file.display().to_string(),
            replace: false,
        },
    );
    assert_eq!(response.report.unwrap().added, 1);
    assert_eq!(typing(&mut host, "yyjq").candidates[0], "韵酒");
    press(&mut host, keysym::ESCAPE);

    std::fs::write(&file, "示例词\tshi li ci\t5000\n").unwrap();
    let response = host.handle(
        1,
        Request::ImportDictionary {
            path: file.display().to_string(),
            name: "示例".into(),
        },
    );
    assert!(response.ok, "{:?}", response.error);
    assert!(typing(&mut host, "shilici")
        .candidates
        .iter()
        .any(|c| c == "示例词"));

    configure(&mut host, |c| c.quick_phrases = false);
    press(&mut host, keysym::ESCAPE);
    assert!(!typing(&mut host, "yyjq")
        .candidates
        .iter()
        .any(|c| c == "韵酒"));
    let _ = std::fs::remove_file(file);
    cleanup(&host);
}

#[test]
fn common_phrases_round_trip_preserves_multiline_and_merges_without_duplicates() {
    let mut source = learning_host("common-source");
    let response = source.handle(1, settings_request(serde_json::json!({
        "op": "set_common_phrases", "phrases": ["您好FreeType", "第一行\n第二行", "您好FreeType"],
    })));
    assert!(response.ok, "{response:?}");
    let exported = source.data_dir().join("common.json");
    assert!(
        source
            .handle(1, learning_request("export_common_phrases", &exported))
            .ok
    );
    let mut target = learning_host("common-target");
    for _ in 0..2 {
        let response = target.handle(
            1,
            settings_request(serde_json::json!({
                "op": "import_common_phrases", "path": exported, "replace": false,
            })),
        );
        assert!(response.ok, "{response:?}");
    }
    let listed = serde_json::to_value(target.handle(
        1,
        settings_request(serde_json::json!({"op": "list_common_phrases"})),
    ))
    .unwrap();
    assert_eq!(
        listed["common_phrases"],
        serde_json::json!(["您好FreeType", "第一行\n第二行"])
    );
    let mut restarted = Host::new(Store::new(target.data_dir()));
    let listed = serde_json::to_value(restarted.handle(
        1,
        settings_request(serde_json::json!({"op": "list_common_phrases"})),
    ))
    .unwrap();
    assert_eq!(
        listed["common_phrases"],
        serde_json::json!(["您好FreeType", "第一行\n第二行"])
    );
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn common_phrases_toolbar_excludes_quick_phrases_while_codes_still_work() {
    let mut host = host("common-toolbar");
    assert!(
        host.handle(
            1,
            settings_request(serde_json::json!({
                "op": "set_common_phrases", "phrases": ["欢迎FreeType第5版"],
            }))
        )
        .ok
    );
    assert!(
        host.handle(
            1,
            Request::SetQuickPhrases {
                phrases: vec![freetype_protocol::QuickPhrase {
                    code: "dz".into(),
                    text: "海淀区".into(),
                    weight: 10
                }],
            }
        )
        .ok
    );
    typing(&mut host, "v");
    press(&mut host, 0x20);
    let frame = press(&mut host, '2' as u32);
    assert_eq!(frame.candidates, vec!["欢迎FreeType第5版"]);
    assert_eq!(
        press(&mut host, '1' as u32).commit.as_deref(),
        Some("欢迎 FreeType 第 5 版")
    );
    typing(&mut host, "dz");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("海淀区"));
    cleanup(&host);
}

#[test]
fn empty_common_phrases_toolbar_does_not_fall_back_to_quick_phrases() {
    let mut host = host("empty-common-toolbar");
    assert!(
        host.handle(
            1,
            Request::SetQuickPhrases {
                phrases: vec![freetype_protocol::QuickPhrase {
                    code: "dz".into(),
                    text: "海淀区".into(),
                    weight: 10
                }],
            }
        )
        .ok
    );
    typing(&mut host, "v");
    press(&mut host, 0x20);
    let frame = press(&mut host, '2' as u32);
    assert!(
        frame.candidates.is_empty(),
        "没有常用语时不能用快捷短语填充：{:?}",
        frame.candidates
    );
    assert_eq!(
        frame.title.as_deref(),
        Some("常用语 · 请在常用语设置页添加内容")
    );
    press(&mut host, keysym::ESCAPE);
    typing(&mut host, "dz");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("海淀区"));
    cleanup(&host);
}

#[test]
fn common_phrases_invalid_import_preserves_data_and_text_import_supports_utf8() {
    let mut host = learning_host("common-invalid");
    assert!(
        host.handle(
            1,
            settings_request(serde_json::json!({
                "op": "set_common_phrases", "phrases": ["保留"],
            }))
        )
        .ok
    );
    let path = host.data_dir().join("import.json");
    for contents in [
        "{bad",
        r#"{"format":"freetype-common-phrases","version":9,"phrases":["覆盖"]}"#,
        r#"{"format":"freetype-common-phrases","version":1,"phrases":["有效"," "]}"#,
    ] {
        std::fs::write(&path, contents).unwrap();
        assert!(
            !host
                .handle(
                    1,
                    settings_request(serde_json::json!({
                        "op": "import_common_phrases", "path": path, "replace": true,
                    }))
                )
                .ok
        );
        let listed = serde_json::to_value(host.handle(
            1,
            settings_request(serde_json::json!({"op": "list_common_phrases"})),
        ))
        .unwrap();
        assert_eq!(listed["common_phrases"], serde_json::json!(["保留"]));
    }
    std::fs::write(&path, "\u{feff}第一条\r\n\r\n第二条\r\n第一条\r\n").unwrap();
    let response = host.handle(
        1,
        settings_request(serde_json::json!({
            "op": "import_common_phrases", "path": path, "replace": true,
        })),
    );
    assert!(response.ok, "{response:?}");
    let listed = serde_json::to_value(host.handle(
        1,
        settings_request(serde_json::json!({"op": "list_common_phrases"})),
    ))
    .unwrap();
    assert_eq!(
        listed["common_phrases"],
        serde_json::json!(["第一条", "第二条"])
    );
    cleanup(&host);
}

#[test]
fn common_phrases_empty_backup_can_restore_an_empty_list() {
    let mut host = learning_host("common-empty");
    let path = host.data_dir().join("empty.json");
    assert!(
        host.handle(1, learning_request("export_common_phrases", &path))
            .ok
    );
    assert!(
        host.handle(
            1,
            settings_request(
                serde_json::json!({"op": "set_common_phrases", "phrases": ["待替换"]})
            )
        )
        .ok
    );
    assert!(
        host.handle(
            1,
            settings_request(
                serde_json::json!({"op": "import_common_phrases", "path": path, "replace": true})
            )
        )
        .ok
    );
    assert!(host.store.load_common_phrases().unwrap().is_empty());
    cleanup(&host);
}

#[test]
fn settings_backup_restores_all_data_including_unsaved_learning_and_custom_lexicon() {
    let mut source = learning_host("settings-source");
    let main_lexicon = source.data_dir().join("source-lexicon.tsv");
    std::fs::write(&main_lexicon, "ni hao\t你好\t1000\na te\t阿特\t900\n").unwrap();
    configure(&mut source, |c| {
        c.page_size = 9;
        c.autospace_digits = false;
        c.tray_icon = false;
        c.auto_update = true;
        c.show_settings_after_update = true;
        c.brand_icon = "custom".into();
        c.custom_brand_icon = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==".into();
        c.candidate_style = "custom".into();
        c.candidate_skin = freetype_protocol::CandidateSkin {
            background: "#24262b".into(),
            border: "#3e424b".into(),
            text: "#eef0f4".into(),
            muted: "#9aa0ac".into(),
            accent: "#2f70e8".into(),
            accent_text: "#ffffff".into(),
            accent_fill: false,
        };
        c.lexicon = main_lexicon.to_string_lossy().into();
        c.default_mode = DefaultMode::Global;
    });
    source.mode = Mode::English;
    assert!(
        source
            .handle(
                1,
                Request::SetQuickPhrases {
                    phrases: vec![freetype_protocol::QuickPhrase {
                        code: "dz".into(),
                        text: "第一行\n第二行".into(),
                        weight: 20
                    }],
                }
            )
            .ok
    );
    assert!(
        source
            .handle(
                1,
                settings_request(
                    serde_json::json!({"op": "set_common_phrases", "phrases": ["常用FreeType"]})
                )
            )
            .ok
    );
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(
        source.data_dir().join("dicts/人名.txt"),
        "阿特\ta te\t800\n",
    )
    .unwrap();
    source.recent_emoji = vec!["😊".into()];
    std::fs::write(source.data_dir().join("statusbar.txt"), "100,200\n").unwrap();
    let engine = source.engine.as_ref().unwrap();
    for _ in 0..3 {
        engine.record_selection_with_pinyin("阿特", Some("a te"));
    }
    engine.set_context("阿特");
    engine.input("nihao");
    engine.record_selection_with_pinyin("你好", Some("ni hao"));
    assert!(!source.store.user_dict_path().exists());
    let exported = source.data_dir().join("all.json");
    let result = source.handle(1, learning_request("export_settings", &exported));
    assert!(result.ok, "{result:?}");
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&exported).unwrap()).unwrap();
    assert_eq!(data["format"], "freetype-settings");
    assert_eq!(data["version"], 1);
    assert_eq!(data["config"]["lexicon"], "", "备份不依赖来源机器的路径");
    assert_eq!(data["config"]["auto_update"], true);
    assert_eq!(data["config"]["show_settings_after_update"], true);
    assert_eq!(data["config"]["brand_icon"], "custom");
    assert_eq!(
        data["config"]["custom_brand_icon"],
        source.config.custom_brand_icon
    );
    assert_eq!(data["config"]["candidate_style"], "custom");
    assert_eq!(data["config"]["candidate_skin"]["background"], "#24262b");
    assert_eq!(data["config"]["candidate_skin"]["accent_fill"], false);

    let mut target = learning_host("settings-target");
    target
        .engine
        .as_ref()
        .unwrap()
        .record_selection_with_pinyin("旧词", Some("jiu ci"));
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    std::fs::write(
        target.data_dir().join("dicts/旧词库.txt"),
        "旧词\tjiu ci\t100\n",
    )
    .unwrap();
    let inspected = target.handle(1, learning_request("inspect_settings_backup", &exported));
    assert!(inspected.ok, "{inspected:?}");
    assert_eq!(target.config.page_size, 7, "预览不能修改设置");
    let result = target.handle(1, learning_request("import_settings", &exported));
    assert!(result.ok, "{result:?}");
    assert_eq!(target.config.page_size, 9);
    assert!(!target.config.autospace_digits && !target.config.tray_icon);
    assert!(
        target.config.auto_update,
        "全部设置备份也应恢复自动更新开关"
    );
    assert!(
        target.config.show_settings_after_update,
        "全部设置备份也应恢复安装后显示设置页的开关"
    );
    assert_eq!(target.config.brand_icon, "custom");
    assert_eq!(
        target.config.custom_brand_icon,
        source.config.custom_brand_icon
    );
    assert_eq!(target.config.candidate_style, "custom");
    assert_eq!(target.config.candidate_skin, source.config.candidate_skin);
    assert_eq!(target.mode, Mode::English);
    assert_eq!(target.store.list_quick_phrases()[0].text, "第一行\n第二行");
    assert_eq!(
        target.store.load_common_phrases().unwrap(),
        vec!["常用FreeType"]
    );
    assert_eq!(target.store.list_dictionaries()[0].name, "人名");
    assert_eq!(
        target.store.list_dictionaries().len(),
        1,
        "恢复快照不应残留旧词库"
    );
    assert_eq!(target.recent_emoji, vec!["😊"]);
    assert_eq!(
        std::fs::read_to_string(target.data_dir().join("statusbar.txt")).unwrap(),
        "100,200\n"
    );
    assert_eq!(
        std::fs::read_to_string(&target.config.lexicon).unwrap(),
        "ni hao\t你好\t1000\na te\t阿特\t900\n"
    );
    assert!(Path::new(&target.config.lexicon).starts_with(target.data_dir()));
    assert_eq!(target.engine.as_ref().unwrap().history_count("阿特"), 3);
    assert_eq!(target.engine.as_ref().unwrap().history_count("旧词"), 0);
    let restarted = Engine::new(
        std::sync::Arc::new(wowtype_core::MemoryLexicon::new()),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    restarted
        .load_user_dict(target.store.user_dict_path())
        .unwrap();
    restarted.load_history(target.store.history_path()).unwrap();
    assert_eq!(restarted.history_count("阿特"), 3);
    assert!(restarted.pair_count() > 0);
    assert!(restarted
        .input("ate")
        .iter()
        .any(|candidate| candidate.text == "阿特"));
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn settings_backup_exports_only_the_chosen_parts() {
    let mut source = full_profile_host("parts-export");
    let path = source.data_dir().join("part.json");
    // 没选的类别整个不在文件里：字段没有，内容也搜不到
    let response = source.handle(
        1,
        export_parts_request(&path, &["quick_phrases", "dictionaries"]),
    );
    assert!(response.ok, "{response:?}");
    assert_eq!(
        backup_keys(&path),
        [
            "dictionaries",
            "format",
            "main_lexicon",
            "quick_phrases",
            "version"
        ]
    );
    let text = std::fs::read_to_string(&path).unwrap();
    for private in ["私密词", "远端常用语", "😊", "page_size"] {
        assert!(!text.contains(private), "没选的数据进了备份：{private}");
    }
    assert_eq!(
        response.report.unwrap().note.as_deref(),
        Some("1 份词库、1 条快捷短语；不含设置、常用语、学习记录")
    );
    // 一类一类地导出：每次只有那一类的字段
    for (part, keys) in [
        (
            "settings",
            vec!["config", "format", "mode", "status_bar_position", "version"],
        ),
        (
            "common_phrases",
            vec!["common_phrases", "format", "version"],
        ),
        (
            "learning",
            vec!["format", "learning", "recent_emoji", "version"],
        ),
    ] {
        assert!(source.handle(1, export_parts_request(&path, &[part])).ok);
        assert_eq!(backup_keys(&path), keys, "{part}");
    }
    // 没选的数据读不了也不妨碍导出：快捷短语文件坏了，只导出设置照样成功
    std::fs::write(
        source.data_dir().join("quick_phrases.txt"),
        "这一行不是快捷短语\n",
    )
    .unwrap();
    assert!(
        source
            .handle(1, export_parts_request(&path, &["settings"]))
            .ok
    );
    assert!(
        !source
            .handle(1, export_parts_request(&path, &["quick_phrases"]))
            .ok
    );
    cleanup(&source);
}

#[test]
fn settings_backup_with_all_parts_is_the_same_file_as_before() {
    let mut source = full_profile_host("parts-all");
    let (plain, chosen) = (
        source.data_dir().join("plain.json"),
        source.data_dir().join("chosen.json"),
    );
    assert!(
        source
            .handle(1, learning_request("export_settings", &plain))
            .ok
    );
    let all = [
        "learning",
        "common_phrases",
        "quick_phrases",
        "dictionaries",
        "settings",
    ];
    assert!(source.handle(1, export_parts_request(&chosen, &all)).ok);
    let text = std::fs::read_to_string(&plain).unwrap();
    assert_eq!(
        text,
        std::fs::read_to_string(&chosen).unwrap(),
        "全选和不写 parts 是同一份文件"
    );
    // 字段顺序和内容保持原来的格式：自动同步按整份内容比较，旧版本也要能导入；学习记录压为单行紧凑 JSON
    assert!(
        text.starts_with(
            "{\n  \"format\": \"freetype-settings\",\n  \"version\": 1,\n  \"config\": {"
        ),
        "{}",
        &text[..80]
    );
    let learning_lines: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("  \"learning\": "))
        .collect();
    assert_eq!(learning_lines.len(), 1);
    assert!(
        learning_lines[0].contains("\"freetype-learning\"") && learning_lines[0].ends_with("},"),
        "学习记录应保持在单行内：{}",
        learning_lines[0]
    );
    let note = source
        .handle(1, learning_request("inspect_settings_backup", &plain))
        .report
        .unwrap()
        .note
        .unwrap();
    assert_eq!(
        note,
        "全部设置、1 份词库、1 条快捷短语、1 条常用语、1 条学到的词及使用记录"
    );
    cleanup(&source);
}

#[test]
fn importing_a_partial_backup_replaces_only_its_parts() {
    let mut source = full_profile_host("parts-source");
    let path = source.data_dir().join("part.json");
    assert!(
        source
            .handle(
                1,
                export_parts_request(&path, &["quick_phrases", "dictionaries"])
            )
            .ok
    );

    let mut target = local_profile_host("parts-target");
    let inspected = target.handle(1, learning_request("inspect_settings_backup", &path));
    assert_eq!(
        inspected.report.unwrap().note.as_deref(),
        Some("1 份词库、1 条快捷短语；不含设置、常用语、学习记录")
    );
    let response = target.handle(1, learning_request("import_settings", &path));
    assert!(response.ok, "{response:?}");
    // 备份里有的两类：整类换成备份里的
    let phrases: Vec<String> = target
        .store
        .list_quick_phrases()
        .into_iter()
        .map(|p| p.code)
        .collect();
    assert_eq!(phrases, ["dz"]);
    let dictionaries: Vec<String> = target
        .store
        .list_dictionaries()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(dictionaries, ["人名"], "词库整类替换，不残留本机原来的");
    assert_eq!(
        std::fs::read_to_string(&target.config.lexicon).unwrap(),
        "ni hao\t你好\t1000\na te\t阿特\t900\n"
    );
    assert!(Path::new(&target.config.lexicon).starts_with(target.data_dir()));
    assert_eq!(
        typing(&mut target, "dz").candidates[0],
        "远端地址",
        "换进来的快捷短语马上能用"
    );
    press(&mut target, keysym::ESCAPE);
    // 备份里没有的三类：本机的原样留着
    assert_eq!(target.config.page_size, 8);
    assert_eq!(target.mode, Mode::Chinese);
    assert_eq!(
        std::fs::read_to_string(target.data_dir().join("statusbar.txt")).unwrap(),
        "1,2\n"
    );
    assert_eq!(target.store.load_common_phrases().unwrap(), ["本机常用语"]);
    assert_eq!(
        response.common_phrases.as_deref(),
        Some(&["本机常用语".to_string()][..])
    );
    assert_eq!(target.recent_emoji, ["🎉"]);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!(
        (engine.history_count("旧词"), engine.history_count("私密词")),
        (1, 0)
    );
    // 重新启动以后也是这样
    let restarted = Host::new(Store::new(target.data_dir()));
    assert_eq!(restarted.config.page_size, 8);
    assert_eq!(restarted.config.lexicon, target.config.lexicon);
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn importing_settings_or_learning_alone_keeps_everything_else() {
    let mut source = full_profile_host("parts-single-source");
    let (settings, learning) = (
        source.data_dir().join("settings.json"),
        source.data_dir().join("learning.json"),
    );
    assert!(
        source
            .handle(1, export_parts_request(&settings, &["settings"]))
            .ok
    );
    assert!(
        source
            .handle(1, export_parts_request(&learning, &["learning"]))
            .ok
    );

    // 只有设置：选项、中英文状态、状态栏位置换掉；本机的主词库、短语、词库、学习记录不动
    let mut target = local_profile_host("parts-single-settings");
    let local_lexicon = target.config.lexicon.clone();
    assert!(
        target
            .handle(1, learning_request("import_settings", &settings))
            .ok
    );
    assert_eq!((target.config.page_size, target.mode), (9, Mode::English));
    assert_eq!(
        std::fs::read_to_string(target.data_dir().join("statusbar.txt")).unwrap(),
        "100,200\n"
    );
    assert_eq!(target.config.lexicon, local_lexicon, "主词库属于词库这一类");
    assert_eq!(
        std::fs::read_to_string(&local_lexicon).unwrap(),
        "ben ji\t本机\t1000\n"
    );
    assert_eq!(target.store.list_quick_phrases()[0].code, "yx");
    assert_eq!(target.store.list_dictionaries()[0].name, "旧词库");
    assert_eq!(target.engine.as_ref().unwrap().history_count("旧词"), 1);
    assert_eq!(target.recent_emoji, ["🎉"]);
    cleanup(&target);

    // 只有学习记录：学到的词和最近的表情换掉，别的不动
    let mut target = local_profile_host("parts-single-learning");
    assert!(
        target
            .handle(1, learning_request("import_settings", &learning))
            .ok
    );
    let engine = target.engine.as_ref().unwrap();
    assert_eq!(
        (engine.history_count("旧词"), engine.history_count("私密词")),
        (0, 3)
    );
    assert_eq!(target.recent_emoji, ["😊"]);
    assert_eq!((target.config.page_size, target.mode), (8, Mode::Chinese));
    assert_eq!(
        target.config.lexicon,
        target
            .data_dir()
            .join("local-lexicon.tsv")
            .to_string_lossy()
    );
    assert_eq!(target.store.list_quick_phrases()[0].code, "yx");
    assert_eq!(target.store.load_common_phrases().unwrap(), ["本机常用语"]);
    assert_eq!(target.store.list_dictionaries()[0].name, "旧词库");
    // 合并（云端恢复用的那种）遇到没有学习记录的文件也不出错
    assert!(
        target
            .handle(1, learning_request("merge_settings", &settings))
            .ok
    );
    assert_eq!(target.engine.as_ref().unwrap().history_count("私密词"), 3);
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn backup_with_no_parts_or_half_a_part_changes_nothing() {
    let mut host = local_profile_host("parts-empty");
    let path = host.data_dir().join("none.json");
    // 一类都没选（设置程序只导出同步账号时）：文件有效，导入不改数据
    assert!(host.handle(1, export_parts_request(&path, &[])).ok);
    assert_eq!(backup_keys(&path), ["format", "version"]);
    let inspected = host.handle(1, learning_request("inspect_settings_backup", &path));
    assert_eq!(
        inspected.report.unwrap().note.as_deref(),
        Some("没有输入法数据")
    );
    assert!(
        host.handle(1, learning_request("import_settings", &path))
            .ok
    );
    assert_eq!(
        (host.config.page_size, host.store.list_quick_phrases().len()),
        (8, 1)
    );
    assert_eq!(host.engine.as_ref().unwrap().history_count("旧词"), 1);
    // 一类数据只有一半字段：拒绝，不当成「没有这一类」
    std::fs::write(
        &path,
        r#"{"format":"freetype-settings","version":1,"recent_emoji":[]}"#,
    )
    .unwrap();
    let response = host.handle(1, learning_request("import_settings", &path));
    assert!(
        !response.ok
            && response
                .error
                .as_deref()
                .is_some_and(|e| e.contains("学习记录")),
        "{response:?}"
    );
    std::fs::write(
        &path,
        r#"{"format":"freetype-settings","version":1,"mode":"english"}"#,
    )
    .unwrap();
    assert!(
        !host
            .handle(1, learning_request("import_settings", &path))
            .ok
    );
    assert_eq!(host.mode, Mode::Chinese);
    cleanup(&host);
}

#[test]
fn settings_backup_rejects_invalid_records_before_changing_any_data() {
    let mut host = learning_host("settings-invalid");
    let original = host.engine.as_ref().unwrap().export_learning().unwrap();
    let path = host.data_dir().join("all.json");
    assert!(
        host.handle(1, learning_request("export_settings", &path))
            .ok
    );
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut corruptions = Vec::new();
    let mut bad = data.clone();
    bad["version"] = serde_json::json!(99);
    corruptions.push(bad);
    let mut bad = data.clone();
    bad["common_phrases"] = serde_json::json!(["有效", " "]);
    corruptions.push(bad);
    let mut bad = data.clone();
    bad["dictionaries"] = serde_json::json!([{"name": "../escape", "text": "词\tci\t100\n"}]);
    corruptions.push(bad);
    let mut bad = data.clone();
    bad["learning"]["history"] = serde_json::json!([["", 1, 1]]);
    corruptions.push(bad);
    let mut bad = data.clone();
    bad["config"]["page_size"] = serde_json::json!(999);
    corruptions.push(bad);
    for bad in corruptions {
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        let result = host.handle(1, learning_request("import_settings", &path));
        assert!(!result.ok, "{result:?}");
        assert_eq!(host.config.page_size, 7);
        assert_eq!(
            host.engine.as_ref().unwrap().export_learning().unwrap(),
            original
        );
        assert!(!host.store.user_dict_path().exists());
    }
    cleanup(&host);
}

#[test]
fn restoring_learning_drains_the_previous_background_save() {
    let mut host = learning_host("restore-pending-save");
    let backup = host.data_dir().join("saved.json");
    assert!(
        host.handle(1, learning_request("export_settings", &backup))
            .ok
    );
    let path = host.store.user_dict_path();
    let old_path = path.clone();
    host.saver
        .enqueue(
            "learning",
            Box::new(move || {
                // 模拟被磁盘延迟的旧快照。恢复必须在该写入结束后才替换文件。
                std::thread::sleep(std::time::Duration::from_millis(200));
                std::fs::write(old_path, "old snapshot").map_err(|error| error.to_string())
            }),
        )
        .unwrap();
    assert!(
        host.handle(1, learning_request("import_settings", &backup))
            .ok
    );
    host.saver.result(true).unwrap();
    assert_ne!(std::fs::read_to_string(path).unwrap(), "old snapshot");
    cleanup(&host);
}

#[test]
fn settings_backup_io_failures_preserve_current_profile() {
    let mut host = learning_host("settings-io");
    let path = host.data_dir().join("all.json");
    assert!(
        host.handle(1, learning_request("export_settings", &path))
            .ok
    );
    assert!(
        !host
            .handle(1, learning_request("export_settings", &host.data_dir()))
            .ok
    );
    let config_path = host.data_dir().join("config.json");
    std::fs::create_dir(&config_path).unwrap();
    let result = host.handle(1, learning_request("import_settings", &path));
    assert!(!result.ok, "目标位置不可写时应报告失败");
    assert_eq!(host.config.page_size, 7);
    assert!(
        !host.store.user_dict_path().exists(),
        "不应留下半份恢复结果"
    );
    cleanup(&host);
}

#[cfg(windows)]
#[test]
fn settings_backup_rolls_back_files_if_a_later_file_is_locked() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut host = learning_host("settings-locked");
    let path = host.data_dir().join("all.json");
    assert!(
        host.handle(1, learning_request("export_settings", &path))
            .ok
    );
    configure(&mut host, |c| c.page_size = 9);
    host.engine
        .as_ref()
        .unwrap()
        .record_selection_with_pinyin("保留词", Some("bao liu ci"));
    host.save_learning().unwrap();
    let config_before = std::fs::read(host.data_dir().join("config.json")).unwrap();
    let history_before = std::fs::read(host.store.history_path()).unwrap();
    let learning_before = host.engine.as_ref().unwrap().export_learning().unwrap();
    // 允许读取和检查属性，但阻止改名；user.tsv 在其他文件替换之后才处理。
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(host.store.user_dict_path())
        .unwrap();
    let response = host.handle(1, learning_request("import_settings", &path));
    assert!(!response.ok);
    assert_eq!(
        std::fs::read(host.data_dir().join("config.json")).unwrap(),
        config_before
    );
    assert_eq!(
        std::fs::read(host.store.history_path()).unwrap(),
        history_before
    );
    assert_eq!(
        host.engine.as_ref().unwrap().export_learning().unwrap(),
        learning_before
    );
    assert_eq!(host.config.page_size, 9);
    drop(locked);
    cleanup(&host);
}

#[cfg(windows)]
#[test]
fn settings_backup_rollback_preserves_case_of_existing_dictionary_names() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut source = learning_host("settings-case-source");
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(
        source.data_dir().join("dicts/terms.txt"),
        "你好\tni hao\t100\n",
    )
    .unwrap();
    let backup = source.data_dir().join("all.json");
    assert!(
        source
            .handle(1, learning_request("export_settings", &backup))
            .ok
    );
    let mut target = learning_host("settings-case-target");
    configure(&mut target, |c| c.page_size = 9);
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    let dictionary = target.data_dir().join("dicts/Terms.txt");
    std::fs::write(&dictionary, "原词\tyuan ci\t500\n").unwrap();
    target.save_learning().unwrap();
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(target.store.user_dict_path())
        .unwrap();
    let response = target.handle(1, learning_request("import_settings", &backup));
    assert!(!response.ok);
    assert_eq!(
        std::fs::read_to_string(&dictionary).unwrap(),
        "原词\tyuan ci\t500\n"
    );
    assert_eq!(
        target.store.list_dictionaries()[0].name,
        "Terms",
        "失败回滚应保留原来的词库名称"
    );
    drop(locked);
    let response = target.handle(1, learning_request("import_settings", &backup));
    assert!(response.ok, "{response:?}");
    assert_eq!(
        target.store.list_dictionaries()[0].name,
        "terms",
        "成功恢复应使用备份中的名称"
    );
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn mixed_learning_is_visible_and_restored_by_all_settings_backup() {
    let mut source = learning_host("mixed-profile-source");
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
        "da\t打\t1000\nda\t大\t800\nta\t他\t700\nge\t个\t600\n",
    )
    .unwrap();
    let engine = Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    engine.load_english_str("tag\n");
    source.install_engine(Ok(engine));
    for trial in 1..=2 {
        let frame = typing(&mut source, "datag");
        let index = frame
            .candidates
            .iter()
            .position(|word| word == "打")
            .unwrap();
        let frame = source.handle(1, Request::Select { index }).frame.unwrap();
        let index = frame
            .candidates
            .iter()
            .position(|word| word == "tag")
            .unwrap();
        assert_eq!(
            source
                .handle(1, Request::Select { index })
                .frame
                .unwrap()
                .commit
                .as_deref()
                .map(str::trim_start),
            Some("打 tag")
        );
        assert_eq!(
            source.user_words().iter().any(|word| word.word == "打tag"),
            trial == 2
        );
    }
    let listed = serde_json::to_value(source.handle(1, Request::ListUserWords)).unwrap();
    let word = listed["user_words"]
        .as_array()
        .unwrap()
        .iter()
        .find(|word| word["word"] == "打tag")
        .unwrap();
    assert_eq!(
        word["display"], "打 tag",
        "设置列表应显示实际的混输格式，同时保留删除用的词条标识"
    );
    assert_eq!(word["pinyin"], "datag");
    let path = source.data_dir().join("all.json");
    assert!(
        source
            .handle(1, learning_request("export_settings", &path))
            .ok
    );
    let mut target = learning_host("mixed-profile-target");
    assert!(
        target
            .handle(1, learning_request("import_settings", &path))
            .ok
    );
    assert_eq!(typing(&mut target, "datag").candidates[0], "打tag");
    assert_eq!(press(&mut target, 0x20).commit.as_deref(), Some("打 tag"));
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn chinese_enter_learns_raw_letters_and_confirmed_delete_undoes_only_last_use() {
    let mut host = learning_host("raw-enter");
    for _ in 0..3 {
        typing(&mut host, "apimode");
        let frame = press(&mut host, keysym::RETURN);
        assert_eq!(frame.commit.as_deref(), Some("apimode"));
        assert!(frame.undo_learning);
    }
    assert!(host.user_words().iter().any(|word| word.word == "apimode"));
    assert!(host.handle(1, Request::UndoCommitLearning).ok);
    assert_eq!(host.engine.as_ref().unwrap().history_count("apimode"), 2);
    assert!(!host.user_words().iter().any(|word| word.word == "apimode"));
    cleanup(&host);
}

#[test]
fn personal_english_and_mixed_words_can_be_added_with_input_codes() {
    let mut host = learning_host("mixed-manual-add");
    let response = host.handle(
        1,
        Request::AddUserWord {
            word: "打tag".into(),
            pinyin: "datag".into(),
        },
    );
    assert!(response.ok, "手动添加混输词应接受实际输入码：{response:?}");
    assert_eq!(typing(&mut host, "datag").candidates[0], "打tag");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("打 tag"));
    let response = host.handle(
        1,
        Request::AddUserWord {
            word: "FreeType".into(),
            pinyin: String::new(),
        },
    );
    assert!(response.ok, "纯英文词应可以直接按原词输入：{response:?}");
    assert!(host
        .engine
        .as_ref()
        .unwrap()
        .user_words()
        .iter()
        .any(|(input, word, _)| input == "freetype" && word == "FreeType"));
    cleanup(&host);
}

#[test]
fn forget_request_removes_learned_word() {
    let mut host = host("forget");
    for _ in 0..3 {
        let frame = typing(&mut host, "pin");
        let index = frame.candidates.iter().position(|c| c == "频").unwrap();
        let frame = host.handle(1, Request::Select { index }).frame.unwrap();
        assert_eq!(frame.commit.as_deref(), Some("频"));
    }
    let frame = typing(&mut host, "pin");
    assert_eq!(frame.candidates[0], "频");
    let frame = host.handle(1, Request::Forget { index: 1 }).frame.unwrap();
    assert_eq!(frame.hint.as_deref(), Some("「拼」不是学来的，删不掉"));
    let frame = host.handle(1, Request::Forget { index: 0 }).frame.unwrap();
    assert_eq!(frame.hint.as_deref(), Some("已删除「频」的记忆"));
    assert_eq!(frame.candidates[0], "拼");
    assert_eq!(frame.preedit, "pin", "编辑区不动");
    cleanup(&host);
}

#[test]
fn learned_words_list_forget_and_clear() {
    let mut host = host("learned");
    typing(&mut host, "ni");
    press(&mut host, '1' as u32);
    typing(&mut host, "hao");
    press(&mut host, '1' as u32);
    for word in ["你", "好"] {
        typing(&mut host, "yun");
        let _ = word;
        press(&mut host, 0x20);
    }
    let response = host.handle(1, Request::ListUserWords);
    assert!(response.ok);
    let response = host.handle(
        1,
        Request::ForgetWords {
            words: vec!["韵".into()],
        },
    );
    assert!(response.ok);
    let response = host.handle(1, Request::ClearLearned);
    assert_eq!(response.user_words, Some(Vec::new()));
    assert!(host
        .handle(1, Request::ListUserWords)
        .user_words
        .unwrap()
        .is_empty());
    cleanup(&host);
}

#[test]
fn lexicon_change_is_handed_to_the_loader() {
    let mut host = host("lexicon");
    assert_eq!(host.take_lexicon_change(), None);
    let file = host.data_dir().with_extension("lexicon.tsv");
    std::fs::write(&file, "ce shi\t测试\t100\n").unwrap();
    let path = file.display().to_string();
    configure(&mut host, |c| c.lexicon = path.clone());
    assert_eq!(host.take_lexicon_change().as_deref(), Some(path.as_str()));
    assert_eq!(host.take_lexicon_change(), None, "只交一次");
    let resources = host.data_dir().join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    let default = resources.join("lexicon.tsv");
    std::fs::write(&default, "ni hao\t你好\t100\n").unwrap();
    let loaded = super::load_engine(&resources, &host.data_dir(), &path).unwrap();
    assert_eq!(loaded.path, file);
    assert!(loaded
        .engine
        .input("ceshi")
        .iter()
        .any(|candidate| candidate.text == "测试"));
    for configured in [
        resources.join("missing.tsv").display().to_string(),
        String::new(),
    ] {
        let loaded = super::load_engine(&resources, &host.data_dir(), &configured).unwrap();
        assert_eq!(loaded.path, default);
        assert!(!loaded.invalid_custom_format);
        assert_eq!(loaded.warning.is_none(), configured.is_empty());
        assert!(loaded
            .engine
            .input("nihao")
            .iter()
            .any(|candidate| candidate.text == "你好"));
    }
    let _ = std::fs::remove_file(file);
    cleanup(&host);
}

#[test]
fn reloading_lexicon_preserves_learning_and_deletions_made_during_loading() {
    let mut host = host("reload-learning");
    let old = host.engine.as_ref().unwrap();
    old.add_user_word("ni hao", "你好", 3000.0).unwrap();
    // The loader read this snapshot, then the user kept typing in the old engine.
    let stale = old.export_learning().unwrap();
    let loaded = Engine::new(
        std::sync::Arc::new(wowtype_core::MemoryLexicon::new()),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    loaded.import_learning(&stale).unwrap();
    old.forget_word("你好");
    old.add_user_word("ce shi", "测试", 4000.0).unwrap();
    let expected = old.export_learning().unwrap();
    host.install_engine(Ok(loaded));
    assert_eq!(
        host.engine.as_ref().unwrap().export_learning().unwrap(),
        expected,
        "switching dictionaries must neither lose new learning nor resurrect deleted words"
    );
    cleanup(&host);
}

#[test]
fn command_config_save_failure_is_reported_without_changing_effective_config() {
    let mut host = host("command-save-error");
    std::fs::create_dir_all(host.data_dir().join("config.json")).unwrap();
    for command in [
        Command::TogglePunct,
        Command::ToggleTraditional,
        Command::ToggleStatusBar,
    ] {
        let before = host.config.clone();
        let response = host.handle(1, Request::Command { command });
        assert!(
            !response.ok,
            "failed storage must not be reported as a successful toggle"
        );
        assert_eq!(
            host.config, before,
            "failed storage must preserve the previous setting"
        );
    }
    cleanup(&host);
}

#[test]
fn import_bare_words_and_add_user_word_use_lexicon_readings() {
    let mut host = host("readings");
    let lexicon = host.data_dir().with_extension("readings.tsv");
    std::fs::write(
        &lexicon,
        "shi\t示\t10\nli\t例\t10\nci\t词\t10\nyun\t韵\t10\njiu\t酒\t10\n",
    )
    .unwrap();
    host.lexicon_in_use = lexicon.display().to_string();
    let file = host.data_dir().with_extension("bare.txt");
    std::fs::write(&file, "示例词\n'yun'jiu 韵酒\n").unwrap();
    let response = host.handle(
        1,
        Request::ImportDictionary {
            path: file.display().to_string(),
            name: "词表".into(),
        },
    );
    let report = response.report.unwrap();
    assert_eq!(report.added, 2, "{:?}", report.errors);
    assert!(report.note.unwrap().contains("按常用读音补充拼音"));
    assert!(typing(&mut host, "shilici")
        .candidates
        .iter()
        .any(|c| c == "示例词"));
    press(&mut host, keysym::ESCAPE);

    let response = host.handle(
        1,
        Request::AddUserWord {
            word: "韵例".into(),
            pinyin: String::new(),
        },
    );
    assert!(response.ok, "{:?}", response.error);
    assert!(response
        .user_words
        .unwrap()
        .iter()
        .any(|w| w.word == "韵例" && w.pinyin == "yun li"));
    assert_eq!(typing(&mut host, "yunli").candidates[0], "韵例");
    press(&mut host, keysym::ESCAPE);
    let bad = host.handle(
        1,
        Request::AddUserWord {
            word: "龘".into(),
            pinyin: String::new(),
        },
    );
    assert!(!bad.ok && bad.error.unwrap().contains("请填上拼音"));
    let typed = host.handle(
        1,
        Request::AddUserWord {
            word: "龘".into(),
            pinyin: "da".into(),
        },
    );
    assert!(typed.ok);
    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_file(lexicon);
    cleanup(&host);
}

#[test]
fn traditional_phrase_conversion_and_learning() {
    let mut host = host("trad-learn");
    let lexicon =
        wowtype_core::MemoryLexicon::from_tsv_str("shen me\t什么\t100\nshen me\t神么\t200\n")
            .unwrap();
    let engine = Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    let s2t = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/data/s2t.tsv");
    engine.load_conversion(s2t).unwrap();
    host.install_engine(Ok(engine));
    configure(&mut host, |c| c.traditional = true);

    for _ in 0..3 {
        let f = typing(&mut host, "shenme");
        let idx = f
            .candidates
            .iter()
            .position(|c| c == "什麼")
            .expect("应包含 OpenCC 繁体候选 什麼");
        let chosen = press(&mut host, b'1' as u32 + idx as u32);
        assert_eq!(chosen.commit.as_deref(), Some("什麼"));
    }

    // 学到的词在后台词库按简体 什么 记录，后续无论繁简都能提升排序。
    let words = host.handle(1, Request::ListUserWords).user_words.unwrap();
    assert!(words.iter().any(|w| w.word == "什么"), "{words:?}");
    let again = typing(&mut host, "shenme");
    assert_eq!(again.candidates.first().map(String::as_str), Some("什麼"));
    cleanup(&host);
}

#[test]
fn add_user_word_accepts_tones_and_uppercase() {
    let mut host = host("tones");
    for (word, raw_pinyin, want_pinyin) in [
        ("倪豪", "Ni Hǎo", "ni hao"),
        ("百色", "bai2se4", "bai se"),
        ("女侠", "lǜ xiá", "lv xia"),
        ("略小", "lu:4xiao3", "lv xiao"),
    ] {
        let res = host.handle(
            1,
            Request::AddUserWord {
                word: word.into(),
                pinyin: raw_pinyin.into(),
            },
        );
        assert!(res.ok, "{raw_pinyin}: {:?}", res.error);
        assert!(
            res.user_words
                .unwrap()
                .iter()
                .any(|w| w.word == word && w.pinyin == want_pinyin),
            "{word} -> {want_pinyin}"
        );
    }
    assert_eq!(typing(&mut host, "baixiao").candidates.is_empty(), false);
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "lvxia")
        .candidates
        .iter()
        .any(|c| c == "女侠"));
    cleanup(&host);
}

#[test]
fn merge_settings_adds_remote_data_without_dropping_local_data() {
    let mut source = learning_host("merge-source");
    configure(&mut source, |c| {
        c.page_size = 9;
        c.predict_next = true;
    });
    assert!(
        source
            .handle(
                1,
                Request::SetQuickPhrases {
                    phrases: vec![freetype_protocol::QuickPhrase {
                        code: "dz".into(),
                        text: "远端地址".into(),
                        weight: 20
                    }],
                }
            )
            .ok
    );
    assert!(source.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["远端常用语", "两边都有"]}))).ok);
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(
        source.data_dir().join("dicts/人名.txt"),
        "阿特\ta te\t800\n",
    )
    .unwrap();
    std::fs::write(
        source.data_dir().join("dicts/共有.txt"),
        "远端\tyuan duan\t800\n",
    )
    .unwrap();
    source.recent_emoji = vec!["😊".into()];
    for _ in 0..3 {
        source
            .engine
            .as_ref()
            .unwrap()
            .record_selection_with_pinyin("阿特", Some("a te"));
    }
    let snapshot = source.data_dir().join("snapshot.json");
    assert!(
        source
            .handle(1, learning_request("export_settings", &snapshot))
            .ok
    );

    let mut target = learning_host("merge-target");
    configure(&mut target, |c| {
        c.font_size = 24;
        c.horizontal = false;
    });
    target.mode = Mode::English;
    assert!(
        target
            .handle(
                1,
                Request::SetQuickPhrases {
                    phrases: vec![freetype_protocol::QuickPhrase {
                        code: "yx".into(),
                        text: "本机邮箱".into(),
                        weight: 10
                    }],
                }
            )
            .ok
    );
    assert!(target.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["本机常用语", "两边都有"]}))).ok);
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    std::fs::write(
        target.data_dir().join("dicts/共有.txt"),
        "本机\tben ji\t100\n",
    )
    .unwrap();
    target.recent_emoji = vec!["🎉".into()];
    target
        .engine
        .as_ref()
        .unwrap()
        .record_selection_with_pinyin("旧词", Some("jiu ci"));

    let result = target.handle(1, learning_request("merge_settings", &snapshot));
    assert!(result.ok, "{result:?}");
    // 设置：采纳远端改过的项，本机改过而远端没改的留着
    assert_eq!(target.config.page_size, 9);
    assert!(target.config.predict_next);
    assert_eq!(target.config.font_size, 24);
    assert!(!target.config.horizontal);
    assert_eq!(target.mode, Mode::English, "中英文状态是各设备自己的");
    let codes: Vec<String> = target
        .store
        .list_quick_phrases()
        .into_iter()
        .map(|p| p.code)
        .collect();
    assert_eq!(codes, ["dz", "yx"], "列表按编码排");
    assert_eq!(
        target.store.load_common_phrases().unwrap(),
        ["本机常用语", "两边都有", "远端常用语"]
    );
    let mut names: Vec<String> = target
        .store
        .list_dictionaries()
        .into_iter()
        .map(|d| d.name)
        .collect();
    names.sort();
    assert_eq!(names, ["人名", "共有"]);
    assert_eq!(
        std::fs::read_to_string(target.data_dir().join("dicts/共有.txt")).unwrap(),
        "本机\tben ji\t100\n",
        "同名词库保留本机的"
    );
    assert_eq!(target.recent_emoji, ["🎉", "😊"]);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!(
        (engine.history_count("阿特"), engine.history_count("旧词")),
        (3, 1)
    );
    assert!(
        engine.input("ate").iter().any(|c| c.text == "阿特"),
        "并进来的词马上能用"
    );
    // 再合并一次不重复、不累加
    let again = target.handle(1, learning_request("merge_settings", &snapshot));
    assert!(again.ok);
    assert_eq!(target.store.list_quick_phrases().len(), 2);
    assert_eq!(target.store.load_common_phrases().unwrap().len(), 3);
    assert_eq!(target.engine.as_ref().unwrap().history_count("阿特"), 3);
    // 坏文件不动数据
    let bad = target.data_dir().join("bad.json");
    std::fs::write(&bad, "{}").unwrap();
    assert!(
        !target
            .handle(1, learning_request("merge_settings", &bad))
            .ok
    );
    assert_eq!(target.config.page_size, 9);
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn typing_while_the_dictionary_loads_says_so_and_recovers_once_ready() {
    let dir = std::env::temp_dir().join(format!("freetype-host-loading-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut host = Host::new(Store::new(dir));
    host.handle(
        1,
        Request::Hello {
            session: true,
            app: "notepad.exe".into(),
        },
    );
    let frame = typing(&mut host, "niha");
    assert_eq!(frame.preedit, "niha");
    assert!(frame.candidates.is_empty());
    assert_eq!(
        frame.title.as_deref(),
        Some(LOADING_TITLE),
        "an empty candidate window must explain the wait"
    );
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str("ni hao\t你好\t1000\n").unwrap();
    host.install_engine(Ok(Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    )));
    // Letters typed during loading stay in the preedit and get candidates on the next key.
    let frame = press(&mut host, 'o' as u32);
    assert_eq!(frame.candidates.first().map(String::as_str), Some("你好"));
    assert_eq!(frame.title, None);
    cleanup(&host);
}

#[test]
fn boundary_and_pending_learning_sync_replay_does_not_multiply_or_revive_deletions() {
    let mut a = host("sentence-sync-a");
    let mut b = host("sentence-sync-b");
    a.engine.as_ref().unwrap().remember_boundary(
        "ni hao shi jie",
        vec![
            ("你".into(), "ni".into()),
            ("好世界".into(), "hao shi jie".into()),
        ],
    );
    a.engine
        .as_ref()
        .unwrap()
        .record_merged_word("你好世界", "ni hao shi jie");
    let file = a.data_dir().join("boundary-sync.json");
    let path = file.display().to_string();
    assert!(
        a.handle(1, Request::ExportSyncSettings { path: path.clone() })
            .ok
    );
    assert!(
        b.handle(1, Request::MergeSettings { path: path.clone() })
            .ok
    );
    assert!(
        b.handle(1, Request::MergeSettings { path: path.clone() })
            .ok
    );
    assert_eq!(b.engine.as_ref().unwrap().sentence_boundaries().len(), 1);
    assert!(!b
        .engine
        .as_ref()
        .unwrap()
        .user_words()
        .iter()
        .any(|(_, w, _)| w == "你好世界"));
    b.engine
        .as_ref()
        .unwrap()
        .record_merged_word("你好世界", "ni hao shi jie");
    assert!(b
        .engine
        .as_ref()
        .unwrap()
        .user_words()
        .iter()
        .any(|(_, w, _)| w == "你好世界"));
    cleanup(&a);
    cleanup(&b);
}
