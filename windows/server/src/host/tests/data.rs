//! 学习词的删除与清空、主词库切换与载入期间的输入。
use super::*;

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
