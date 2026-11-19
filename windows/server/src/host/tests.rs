use super::*;
use freetype_protocol::{digit_punct, keysym, Panel};

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

#[test]
fn brand_icon_changes_persist_without_interrupting_composition() {
    let mut host = host("brand-icon");
    typing(&mut host, "ni");
    configure(&mut host, |config| config.brand_icon = "white".into());
    let frame = typing(&mut host, "hao");
    assert_eq!(frame.candidates.first().map(String::as_str), Some("你好"));
    let restarted = Host::new(Store::new(host.data_dir().to_path_buf()));
    assert_eq!(restarted.config().brand_icon, "white");
    cleanup(&host);
}

#[test]
fn language_bar_reads_only_changed_brand_images_without_an_input_session() {
    let mut host = host("language-bar-brand");
    let request = |known: Option<u64>| serde_json::from_value(serde_json::json!({
        "op": "get_brand_icon", "known_revision": known
    })).expect("语言栏必须能单独读取用于显示的品牌图像");
    let first = serde_json::to_value(host.handle(99, request(None))).unwrap();
    assert_eq!(first["brand_icon"]["appearance"], "default");
    assert!(first.get("config").is_none(), "图标读取不能暴露其他设置");
    assert!(!host.clients.contains_key(&99), "读取图像不创建输入会话");
    let revision = first["brand_icon"]["revision"].as_u64().unwrap();
    let unchanged = serde_json::to_value(host.handle(99, request(Some(revision)))).unwrap();
    assert!(unchanged.get("brand_icon").is_none(), "未变图标不能反复传输或解码");
    configure(&mut host, |config| config.font_size = 20);
    assert!(serde_json::to_value(host.handle(99, request(Some(revision)))).unwrap().get("brand_icon").is_none());
    configure(&mut host, |config| config.brand_icon = "white".into());
    let white = serde_json::to_value(host.handle(99, request(Some(revision)))).unwrap();
    assert_eq!(white["brand_icon"]["appearance"], "white");
    assert_ne!(white["brand_icon"]["revision"].as_u64().unwrap(), revision);
    configure(&mut host, |config| {
        config.brand_icon = "custom".into();
        config.custom_brand_icon = "saved-png".into();
    });
    let custom = serde_json::to_value(host.handle(99, request(None))).unwrap();
    assert_eq!(custom["brand_icon"]["custom_png"], "saved-png");
    cleanup(&host);
}

#[test]
fn language_bar_is_notified_only_when_the_displayed_brand_changes() {
    struct Display(std::rc::Rc<std::cell::Cell<u32>>);
    impl Desktop for Display {
        fn update(&self, _: &BarState) {}
        fn open_settings(&self) {}
        fn brand_icon_changed(&self) { self.0.set(self.0.get() + 1); }
    }
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut host = host("brand-notify");
    host.set_desktop(Box::new(Display(notifications.clone())));
    configure(&mut host, |config| config.font_size = 20);
    configure(&mut host, |config| config.custom_brand_icon = "saved-png".into());
    assert_eq!(notifications.get(), 0, "普通设置和未选中的自定义图像不应刷新语言栏");
    configure(&mut host, |config| config.brand_icon = "white".into());
    assert_eq!(notifications.get(), 1);
    configure(&mut host, |config| config.brand_icon = "white".into());
    assert_eq!(notifications.get(), 1, "同一外观不重复通知");
    configure(&mut host, |config| config.brand_icon = "custom".into());
    assert_eq!(notifications.get(), 2);
    configure(&mut host, |config| config.custom_brand_icon = "new-png".into());
    assert_eq!(notifications.get(), 3, "重新导入自定义图标也应立即刷新");
    cleanup(&host);
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
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, '\\' as u32).commit.as_deref(), Some("、"), "数字后面顿号默认仍为全角");
    cleanup(&host);
}

#[test]
fn digit_punctuation_options_and_exceptions() {
    let mut host = host("digit-punct");
    // 默认开启数字标点保持半角与例外，且默认仅勾选顿号
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, ':' as u32).commit.as_deref(), Some(":"));
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, '\\' as u32).commit.as_deref(), Some("、"));

    // 勾选冒号、圆括号作为数字后全角例外
    configure(&mut host, |c| {
        c.digit_punct_fullwidth = digit_punct::DUNHAO | digit_punct::COLON | digit_punct::PAREN;
    });
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, ':' as u32).commit.as_deref(), Some("："));
    press(&mut host, '2' as u32);
    let frame = press(&mut host, '(' as u32);
    assert_eq!((frame.commit.as_deref(), frame.caret_back), (Some("（）"), 1));
    press(&mut host, '3' as u32);
    assert_eq!(press(&mut host, '.' as u32).commit.as_deref(), Some("."), "未勾选的句号仍保持半角");

    // 关掉例外开关：数字后面的顿号、冒号也全部保持半角
    configure(&mut host, |c| c.digit_punct_exceptions = false);
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, '\\' as u32).commit.as_deref(), Some("\\"));
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, ':' as u32).commit.as_deref(), Some(":"));

    // 关掉数字标点保持半角：数字后面的标点一律按全角输出
    configure(&mut host, |c| c.digit_halfwidth_punct = false);
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, '.' as u32).commit.as_deref(), Some("。"));
    press(&mut host, '1' as u32);
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some("，"));
    cleanup(&host);
}

#[test]
fn corner_quotes_replace_curved_double_quotes() {
    let mut host = host("corner-quotes");
    configure(&mut host, |c| c.corner_quotes = true);
    let frame = press(&mut host, '"' as u32);
    assert_eq!((frame.commit.as_deref(), frame.caret_back), (Some("「」"), 1));
    let single = press(&mut host, '\'' as u32);
    assert_eq!((single.commit.as_deref(), single.caret_back), (Some("‘’"), 1));

    configure(&mut host, |c| c.auto_pair = false);
    assert_eq!(press(&mut host, '"' as u32).commit.as_deref(), Some("「"));
    typing(&mut host, "nihao");
    assert_eq!(press(&mut host, '"' as u32).commit.as_deref(), Some("你好」"));
    assert_eq!(press(&mut host, '"' as u32).commit.as_deref(), Some("「"));

    configure(&mut host, |c| c.fullwidth_punct = false);
    assert_eq!(press(&mut host, '"' as u32).commit.as_deref(), Some("\""));
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
    assert_eq!(tested.page_revision, 0, "按键预判不能发布候选页");
    assert_ne!(real.page_revision, 0);
    assert_eq!(Frame { page_revision: 0, ..real }, tested);
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
    let at_top = press(&mut host, keysym::LEFT);
    assert_eq!(at_top.cursor, Some(0), "邮箱补全开头按左键应停在第一项");
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
fn caps_lock_types_english_and_restores_the_remembered_mode() {
    let mut host = host("caps-english");
    let on = host.handle(1, Request::CapsLock { on: true }).frame.unwrap();
    assert_eq!(on.mode, Mode::English, "大写锁定开着：显示和输入都是英文");
    assert_eq!(on.hint.as_deref(), Some("A"));
    assert_eq!(host.handle(1, Request::Status).status.unwrap().mode, Mode::English);
    for sym in [',' as u32, '1' as u32, 'A' as u32] {
        let frame = key_with(&mut host, 1, sym, modifier::CAPS);
        assert!(!frame.handled && frame.commit.is_none(), "标点、数字、字母都原样交给应用");
    }
    let off = host.handle(1, Request::CapsLock { on: false }).frame.unwrap();
    assert_eq!(off.mode, Mode::Chinese, "关掉后回到原来的中文");
    assert_eq!(off.hint.as_deref(), Some("中"));
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some("，"));
    // 重复报同一个状态（输入框得到焦点时也报）不提示
    assert_eq!(host.handle(1, Request::CapsLock { on: false }).frame.unwrap().hint, None);
    cleanup(&host);
}

#[test]
fn caps_lock_commits_half_typed_letters() {
    let mut host = host("caps-composing");
    assert_eq!(typing(&mut host, "ni").preedit, "ni");
    let frame = host.handle(1, Request::CapsLock { on: true }).frame.unwrap();
    assert_eq!(frame.commit.as_deref(), Some("ni"), "打到一半的字母原样上屏");
    assert!(frame.preedit.is_empty() && frame.candidates.is_empty());
    cleanup(&host);
}

#[test]
fn caps_lock_state_follows_key_modifiers_without_a_notification() {
    // 在没有 FreeType 的窗口里开关了大写锁定：服务进程从下一个键带的状态里跟上
    let mut host = host("caps-keys");
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    host.set_desktop(Box::new(Recorder(log.clone())));
    assert!(!key_with(&mut host, 1, ',' as u32, modifier::CAPS).handled);
    assert_eq!(log.borrow().last().unwrap().mode, Mode::English, "状态栏跟着变");
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some("，"), "下一个键没带大写锁定：回到中文");
    assert_eq!(log.borrow().last().unwrap().mode, Mode::Chinese);
    cleanup(&host);
}

#[test]
fn caps_lock_keeps_the_remembered_mode_from_being_toggled() {
    let mut host = host("caps-toggle");
    host.handle(1, Request::CapsLock { on: true });
    let frame = host
        .handle(1, Request::ToggleMode { shift: Some("left".into()) })
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint.as_deref(), Some("A"), "提示还在大写锁定");
    host.handle(1, Request::CapsLock { on: false });
    assert!(press(&mut host, 'n' as u32).handled, "记住的中文没有被切走");
    cleanup(&host);
}

/// 桌面：记状态栏，前台程序和开着的窗口由测试指定。
struct Foreground(std::rc::Rc<std::cell::RefCell<Option<String>>>);

impl Desktop for Foreground {
    fn update(&self, _state: &BarState) {}
    fn open_settings(&self) {}
    fn foreground_app(&self) -> Option<String> {
        self.0.borrow().clone()
    }
    fn open_apps(&self) -> Vec<freetype_protocol::AppWindow> {
        vec![freetype_protocol::AppWindow { app: "code.exe".into(), title: "main.rs".into() }]
    }
}

fn preset(app: &str, mode: Mode) -> freetype_protocol::AppPreset {
    freetype_protocol::AppPreset { app: app.into(), mode }
}

fn status_mode(host: &mut Host) -> Mode {
    host.handle(1, Request::Status).status.unwrap().mode
}

#[test]
fn app_preset_applies_once_when_switching_into_the_app() {
    let mut host = host("preset");
    configure(&mut host, |c| {
        c.app_presets = true;
        // 规则里的大小写不影响匹配
        c.app_preset_rules = vec![preset("Code.EXE", Mode::English), preset("wechat.exe", Mode::Chinese)];
    });
    host.foreground("notepad.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "没有规则的程序不动");
    host.connect(2, "code.exe".into());
    host.foreground("code.exe");
    assert_eq!(status_mode(&mut host), Mode::English, "切进规则里的程序：切一次");
    // 在这个程序里手动切回中文，再换它自己的窗口（查找框、对话框）：不重置
    host.handle(2, Request::ToggleMode { shift: None });
    host.foreground("code.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "同一个程序里不再干预");
    host.foreground("explorer.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "没有规则的程序恢复原状态");
    host.foreground("code.exe");
    assert_eq!(status_mode(&mut host), Mode::English, "离开后再切回来：重新套用一次");
    host.foreground("wechat.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "规则也可以指定中文");
    cleanup(&host);
}

#[test]
fn app_preset_does_nothing_when_switched_off_or_already_in_that_mode() {
    let mut host = host("preset-off");
    configure(&mut host, |c| c.app_preset_rules = vec![preset("code.exe", Mode::English)]);
    host.foreground("code.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "总开关关着：规则不生效");
    // 打开开关只管之后的切入，不追溯现在所在的程序
    configure(&mut host, |c| c.app_presets = true);
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    host.foreground("notepad.exe");
    host.handle(1, Request::ToggleMode { shift: None });
    host.foreground("code.exe");
    let frame = host.handle(1, Request::Focus { active: true, before: None }).frame.unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint, None, "已经是英文：不切换，也不提示");
    cleanup(&host);
}

#[test]
fn app_preset_is_applied_by_focus_and_hints_once() {
    let mut host = host("preset-focus");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    let foreground = std::rc::Rc::new(std::cell::RefCell::new(None));
    host.set_desktop(Box::new(Foreground(foreground.clone())));
    host.handle(2, Request::Hello { session: true, app: "Code.exe".into() });
    // 前台事件还没到，输入框先得到了焦点：焦点这一步自己查前台程序
    *foreground.borrow_mut() = Some("code.exe".to_string());
    let frame = host.handle(2, Request::Focus { active: true, before: None }).frame.unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint.as_deref(), Some("英"), "预设切换了状态：提示一次");
    let frame = host.handle(2, Request::Focus { active: true, before: None }).frame.unwrap();
    assert_eq!(frame.hint, None, "同一个程序里换输入框：不再提示");
    assert!(!key_with(&mut host, 2, 'a' as u32, 0).handled);
    // FreeType 自己的窗口（桌面报 None）不算离开
    *foreground.borrow_mut() = None;
    host.handle(2, Request::ToggleMode { shift: None });
    let frame = host.handle(2, Request::Focus { active: true, before: None }).frame.unwrap();
    assert_eq!(frame.mode, Mode::Chinese, "手动切换的结果留着");
    let apps = host.handle(9, Request::ListApps).apps.unwrap();
    assert_eq!(apps[0].app, "code.exe");
    cleanup(&host);
}

#[test]
fn app_preset_sets_the_apps_own_mode_when_modes_are_per_app() {
    let mut host = host("preset-app-scope");
    configure(&mut host, |c| {
        c.mode_scope = "app".into();
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.handle(2, Request::Hello { session: true, app: "code.exe".into() });
    host.foreground("code.exe");
    assert!(!key_with(&mut host, 2, 'a' as u32, 0).handled, "规则里的程序切成英文");
    assert!(press(&mut host, 'n' as u32).handled, "记事本各记各的，还是中文");
    cleanup(&host);
}

#[test]
fn app_preset_restores_normal_mode_after_leaving() {
    for (name, default_mode, scope) in [
        ("shared", DefaultMode::Chinese, "global"),
        ("remembered", DefaultMode::Global, "app"),
        ("per-app", DefaultMode::Chinese, "app"),
    ] {
        let mut host = host(&format!("preset-restore-{name}"));
        configure(&mut host, |c| {
            c.default_mode = default_mode;
            c.mode_scope = scope.into();
            c.app_presets = true;
            c.app_preset_rules = vec![preset("code.exe", Mode::English)];
        });
        host.connect(2, "code.exe".into());
        host.foreground("notepad.exe");
        host.handle(1, Request::Focus { active: true, before: None });
        for _ in 0..3 {
            host.foreground("code.exe");
            assert_eq!(host.handle(2, Request::Focus { active: true, before: None }).frame.unwrap().mode, Mode::English);
            assert!(!key_with(&mut host, 2, 'n' as u32, 0).handled);
            host.foreground("notepad.exe");
            // 前台通知先于旧输入框失焦到达时，状态也必须已经恢复。
            assert_eq!(status_mode(&mut host), Mode::Chinese, "{name}: 规则不能污染原状态");
            host.handle(1, Request::Focus { active: true, before: None });
            host.handle(2, Request::Focus { active: false, before: None });
            assert!(press(&mut host, 'n' as u32).handled);
            press(&mut host, keysym::ESCAPE);
        }
        cleanup(&host);
    }
}

#[test]
fn app_preset_same_mode_still_isolates_manual_changes_and_chained_rules() {
    let mut host = host("preset-same-mode");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English), preset("chat.exe", Mode::Chinese)];
    });
    host.connect(2, "code.exe".into());
    host.connect(3, "chat.exe".into());
    host.foreground("notepad.exe");
    host.handle(1, Request::ToggleMode { shift: None });
    host.foreground("code.exe");
    assert_eq!(host.handle(2, Request::ToggleMode { shift: None }).frame.unwrap().mode, Mode::Chinese);
    host.foreground("code.exe");
    assert_eq!(key_with(&mut host, 2, 'n' as u32, 0).mode, Mode::Chinese, "软件内部手动切换必须保留");
    host.foreground("chat.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    host.foreground("notepad.exe");
    assert_eq!(status_mode(&mut host), Mode::English, "连续经过多条规则后仍恢复原英文");
    assert!(!press(&mut host, 'n' as u32).handled);
    cleanup(&host);
}

#[test]
fn app_preset_does_not_persist_temporary_mode_when_saving_settings_or_restarting() {
    let mut host = host("preset-persistence");
    configure(&mut host, |c| {
        c.default_mode = DefaultMode::Global;
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    host.foreground("code.exe");
    host.handle(2, Request::Focus { active: true, before: None });
    host.handle(2, Request::ToggleMode { shift: None });
    host.handle(2, Request::ToggleMode { shift: None });
    configure(&mut host, |c| c.font_size = 22);
    assert_eq!(status_mode(&mut host), Mode::English, "保存无关设置不能重置软件内状态");
    let mut restarted = Host::new(Store::new(host.data_dir()));
    assert_eq!(status_mode(&mut restarted), Mode::Chinese, "临时规则及软件内切换不能写成全局记忆");
    host.foreground("notepad.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    cleanup(&host);
}

#[test]
fn app_preset_rejects_delayed_foreground_and_background_focus_notifications() {
    let mut host = host("preset-stale-events");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    let foreground = std::rc::Rc::new(std::cell::RefCell::new(Some("code.exe".into())));
    host.set_desktop(Box::new(Foreground(foreground.clone())));
    host.handle(2, Request::Focus { active: true, before: None });
    host.handle(2, Request::ToggleMode { shift: None });
    // 输入框已查询到 Code 是前台，旧的 Notepad 通知才从另一个队列送达。
    host.foreground("notepad.exe");
    host.foreground("code.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "延迟通知不能重新套用英文规则");
    *foreground.borrow_mut() = Some("notepad.exe".into());
    host.handle(1, Request::Focus { active: true, before: None });
    host.handle(2, Request::Focus { active: true, before: None });
    host.foreground("code.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    // 状态栏按钮必须操作当前软件，不能被后台输入框抢走。
    host.handle(99, Request::ToggleMode { shift: None });
    assert!(!press(&mut host, 'n' as u32).handled);
    cleanup(&host);
}

#[test]
fn app_preset_disabling_or_removing_rule_restores_base_mode() {
    for remove in [false, true] {
        let mut host = host(&format!("preset-disable-{remove}"));
        configure(&mut host, |c| {
            c.default_mode = DefaultMode::Global;
            c.app_presets = true;
            c.app_preset_rules = vec![preset("code.exe", Mode::English)];
        });
        host.connect(2, "code.exe".into());
        host.foreground("code.exe");
        host.handle(2, Request::Focus { active: true, before: None });
        configure(&mut host, |c| {
            if remove { c.app_preset_rules.clear(); } else { c.app_presets = false; }
        });
        assert_eq!(status_mode(&mut host), Mode::Chinese, "关闭或移除规则后恢复原状态");
        cleanup(&host);
    }
}

#[test]
fn app_preset_first_key_observes_foreground_before_delayed_events() {
    let mut host = host("preset-first-key");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    let foreground = std::rc::Rc::new(std::cell::RefCell::new(Some("code.exe".into())));
    host.set_desktop(Box::new(Foreground(foreground.clone())));
    assert!(!key_with(&mut host, 2, 'n' as u32, 0).handled, "首键先于焦点与桌面事件到达仍用预设");
    host.handle(2, Request::ToggleMode { shift: None });
    let preview = Key { keysym: 'n' as u32, test: true, ..Key::default() };
    assert!(host.handle(2, Request::Key { key: preview, before: None }).frame.unwrap().handled);
    *foreground.borrow_mut() = Some("notepad.exe".into());
    host.handle(1, Request::Focus { active: true, before: None });
    *foreground.borrow_mut() = Some("code.exe".into());
    let frame = key_with(&mut host, 2, 'n' as u32, 0);
    assert_eq!(frame.mode, Mode::English);
    assert!(!frame.handled && frame.preedit.is_empty(), "重入后旧中文按键预判必须失效");
    cleanup(&host);
}

#[test]
fn app_preset_keeps_caps_lock_separate_from_restored_mode() {
    let mut host = host("preset-caps");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    host.foreground("code.exe");
    host.handle(2, Request::CapsLock { on: true });
    host.foreground("notepad.exe");
    assert_eq!(status_mode(&mut host), Mode::English);
    host.handle(1, Request::CapsLock { on: false });
    assert_eq!(status_mode(&mut host), Mode::Chinese, "关闭大写锁定后恢复规则之前的中文");
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
        Request::SetCommonPhrases {
            phrases: vec!["海淀区".into()],
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
    assert_eq!(frame.candidates, vec!["海淀区"]);
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
fn common_phrases_toolbar_excludes_quick_phrases_while_codes_still_work() {
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
    assert_eq!(frame.candidates, vec!["欢迎FreeType第5版"]);
    assert_eq!(press(&mut host, '1' as u32).commit.as_deref(), Some("欢迎 FreeType 第 5 版"));
    typing(&mut host, "dz");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("海淀区"));
    cleanup(&host);
}

#[test]
fn empty_common_phrases_toolbar_does_not_fall_back_to_quick_phrases() {
    let mut host = host("empty-common-toolbar");
    assert!(host.handle(1, Request::SetQuickPhrases {
        phrases: vec![freetype_protocol::QuickPhrase { code: "dz".into(), text: "海淀区".into(), weight: 10 }],
    }).ok);
    typing(&mut host, "v");
    press(&mut host, 0x20);
    let frame = press(&mut host, '2' as u32);
    assert!(frame.candidates.is_empty(), "没有常用语时不能用快捷短语填充：{:?}", frame.candidates);
    assert_eq!(frame.title.as_deref(), Some("常用语 · 请在常用语设置页添加内容"));
    press(&mut host, keysym::ESCAPE);
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
        c.show_settings_after_update = true;
        c.brand_icon = "custom".into();
        c.custom_brand_icon = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==".into();
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
    assert_eq!(data["config"]["show_settings_after_update"], true);
    assert_eq!(data["config"]["brand_icon"], "custom");
    assert_eq!(data["config"]["custom_brand_icon"], source.config.custom_brand_icon);

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
    assert!(target.config.show_settings_after_update, "全部设置备份也应恢复安装后显示设置页的开关");
    assert_eq!(target.config.brand_icon, "custom");
    assert_eq!(target.config.custom_brand_icon, source.config.custom_brand_icon);
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

/// 只导出其中几类数据的请求。
fn export_parts_request(path: &Path, parts: &[&str]) -> Request {
    settings_request(serde_json::json!({"op": "export_settings", "path": path.to_string_lossy(), "parts": parts}))
}

/// 每一类数据都有内容的一台「旧电脑」。
fn full_profile_host(name: &str) -> Host {
    let mut source = learning_host(name);
    let main_lexicon = source.data_dir().join("source-lexicon.tsv");
    std::fs::write(&main_lexicon, "ni hao\t你好\t1000\na te\t阿特\t900\n").unwrap();
    configure(&mut source, |c| {
        c.page_size = 9;
        c.lexicon = main_lexicon.to_string_lossy().into();
        c.default_mode = DefaultMode::Global;
    });
    source.mode = Mode::English;
    assert!(source.handle(1, Request::SetQuickPhrases {
        phrases: vec![freetype_protocol::QuickPhrase { code: "dz".into(), text: "远端地址".into(), weight: 20 }],
    }).ok);
    assert!(source.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["远端常用语"]}))).ok);
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(source.data_dir().join("dicts/人名.txt"), "阿特\ta te\t800\n").unwrap();
    source.recent_emoji = vec!["😊".into()];
    std::fs::write(source.data_dir().join("statusbar.txt"), "100,200\n").unwrap();
    for _ in 0..3 { source.engine.as_ref().unwrap().record_selection_with_pinyin("私密词", Some("si mi ci")); }
    source
}

/// 每一类数据都和「旧电脑」不一样的一台「本机」。
fn local_profile_host(name: &str) -> Host {
    let mut target = learning_host(name);
    let main_lexicon = target.data_dir().join("local-lexicon.tsv");
    std::fs::write(&main_lexicon, "ben ji\t本机\t1000\n").unwrap();
    configure(&mut target, |c| {
        c.page_size = 8;
        c.lexicon = main_lexicon.to_string_lossy().into();
    });
    assert!(target.handle(1, Request::SetQuickPhrases {
        phrases: vec![freetype_protocol::QuickPhrase { code: "yx".into(), text: "本机邮箱".into(), weight: 10 }],
    }).ok);
    assert!(target.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["本机常用语"]}))).ok);
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    std::fs::write(target.data_dir().join("dicts/旧词库.txt"), "旧词\tjiu ci\t100\n").unwrap();
    target.recent_emoji = vec!["🎉".into()];
    std::fs::write(target.data_dir().join("statusbar.txt"), "1,2\n").unwrap();
    target.engine.as_ref().unwrap().record_selection_with_pinyin("旧词", Some("jiu ci"));
    target
}

fn backup_keys(path: &Path) -> Vec<String> {
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    data.as_object().unwrap().keys().cloned().collect()
}

#[test]
fn settings_backup_exports_only_the_chosen_parts() {
    let mut source = full_profile_host("parts-export");
    let path = source.data_dir().join("part.json");
    // 没选的类别整个不在文件里：字段没有，内容也搜不到
    let response = source.handle(1, export_parts_request(&path, &["quick_phrases", "dictionaries"]));
    assert!(response.ok, "{response:?}");
    assert_eq!(backup_keys(&path), ["dictionaries", "format", "main_lexicon", "quick_phrases", "version"]);
    let text = std::fs::read_to_string(&path).unwrap();
    for private in ["私密词", "远端常用语", "😊", "page_size"] {
        assert!(!text.contains(private), "没选的数据进了备份：{private}");
    }
    assert_eq!(response.report.unwrap().note.as_deref(), Some("1 份词库、1 条快捷短语；不含设置、常用语、学习记录"));
    // 一类一类地导出：每次只有那一类的字段
    for (part, keys) in [
        ("settings", vec!["config", "format", "mode", "status_bar_position", "version"]),
        ("common_phrases", vec!["common_phrases", "format", "version"]),
        ("learning", vec!["format", "learning", "recent_emoji", "version"]),
    ] {
        assert!(source.handle(1, export_parts_request(&path, &[part])).ok);
        assert_eq!(backup_keys(&path), keys, "{part}");
    }
    // 没选的数据读不了也不妨碍导出：快捷短语文件坏了，只导出设置照样成功
    std::fs::write(source.data_dir().join("quick_phrases.txt"), "这一行不是快捷短语\n").unwrap();
    assert!(source.handle(1, export_parts_request(&path, &["settings"])).ok);
    assert!(!source.handle(1, export_parts_request(&path, &["quick_phrases"])).ok);
    cleanup(&source);
}

#[test]
fn settings_backup_with_all_parts_is_the_same_file_as_before() {
    let mut source = full_profile_host("parts-all");
    let (plain, chosen) = (source.data_dir().join("plain.json"), source.data_dir().join("chosen.json"));
    assert!(source.handle(1, learning_request("export_settings", &plain)).ok);
    let all = ["learning", "common_phrases", "quick_phrases", "dictionaries", "settings"];
    assert!(source.handle(1, export_parts_request(&chosen, &all)).ok);
    let text = std::fs::read_to_string(&plain).unwrap();
    assert_eq!(text, std::fs::read_to_string(&chosen).unwrap(), "全选和不写 parts 是同一份文件");
    // 字段顺序和内容保持原来的格式：自动同步按整份内容比较，旧版本也要能导入；学习记录压为单行紧凑 JSON
    assert!(text.starts_with("{\n  \"format\": \"freetype-settings\",\n  \"version\": 1,\n  \"config\": {"), "{}", &text[..80]);
    let learning_lines: Vec<_> = text.lines().filter(|line| line.starts_with("  \"learning\": ")).collect();
    assert_eq!(learning_lines.len(), 1);
    assert!(learning_lines[0].contains("\"freetype-learning\"") && learning_lines[0].ends_with("},"), "学习记录应保持在单行内：{}", learning_lines[0]);
    let note = source.handle(1, learning_request("inspect_settings_backup", &plain)).report.unwrap().note.unwrap();
    assert_eq!(note, "全部设置、1 份词库、1 条快捷短语、1 条常用语、1 条学到的词及使用记录");
    cleanup(&source);
}

#[test]
fn importing_a_partial_backup_replaces_only_its_parts() {
    let mut source = full_profile_host("parts-source");
    let path = source.data_dir().join("part.json");
    assert!(source.handle(1, export_parts_request(&path, &["quick_phrases", "dictionaries"])).ok);

    let mut target = local_profile_host("parts-target");
    let inspected = target.handle(1, learning_request("inspect_settings_backup", &path));
    assert_eq!(inspected.report.unwrap().note.as_deref(), Some("1 份词库、1 条快捷短语；不含设置、常用语、学习记录"));
    let response = target.handle(1, learning_request("import_settings", &path));
    assert!(response.ok, "{response:?}");
    // 备份里有的两类：整类换成备份里的
    let phrases: Vec<String> = target.store.list_quick_phrases().into_iter().map(|p| p.code).collect();
    assert_eq!(phrases, ["dz"]);
    let dictionaries: Vec<String> = target.store.list_dictionaries().into_iter().map(|d| d.name).collect();
    assert_eq!(dictionaries, ["人名"], "词库整类替换，不残留本机原来的");
    assert_eq!(std::fs::read_to_string(&target.config.lexicon).unwrap(), "ni hao\t你好\t1000\na te\t阿特\t900\n");
    assert!(Path::new(&target.config.lexicon).starts_with(target.data_dir()));
    assert_eq!(typing(&mut target, "dz").candidates[0], "远端地址", "换进来的快捷短语马上能用");
    press(&mut target, keysym::ESCAPE);
    // 备份里没有的三类：本机的原样留着
    assert_eq!(target.config.page_size, 8);
    assert_eq!(target.mode, Mode::Chinese);
    assert_eq!(std::fs::read_to_string(target.data_dir().join("statusbar.txt")).unwrap(), "1,2\n");
    assert_eq!(target.store.load_common_phrases().unwrap(), ["本机常用语"]);
    assert_eq!(response.common_phrases.as_deref(), Some(&["本机常用语".to_string()][..]));
    assert_eq!(target.recent_emoji, ["🎉"]);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!((engine.history_count("旧词"), engine.history_count("私密词")), (1, 0));
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
    let (settings, learning) = (source.data_dir().join("settings.json"), source.data_dir().join("learning.json"));
    assert!(source.handle(1, export_parts_request(&settings, &["settings"])).ok);
    assert!(source.handle(1, export_parts_request(&learning, &["learning"])).ok);

    // 只有设置：选项、中英文状态、状态栏位置换掉；本机的主词库、短语、词库、学习记录不动
    let mut target = local_profile_host("parts-single-settings");
    let local_lexicon = target.config.lexicon.clone();
    assert!(target.handle(1, learning_request("import_settings", &settings)).ok);
    assert_eq!((target.config.page_size, target.mode), (9, Mode::English));
    assert_eq!(std::fs::read_to_string(target.data_dir().join("statusbar.txt")).unwrap(), "100,200\n");
    assert_eq!(target.config.lexicon, local_lexicon, "主词库属于词库这一类");
    assert_eq!(std::fs::read_to_string(&local_lexicon).unwrap(), "ben ji\t本机\t1000\n");
    assert_eq!(target.store.list_quick_phrases()[0].code, "yx");
    assert_eq!(target.store.list_dictionaries()[0].name, "旧词库");
    assert_eq!(target.engine.as_ref().unwrap().history_count("旧词"), 1);
    assert_eq!(target.recent_emoji, ["🎉"]);
    cleanup(&target);

    // 只有学习记录：学到的词和最近的表情换掉，别的不动
    let mut target = local_profile_host("parts-single-learning");
    assert!(target.handle(1, learning_request("import_settings", &learning)).ok);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!((engine.history_count("旧词"), engine.history_count("私密词")), (0, 3));
    assert_eq!(target.recent_emoji, ["😊"]);
    assert_eq!((target.config.page_size, target.mode), (8, Mode::Chinese));
    assert_eq!(target.config.lexicon, target.data_dir().join("local-lexicon.tsv").to_string_lossy());
    assert_eq!(target.store.list_quick_phrases()[0].code, "yx");
    assert_eq!(target.store.load_common_phrases().unwrap(), ["本机常用语"]);
    assert_eq!(target.store.list_dictionaries()[0].name, "旧词库");
    // 合并（云端恢复用的那种）遇到没有学习记录的文件也不出错
    assert!(target.handle(1, learning_request("merge_settings", &settings)).ok);
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
    assert_eq!(inspected.report.unwrap().note.as_deref(), Some("没有输入法数据"));
    assert!(host.handle(1, learning_request("import_settings", &path)).ok);
    assert_eq!((host.config.page_size, host.store.list_quick_phrases().len()), (8, 1));
    assert_eq!(host.engine.as_ref().unwrap().history_count("旧词"), 1);
    // 一类数据只有一半字段：拒绝，不当成「没有这一类」
    std::fs::write(&path, r#"{"format":"freetype-settings","version":1,"recent_emoji":[]}"#).unwrap();
    let response = host.handle(1, learning_request("import_settings", &path));
    assert!(!response.ok && response.error.as_deref().is_some_and(|e| e.contains("学习记录")), "{response:?}");
    std::fs::write(&path, r#"{"format":"freetype-settings","version":1,"mode":"english"}"#).unwrap();
    assert!(!host.handle(1, learning_request("import_settings", &path)).ok);
    assert_eq!(host.mode, Mode::Chinese);
    cleanup(&host);
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
fn restoring_learning_drains_the_previous_background_save() {
    let mut host = learning_host("restore-pending-save");
    let backup = host.data_dir().join("saved.json");
    assert!(host.handle(1, learning_request("export_settings", &backup)).ok);
    let path = host.store.user_dict_path();
    let old_path = path.clone();
    host.saver.enqueue("learning", Box::new(move || {
        // 模拟被磁盘延迟的旧快照。恢复必须在该写入结束后才替换文件。
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::write(old_path, "old snapshot").map_err(|error| error.to_string())
    })).unwrap();
    assert!(host.handle(1, learning_request("import_settings", &backup)).ok);
    host.saver.result(true).unwrap();
    assert_ne!(std::fs::read_to_string(path).unwrap(), "old snapshot");
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
    for trial in 1..=2 {
        let frame = typing(&mut source, "datag");
        let index = frame.candidates.iter().position(|word| word == "打").unwrap();
        let frame = source.handle(1, Request::Select { index }).frame.unwrap();
        let index = frame.candidates.iter().position(|word| word == "tag").unwrap();
        assert_eq!(source.handle(1, Request::Select { index }).frame.unwrap().commit.as_deref().map(str::trim_start), Some("打 tag"));
        assert_eq!(source.user_words().iter().any(|word| word.word == "打tag"), trial == 2);
    }
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
fn reloading_lexicon_preserves_learning_and_deletions_made_during_loading() {
    let mut host = host("reload-learning");
    let old = host.engine.as_ref().unwrap();
    old.add_user_word("ni hao", "你好", 3000.0).unwrap();
    // The loader read this snapshot, then the user kept typing in the old engine.
    let stale = old.export_learning().unwrap();
    let loaded = Engine::new(std::sync::Arc::new(wowtype_core::MemoryLexicon::new()),
        std::sync::Arc::new(wowtype_core::BigramLm::default()));
    loaded.import_learning(&stale).unwrap();
    old.forget_word("你好");
    old.add_user_word("ce shi", "测试", 4000.0).unwrap();
    let expected = old.export_learning().unwrap();
    host.install_engine(Ok(loaded));
    assert_eq!(host.engine.as_ref().unwrap().export_learning().unwrap(), expected,
        "switching dictionaries must neither lose new learning nor resurrect deleted words");
    cleanup(&host);
}

#[test]
fn command_config_save_failure_is_reported_without_changing_effective_config() {
    let mut host = host("command-save-error");
    std::fs::create_dir_all(host.data_dir().join("config.json")).unwrap();
    for command in [Command::TogglePunct, Command::ToggleTraditional, Command::ToggleStatusBar] {
        let before = host.config.clone();
        let response = host.handle(1, Request::Command { command });
        assert!(!response.ok, "failed storage must not be reported as a successful toggle");
        assert_eq!(host.config, before, "failed storage must preserve the previous setting");
    }
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
    assert!(report.note.unwrap().contains("按常用读音补充拼音"));
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

#[test]
fn new_fuzzy_groups_and_yong_iong_match() {
    use freetype_protocol::fuzzy;
    let mut host = host("fuzzy-new");
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
        "gai\t改\t100\nwang\t望\t100\nxiong\t兄\t100\ndeng\t灯\t100\nyong\t用\t100\n",
    )
    .unwrap();
    let engine = Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    host.install_engine(Ok(engine));
    configure(&mut host, |c| {
        c.fuzzy = true;
        c.fuzzy_groups = fuzzy::AN_AI | fuzzy::HUANG_WANG | fuzzy::UN_IONG | fuzzy::ENG_ONG;
    });

    assert!(typing(&mut host, "gan").candidates.iter().any(|c| c == "改"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "huang").candidates.iter().any(|c| c == "望"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "xun").candidates.iter().any(|c| c == "兄"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "dong").candidates.iter().any(|c| c == "灯"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "yun").candidates.iter().any(|c| c == "用"));
    cleanup(&host);
}

#[test]
fn arrow_keys_caret_and_shuangpin_show_keys() {
    let mut host = host("caret-shuangpin");
    configure(&mut host, |c| c.arrow_keys = "caret".into());

    let f = typing(&mut host, "nihao");
    assert_eq!(f.preedit, "ni'hao");
    assert_eq!(f.preedit_cursor, 6);

    // 左移三次依次越过 o、a、h，回到 ni 后面（' 会随分节缩回去）
    let f = press(&mut host, keysym::LEFT);
    assert_eq!(f.preedit_cursor, 5);
    let f = press(&mut host, keysym::LEFT);
    assert_eq!(f.preedit_cursor, 4);
    let f = press(&mut host, keysym::LEFT);
    assert_eq!(f.preedit_cursor, 2);
    assert_eq!(f.candidates.first().map(String::as_str), Some("你好"));

    // OnTestKeyDown 预判也必须接收 Home / End / Delete
    let preview_home = host
        .handle(
            1,
            Request::Key {
                key: Key {
                    keysym: keysym::HOME,
                    modifiers: 0,
                    test: true,
                },
                before: None,
            },
        )
        .frame
        .unwrap();
    assert!(preview_home.handled);
    let f = press(&mut host, keysym::HOME);
    assert_eq!(f.preedit_cursor, 0);
    let f = press(&mut host, keysym::DELETE);
    assert_eq!(f.preedit, "ihao");
    assert_eq!(f.preedit_cursor, 0);
    let f = press(&mut host, keysym::END);
    assert_eq!(f.preedit_cursor, 4);
    press(&mut host, keysym::ESCAPE);

    // 双拼展示键位 + 双拼下大写 V 计算器
    configure(&mut host, |c| {
        c.shuangpin = true;
        c.shuangpin_show_keys = false;
    });
    let plain = typing(&mut host, "nihc");
    assert_eq!(plain.preedit, "ni'hao");
    press(&mut host, keysym::ESCAPE);

    configure(&mut host, |c| c.shuangpin_show_keys = true);
    let with_keys = typing(&mut host, "nihc");
    assert_eq!(with_keys.preedit, "nihc");
    press(&mut host, keysym::ESCAPE);

    let calc = typing(&mut host, "V1+2");
    assert!(calc.candidates.iter().any(|c| c == "3"), "cands={:?}", calc.candidates);
    cleanup(&host);
}

#[test]
fn traditional_phrase_conversion_and_learning() {
    let mut host = host("trad-learn");
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
        "shen me\t什么\t100\nshen me\t神么\t200\n",
    )
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
        let idx = f.candidates.iter().position(|c| c == "什麼").expect("应包含 OpenCC 繁体候选 什麼");
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
    assert!(typing(&mut host, "lvxia").candidates.iter().any(|c| c == "女侠"));
    cleanup(&host);
}

/// 上屏联想的测试宿主：「今天」后面模型里有「下雨」「天气」。
fn predict_host(name: &str) -> Host {
    let mut host = host(name);
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
        "jin tian\t今天\t500000\nxia yu\t下雨\t20000\ntian qi\t天气\t60000\nkai hui\t开会\t30000\nbu cuo\t不错\t40000\n",
    )
    .unwrap();
    let lm = wowtype_core::BigramLm::from_bigram_tsv_str(
        "今天\t下雨\t300\n今天\t天气\t200\n天气\t不错\t300\n开会\t今天\t5\n不错\t今天\t5\n下雨\t今天\t5\n",
    )
    .unwrap();
    host.install_engine(Ok(Engine::new(std::sync::Arc::new(lexicon), std::sync::Arc::new(lm))));
    host
}

fn commit_today(host: &mut Host) -> Frame {
    typing(host, "jintian");
    press(host, 0x20)
}

#[test]
fn prediction_is_off_by_default() {
    let mut host = predict_host("predict-off");
    assert!(!host.config().predict_next);
    let frame = commit_today(&mut host);
    assert_eq!(frame.commit.as_deref(), Some("今天"));
    assert!(frame.candidates.is_empty() && !frame.predict);
    cleanup(&host);
}

#[test]
fn prediction_follows_a_commit_and_chains_with_digits_space_and_mouse() {
    let mut host = predict_host("predict-chain");
    configure(&mut host, |c| {
        c.predict_next = true;
        c.predict_digits = true;
        c.predict_double_space = false;
    });
    let frame = commit_today(&mut host);
    assert_eq!(frame.commit.as_deref(), Some("今天"));
    assert!(frame.predict && frame.preedit.is_empty() && !frame.forgettable);
    assert_eq!(frame.candidates, ["下雨", "天气"]);
    assert_eq!(frame.cursor, Some(0), "空格选的那个高亮");
    // 数字键选第二个：上屏并接着联想
    let frame = press(&mut host, '2' as u32);
    assert!(frame.handled);
    assert_eq!(frame.commit.as_deref(), Some("天气"));
    assert_eq!(frame.candidates, ["不错"]);
    // 空格选第一个；后面没有可联想的了，候选窗收起
    let frame = press(&mut host, 0x20);
    assert_eq!(frame.commit.as_deref(), Some("不错"));
    assert!(frame.candidates.is_empty() && !frame.predict);
    // 鼠标点
    let shown = commit_today(&mut host).candidates;
    let frame = host.handle(1, Request::Select { index: 1 }).frame.unwrap();
    assert_eq!(frame.commit, Some(shown[1].clone()));
    cleanup(&host);
}

#[test]
fn prediction_double_space_is_enabled_by_default_and_selects_highlight() {
    let mut host = predict_host("predict-double-space");
    configure(&mut host, |config| config.predict_next = true);
    let start = commit_today(&mut host);
    assert!(start.predict_double_space);
    let single = press(&mut host, 0x20);
    assert!(!single.handled && single.commit.is_none() && single.predict);
    assert_eq!(single.page_revision, start.page_revision);
    let moved = press(&mut host, keysym::RIGHT);
    let selected = moved.candidates[moved.cursor.unwrap()].clone();
    let result = press(&mut host, keysym::SPACE_DOUBLE);
    assert_eq!(result.commit.as_deref(), Some(selected.as_str()));
    assert!(result.replace_space);
    cleanup(&host);
}

#[test]
fn legacy_prediction_shortcut_is_ignored_and_not_saved() {
    let mut host = predict_host("predict-legacy-shortcut");
    let path = host.data_dir().join("config.json");
    std::fs::create_dir_all(host.data_dir()).unwrap();
    for (shortcut, sym, mods) in [
        ("alt", 0x11_0000, 0), // 旧版 TSF 发送的 Alt 松开键码
        ("ctrl+j", u32::from(b'j'), modifier::CTRL),
        ("f9", keysym::F1 + 8, 0),
    ] {
        let legacy = serde_json::json!({ "predict_next": true, "predict_shortcut": shortcut });
        std::fs::write(&path, legacy.to_string()).unwrap();
        let config = host.store.load_config();
        let result = host.handle(1, Request::SetConfig { config });
        assert!(result.ok, "{:?}", result.error);
        assert!(commit_today(&mut host).predict, "旧设置仍可开启联想");
        let frame = key_with(&mut host, 1, sym, mods);
        assert!(!frame.handled && frame.commit.is_none(), "{shortcut} 不再选词");
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(saved.get("predict_shortcut").is_none(), "重存时不再写入旧设置");
    }
    cleanup(&host);
}

#[test]
fn stale_candidate_action_does_not_select_and_fixed_slot_survives_reload() {
    let mut host = predict_host("fixed-candidate");
    host.engine.as_ref().unwrap().add_user_word("jin tian", "今填", 1000.0).unwrap();
    let frame = typing(&mut host, "jintian");
    let index = frame.candidates.iter().position(|word| word == "今填").unwrap();
    let pinned = host.handle(1, Request::CandidateAction { revision: frame.page_revision, action: CandidateAction::Pin { index, place: 1 } }).frame.unwrap();
    assert_eq!(pinned.candidates[0], "今填");
    assert_eq!(pinned.pin_slots[0], Some(1));
    let stale = host.handle(1, Request::CandidateAction { revision: frame.page_revision, action: CandidateAction::Select { index: 0 } }).frame.unwrap();
    assert!(stale.commit.is_none());
    let pins = read_pins(&host.data_dir().join("candidate-pins.json")).unwrap();
    assert_eq!(pins["quanpin:jintian"][&1], "jin tian\t今填");
    let unpin = host.handle(1, Request::CandidateAction { revision: stale.page_revision, action: CandidateAction::Pin { index: 0, place: 0 } }).frame.unwrap();
    assert_ne!(unpin.candidates[0], "今填");
    cleanup(&host);
}

#[test]
fn prediction_steps_aside_for_other_keys() {
    let mut host = predict_host("predict-keys");
    configure(&mut host, |c| c.predict_next = true);
    // 字母：直接开始打拼音
    commit_today(&mut host);
    let frame = press(&mut host, 'k' as u32);
    assert!(frame.handled && !frame.predict);
    assert_eq!(frame.preedit, "k");
    press(&mut host, keysym::ESCAPE);
    // Esc：吃掉这个键，只收起联想
    assert!(commit_today(&mut host).predict);
    let frame = press(&mut host, keysym::ESCAPE);
    assert!(frame.handled && frame.candidates.is_empty());
    assert!(!press(&mut host, keysym::ESCAPE).handled, "收起以后 Esc 归应用");
    // 默认不让数字键选词：即使数字对应现有联想项，也照常输入。
    let suggestions = commit_today(&mut host);
    assert!(suggestions.predict);
    assert_eq!(suggestions.labels.len(), suggestions.candidates.len());
    assert!(suggestions.labels.iter().all(String::is_empty));
    let frame = press(&mut host, '1' as u32);
    assert_eq!(frame.commit.as_deref(), Some(" 1"));
    assert!(frame.candidates.is_empty());
    // 退格、上下方向键、回车：交给应用，联想收起
    for sym in [keysym::BACKSPACE, keysym::UP, keysym::DOWN, keysym::RETURN] {
        assert!(commit_today(&mut host).predict);
        let frame = press(&mut host, sym);
        assert!(!frame.handled && frame.candidates.is_empty() && frame.commit.is_none(), "{sym:#x}");
        assert!(!press(&mut host, 0x20).handled, "收起以后空格归应用");
    }
    // 标点照常上屏
    assert!(commit_today(&mut host).predict);
    let frame = press(&mut host, ',' as u32);
    assert_eq!(frame.commit.as_deref(), Some("，"));
    assert!(frame.candidates.is_empty());
    // 即使开启数字选词，超出候选数量的数字仍照常输入。
    configure(&mut host, |c| c.predict_digits = true);
    let suggestions = commit_today(&mut host);
    assert!(suggestions.predict && suggestions.candidates.len() < 9);
    let frame = press(&mut host, '9' as u32);
    assert_eq!(frame.commit.as_deref(), Some(" 9"));
    assert!(frame.candidates.is_empty());
    cleanup(&host);
}

#[test]
fn prediction_keys_can_be_switched_off_separately() {
    let mut host = predict_host("predict-switches");
    configure(&mut host, |c| {
        c.predict_next = true;
        c.predict_digits = false;
        c.predict_space = false;
        c.predict_arrows = false;
    });
    let frame = commit_today(&mut host);
    assert_eq!(frame.labels, ["", ""], "数字键不选词时不标序号");
    assert_eq!(frame.cursor, None);
    assert_eq!(press(&mut host, '1' as u32).commit.as_deref(), Some(" 1"), "数字照常打出来");
    assert!(commit_today(&mut host).predict);
    assert!(!press(&mut host, 0x20).handled, "空格照常打出来");
    cleanup(&host);
}

#[test]
fn prediction_arrows_move_highlight_and_space_commits_the_selection() {
    for horizontal in [true, false] {
        let mut host = predict_host(&format!("predict-arrows-{horizontal}"));
        configure(&mut host, |c| {
            c.predict_next = true;
            c.predict_double_space = false;
            c.horizontal = horizontal;
            c.arrow_keys = "caret".into();
        });
        assert!(host.config().predict_arrows, "开启联想后默认支持左右选词，与组字光标设置无关");
        let shown = commit_today(&mut host).candidates;
        let learned = host.engine.as_ref().unwrap().export_learning().unwrap();
        for (sym, cursor) in [(keysym::LEFT, 0), (keysym::RIGHT, 1), (keysym::RIGHT, 1), (keysym::LEFT, 0), (keysym::RIGHT, 1)] {
            let frame = press(&mut host, sym);
            assert!(frame.handled && frame.predict && frame.preedit.is_empty());
            assert!(frame.commit.is_none(), "移动高亮不能上屏");
            assert_eq!(frame.candidates, shown);
            assert_eq!(frame.cursor, Some(cursor), "两端停住，不循环");
        }
        assert_eq!(host.engine.as_ref().unwrap().export_learning().unwrap(), learned, "移动高亮不能学习");
        let frame = press(&mut host, 0x20);
        assert_eq!(frame.commit.as_deref(), Some("天气"));
        assert_eq!(frame.candidates, ["不错"]);
        assert_eq!(frame.cursor, Some(0), "下一轮联想重新从第一个词开始");
        assert_eq!(press(&mut host, keysym::RIGHT).cursor, Some(0), "只有一个候选时仍吃掉方向键");
        assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("不错"));
        cleanup(&host);
    }
}

#[test]
fn prediction_arrows_switch_and_modified_keys_preserve_application_navigation() {
    let mut host = predict_host("predict-arrows-pass");
    configure(&mut host, |c| {
        c.predict_next = true;
        c.predict_arrows = false;
    });
    for sym in [keysym::LEFT, keysym::RIGHT] {
        assert!(commit_today(&mut host).predict);
        let frame = press(&mut host, sym);
        assert!(!frame.handled && !frame.predict && frame.commit.is_none() && frame.candidates.is_empty());
    }
    configure(&mut host, |c| c.predict_arrows = true);
    for modifiers in [modifier::SHIFT, modifier::CTRL, modifier::ALT, modifier::SUPER] {
        for sym in [keysym::LEFT, keysym::RIGHT] {
            assert!(commit_today(&mut host).predict);
            let key = Key { keysym: sym, modifiers, ..Key::default() };
            let frame = host.handle(1, Request::Key { key, before: None }).frame.unwrap();
            assert!(!frame.handled && !frame.predict && frame.commit.is_none() && frame.candidates.is_empty());
        }
    }
    cleanup(&host);
}

#[test]
fn prediction_arrows_work_independently_of_digit_and_space_switches() {
    let mut host = predict_host("predict-arrows-only");
    configure(&mut host, |c| {
        c.predict_next = true;
        c.predict_digits = false;
        c.predict_space = false;
    });
    assert_eq!(commit_today(&mut host).cursor, Some(0));
    let frame = press(&mut host, keysym::RIGHT);
    assert_eq!(frame.cursor, Some(1));
    assert_eq!(frame.labels, ["", ""]);
    assert!(!press(&mut host, 0x20).handled, "左右选词不能悄悄开启空格上屏");
    let shown = commit_today(&mut host);
    assert!(shown.predict && shown.candidates.len() >= 2);
    assert_eq!(press(&mut host, keysym::RIGHT).cursor, Some(1));
    let frame = host.handle(1, Request::Select { index: 0 }).frame.unwrap();
    assert_eq!(frame.commit.as_deref(), Some(shown.candidates[0].as_str()), "鼠标仍按点中的词上屏");
    cleanup(&host);
}

#[test]
fn prediction_arrow_previews_do_not_move_the_live_selection() {
    let mut host = predict_host("predict-arrows-preview");
    configure(&mut host, |c| {
        c.predict_next = true;
        c.predict_double_space = false;
    });
    commit_today(&mut host);
    let preview = Key { keysym: keysym::RIGHT, test: true, ..Key::default() };
    for _ in 0..2 {
        let frame = host.handle(1, Request::Key { key: preview.clone(), before: None }).frame.unwrap();
        assert!(frame.handled && frame.commit.is_none());
        assert_eq!(frame.cursor, Some(1), "重复预判从真实位置计算");
    }
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("下雨"), "没有真按右键就仍选第一个");
    press(&mut host, keysym::ESCAPE);
    commit_today(&mut host);
    host.handle(1, Request::Key { key: preview, before: None });
    assert_eq!(press(&mut host, keysym::RIGHT).cursor, Some(1));
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some("天气"));
    cleanup(&host);
}

#[test]
fn prediction_is_dropped_when_caret_focus_or_mode_changes() {
    let mut host = predict_host("predict-drop");
    configure(&mut host, |c| {
        c.predict_next = true;
        c.predict_digits = true;
    });
    // 光标挪走了（DLL 带来的光标前的字变了）
    assert!(commit_today(&mut host).predict);
    let key = Key { keysym: '1' as u32, ..Key::default() };
    let frame = host.handle(1, Request::Key { key, before: Some("今天，".into()) }).frame.unwrap();
    assert!(!frame.handled && frame.commit.is_none() && frame.candidates.is_empty());
    // 换输入框
    assert!(commit_today(&mut host).predict);
    let frame = host.handle(1, Request::Focus { active: true, before: Some("今天".into()) }).frame.unwrap();
    assert!(frame.candidates.is_empty());
    // 收起（一会儿没选）
    assert!(commit_today(&mut host).predict);
    assert!(host.handle(1, Request::Reset { commit: false }).frame.unwrap().candidates.is_empty());
    assert!(!press(&mut host, 0x20).handled);
    // 切到英文
    assert!(commit_today(&mut host).predict);
    let frame = host.handle(1, Request::ToggleMode { shift: None }).frame.unwrap();
    assert!(frame.candidates.is_empty());
    cleanup(&host);
}

#[test]
fn chosen_predictions_are_learned_once_and_rank_first_later() {
    let mut host = predict_host("predict-learn");
    configure(&mut host, |c| {
        c.predict_next = true;
        c.predict_digits = true;
    });
    assert_eq!(commit_today(&mut host).candidates[1], "天气");
    // 预判（OnTestKeyDown）不学习
    let learned = host.engine.as_ref().unwrap().export_learning().unwrap();
    let preview = Key { keysym: '2' as u32, test: true, ..Key::default() };
    assert!(host.handle(1, Request::Key { key: preview, before: None }).frame.unwrap().handled);
    assert_eq!(host.engine.as_ref().unwrap().export_learning().unwrap(), learned);
    let pairs = host.engine.as_ref().unwrap().pair_count();
    assert_eq!(press(&mut host, '2' as u32).commit.as_deref(), Some("天气"));
    assert_eq!(host.engine.as_ref().unwrap().pair_count(), pairs + 1, "只学一次");
    press(&mut host, keysym::ESCAPE);
    // 选过的搭配排到「下雨」前面
    assert_eq!(commit_today(&mut host).candidates[0], "天气");
    // 模型里没有的搭配：用户自己连着打过两次也会出现
    for _ in 0..2 {
        press(&mut host, keysym::ESCAPE);
        commit_today(&mut host);
        typing(&mut host, "kaihui");
        press(&mut host, 0x20);
    }
    press(&mut host, keysym::ESCAPE);
    assert!(commit_today(&mut host).candidates.iter().any(|c| c == "开会"));
    cleanup(&host);
}

#[test]
fn merge_settings_adds_remote_data_without_dropping_local_data() {
    let mut source = learning_host("merge-source");
    configure(&mut source, |c| {
        c.page_size = 9;
        c.predict_next = true;
    });
    assert!(source.handle(1, Request::SetQuickPhrases {
        phrases: vec![freetype_protocol::QuickPhrase { code: "dz".into(), text: "远端地址".into(), weight: 20 }],
    }).ok);
    assert!(source.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["远端常用语", "两边都有"]}))).ok);
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(source.data_dir().join("dicts/人名.txt"), "阿特\ta te\t800\n").unwrap();
    std::fs::write(source.data_dir().join("dicts/共有.txt"), "远端\tyuan duan\t800\n").unwrap();
    source.recent_emoji = vec!["😊".into()];
    for _ in 0..3 { source.engine.as_ref().unwrap().record_selection_with_pinyin("阿特", Some("a te")); }
    let snapshot = source.data_dir().join("snapshot.json");
    assert!(source.handle(1, learning_request("export_settings", &snapshot)).ok);

    let mut target = learning_host("merge-target");
    configure(&mut target, |c| {
        c.font_size = 24;
        c.horizontal = false;
    });
    target.mode = Mode::English;
    assert!(target.handle(1, Request::SetQuickPhrases {
        phrases: vec![freetype_protocol::QuickPhrase { code: "yx".into(), text: "本机邮箱".into(), weight: 10 }],
    }).ok);
    assert!(target.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["本机常用语", "两边都有"]}))).ok);
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    std::fs::write(target.data_dir().join("dicts/共有.txt"), "本机\tben ji\t100\n").unwrap();
    target.recent_emoji = vec!["🎉".into()];
    target.engine.as_ref().unwrap().record_selection_with_pinyin("旧词", Some("jiu ci"));

    let result = target.handle(1, learning_request("merge_settings", &snapshot));
    assert!(result.ok, "{result:?}");
    // 设置：采纳远端改过的项，本机改过而远端没改的留着
    assert_eq!(target.config.page_size, 9);
    assert!(target.config.predict_next);
    assert_eq!(target.config.font_size, 24);
    assert!(!target.config.horizontal);
    assert_eq!(target.mode, Mode::English, "中英文状态是各设备自己的");
    let codes: Vec<String> = target.store.list_quick_phrases().into_iter().map(|p| p.code).collect();
    assert_eq!(codes, ["dz", "yx"], "列表按编码排");
    assert_eq!(target.store.load_common_phrases().unwrap(), ["本机常用语", "两边都有", "远端常用语"]);
    let mut names: Vec<String> = target.store.list_dictionaries().into_iter().map(|d| d.name).collect();
    names.sort();
    assert_eq!(names, ["人名", "共有"]);
    assert_eq!(std::fs::read_to_string(target.data_dir().join("dicts/共有.txt")).unwrap(), "本机\tben ji\t100\n", "同名词库保留本机的");
    assert_eq!(target.recent_emoji, ["🎉", "😊"]);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!((engine.history_count("阿特"), engine.history_count("旧词")), (3, 1));
    assert!(engine.input("ate").iter().any(|c| c.text == "阿特"), "并进来的词马上能用");
    // 再合并一次不重复、不累加
    let again = target.handle(1, learning_request("merge_settings", &snapshot));
    assert!(again.ok);
    assert_eq!(target.store.list_quick_phrases().len(), 2);
    assert_eq!(target.store.load_common_phrases().unwrap().len(), 3);
    assert_eq!(target.engine.as_ref().unwrap().history_count("阿特"), 3);
    // 坏文件不动数据
    let bad = target.data_dir().join("bad.json");
    std::fs::write(&bad, "{}").unwrap();
    assert!(!target.handle(1, learning_request("merge_settings", &bad)).ok);
    assert_eq!(target.config.page_size, 9);
    cleanup(&source);
    cleanup(&target);
}
