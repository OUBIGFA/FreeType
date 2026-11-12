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
    configure(&mut host, |c| c.default_mode = Mode::English);
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
