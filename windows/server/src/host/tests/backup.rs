//! 学习备份与全部设置备份的导出、导入、合并与回滚。
use super::*;

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
