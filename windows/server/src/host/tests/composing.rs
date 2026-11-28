//! 组字、选词翻页、标点、补全、计算器、工具栏与其他按键处理。
use super::*;

#[test]
fn lagging_surrounding_text_cannot_change_chinese_punctuation_after_commit() {
    let mut host = host("lagging-punctuation");
    host.handle(
        1,
        Request::Focus {
            active: true,
            before: Some("1:1".into()),
        },
    );
    typing(&mut host, "nihao");
    assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some(" 你好"));
    let dot = host
        .handle(
            1,
            Request::Key {
                key: Key {
                    keysym: '.' as u32,
                    modifiers: 0,
                    test: false,
                },
                before: Some("1:1".into()),
            },
        )
        .frame
        .unwrap();
    assert_eq!(
        dot.commit.as_deref(),
        Some("。"),
        "宿主迟报的数字不能盖掉刚上屏的汉字"
    );
    let dot = host
        .handle(
            1,
            Request::Key {
                key: Key {
                    keysym: '.' as u32,
                    modifiers: 0,
                    test: false,
                },
                before: Some("3".into()),
            },
        )
        .frame
        .unwrap();
    assert!(
        dot.commit.as_deref() == Some(".") || !dot.handled,
        "真挪到其他数字后仍输出半角"
    );
    cleanup(&host);
}

#[test]
fn lookup_modes_commit_only_selected_text_and_keep_raw_enter_unchanged() {
    let Some(resources) = std::env::var_os("FREETYPE_TEST_INPUT_TOOLS") else {
        return;
    };
    let mut host = host("lookup-modes");
    host.engine_ref()
        .unwrap()
        .load_input_tools(resources)
        .unwrap();
    let open = |host: &mut Host, command: &str| {
        let request = serde_json::from_value(serde_json::json!({"op":"command","command":command}))
            .expect("查询模式入口");
        let response = host.handle(1, request);
        assert!(response.ok);
        response.frame.unwrap()
    };
    assert!(open(&mut host, "english_candidates")
        .title
        .as_deref()
        .unwrap()
        .contains("英文"));
    let frame = typing(&mut host, "inspiteof");
    assert!(
        frame.vertical,
        "带音标和释义的候选按行显示，不能撑成超长横条"
    );
    assert!(frame.annotations.iter().any(|text| !text.is_empty()));
    let index = frame
        .candidates
        .iter()
        .position(|text| text == "in spite of")
        .expect("短语候选");
    let selected = host.handle(1, Request::Select { index }).frame.unwrap();
    assert_eq!(selected.commit.as_deref(), Some("in spite of "));
    assert!(
        selected.title.as_deref().unwrap().contains("英文"),
        "英文模式持续输入，Esc 或快捷键退出"
    );
    typing(&mut host, "conectivity");
    assert_eq!(
        press(&mut host, keysym::RETURN).commit.as_deref(),
        Some("conectivity")
    );
    press(&mut host, keysym::ESCAPE);
    open(&mut host, "radical_lookup");
    let frame = typing(&mut host, "ririri");
    let index = frame
        .candidates
        .iter()
        .position(|text| text == "晶")
        .unwrap();
    assert_eq!(
        host.handle(1, Request::Select { index })
            .frame
            .unwrap()
            .commit
            .as_deref(),
        Some("晶")
    );
    assert!(
        press(&mut host, 'n' as u32).preedit.starts_with('n'),
        "查字上屏后返回普通拼音"
    );
    cleanup(&host);
}

#[test]
fn inline_space_separator_changes_only_display_and_preserves_selection() {
    let mut host = host("inline-spaces");
    configure(&mut host, |c| c.inline_preedit = "spaces".into());
    let frame = typing(&mut host, "nihao");
    assert_eq!(frame.inline_preedit.as_deref(), Some("ni hao"));
    assert_eq!(frame.preedit, "ni'hao", "候选窗仍展示原来的拼音分词");
    assert_eq!(frame.inline_cursor, 6);
    assert_eq!(
        key_with(&mut host, 1, 0x20, 0).commit.as_deref(),
        Some("你好")
    );
    assert_eq!(
        Host::new(Store::new(host.data_dir().to_path_buf()))
            .config()
            .inline_preedit,
        "spaces"
    );
    cleanup(&host);
}

#[test]
fn long_word_completion_shows_plain_candidates_commits_and_persists_its_switch() {
    let mut host = host("long-completion");
    host.install_engine(Ok(Engine::new(
        std::sync::Arc::new(wowtype_core::MemoryLexicon::from_tsv_str(
            "zhong hua ren min\t中华人民\t3000\nzhong hua ren min gong he guo\t中华人民共和国\t6000\n").unwrap()),
        std::sync::Arc::new(wowtype_core::BigramLm::default()))));
    configure(&mut host, |config| {
        config.typo_mark = false;
        config.pin_badge = "off".into();
    });
    let frame = typing(&mut host, "zhonghuarenmin");
    assert_eq!(frame.candidates[0], "中华人民");
    assert_eq!(frame.candidates[1], "中华人民共和国");
    assert!(
        frame.badges.is_empty(),
        "补全候选应和普通候选一样显示，不附加标记"
    );
    assert!(frame.pin_slots[1].is_none());
    assert_eq!(
        press(&mut host, '2' as u32).commit.as_deref(),
        Some("中华人民共和国")
    );
    configure(&mut host, |config| config.long_word_completion = false);
    assert!(!typing(&mut host, "zhonghuarenmin")
        .candidates
        .contains(&"中华人民共和国".into()));
    assert!(
        !Host::new(Store::new(host.data_dir().to_path_buf()))
            .config()
            .long_word_completion
    );
    cleanup(&host);
}

#[test]
fn letters_compose_and_space_commits() {
    let mut host = host("compose");
    let frame = typing(&mut host, "nihao");
    assert!(frame.handled);
    assert_eq!(frame.candidates[0], "你好");
    assert!(frame.forgettable);
    assert_eq!(frame.font_family, freetype_protocol::DEFAULT_FONT);
    assert_eq!(
        frame.candidate_style, "pink",
        "候选窗拿到设置里的强调色，默认粉色"
    );
    assert_eq!(frame.candidate_skin, None, "内置样式不下发自定义皮肤颜色");
    let frame = press(&mut host, 0x20);
    assert_eq!(frame.commit.as_deref(), Some("你好"));
    assert!(frame.preedit.is_empty());
    assert!(frame.candidates.is_empty());
    cleanup(&host);
}

#[test]
fn custom_candidate_skin_is_sent_only_when_selected_and_preserved_in_config() {
    let mut host = host("custom-skin");
    configure(&mut host, |c| {
        c.candidate_style = "custom".into();
        c.candidate_skin = freetype_protocol::CandidateSkin {
            background: " #FBF8F1 ".into(),
            border: "#E2D9C8".into(),
            text: "#2C261F".into(),
            muted: "#7A7062".into(),
            accent: "#B45309".into(),
            accent_text: "#FFFFFF".into(),
            accent_fill: false,
        };
    });
    let frame = typing(&mut host, "ni");
    assert_eq!(frame.candidate_style, "custom");
    let skin = frame
        .candidate_skin
        .expect("选择自定义配色时应下发皮肤颜色");
    assert_eq!(skin.background, "#fbf8f1");
    assert_eq!(skin.accent, "#b45309");
    assert!(!skin.accent_fill);

    configure(&mut host, |c| c.candidate_style = "blue".into());
    let frame = typing(&mut host, "h");
    assert_eq!(frame.candidate_style, "blue");
    assert_eq!(frame.candidate_skin, None);
    assert_eq!(
        host.config.candidate_skin.background, "#fbf8f1",
        "切回内置样式后应保留自定义配色"
    );
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
    assert_eq!(
        press(&mut host, '.' as u32).commit.as_deref(),
        Some("."),
        "数字后面原样"
    );
    press(&mut host, '1' as u32);
    assert_eq!(
        press(&mut host, '\\' as u32).commit.as_deref(),
        Some("、"),
        "数字后面顿号默认仍为全角"
    );
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
    assert_eq!(
        (frame.commit.as_deref(), frame.caret_back),
        (Some("（）"), 1)
    );
    press(&mut host, '3' as u32);
    assert_eq!(
        press(&mut host, '.' as u32).commit.as_deref(),
        Some("."),
        "未勾选的句号仍保持半角"
    );

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
    assert_eq!(
        (frame.commit.as_deref(), frame.caret_back),
        (Some("「」"), 1)
    );
    let single = press(&mut host, '\'' as u32);
    assert_eq!(
        (single.commit.as_deref(), single.caret_back),
        (Some("‘’"), 1)
    );

    configure(&mut host, |c| c.auto_pair = false);
    assert_eq!(press(&mut host, '"' as u32).commit.as_deref(), Some("「"));
    typing(&mut host, "nihao");
    assert_eq!(
        press(&mut host, '"' as u32).commit.as_deref(),
        Some("你好」")
    );
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
    assert_eq!(
        press(&mut host, '"' as u32).commit.as_deref(),
        Some("”"),
        "左右交替"
    );
    let frame = press(&mut host, '(' as u32);
    assert_eq!((frame.commit.as_deref(), frame.caret_back), (Some("（"), 0));
    configure(&mut host, |c| c.fullwidth_punct = false);
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some(","));
    cleanup(&host);
}

/// 「连打两半时不重复右标点」默认关闭，开启后随成对标点一起告诉 DLL；关掉补全时不生效。
#[test]
fn absorbing_a_typed_closing_mark_follows_its_switch() {
    let mut host = host("pair-absorb");
    let frame = press(&mut host, '(' as u32);
    assert!(frame.pair_enabled && !frame.pair_absorb_closing);
    configure(&mut host, |c| c.auto_pair_absorb_closing = true);
    assert!(press(&mut host, '(' as u32).pair_absorb_closing);
    configure(&mut host, |c| c.auto_pair = false);
    assert!(!press(&mut host, '(' as u32).pair_absorb_closing);
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
    let tested = host
        .handle(1, Request::Key { key, before: None })
        .frame
        .unwrap();
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
    assert_eq!(
        Frame {
            page_revision: 0,
            ..real
        },
        tested
    );
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
    assert!(
        !press(&mut host, keysym::ESCAPE).handled,
        "没有真的进编辑区，Esc 交给应用"
    );
    cleanup(&host);
}

#[test]
fn emoji_shortcut_toggles_the_panel_like_upstream() {
    let mut host = host("emoji-toggle");
    configure(&mut host, |c| c.hotkey_emoji = "ctrl+shift+e".into());
    let modifiers = modifier::CTRL | modifier::SHIFT;
    let opened = key_with(&mut host, 1, 'e' as u32, modifiers);
    assert!(!opened.candidates.is_empty());
    let key = Key {
        keysym: 'e' as u32,
        modifiers,
        test: true,
    };
    host.handle(1, Request::Key { key, before: None });
    let closed = key_with(&mut host, 1, 'e' as u32, modifiers);
    assert!(
        closed.preedit.is_empty() && closed.candidates.is_empty() && closed.title.is_none(),
        "与上游一样，再按一次表情快捷键应关闭面板"
    );
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
fn composing_swallows_ctrl_combinations() {
    let mut host = host("ctrl");
    typing(&mut host, "ni");
    let frame = key_with(&mut host, 1, 'c' as u32, modifier::CTRL);
    assert!(frame.handled);
    assert!(frame.commit.is_none());
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
    let frame = host
        .handle(1, Request::Reset { commit: true })
        .frame
        .unwrap();
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
    assert_eq!(
        frame.candidates,
        vec!["剪贴板", "常用语", "符号", "表情", "英文候选", "拆字查字"]
    );
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

#[test]
fn emoji_panel_hotkey_records_recent() {
    let mut host = host("emoji");
    configure(&mut host, |c| c.hotkey_emoji = "ctrl+shift+e".into());
    let test = Key {
        keysym: 'E' as u32,
        modifiers: modifier::CTRL | modifier::SHIFT,
        test: true,
    };
    let frame = host
        .handle(
            1,
            Request::Key {
                key: test,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert!(frame.handled && frame.title.is_none(), "test 只说要这个键");
    let frame = key_with(&mut host, 1, 'E' as u32, modifier::CTRL | modifier::SHIFT);
    assert_eq!(frame.panel, Panel::Grid);
    let first = frame.candidates[0].clone();
    let frame = press(&mut host, '1' as u32);
    assert_eq!(frame.commit.as_deref(), Some(first.as_str()));
    let frame = host
        .handle(
            1,
            Request::Command {
                command: Command::EmojiPanel,
            },
        )
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
    assert_eq!(
        press(&mut host, keysym::RETURN).commit.as_deref(),
        Some("ni@123")
    );
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
    assert_eq!(
        press(&mut host, '.' as u32).commit.as_deref(),
        Some("你好。")
    );

    configure(&mut host, |c| c.email_completion = false);
    let frame = press(&mut host, '@' as u32);
    assert_eq!(frame.commit.as_deref(), Some("@"));
    cleanup(&host);
}

#[test]
fn calculator_decimals_do_not_start_address_completion() {
    let mut host = host("calculator-decimals");
    for shuangpin in [false, true] {
        configure(&mut host, |c| {
            c.shuangpin = shuangpin;
            c.autospace = false;
        });
        let prefix = if shuangpin { 'V' } else { 'v' };
        for numpad in [true, false] {
            for (expression, expected) in [
                ("1.5+2", "3.5"),
                ("0.1+0.2", "0.3"),
                (".5*2", "1"),
                ("-1.5+2", "0.5"),
                ("(1.5-0.5)/4", "0.25"),
                ("1+2.5", "3.5"),
            ] {
                let input = format!("{prefix}{expression}");
                let mut typed = String::new();
                for c in input.chars() {
                    let sym = match (numpad, c) {
                        (true, '0'..='9') => keysym::KP_0 + c as u32 - '0' as u32,
                        (true, '.') => keysym::KP_DECIMAL,
                        (true, '+') => keysym::KP_ADD,
                        (true, '-') => keysym::KP_SUBTRACT,
                        (true, '*') => keysym::KP_MULTIPLY,
                        (true, '/') => keysym::KP_DIVIDE,
                        _ => c as u32,
                    };
                    typed.push(c);
                    let frame = press(&mut host, sym);
                    assert!(frame.handled, "{input}, numpad={numpad}, at {typed}");
                    assert!(
                        frame.commit.is_none(),
                        "{input} committed early at {typed}: {:?}",
                        frame.commit
                    );
                    assert_eq!(frame.preedit, typed);
                }
                let frame = press(&mut host, '=' as u32);
                assert_eq!(
                    frame.commit.as_deref(),
                    Some(expected),
                    "{input}, numpad={numpad}"
                );
            }
        }
    }
    cleanup(&host);
}

#[test]
fn calculator_labels_and_preview_match_letter_selection() {
    let mut host = host("calculator-letters");
    for shuangpin in [false, true] {
        configure(&mut host, |c| {
            c.shuangpin = shuangpin;
            c.autospace = false;
        });
        let prefix = if shuangpin { "V" } else { "v" };
        let frame = typing(&mut host, &format!("{prefix}1+2"));
        assert_eq!(frame.labels[..3], ["a", "b", "c"]);
        let key = Key {
            keysym: 'b' as u32,
            test: true,
            ..Key::default()
        };
        let preview = host
            .handle(
                1,
                Request::Key {
                    key: key.clone(),
                    before: None,
                },
            )
            .frame
            .unwrap();
        assert_eq!(preview.commit.as_deref(), Some("1+2=3"));
        let actual = host
            .handle(
                1,
                Request::Key {
                    key: Key { test: false, ..key },
                    before: None,
                },
            )
            .frame
            .unwrap();
        assert_eq!(actual.commit, preview.commit);
        let frame = typing(&mut host, &format!("{prefix}123.456"));
        assert!(!frame.labels.is_empty());
        assert_eq!(
            press(&mut host, '$' as u32).commit.as_deref(),
            Some("壹佰贰拾叁元肆角陆分")
        );
    }
    typing(&mut host, "nihao");
    assert!(
        press(&mut host, keysym::LEFT).labels.is_empty(),
        "ordinary candidates retain numeric labels"
    );
    cleanup(&host);
}

#[test]
fn numpad_dot_is_plain_or_starts_completion() {
    let mut host = host("numpad");
    assert_eq!(
        press(&mut host, keysym::KP_DECIMAL).commit.as_deref(),
        Some(".")
    );
    typing(&mut host, "qq");
    let frame = press(&mut host, keysym::KP_DECIMAL);
    assert_eq!(frame.preedit, "qq.");
    assert_eq!(frame.candidates[1], "qq.com");
    press(&mut host, keysym::ESCAPE);

    // 关闭计算器时，v 仍按普通字母补全网址。
    configure(&mut host, |c| c.calculator = false);
    typing(&mut host, "v");
    let frame = press(&mut host, keysym::KP_DECIMAL);
    assert_eq!(frame.preedit, "v.");
    assert_eq!(frame.candidates[1], "v.com");
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
    assert!(
        !press(&mut host, 'b' as u32).handled,
        "字母后面的字母照常交给应用"
    );
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
        (
            true,
            true,
            true,
            "自由输入法FreeType第5版",
            "自由输入法 FreeType 第 5 版",
        ),
        (true, true, false, "FreeType第5版", "FreeType 第5版"),
        (true, false, true, "FreeType第5版", "FreeType第 5 版"),
        (
            false,
            true,
            true,
            "自由输入法FreeType第5版",
            "自由输入法FreeType第5版",
        ),
        (
            true,
            true,
            true,
            "你好 FreeType\n第 5 版\tOK，𠀀A",
            "你好 FreeType\n第 5 版\tOK，𠀀 A",
        ),
    ];
    for (i, (enabled, letters, digits, text, expected)) in cases.into_iter().enumerate() {
        let mut host = host(&format!("autospace-inside-{i}"));
        configure(&mut host, |c| {
            c.autospace = enabled;
            c.autospace_letters = letters;
            c.autospace_digits = digits;
        });
        assert!(
            host.handle(
                1,
                Request::SetQuickPhrases {
                    phrases: vec![freetype_protocol::QuickPhrase {
                        code: "dz".into(),
                        text: text.into(),
                        weight: 10
                    }],
                }
            )
            .ok
        );
        typing(&mut host, "dz");
        assert_eq!(press(&mut host, 0x20).commit.as_deref(), Some(expected));
        cleanup(&host);
    }
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

    assert!(typing(&mut host, "gan")
        .candidates
        .iter()
        .any(|c| c == "改"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "huang")
        .candidates
        .iter()
        .any(|c| c == "望"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "xun")
        .candidates
        .iter()
        .any(|c| c == "兄"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "dong")
        .candidates
        .iter()
        .any(|c| c == "灯"));
    press(&mut host, keysym::ESCAPE);
    assert!(typing(&mut host, "yun")
        .candidates
        .iter()
        .any(|c| c == "用"));
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
    assert!(
        calc.candidates.iter().any(|c| c == "3"),
        "cands={:?}",
        calc.candidates
    );
    cleanup(&host);
}
