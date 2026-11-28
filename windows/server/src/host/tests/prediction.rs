//! 上屏联想：开关、按键、选词、失效与学习。
use super::*;

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
    assert_eq!(result.candidates, ["不错"]);
    let single = press(&mut host, 0x20);
    assert!(!single.handled && single.commit.is_none() && single.predict);
    assert_eq!(
        single.page_revision, result.page_revision,
        "下一轮第一次空格必须保留新联想页"
    );
    let next = press(&mut host, keysym::SPACE_DOUBLE);
    assert_eq!(next.commit.as_deref(), Some("不错"));
    assert!(next.replace_space && !next.predict);
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
        assert!(
            !frame.handled && frame.commit.is_none(),
            "{shortcut} 不再选词"
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            saved.get("predict_shortcut").is_none(),
            "重存时不再写入旧设置"
        );
    }
    cleanup(&host);
}

#[test]
fn stale_candidate_action_does_not_select_and_fixed_slot_survives_reload() {
    let mut host = predict_host("fixed-candidate");
    host.engine
        .as_ref()
        .unwrap()
        .add_user_word("jin tian", "今填", 1000.0)
        .unwrap();
    let frame = typing(&mut host, "jintian");
    let index = frame
        .candidates
        .iter()
        .position(|word| word == "今填")
        .unwrap();
    let pinned = host
        .handle(
            1,
            Request::CandidateAction {
                revision: frame.page_revision,
                action: CandidateAction::Pin { index, place: 1 },
            },
        )
        .frame
        .unwrap();
    assert_eq!(pinned.candidates[0], "今填");
    assert_eq!(pinned.pin_slots[0], Some(1));
    let stale = host
        .handle(
            1,
            Request::CandidateAction {
                revision: frame.page_revision,
                action: CandidateAction::Select { index: 0 },
            },
        )
        .frame
        .unwrap();
    assert!(stale.commit.is_none());
    let pins = read_pins(&host.data_dir().join("candidate-pins.json")).unwrap();
    assert_eq!(pins["quanpin:jintian"][&1], "jin tian\t今填");
    let unpin = host
        .handle(
            1,
            Request::CandidateAction {
                revision: stale.page_revision,
                action: CandidateAction::Pin { index: 0, place: 0 },
            },
        )
        .frame
        .unwrap();
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
    assert!(
        !press(&mut host, keysym::ESCAPE).handled,
        "收起以后 Esc 归应用"
    );
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
        assert!(
            !frame.handled && frame.candidates.is_empty() && frame.commit.is_none(),
            "{sym:#x}"
        );
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
    assert_eq!(
        press(&mut host, '1' as u32).commit.as_deref(),
        Some(" 1"),
        "数字照常打出来"
    );
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
        assert!(
            host.config().predict_arrows,
            "开启联想后默认支持左右选词，与组字光标设置无关"
        );
        let shown = commit_today(&mut host).candidates;
        let learned = host.engine.as_ref().unwrap().export_learning().unwrap();
        for (sym, cursor) in [
            (keysym::LEFT, 0),
            (keysym::RIGHT, 1),
            (keysym::RIGHT, 1),
            (keysym::LEFT, 0),
            (keysym::RIGHT, 1),
        ] {
            let frame = press(&mut host, sym);
            assert!(frame.handled && frame.predict && frame.preedit.is_empty());
            assert!(frame.commit.is_none(), "移动高亮不能上屏");
            assert_eq!(frame.candidates, shown);
            assert_eq!(frame.cursor, Some(cursor), "两端停住，不循环");
        }
        assert_eq!(
            host.engine.as_ref().unwrap().export_learning().unwrap(),
            learned,
            "移动高亮不能学习"
        );
        let frame = press(&mut host, 0x20);
        assert_eq!(frame.commit.as_deref(), Some("天气"));
        assert_eq!(frame.candidates, ["不错"]);
        assert_eq!(frame.cursor, Some(0), "下一轮联想重新从第一个词开始");
        assert_eq!(
            press(&mut host, keysym::RIGHT).cursor,
            Some(0),
            "只有一个候选时仍吃掉方向键"
        );
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
        assert!(
            !frame.handled
                && !frame.predict
                && frame.commit.is_none()
                && frame.candidates.is_empty()
        );
    }
    configure(&mut host, |c| c.predict_arrows = true);
    for modifiers in [
        modifier::SHIFT,
        modifier::CTRL,
        modifier::ALT,
        modifier::SUPER,
    ] {
        for sym in [keysym::LEFT, keysym::RIGHT] {
            assert!(commit_today(&mut host).predict);
            let key = Key {
                keysym: sym,
                modifiers,
                ..Key::default()
            };
            let frame = host
                .handle(1, Request::Key { key, before: None })
                .frame
                .unwrap();
            assert!(
                !frame.handled
                    && !frame.predict
                    && frame.commit.is_none()
                    && frame.candidates.is_empty()
            );
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
    assert!(
        !press(&mut host, 0x20).handled,
        "左右选词不能悄悄开启空格上屏"
    );
    let shown = commit_today(&mut host);
    assert!(shown.predict && shown.candidates.len() >= 2);
    assert_eq!(press(&mut host, keysym::RIGHT).cursor, Some(1));
    let frame = host.handle(1, Request::Select { index: 0 }).frame.unwrap();
    assert_eq!(
        frame.commit.as_deref(),
        Some(shown.candidates[0].as_str()),
        "鼠标仍按点中的词上屏"
    );
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
    let preview = Key {
        keysym: keysym::RIGHT,
        test: true,
        ..Key::default()
    };
    for _ in 0..2 {
        let frame = host
            .handle(
                1,
                Request::Key {
                    key: preview.clone(),
                    before: None,
                },
            )
            .frame
            .unwrap();
        assert!(frame.handled && frame.commit.is_none());
        assert_eq!(frame.cursor, Some(1), "重复预判从真实位置计算");
    }
    assert_eq!(
        press(&mut host, 0x20).commit.as_deref(),
        Some("下雨"),
        "没有真按右键就仍选第一个"
    );
    press(&mut host, keysym::ESCAPE);
    commit_today(&mut host);
    host.handle(
        1,
        Request::Key {
            key: preview,
            before: None,
        },
    );
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
    let key = Key {
        keysym: '1' as u32,
        ..Key::default()
    };
    let frame = host
        .handle(
            1,
            Request::Key {
                key,
                before: Some("今天，".into()),
            },
        )
        .frame
        .unwrap();
    assert!(!frame.handled && frame.commit.is_none() && frame.candidates.is_empty());
    // 换输入框
    assert!(commit_today(&mut host).predict);
    let frame = host
        .handle(
            1,
            Request::Focus {
                active: true,
                before: Some("今天".into()),
            },
        )
        .frame
        .unwrap();
    assert!(frame.candidates.is_empty());
    // 收起（一会儿没选）
    assert!(commit_today(&mut host).predict);
    assert!(host
        .handle(1, Request::Reset { commit: false })
        .frame
        .unwrap()
        .candidates
        .is_empty());
    assert!(!press(&mut host, 0x20).handled);
    // 切到英文
    assert!(commit_today(&mut host).predict);
    let frame = host
        .handle(1, Request::ToggleMode { shift: None })
        .frame
        .unwrap();
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
    let preview = Key {
        keysym: '2' as u32,
        test: true,
        ..Key::default()
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
        learned
    );
    let pairs = host.engine.as_ref().unwrap().pair_count();
    assert_eq!(press(&mut host, '2' as u32).commit.as_deref(), Some("天气"));
    assert_eq!(
        host.engine.as_ref().unwrap().pair_count(),
        pairs + 1,
        "只学一次"
    );
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
    assert!(commit_today(&mut host)
        .candidates
        .iter()
        .any(|c| c == "开会"));
    cleanup(&host);
}
