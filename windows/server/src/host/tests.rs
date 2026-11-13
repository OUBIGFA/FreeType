use super::*;
use freetype_protocol::{keysym, Panel};

fn host(name: &str) -> Host {
    let dir = std::env::temp_dir().join(format!("freetype-host-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut host = Host::new(Store::new(dir));
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
        "ni hao\t你好\t1000\nni\t你\t900\nhao\t好\t800\nshi jie\t世界\t700\nyun\t韵\t10\npin\t拼\t900\npin\t频\t500\n",
    )
    .unwrap();
    let engine = Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    let _ = engine.load_english_str("GitHub\nhello\n");
    host.install_engine(Ok(engine));
    host.handle(
        1,
        Request::Hello {
            session: true,
            app: "notepad.exe".into(),
        },
    );
    host
}

fn configure(host: &mut Host, change: impl FnOnce(&mut Config)) {
    let mut config = host.handle(1, Request::GetConfig).config.unwrap();
    change(&mut config);
    assert!(host.handle(1, Request::SetConfig { config }).ok);
}

fn key_with(host: &mut Host, id: ClientId, sym: u32, modifiers: u32) -> Frame {
    let key = Key {
        keysym: sym,
        modifiers,
        test: false,
    };
    host.handle(id, Request::Key { key, before: None }).frame.unwrap()
}

fn press(host: &mut Host, sym: u32) -> Frame {
    key_with(host, 1, sym, 0)
}

fn typing(host: &mut Host, text: &str) -> Frame {
    let mut frame = Frame::default();
    for c in text.chars() {
        frame = press(host, c as u32);
    }
    frame
}

fn cleanup(host: &Host) {
    let _ = std::fs::remove_dir_all(host.data_dir());
}

fn learning_request(operation: &str, path: &Path) -> Request {
    serde_json::from_value(serde_json::json!({
        "op": operation,
        "path": path.to_string_lossy(),
    }))
    .expect("学习备份请求应被协议支持")
}

fn learning_host(name: &str) -> Host {
    let host = host(name);
    std::fs::create_dir_all(host.data_dir()).unwrap();
    host
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
    assert!(!source.store.user_dict_path().exists(), "导出必须包含尚未定时存盘的学习");
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
    assert!(response.user_words.unwrap().iter().any(|word| word.word == "阿特"));
    let restarted = Engine::new(
        std::sync::Arc::new(wowtype_core::MemoryLexicon::new()),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    restarted.load_user_dict(restored.store.user_dict_path()).unwrap();
    restarted.load_history(restored.store.history_path()).unwrap();
    assert_eq!(restarted.history_count("阿特"), 3);
    assert_eq!(restarted.history_count("你好"), 1);
    assert!(restarted.pair_count() > 0);
    assert!(restarted.input("ate").iter().any(|candidate| candidate.text == "阿特"));
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
    assert!(engine.user_words().iter().any(|(_, word, _)| word == "霜频阁"));
    let backup = target.data_dir().join("backup.json");
    std::fs::write(&backup, serde_json::to_vec(&serde_json::json!({
        "format": "freetype-learning", "version": 1,
        "words": [["a te", "阿特", 3000.0]],
        "history": [["阿特", 3, 1234], ["你好", 1, 1234]],
        "pairs": [["阿特", "你好", 2, 1234]]
    })).unwrap()).unwrap();
    assert!(target.handle(1, learning_request("import_learning", &backup)).ok);
    let first = std::fs::read(target.store.history_path()).unwrap();
    assert!(target.handle(1, learning_request("import_learning", &backup)).ok);
    assert_eq!(std::fs::read(target.store.history_path()).unwrap(), first);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!(engine.history_count("你好"), 5, "旧备份不能覆盖较新的使用记录");
    assert_eq!(engine.history_count("阿特"), 3, "重复导入不能累加次数");
    assert!(engine.user_words().iter().any(|(_, word, _)| word == "霜频阁"));
    assert_eq!(engine.pair_count(), 1);
    cleanup(&target);
}

#[test]
fn learning_backup_rejects_invalid_records_without_partial_import() {
    let mut target = learning_host("learning-invalid");
    target.engine.as_ref().unwrap().record_selection_with_pinyin("你好", Some("ni hao"));
    let backup = target.data_dir().join("invalid.json");
    let valid = serde_json::json!({
        "format": "freetype-learning", "version": 1,
        "words": [["a te", "阿特", 3000.0]],
        "history": [["阿特", 3, 1234]], "pairs": []
    });
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("format", serde_json::json!("other-app")),
        ("words", serde_json::json!([["a te", "阿特", 3000], ["bad-code!", "坏词", 3]])),
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
        assert!(!engine.user_words().iter().any(|(_, word, _)| word == "阿特"));
    }
    cleanup(&target);
}

#[test]
fn learning_backup_reports_io_failures() {
    let mut target = learning_host("learning-io");
    let missing = target.data_dir().join("missing.json");
    assert!(!target.handle(1, learning_request("import_learning", &missing)).ok);
    let directory = target.data_dir().to_path_buf();
    assert!(!target.handle(1, learning_request("export_learning", &directory)).ok);
    let backup = target.data_dir().join("backup.json");
    target.engine.as_ref().unwrap().record_selection_with_pinyin("阿特", Some("a te"));
    assert!(target.handle(1, learning_request("export_learning", &backup)).ok);
    std::fs::create_dir(target.store.history_path()).unwrap();
    let response = target.handle(1, learning_request("import_learning", &backup));
    assert!(!response.ok, "无法持久化时不能声称恢复成功");
    assert!(target.dirty, "保存失败后仍需重试存盘");
    cleanup(&target);
}

#[test]
fn letters_compose_and_space_commits() {
    let mut host = host("compose");
    let frame = typing(&mut host, "nihao");
    assert!(frame.handled);
    assert_eq!(frame.candidates[0], "你好");
    assert!(frame.forgettable);
    assert_eq!(frame.font_family, freetype_protocol::DEFAULT_FONT);
    assert_eq!(frame.candidate_style, "pink", "候选窗拿到设置里的强调色，默认粉色");
    let frame = press(&mut host, 0x20);
    assert_eq!(frame.commit.as_deref(), Some("你好"));
    assert!(frame.preedit.is_empty());
    assert!(frame.candidates.is_empty());
    cleanup(&host);
}

#[test]
fn punctuation_commits_candidate_then_mark() {
    let mut host = host("punct");
    typing(&mut host, "nihao");
    let frame = press(&mut host, ',' as u32);
    assert!(frame.handled);
    assert_eq!(frame.commit.as_deref(), Some("你好，"));
    assert_eq!(press(&mut host, '.' as u32).commit.as_deref(), Some("。"));
    let frame = press(&mut host, '1' as u32);
    assert!(!frame.handled, "没在组字时数字交给应用");
    assert_eq!(press(&mut host, '.' as u32).commit.as_deref(), Some("."), "数字后面原样");
    cleanup(&host);
}

#[test]
fn auto_pair_puts_caret_inside() {
    let mut host = host("pair");
    let frame = press(&mut host, '(' as u32);
    assert_eq!(frame.commit.as_deref(), Some("（）"));
    assert_eq!(frame.caret_back, 1);
    let frame = press(&mut host, '"' as u32);
    assert_eq!(frame.commit.as_deref(), Some("“”"));
    typing(&mut host, "nihao");
    let frame = press(&mut host, '<' as u32);
    assert_eq!(frame.commit.as_deref(), Some("你好《》"));
    assert_eq!(frame.caret_back, 1);

    configure(&mut host, |c| c.auto_pair = false);
    assert_eq!(press(&mut host, '"' as u32).commit.as_deref(), Some("“"));
    assert_eq!(press(&mut host, '"' as u32).commit.as_deref(), Some("”"), "左右交替");
    let frame = press(&mut host, '(' as u32);
    assert_eq!((frame.commit.as_deref(), frame.caret_back), (Some("（"), 0));
    configure(&mut host, |c| c.fullwidth_punct = false);
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some(","));
    cleanup(&host);
}

#[test]
fn test_then_real_key_uses_one_result() {
    let mut host = host("test-key");
    let key = Key {
        keysym: 'n' as u32,
        modifiers: 0,
        test: true,
    };
    let tested = host.handle(1, Request::Key { key, before: None }).frame.unwrap();
    assert!(tested.handled);
    let real = host
        .handle(
            1,
            Request::Key {
                key: Key { test: false, ..key },
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(real, tested);
    assert_eq!(press(&mut host, 'i' as u32).candidates[0], "你");
    cleanup(&host);
}

#[test]
fn quotes_are_not_flipped_twice_by_test_then_real() {
    let mut host = host("quote-test");
    configure(&mut host, |c| c.auto_pair = false);
    for expected in ["“", "”", "“"] {
        let key = Key {
            keysym: '"' as u32,
            modifiers: 0,
            test: true,
        };
        host.handle(1, Request::Key { key, before: None });
        let frame = host
            .handle(
                1,
                Request::Key {
                    key: Key { test: false, ..key },
                    before: Some("好".into()),
                },
            )
            .frame
            .unwrap();
        assert_eq!(frame.commit.as_deref(), Some(expected));
    }
    cleanup(&host);
}

#[test]
fn test_alone_does_not_change_state() {
    let mut host = host("test-only");
    let key = Key {
        keysym: 'n' as u32,
        modifiers: 0,
        test: true,
    };
    host.handle(1, Request::Key { key, before: None });
    assert!(host.handle(1, Request::Status).ok);
    assert!(!press(&mut host, keysym::ESCAPE).handled, "没有真的进编辑区，Esc 交给应用");
    cleanup(&host);
}

#[test]
fn english_mode_passes_keys_and_toggle_commits_letters() {
    let mut host = host("mode");
    typing(&mut host, "nih");
    let frame = host.handle(1, Request::ToggleMode { shift: None }).frame.unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint.as_deref(), Some("英"));
    assert_eq!(frame.commit.as_deref(), Some("nih"));
    assert!(!press(&mut host, 'a' as u32).handled);
    let frame = host.handle(1, Request::ToggleMode { shift: None }).frame.unwrap();
    assert_eq!(frame.mode, Mode::Chinese);
    assert!(press(&mut host, 'a' as u32).handled);
    cleanup(&host);
}

#[test]
fn shift_side_follows_setting_and_hint_can_be_off() {
    let mut host = host("shift-side");
    configure(&mut host, |c| {
        c.shift_toggle = "left".into();
        c.hint_on_toggle = false;
    });
    let frame = host
        .handle(1, Request::ToggleMode { shift: Some("right".into()) })
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::Chinese, "右 Shift 不切");
    let frame = host
        .handle(1, Request::ToggleMode { shift: Some("left".into()) })
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint, None);
    cleanup(&host);
}

#[test]
fn per_app_mode_scope() {
    let mut host = host("app-scope");
    configure(&mut host, |c| c.mode_scope = "app".into());
    host.handle(
        2,
        Request::Hello {
            session: true,
            app: "Code.exe".into(),
        },
    );
    host.handle(1, Request::ToggleMode { shift: None });
    assert!(!press(&mut host, 'a' as u32).handled, "记事本切成英文");
    assert!(key_with(&mut host, 2, 'a' as u32, 0).handled, "另一个应用还是中文");
    configure(&mut host, |c| c.default_mode = DefaultMode::English);
    host.handle(
        3,
        Request::Hello {
            session: true,
            app: "new.exe".into(),
        },
    );
    assert!(!key_with(&mut host, 3, 'a' as u32, 0).handled, "新应用用默认的英文");
    cleanup(&host);
}

#[test]
fn global_default_keeps_manual_mode_across_apps_settings_and_restart() {
    let mut host = host("global-default");
    // 配置经由真实 JSON 协议读取，旧的中文、英文写法继续兼容。
    let parsed = serde_json::from_str::<Config>(r#"{"default_mode":"global","mode_scope":"app"}"#);
    assert!(parsed.is_ok(), "默认状态必须接受全局选项：{parsed:?}");
    let response = host.handle(1, Request::SetConfig { config: parsed.unwrap() });
    assert!(response.ok, "{response:?}");
    let frame = host.handle(1, Request::ToggleMode { shift: None }).frame.unwrap();
    assert_eq!(frame.mode, Mode::English);
    host.handle(1, Request::Focus { active: false, before: None });
    host.handle(2, Request::Hello { session: true, app: "browser.exe".into() });
    let focused = host.handle(2, Request::Focus { active: true, before: None }).frame.unwrap();
    assert_eq!(focused.mode, Mode::English, "全局模式不受旧的按应用设置影响");
    assert!(!key_with(&mut host, 2, 'a' as u32, 0).handled);
    configure(&mut host, |c| c.font_size = 22);
    assert_eq!(host.handle(2, Request::Status).status.unwrap().mode, Mode::English);

    let dir = host.data_dir();
    drop(host);
    let mut restarted = Host::new(Store::new(dir));
    restarted.connect(3, "new.exe".into());
    assert_eq!(restarted.handle(3, Request::Status).status.unwrap().mode, Mode::English);
    let frame = restarted.handle(3, Request::ToggleMode { shift: None }).frame.unwrap();
    assert_eq!(frame.mode, Mode::Chinese);
    let dir = restarted.data_dir();
    drop(restarted);
    let mut restarted = Host::new(Store::new(dir));
    assert_eq!(restarted.handle(3, Request::Status).status.unwrap().mode, Mode::Chinese);
    cleanup(&restarted);
}

#[test]
fn selecting_global_default_keeps_the_active_apps_current_mode() {
    let mut host = host("enable-global");
    configure(&mut host, |c| c.mode_scope = "app".into());
    host.handle(1, Request::ToggleMode { shift: None });
    host.handle(1, Request::Focus { active: true, before: None });
    let mut json = serde_json::to_value(host.config()).unwrap();
    json["default_mode"] = serde_json::json!("global");
    let config = serde_json::from_value::<Config>(json);
    assert!(config.is_ok(), "默认状态必须接受全局选项");
    assert!(host.handle(1, Request::SetConfig { config: config.unwrap() }).ok);
    assert_eq!(host.handle(1, Request::Status).status.unwrap().mode, Mode::English);
    let dir = host.data_dir();
    drop(host);
    let mut restarted = Host::new(Store::new(dir));
    assert_eq!(restarted.handle(1, Request::Status).status.unwrap().mode, Mode::English);
    cleanup(&restarted);
}

#[test]
fn selecting_global_in_settings_after_focus_leaves_keeps_last_manual_mode() {
    let mut host = host("global-after-blur");
    configure(&mut host, |c| c.mode_scope = "app".into());
    host.handle(1, Request::Focus { active: true, before: None });
    host.handle(1, Request::ToggleMode { shift: None });
    // 打开设置窗口会让原来的文本框失焦，设置连接本身没有输入会话。
    host.handle(1, Request::Focus { active: false, before: None });
    let mut config = host.config().clone();
    config.default_mode = DefaultMode::Global;
    assert!(host.handle(90, Request::SetConfig { config }).ok);
    assert_eq!(host.handle(90, Request::Status).status.unwrap().mode, Mode::English,
        "从设置启用全局不能因原应用已失焦而丢掉最后手动选择的英文");
    cleanup(&host);
}

#[test]
fn key_test_does_not_learn_or_forget_before_the_key_is_accepted() {
    let mut host = host("test-learning");
    typing(&mut host, "nihao");
    let before = host.engine.as_ref().unwrap().export_learning().unwrap();
    let preview = Key { keysym: 0x20, test: true, ..Key::default() };
    for _ in 0..3 {
        assert!(host.handle(1, Request::Key { key: preview, before: None }).frame.unwrap().handled);
        assert_eq!(host.engine.as_ref().unwrap().export_learning().unwrap(), before, "反复预判不能提前学习");
    }
    let result = press(&mut host, 0x20);
    assert_eq!(result.commit.as_deref(), Some("你好"));
    let learned = host.engine.as_ref().unwrap().export_learning().unwrap();
    assert_ne!(learned, before, "真正接受按键后才学习");
    let records: serde_json::Value = serde_json::from_str(&learned).unwrap();
    assert_eq!(records["history"][0][1], 1, "重复预判之后一次上屏仍然只学习一次");

    typing(&mut host, "nihao");
    let preview = Key { keysym: keysym::DELETE, modifiers: modifier::CTRL, test: true };
    assert!(host.handle(1, Request::Key { key: preview, before: None }).frame.unwrap().handled);
    assert_eq!(host.engine.as_ref().unwrap().export_learning().unwrap(), learned, "预判删除键不能删除用户记录");
    key_with(&mut host, 1, keysym::DELETE, modifier::CTRL);
    assert_ne!(host.engine.as_ref().unwrap().export_learning().unwrap(), learned);
    cleanup(&host);
}

#[test]
fn global_mode_toggle_invalidates_other_clients_key_predictions() {
    let mut host = host("global-prediction");
    host.connect(2, "browser.exe".into());
    let key = Key { keysym: 'n' as u32, test: true, ..Key::default() };
    assert!(host.handle(1, Request::Key { key, before: None }).frame.unwrap().handled);
    host.handle(2, Request::ToggleMode { shift: None });
    let frame = press(&mut host, 'n' as u32);
    assert_eq!(frame.mode, Mode::English);
    assert!(!frame.handled && frame.preedit.is_empty(), "切换全局状态后不能接纳旧中文状态的按键预判");
    cleanup(&host);
}

#[test]
fn emoji_shortcut_toggles_the_panel_like_upstream() {
    let mut host = host("emoji-toggle");
    configure(&mut host, |c| c.hotkey_emoji = "ctrl+shift+e".into());
    let modifiers = modifier::CTRL | modifier::SHIFT;
    let opened = key_with(&mut host, 1, 'e' as u32, modifiers);
    assert!(!opened.candidates.is_empty());
    let key = Key { keysym: 'e' as u32, modifiers, test: true };
    host.handle(1, Request::Key { key, before: None });
    let closed = key_with(&mut host, 1, 'e' as u32, modifiers);
    assert!(closed.preedit.is_empty() && closed.candidates.is_empty() && closed.title.is_none(),
        "与上游一样，再按一次表情快捷键应关闭面板");
    cleanup(&host);
}

#[test]
fn email_completion_page_keys_match_the_visible_page_arrows() {
    let mut host = host("email-page-keys");
    let first = typing(&mut host, "abc@");
    assert!(first.has_next);
    let next = press(&mut host, keysym::PAGE_DOWN);
    assert!(next.has_prev, "邮箱候选显示下一页按钮时 PgDn 也必须翻页");
    assert_ne!(next.candidates, first.candidates);
    let previous = press(&mut host, keysym::PAGE_UP);
    assert_eq!(previous.candidates, first.candidates);
    assert!(!previous.has_prev);
    cleanup(&host);
}

#[test]
fn caps_lock_letters_go_to_app() {
    let mut host = host("caps");
    assert!(!key_with(&mut host, 1, 'A' as u32, modifier::CAPS).handled);
    cleanup(&host);
}

#[test]
fn composing_swallows_ctrl_combinations() {
    let mut host = host("ctrl");
    typing(&mut host, "ni");
    let frame = key_with(&mut host, 1, 'c' as u32, modifier::CTRL);
    assert!(frame.handled);
    assert!(frame.commit.is_none());
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
    assert!(typing(&mut host, "shilici").candidates.iter().any(|c| c == "示例词"));

    configure(&mut host, |c| c.quick_phrases = false);
    press(&mut host, keysym::ESCAPE);
    assert!(!typing(&mut host, "yyjq").candidates.iter().any(|c| c == "韵酒"));
    let _ = std::fs::remove_file(file);
    cleanup(&host);
}

#[test]
fn select_and_page_and_reset() {
    let mut host = host("select");
    configure(&mut host, |c| c.autospace = false);
    typing(&mut host, "ni");
    let frame = host.handle(1, Request::Select { index: 0 }).frame.unwrap();
    assert_eq!(frame.commit.as_deref(), Some("你"));
    typing(&mut host, "hao");
    let frame = host.handle(1, Request::Reset { commit: true }).frame.unwrap();
    assert_eq!(frame.commit.as_deref(), Some("hao"));
    assert!(host.handle(1, Request::Page { next: true }).ok);
    cleanup(&host);
}

#[test]
fn keys_before_engine_loads_still_compose() {
    let dir = std::env::temp_dir().join(format!("freetype-host-noengine-{}", std::process::id()));
    let mut host = Host::new(Store::new(dir.clone()));
    host.handle(
        7,
        Request::Hello {
            session: true,
            app: String::new(),
        },
    );
    assert!(key_with(&mut host, 7, 'n' as u32, 0).handled);
    assert!(!host.handle(7, Request::Status).status.unwrap().ready);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn v_space_opens_toolbar_and_phrases_commit() {
    let mut host = host("toolbar");
    host.handle(
        1,
        Request::SetQuickPhrases {
            phrases: vec![freetype_protocol::QuickPhrase {
                code: "dz".into(),
                text: "海淀区".into(),
                weight: 10,
            }],
        },
    );
    typing(&mut host, "v");
    let frame = press(&mut host, 0x20);
    assert!(frame.handled);
    assert_eq!(frame.title.as_deref(), Some("工具栏"));
    assert_eq!(frame.candidates, vec!["剪贴板", "常用语", "符号", "表情"]);
    assert!(!frame.forgettable);
    let frame = press(&mut host, '2' as u32);
    assert_eq!(frame.panel, Panel::List);
    assert_eq!(frame.candidates[0], "海淀区  dz");
    let frame = press(&mut host, '1' as u32);
    assert_eq!(frame.commit.as_deref(), Some("海淀区"));
    assert!(frame.title.is_none() && frame.candidates.is_empty());

    configure(&mut host, |c| c.toolbar = false);
    typing(&mut host, "v");
    let frame = press(&mut host, 0x20);
    assert_ne!(frame.title.as_deref(), Some("工具栏"));
    cleanup(&host);
}

fn settings_request(value: serde_json::Value) -> Request {
    serde_json::from_value(value).expect("设置数据请求应被协议支持")
}

#[test]
fn common_phrases_round_trip_preserves_multiline_and_merges_without_duplicates() {
    let mut source = learning_host("common-source");
    let response = source.handle(1, settings_request(serde_json::json!({
        "op": "set_common_phrases", "phrases": ["您好FreeType", "第一行\n第二行", "您好FreeType"],
    })));
    assert!(response.ok, "{response:?}");
    let exported = source.data_dir().join("common.json");
    assert!(source.handle(1, learning_request("export_common_phrases", &exported)).ok);
    let mut target = learning_host("common-target");
    for _ in 0..2 {
        let response = target.handle(1, settings_request(serde_json::json!({
            "op": "import_common_phrases", "path": exported, "replace": false,
        })));
        assert!(response.ok, "{response:?}");
    }
    let listed = serde_json::to_value(target.handle(1, settings_request(serde_json::json!({"op": "list_common_phrases"})))).unwrap();
    assert_eq!(listed["common_phrases"], serde_json::json!(["您好FreeType", "第一行\n第二行"]));
    let mut restarted = Host::new(Store::new(target.data_dir()));
    let listed = serde_json::to_value(restarted.handle(1, settings_request(serde_json::json!({"op": "list_common_phrases"})))).unwrap();
    assert_eq!(listed["common_phrases"], serde_json::json!(["您好FreeType", "第一行\n第二行"]));
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn common_phrases_toolbar_uses_new_list_and_keeps_existing_quick_phrases() {
    let mut host = host("common-toolbar");
    assert!(host.handle(1, settings_request(serde_json::json!({
        "op": "set_common_phrases", "phrases": ["欢迎FreeType第5版"],
    }))).ok);
    assert!(host.handle(1, Request::SetQuickPhrases {
        phrases: vec![freetype_protocol::QuickPhrase { code: "dz".into(), text: "海淀区".into(), weight: 10 }],
    }).ok);
    typing(&mut host, "v");
    press(&mut host, 0x20);
    let frame = press(&mut host, '2' as u32);
    assert_eq!(frame.candidates, vec!["欢迎FreeType第5版", "海淀区  dz"]);
    assert_eq!(press(&mut host, '1' as u32).commit.as_deref(), Some("欢迎 FreeType 第 5 版"));
    typing(&mut host, "dz");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("海淀区"));
    cleanup(&host);
}

#[test]
fn common_phrases_invalid_import_preserves_data_and_text_import_supports_utf8() {
    let mut host = learning_host("common-invalid");
    assert!(host.handle(1, settings_request(serde_json::json!({
        "op": "set_common_phrases", "phrases": ["保留"],
    }))).ok);
    let path = host.data_dir().join("import.json");
    for contents in ["{bad", r#"{"format":"freetype-common-phrases","version":9,"phrases":["覆盖"]}"#,
        r#"{"format":"freetype-common-phrases","version":1,"phrases":["有效"," "]}"#] {
        std::fs::write(&path, contents).unwrap();
        assert!(!host.handle(1, settings_request(serde_json::json!({
            "op": "import_common_phrases", "path": path, "replace": true,
        }))).ok);
        let listed = serde_json::to_value(host.handle(1, settings_request(serde_json::json!({"op": "list_common_phrases"})))).unwrap();
        assert_eq!(listed["common_phrases"], serde_json::json!(["保留"]));
    }
    std::fs::write(&path, "\u{feff}第一条\r\n\r\n第二条\r\n第一条\r\n").unwrap();
    let response = host.handle(1, settings_request(serde_json::json!({
        "op": "import_common_phrases", "path": path, "replace": true,
    })));
    assert!(response.ok, "{response:?}");
    let listed = serde_json::to_value(host.handle(1, settings_request(serde_json::json!({"op": "list_common_phrases"})))).unwrap();
    assert_eq!(listed["common_phrases"], serde_json::json!(["第一条", "第二条"]));
    cleanup(&host);
}

#[test]
fn common_phrases_empty_backup_can_restore_an_empty_list() {
    let mut host = learning_host("common-empty");
    let path = host.data_dir().join("empty.json");
    assert!(host.handle(1, learning_request("export_common_phrases", &path)).ok);
    assert!(host.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["待替换"]}))).ok);
    assert!(host.handle(1, settings_request(serde_json::json!({"op": "import_common_phrases", "path": path, "replace": true}))).ok);
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
        c.lexicon = main_lexicon.to_string_lossy().into();
        c.default_mode = DefaultMode::Global;
    });
    source.mode = Mode::English;
    assert!(source.handle(1, Request::SetQuickPhrases {
        phrases: vec![freetype_protocol::QuickPhrase { code: "dz".into(), text: "第一行\n第二行".into(), weight: 20 }],
    }).ok);
    assert!(source.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["常用FreeType"]}))).ok);
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(source.data_dir().join("dicts/人名.txt"), "阿特\ta te\t800\n").unwrap();
    source.recent_emoji = vec!["😊".into()];
    std::fs::write(source.data_dir().join("statusbar.txt"), "100,200\n").unwrap();
    let engine = source.engine.as_ref().unwrap();
    for _ in 0..3 { engine.record_selection_with_pinyin("阿特", Some("a te")); }
    engine.set_context("阿特");
    engine.input("nihao");
    engine.record_selection_with_pinyin("你好", Some("ni hao"));
    assert!(!source.store.user_dict_path().exists());
    let exported = source.data_dir().join("all.json");
    let result = source.handle(1, learning_request("export_settings", &exported));
    assert!(result.ok, "{result:?}");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(&exported).unwrap()).unwrap();
    assert_eq!(data["format"], "freetype-settings");
    assert_eq!(data["version"], 1);
    assert_eq!(data["config"]["lexicon"], "", "备份不依赖来源机器的路径");
    assert_eq!(data["config"]["auto_update"], true);

    let mut target = learning_host("settings-target");
    target.engine.as_ref().unwrap().record_selection_with_pinyin("旧词", Some("jiu ci"));
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    std::fs::write(target.data_dir().join("dicts/旧词库.txt"), "旧词\tjiu ci\t100\n").unwrap();
    let inspected = target.handle(1, learning_request("inspect_settings_backup", &exported));
    assert!(inspected.ok, "{inspected:?}");
    assert_eq!(target.config.page_size, 7, "预览不能修改设置");
    let result = target.handle(1, learning_request("import_settings", &exported));
    assert!(result.ok, "{result:?}");
    assert_eq!(target.config.page_size, 9);
    assert!(!target.config.autospace_digits && !target.config.tray_icon);
    assert!(target.config.auto_update, "全部设置备份也应恢复自动更新开关");
    assert_eq!(target.mode, Mode::English);
    assert_eq!(target.store.list_quick_phrases()[0].text, "第一行\n第二行");
    assert_eq!(target.store.load_common_phrases().unwrap(), vec!["常用FreeType"]);
    assert_eq!(target.store.list_dictionaries()[0].name, "人名");
    assert_eq!(target.store.list_dictionaries().len(), 1, "恢复快照不应残留旧词库");
    assert_eq!(target.recent_emoji, vec!["😊"]);
    assert_eq!(std::fs::read_to_string(target.data_dir().join("statusbar.txt")).unwrap(), "100,200\n");
    assert_eq!(std::fs::read_to_string(&target.config.lexicon).unwrap(), "ni hao\t你好\t1000\na te\t阿特\t900\n");
    assert!(Path::new(&target.config.lexicon).starts_with(target.data_dir()));
    assert_eq!(target.engine.as_ref().unwrap().history_count("阿特"), 3);
    assert_eq!(target.engine.as_ref().unwrap().history_count("旧词"), 0);
    let restarted = Engine::new(std::sync::Arc::new(wowtype_core::MemoryLexicon::new()), std::sync::Arc::new(wowtype_core::BigramLm::default()));
    restarted.load_user_dict(target.store.user_dict_path()).unwrap();
    restarted.load_history(target.store.history_path()).unwrap();
    assert_eq!(restarted.history_count("阿特"), 3);
    assert!(restarted.pair_count() > 0);
    assert!(restarted.input("ate").iter().any(|candidate| candidate.text == "阿特"));
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn settings_backup_rejects_invalid_records_before_changing_any_data() {
    let mut host = learning_host("settings-invalid");
    let original = host.engine.as_ref().unwrap().export_learning().unwrap();
    let path = host.data_dir().join("all.json");
    assert!(host.handle(1, learning_request("export_settings", &path)).ok);
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut corruptions = Vec::new();
    let mut bad = data.clone(); bad["version"] = serde_json::json!(99); corruptions.push(bad);
    let mut bad = data.clone(); bad["common_phrases"] = serde_json::json!(["有效", " "]); corruptions.push(bad);
    let mut bad = data.clone(); bad["dictionaries"] = serde_json::json!([{"name": "../escape", "text": "词\tci\t100\n"}]); corruptions.push(bad);
    let mut bad = data.clone(); bad["learning"]["history"] = serde_json::json!([["", 1, 1]]); corruptions.push(bad);
    let mut bad = data.clone(); bad["config"]["page_size"] = serde_json::json!(999); corruptions.push(bad);
    for bad in corruptions {
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        let result = host.handle(1, learning_request("import_settings", &path));
        assert!(!result.ok, "{result:?}");
        assert_eq!(host.config.page_size, 7);
        assert_eq!(host.engine.as_ref().unwrap().export_learning().unwrap(), original);
        assert!(!host.store.user_dict_path().exists());
    }
    cleanup(&host);
}

#[test]
fn settings_backup_io_failures_preserve_current_profile() {
    let mut host = learning_host("settings-io");
    let path = host.data_dir().join("all.json");
    assert!(host.handle(1, learning_request("export_settings", &path)).ok);
    assert!(!host.handle(1, learning_request("export_settings", &host.data_dir())).ok);
    let config_path = host.data_dir().join("config.json");
    std::fs::create_dir(&config_path).unwrap();
    let result = host.handle(1, learning_request("import_settings", &path));
    assert!(!result.ok, "目标位置不可写时应报告失败");
    assert_eq!(host.config.page_size, 7);
    assert!(!host.store.user_dict_path().exists(), "不应留下半份恢复结果");
    cleanup(&host);
}

#[cfg(windows)]
#[test]
fn settings_backup_rolls_back_files_if_a_later_file_is_locked() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut host = learning_host("settings-locked");
    let path = host.data_dir().join("all.json");
    assert!(host.handle(1, learning_request("export_settings", &path)).ok);
    configure(&mut host, |c| c.page_size = 9);
    host.engine.as_ref().unwrap().record_selection_with_pinyin("保留词", Some("bao liu ci"));
    host.save_learning().unwrap();
    let config_before = std::fs::read(host.data_dir().join("config.json")).unwrap();
    let history_before = std::fs::read(host.store.history_path()).unwrap();
    let learning_before = host.engine.as_ref().unwrap().export_learning().unwrap();
    // 允许读取和检查属性，但阻止改名；user.tsv 在其他文件替换之后才处理。
    let locked = std::fs::OpenOptions::new().read(true).share_mode(1).open(host.store.user_dict_path()).unwrap();
    let response = host.handle(1, learning_request("import_settings", &path));
    assert!(!response.ok);
    assert_eq!(std::fs::read(host.data_dir().join("config.json")).unwrap(), config_before);
    assert_eq!(std::fs::read(host.store.history_path()).unwrap(), history_before);
    assert_eq!(host.engine.as_ref().unwrap().export_learning().unwrap(), learning_before);
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
    std::fs::write(source.data_dir().join("dicts/terms.txt"), "你好\tni hao\t100\n").unwrap();
    let backup = source.data_dir().join("all.json");
    assert!(source.handle(1, learning_request("export_settings", &backup)).ok);
    let mut target = learning_host("settings-case-target");
    configure(&mut target, |c| c.page_size = 9);
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    let dictionary = target.data_dir().join("dicts/Terms.txt");
    std::fs::write(&dictionary, "原词\tyuan ci\t500\n").unwrap();
    target.save_learning().unwrap();
    let locked = std::fs::OpenOptions::new().read(true).share_mode(1).open(target.store.user_dict_path()).unwrap();
    let response = target.handle(1, learning_request("import_settings", &backup));
    assert!(!response.ok);
    assert_eq!(std::fs::read_to_string(&dictionary).unwrap(), "原词\tyuan ci\t500\n");
    assert_eq!(target.store.list_dictionaries()[0].name, "Terms", "失败回滚应保留原来的词库名称");
    drop(locked);
    let response = target.handle(1, learning_request("import_settings", &backup));
    assert!(response.ok, "{response:?}");
    assert_eq!(target.store.list_dictionaries()[0].name, "terms", "成功恢复应使用备份中的名称");
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn mixed_learning_is_visible_and_restored_by_all_settings_backup() {
    let mut source = learning_host("mixed-profile-source");
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str("da\t打\t1000\nda\t大\t800\nta\t他\t700\nge\t个\t600\n").unwrap();
    let engine = Engine::new(std::sync::Arc::new(lexicon), std::sync::Arc::new(wowtype_core::BigramLm::default()));
    engine.load_english_str("tag\n");
    source.install_engine(Ok(engine));
    let frame = typing(&mut source, "datag");
    let index = frame.candidates.iter().position(|word| word == "打").unwrap();
    let frame = source.handle(1, Request::Select { index }).frame.unwrap();
    let index = frame.candidates.iter().position(|word| word == "tag").unwrap();
    assert_eq!(source.handle(1, Request::Select { index }).frame.unwrap().commit.as_deref(), Some("打 tag"));
    let listed = serde_json::to_value(source.handle(1, Request::ListUserWords)).unwrap();
    let word = listed["user_words"].as_array().unwrap().iter().find(|word| word["word"] == "打tag").unwrap();
    assert_eq!(word["display"], "打 tag", "设置列表应显示实际的混输格式，同时保留删除用的词条标识");
    assert_eq!(word["pinyin"], "datag");
    let path = source.data_dir().join("all.json");
    assert!(source.handle(1, learning_request("export_settings", &path)).ok);
    let mut target = learning_host("mixed-profile-target");
    assert!(target.handle(1, learning_request("import_settings", &path)).ok);
    assert_eq!(typing(&mut target, "datag").candidates[0], "打tag");
    assert_eq!(press(&mut target, 0x20).commit.as_deref(), Some("打 tag"));
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn personal_english_and_mixed_words_can_be_added_with_input_codes() {
    let mut host = learning_host("mixed-manual-add");
    let response = host.handle(1, Request::AddUserWord { word: "打tag".into(), pinyin: "datag".into() });
    assert!(response.ok, "手动添加混输词应接受实际输入码：{response:?}");
    assert_eq!(typing(&mut host, "datag").candidates[0], "打tag");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("打 tag"));
    let response = host.handle(1, Request::AddUserWord { word: "FreeType".into(), pinyin: String::new() });
    assert!(response.ok, "纯英文词应可以直接按原词输入：{response:?}");
    assert!(host.engine.as_ref().unwrap().user_words().iter().any(|(input, word, _)| input == "freetype" && word == "FreeType"));
    cleanup(&host);
}

#[test]
fn emoji_panel_hotkey_records_recent() {
    let mut host = host("emoji");
    configure(&mut host, |c| c.hotkey_emoji = "ctrl+shift+e".into());
    let test = Key {
        keysym: 'E' as u32,
        modifiers: modifier::CTRL | modifier::SHIFT,
        test: true,
    };
    let frame = host.handle(1, Request::Key { key: test, before: None }).frame.unwrap();
    assert!(frame.handled && frame.title.is_none(), "test 只说要这个键");
    let frame = key_with(&mut host, 1, 'E' as u32, modifier::CTRL | modifier::SHIFT);
    assert_eq!(frame.panel, Panel::Grid);
    let first = frame.candidates[0].clone();
    let frame = press(&mut host, '1' as u32);
    assert_eq!(frame.commit.as_deref(), Some(first.as_str()));
    let frame = host
        .handle(1, Request::Command { command: Command::EmojiPanel })
        .frame
        .unwrap();
    assert_eq!(frame.candidates[0], first, "最近用过的排最前");
    cleanup(&host);
}

#[test]
fn at_sign_offers_mail_domains() {
    let mut host = host("email");
    let frame = press(&mut host, '@' as u32);
    assert_eq!(frame.preedit, "@");
    assert_eq!(frame.candidates[0], "@");
    assert_eq!(frame.candidates[1], "@qq.com");
    assert_eq!(frame.labels[0], "", "数字是正文，不标号");
    let frame = typing(&mut host, "g");
    assert_eq!(frame.candidates, vec!["@g", "@gmail.com"]);
    press(&mut host, keysym::RIGHT);
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("@gmail.com"));

    typing(&mut host, "ni");
    let frame = press(&mut host, '@' as u32);
    assert_eq!(frame.preedit, "ni@");
    let frame = typing(&mut host, "123");
    assert_eq!(frame.preedit, "ni@123", "数字进编辑区");
    assert_eq!(press(&mut host, keysym::RETURN).commit.as_deref(), Some("ni@123"));
    cleanup(&host);
}

#[test]
fn site_name_then_period_completes_tld() {
    let mut host = host("tld");
    configure(&mut host, |c| c.autospace = false);
    typing(&mut host, "gmail");
    let frame = press(&mut host, '.' as u32);
    assert_eq!(frame.preedit, "gmail.");
    assert_eq!(frame.candidates[1], "gmail.com");
    press(&mut host, keysym::DOWN);
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("gmail.com"));
    // 英文单词也算（引擎首选就是它）
    typing(&mut host, "github");
    assert_eq!(press(&mut host, '.' as u32).preedit, "github.");
    press(&mut host, keysym::ESCAPE);
    // 普通拼音照常是句号
    typing(&mut host, "nihao");
    assert_eq!(press(&mut host, '.' as u32).commit.as_deref(), Some("你好。"));

    configure(&mut host, |c| c.email_completion = false);
    let frame = press(&mut host, '@' as u32);
    assert_eq!(frame.commit.as_deref(), Some("@"));
    cleanup(&host);
}

#[test]
fn numpad_dot_is_plain_or_starts_completion() {
    let mut host = host("numpad");
    assert_eq!(press(&mut host, keysym::KP_DECIMAL).commit.as_deref(), Some("."));
    typing(&mut host, "qq");
    let frame = press(&mut host, keysym::KP_DECIMAL);
    assert_eq!(frame.preedit, "qq.");
    assert_eq!(frame.candidates[1], "qq.com");
    cleanup(&host);
}

#[test]
fn hotkeys_toggle_punct_and_traditional_only_when_set() {
    let mut host = host("hotkeys");
    let frame = key_with(&mut host, 1, '.' as u32, modifier::CTRL);
    assert!(!frame.handled, "默认没设快捷键，交给应用");
    configure(&mut host, |c| {
        c.hotkey_punct = "ctrl+period".into();
        c.hotkey_traditional = "ctrl+shift+f".into();
    });
    let frame = key_with(&mut host, 1, '.' as u32, modifier::CTRL);
    assert!(frame.handled);
    assert_eq!(frame.hint.as_deref(), Some("英文标点"));
    assert!(!frame.chinese_punct);
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some(","));
    let frame = key_with(&mut host, 1, 'F' as u32, modifier::CTRL | modifier::SHIFT);
    assert_eq!(frame.hint.as_deref(), Some("繁體"));
    assert!(host.handle(1, Request::GetConfig).config.unwrap().traditional, "存进了设置");
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
    assert!(host.handle(1, Request::ListUserWords).user_words.unwrap().is_empty());
    cleanup(&host);
}

#[test]
fn autospace_between_han_and_ascii() {
    let mut host = host("autospace");
    configure(&mut host, |c| c.autospace = true);
    typing(&mut host, "nihao");
    let frame = press(&mut host, 0x20);
    assert_eq!(frame.commit.as_deref(), Some("你好"));
    assert!(frame.want_before);
    host.handle(1, Request::ToggleMode { shift: None });
    let frame = press(&mut host, 'a' as u32);
    assert!(frame.handled);
    assert_eq!(frame.commit.as_deref(), Some(" a"));
    assert!(!press(&mut host, 'b' as u32).handled, "字母后面的字母照常交给应用");
    host.handle(1, Request::ToggleMode { shift: None });
    typing(&mut host, "shijie");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some(" 世界"));
    // DLL 读到的光标前文字优先
    let key = Key {
        keysym: 'n' as u32,
        modifiers: 0,
        test: false,
    };
    host.handle(
        1,
        Request::Key {
            key,
            before: Some(String::new()),
        },
    );
    typing(&mut host, "i");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("你"));
    cleanup(&host);
}

#[test]
fn autospace_formats_boundaries_inside_a_single_commit() {
    // 一条中英混合候选会整段上屏，不能只检查它与光标前文字的交界。
    let cases = [
        (true, true, true, "自由输入法FreeType第5版", "自由输入法 FreeType 第 5 版"),
        (true, true, false, "FreeType第5版", "FreeType 第5版"),
        (true, false, true, "FreeType第5版", "FreeType第 5 版"),
        (false, true, true, "自由输入法FreeType第5版", "自由输入法FreeType第5版"),
        (true, true, true, "你好 FreeType\n第 5 版\tOK，𠀀A", "你好 FreeType\n第 5 版\tOK，𠀀 A"),
    ];
    for (i, (enabled, letters, digits, text, expected)) in cases.into_iter().enumerate() {
        let mut host = host(&format!("autospace-inside-{i}"));
        configure(&mut host, |c| {
            c.autospace = enabled;
            c.autospace_letters = letters;
            c.autospace_digits = digits;
        });
        assert!(host.handle(1, Request::SetQuickPhrases {
            phrases: vec![freetype_protocol::QuickPhrase { code: "dz".into(), text: text.into(), weight: 10 }],
        }).ok);
        typing(&mut host, "dz");
        assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some(expected));
        cleanup(&host);
    }
}

#[test]
fn focus_reports_mode_hint_when_enabled() {
    let mut host = host("focus");
    let frame = host
        .handle(1, Request::Focus { active: true, before: None })
        .frame
        .unwrap();
    assert_eq!(frame.hint, None);
    configure(&mut host, |c| c.hint_on_focus = true);
    let frame = host
        .handle(1, Request::Focus { active: true, before: None })
        .frame
        .unwrap();
    assert_eq!(frame.hint.as_deref(), Some("中"));
    cleanup(&host);
}

#[test]
fn expanded_panel_marks_cursor_row() {
    let mut host = host("grid");
    let frame = typing(&mut host, "pin");
    assert!(frame.candidates.len() > 1);
    let frame = press(&mut host, keysym::DOWN);
    assert_eq!(frame.panel, Panel::Grid);
    assert_eq!(frame.columns, wowtype_core::session::GRID_COLS);
    assert_eq!(frame.labels[0], "1");
    cleanup(&host);
}

struct Recorder(std::rc::Rc<std::cell::RefCell<Vec<BarState>>>);

impl Desktop for Recorder {
    fn update(&self, state: &BarState) {
        self.0.borrow_mut().push(state.clone());
    }
    fn open_settings(&self) {}
}

#[test]
fn status_bar_follows_focus_and_mode() {
    let mut host = host("bar");
    configure(&mut host, |c| c.status_bar = true);
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    host.set_desktop(Box::new(Recorder(log.clone())));
    assert!(!log.borrow().last().unwrap().visible, "没有输入框时不显示");
    host.handle(1, Request::Focus { active: true, before: None });
    assert!(log.borrow().last().unwrap().visible);
    host.handle(1, Request::ToggleMode { shift: None });
    assert_eq!(log.borrow().last().unwrap().mode, Mode::English);
    host.handle(1, Request::Focus { active: false, before: None });
    assert!(!log.borrow().last().unwrap().visible);
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
    // 文件在就用它，不在用安装目录里的默认词库
    let resources = std::path::Path::new(r"C:\FreeType\data");
    assert_eq!(super::lexicon_path(resources, &path), file);
    assert_eq!(super::lexicon_path(resources, r"C:\没有这个\lexicon.tsv"), resources.join("lexicon.tsv"));
    assert_eq!(super::lexicon_path(resources, ""), resources.join("lexicon.tsv"));
    let engine = super::load_engine(resources, &host.data_dir(), &path).unwrap();
    assert!(engine.input("ceshi").iter().any(|c| c.text == "测试"));
    let _ = std::fs::remove_file(file);
    cleanup(&host);
}

#[test]
fn import_bare_words_and_add_user_word_use_lexicon_readings() {
    let mut host = host("readings");
    let lexicon = host.data_dir().with_extension("readings.tsv");
    std::fs::write(&lexicon, "shi\t示\t10\nli\t例\t10\nci\t词\t10\nyun\t韵\t10\njiu\t酒\t10\n").unwrap();
    host.set_lexicon_in_use(lexicon.display().to_string());
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
    assert!(report.note.unwrap().contains("自动注音"));
    assert!(typing(&mut host, "shilici").candidates.iter().any(|c| c == "示例词"));
    press(&mut host, keysym::ESCAPE);

    let response = host.handle(
        1,
        Request::AddUserWord {
            word: "韵例".into(),
            pinyin: String::new(),
        },
    );
    assert!(response.ok, "{:?}", response.error);
    assert!(response.user_words.unwrap().iter().any(|w| w.word == "韵例" && w.pinyin == "yun li"));
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
