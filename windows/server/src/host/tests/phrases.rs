//! 快捷短语、常用语、导入词库与个人词。
use super::*;

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
