//! 选词与翻页：数字、空格、回车、翻页键、展开面板和以词定字。
use super::*;

#[test]
fn choose_sentence_start_then_the_rest() {
    // key_harness：「句首词选完接着选剩下的」「整句上屏」
    let mut session = with(engine());
    type_keys(&mut session, "nihaozhongguo");
    let frame = choose_text(&mut session, "你好");
    assert!(frame.handled);
    assert_eq!(frame.commit, None, "句首词先进已选栈");
    assert_eq!(session.remaining(), "zhongguo");
    assert_eq!(texts(&mut session)[0], "中国");
    let frame = session.choose(0);
    assert!(frame.handled);
    assert_eq!(frame.commit.as_deref(), Some("你好中国"));
    assert!(frame.preedit.is_empty());
    assert!(!session.composing());
}

#[test]
fn bad_index_is_not_handled() {
    let mut session = with(engine());
    type_keys(&mut session, "nihao");
    let total = session.candidates().len();
    let frame = session.choose(total);
    assert!(!frame.handled);
    assert_eq!(frame.commit, None);
    assert_eq!(frame.preedit, "ni'hao", "选错序号什么也不改");

    // 没有引擎就没有候选
    let mut session = fresh();
    type_keys(&mut session, "nihao");
    assert!(!session.choose(0).handled);
}

#[test]
fn commit_input_commits_selected_and_raw() {
    // key_harness：「回车：已选的词 + 原始字母上屏」
    let mut session = with(engine());
    type_keys(&mut session, "nihaokaixin");
    choose_text(&mut session, "你好");
    let frame = session.commit_input();
    assert!(frame.handled);
    assert_eq!(frame.commit.as_deref(), Some("你好kaixin"));
    assert!(!session.composing());

    // 什么也没打：没有上屏
    let mut session = with(engine());
    assert_eq!(session.commit_input().commit, None);

    // 不带引擎也能上屏原始输入
    let mut session = fresh();
    type_keys(&mut session, "abc");
    assert_eq!(session.commit_input().commit.as_deref(), Some("abc"));
}

#[test]
fn zero_selects_the_tenth_visible_candidate_on_ten_item_pages() {
    for sym in [keysym::DIGIT_0, keysym::KP_0] {
        for next_page in [false, true] {
            let mut session = with(many_engine());
            session.configure(Config {
                page_size: 10,
                emoji_suggest: false,
                ..Config::default()
            });
            type_keys(&mut session, "shi");
            if next_page {
                type_keys(&mut session, "<pgdn>");
            }
            let shown = shown_texts(&mut session);
            assert_eq!(shown.len(), 10);
            let frame = session.key(&Key::press(sym));
            assert_eq!(
                frame.commit.as_ref(),
                Some(&shown[9]),
                "0 应选当前页显示的第 10 个候选"
            );
            assert!(!session.composing());
        }
    }
    let mut session = with(many_engine());
    let frame = type_keys(&mut session, "v10+2<sp>");
    assert_eq!(
        frame.commit.as_deref(),
        Some("12"),
        "计算器里的 0 仍是数字正文"
    );
}

#[test]
fn space_commits_cursor_candidate() {
    // key_harness：「空格上屏首选」「右键移光标后空格上屏光标处」
    let (mut session, all) = default_paged();
    let frame = session.frame(false);
    assert_eq!((frame.cand_start, frame.cand_count), (0, 5));
    assert_eq!(frame.cand_cursor, Some(0));
    assert!(!frame.has_prev && frame.has_next);
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some(all[0].as_str())
    );
    assert!(!session.composing());

    let (mut session, all) = default_paged();
    assert_eq!(type_keys(&mut session, "<right>").cand_cursor, Some(1));
    let frame = session.key(&Key::press(keysym::SPACE));
    assert_eq!(frame.commit.as_deref(), Some(all[1].as_str()));
    assert_eq!(
        session.last_choice().map(|c| c.text.as_str()),
        Some(all[1].as_str()),
        "外壳靠 last_choice 知道上屏的是哪个候选"
    );
    // 下一次调用（包括松开键）清掉 last_choice
    session.key(&Key {
        release: true,
        ..Key::press(keysym::SPACE)
    });
    assert!(session.last_choice().is_none());
}

#[test]
fn return_commits_raw_input() {
    let (mut session, _) = default_paged();
    type_keys(&mut session, "<right>");
    assert_eq!(
        type_keys(&mut session, "<cr>").commit.as_deref(),
        Some("shi")
    );
    assert!(session.last_choice().is_none(), "回车不是选候选");
}

#[test]
fn digits_choose_on_the_page() {
    let (mut session, all) = default_paged();
    assert_eq!(
        type_keys(&mut session, "3").commit.as_deref(),
        Some(all[2].as_str())
    );

    // 超出这一页、0：吃掉，什么也不做
    let (mut session, _) = default_paged();
    for keys in ["6", "0"] {
        let frame = type_keys(&mut session, keys);
        assert!(frame.handled, "{keys}");
        assert_eq!(frame.commit, None);
        assert!(session.composing());
    }

    // 小键盘数字一样；带 Shift 的数字不是选词键
    let (mut session, all) = default_paged();
    let frame = session.key(&Key::press(keysym::KP_0 + 2));
    assert_eq!(frame.commit.as_deref(), Some(all[1].as_str()));
    let (mut session, _) = default_paged();
    let frame = session.key(&Key {
        modifiers: wowtype_core::session::modifier::SHIFT,
        ..Key::char('2')
    });
    assert!(!frame.handled);

    // 翻到第二页，数字按第二页选
    let (mut session, all) = default_paged();
    type_keys(&mut session, "<pgdn>");
    assert_eq!(
        type_keys(&mut session, "2").commit.as_deref(),
        Some(all[6].as_str())
    );
}

#[test]
fn paging_keys() {
    // key_harness：「PgDn 翻页」「- = 翻页」「翻到头吃掉翻页键」
    let (mut session, all) = default_paged();
    let frame = type_keys(&mut session, "<pgdn>");
    assert!(frame.handled);
    assert_eq!((frame.cand_start, frame.cand_count), (5, 5));
    assert!(frame.has_prev && frame.has_next);
    assert_eq!(frame.cand_cursor, Some(0), "翻页时光标跟到新的一页开头");
    assert_eq!(shown_texts(&mut session), all[5..10]);
    assert_eq!(type_keys(&mut session, "=").cand_start, 10);
    assert_eq!(type_keys(&mut session, "-").cand_start, 5);
    assert_eq!(
        type_keys(&mut session, "<up>").cand_start,
        0,
        "横排上键翻页"
    );
    // 第一页再按 -：吃掉，不当标点
    let frame = type_keys(&mut session, "-");
    assert!(frame.handled);
    assert_eq!(frame.cand_start, 0);
    // PgUp 翻不动：交给外壳
    assert!(!session.key(&Key::press(keysym::PAGE_UP)).handled);
    // 翻到最后一页
    let last = (all.len() - 1) / 5 * 5;
    for _ in 0..10 {
        type_keys(&mut session, "<pgdn>");
    }
    let frame = session.frame(false);
    assert_eq!(frame.cand_start, last);
    assert!(!frame.has_next);
    assert!(
        type_keys(&mut session, "=").handled,
        "最后一页再按 = 也吃掉"
    );

    // 没打开的翻页键交给外壳（, . [ ] Tab 默认不翻页）
    let (mut session, _) = default_paged();
    for keys in [",", "[", "<tab>"] {
        assert!(!type_keys(&mut session, keys).handled, "{keys}");
    }

    // 只有一页：- 交给外壳（当标点）
    let mut session = with(many_engine());
    type_keys(&mut session, "nihao");
    assert!(session.frame(false).cand_count < 9 && !session.frame(false).has_next);
    assert!(!type_keys(&mut session, "-").handled);
}

#[test]
fn configured_page_keys() {
    use wowtype_core::session::page_keys;
    let (mut session, _) = paged(Config {
        page_keys: page_keys::BRACKET | page_keys::COMMA | page_keys::TAB,
        ..Config::default()
    });
    assert_eq!(type_keys(&mut session, "]").cand_start, 5);
    assert_eq!(type_keys(&mut session, ".").cand_start, 10);
    assert_eq!(type_keys(&mut session, "<tab>").cand_start, 15);
    let shift_tab = Key {
        modifiers: wowtype_core::session::modifier::SHIFT,
        ..Key::press(keysym::TAB)
    };
    assert_eq!(session.key(&shift_tab).cand_start, 10);
    assert_eq!(session.key(&Key::press(keysym::ISO_LEFT_TAB)).cand_start, 5);
    assert_eq!(type_keys(&mut session, ",").cand_start, 0);
    assert_eq!(type_keys(&mut session, "[").cand_start, 0);
    // - = 没打开：交给外壳
    assert!(!type_keys(&mut session, "-").handled);
}

#[test]
fn cursor_moves_across_pages_but_stops_at_both_ends() {
    let (mut session, all) = default_paged();
    for _ in 0..5 {
        type_keys(&mut session, "<right>");
    }
    let frame = session.frame(false);
    assert_eq!(frame.cand_start, 5, "光标移出这一页，跟着翻页");
    assert_eq!(frame.cand_cursor, Some(0));
    let frame = type_keys(&mut session, "<left>");
    assert_eq!(
        (frame.cand_start, frame.cand_cursor),
        (0, Some(4)),
        "往回也跨页"
    );

    // 第一个往左：停在第一个，不绕到末页的生僻字；吃掉，不交给应用
    let (mut session, _) = default_paged();
    let frame = type_keys(&mut session, "<left>");
    assert!(frame.handled);
    assert_eq!((frame.cand_start, frame.cand_cursor), (0, Some(0)));
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some(all[0].as_str())
    );

    // 最后一个往右：停在最后一个，不绕回第一个
    let (mut session, _) = default_paged();
    for _ in 0..all.len() + 3 {
        type_keys(&mut session, "<right>");
    }
    let frame = session.frame(false);
    assert_eq!(frame.cand_start + frame.cand_cursor.unwrap(), all.len() - 1);
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some(all[all.len() - 1].as_str())
    );
}

#[test]
fn actions_from_the_shell() {
    // 外壳把 fcitx5 全局的翻页、移动候选热键（或鼠标点的翻页按钮）翻成动作传进来
    use wowtype_core::session::KeyAction;
    let (mut session, all) = default_paged();
    let frame = session.key(&Key::action(KeyAction::NextPage));
    assert!(frame.handled);
    assert_eq!(frame.cand_start, 5);
    assert_eq!(
        session
            .key(&Key::action(KeyAction::NextCandidate))
            .cand_cursor,
        Some(1)
    );
    assert_eq!(
        session
            .key(&Key::action(KeyAction::PrevCandidate))
            .cand_cursor,
        Some(0)
    );
    assert_eq!(session.key(&Key::action(KeyAction::PrevPage)).cand_start, 0);
    // 第一页再往前：还是吃掉
    assert!(session.key(&Key::action(KeyAction::PrevPage)).handled);
    // 动作带着键码：动作优先（比如 fcitx5 设成 , . 翻页）
    let comma_next = Key {
        action: KeyAction::NextPage,
        ..Key::char('.')
    };
    assert_eq!(session.key(&comma_next).cand_start, 5);
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some(all[5].as_str())
    );

    // 方向键带了动作：照横排竖排的规则走，不按动作
    let (mut session, _) = default_paged();
    let down = Key {
        action: KeyAction::NextPage,
        ..Key::press(keysym::DOWN)
    };
    assert_eq!(session.key(&down).panel, Panel::Expanded);
}

#[test]
fn vertical_list() {
    // 竖排：上下移光标，左右翻页，下键不展开
    let (mut session, all) = paged(Config {
        horizontal: false,
        ..Config::default()
    });
    let frame = type_keys(&mut session, "<down>");
    assert_eq!(frame.panel, Panel::Normal);
    assert_eq!(frame.cand_cursor, Some(1));
    assert_eq!(type_keys(&mut session, "<right>").cand_start, 5);
    assert_eq!(type_keys(&mut session, "<left>").cand_start, 0);
    let frame = type_keys(&mut session, "<up>");
    assert_eq!(
        (frame.cand_start, frame.cand_cursor),
        (0, Some(0)),
        "第一个再往上：停住，不绕到最后一个"
    );
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some(all[0].as_str())
    );
}

#[test]
fn expanded_panel() {
    // key_harness：「横排下键展开候选面板」「面板里方向键、数字选当前行、Esc 收起」
    let (mut session, all) = default_paged();
    type_keys(&mut session, "<right>");
    let frame = type_keys(&mut session, "<down>");
    assert_eq!(frame.panel, Panel::Expanded);
    assert_eq!(frame.cand_start, 0);
    assert_eq!(frame.cand_count, all.len().min(36));
    assert_eq!(frame.cand_cursor, Some(1), "光标停在展开前选中的词上");
    assert!(session.view().expanded);

    assert_eq!(type_keys(&mut session, "<down>").cand_cursor, Some(7));
    assert_eq!(type_keys(&mut session, "<left>").cand_cursor, Some(6));
    // 数字选光标所在行
    assert_eq!(
        type_keys(&mut session, "3").commit.as_deref(),
        Some(all[8].as_str())
    );
    assert!(!session.composing());

    // 第一行再往上：收起，回到第一页
    let (mut session, _) = default_paged();
    type_keys(&mut session, "<down>");
    let frame = type_keys(&mut session, "<up>");
    assert_eq!(frame.panel, Panel::Normal);
    assert_eq!(frame.cand_start, 0);
    assert_eq!(frame.cand_cursor, Some(0));

    // Esc 收起，不清空
    let (mut session, all) = default_paged();
    type_keys(&mut session, "<down><right><right>");
    let frame = type_keys(&mut session, "<esc>");
    assert_eq!(frame.panel, Panel::Normal);
    assert!(session.composing());
    assert_eq!(frame.cand_cursor, Some(2));
    // 空格上屏面板光标处
    type_keys(&mut session, "<down>");
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some(all[2].as_str())
    );

    // 面板里按别的键：先收起，键照普通规则走（Tab 交给外壳吃掉）
    let (mut session, _) = default_paged();
    type_keys(&mut session, "<down><right>");
    let frame = type_keys(&mut session, "<tab>");
    assert!(!frame.handled);
    assert!(!session.view().expanded);
    assert_eq!(session.view().cursor, 1);

    // 退格：收起再删字母
    let (mut session, _) = default_paged();
    type_keys(&mut session, "<down>");
    let frame = type_keys(&mut session, "<bs>");
    assert_eq!(frame.panel, Panel::Normal);
    assert_eq!(frame.preedit, "sh");

    // 只有一个候选不展开（下键交给外壳吃掉）
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("heng xiang\t横向\t5000\n").unwrap());
    let engine: &'static Engine = Box::leak(Box::new(Engine::new(
        lexicon,
        Arc::new(BigramLm::default()),
    )));
    let mut session = with(engine);
    session.configure(Config {
        emoji_suggest: false,
        date_time: false,
        ..Config::default()
    });
    type_keys(&mut session, "hengxiang");
    assert_eq!(texts(&mut session), ["横向"]);
    let frame = type_keys(&mut session, "<down>");
    assert!(!frame.handled);
    assert_eq!(frame.panel, Panel::Normal);
}

#[test]
fn expanded_panel_second_screen() {
    // key_harness：「展开后下移 7 行：面板翻到第二屏」「第二屏收起后，横排候选框停在光标那一页」
    let tsv: String = (0..60u32)
        .map(|i| {
            format!(
                "shi\t{}\t{}\n",
                char::from_u32(0x4e00 + i).unwrap(),
                9000 - i
            )
        })
        .collect();
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(&tsv).unwrap());
    let mut engine = Engine::new(lexicon, Arc::new(BigramLm::default()));
    engine.set_max_candidates(300);
    let engine: &'static Engine = Box::leak(Box::new(engine));
    let mut session = with(engine);
    session.configure(Config {
        page_size: 5,
        emoji_suggest: false,
        date_time: false,
        ..Config::default()
    });
    type_keys(&mut session, "shi");
    let all = texts(&mut session);
    assert!(all.len() > 8 * 6, "要超过 8 行：{}", all.len());

    // 一直按下键走到第二屏：面板显示光标那一屏
    let frame = type_keys(
        &mut session,
        "<down><down><down><down><down><down><down><down>",
    );
    assert_eq!(frame.panel, Panel::Expanded);
    assert_eq!(frame.cand_start, 36);
    assert_eq!(frame.cand_cursor, Some(6));
    assert_eq!(session.view().grid_cursor, 42);

    // 收起：停在光标那一页，光标看得见
    let frame = type_keys(&mut session, "<esc>");
    assert_eq!(frame.panel, Panel::Normal);
    assert_eq!(session.view().page, 42 / 5);
    assert_eq!(frame.cand_start, 40);
    assert_eq!(frame.cand_cursor, Some(2));
    assert!(frame.has_prev);
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some(all[42].as_str())
    );
}

#[test]
fn select_by_shown_index() {
    // 点候选：序号是在显示的那些（这一页，或展开面板的这一屏）里的
    let (mut session, all) = default_paged();
    type_keys(&mut session, "<pgdn>");
    assert_eq!(session.select(1).commit.as_deref(), Some(all[6].as_str()));

    let (mut session, _) = default_paged();
    assert!(!session.select(5).handled, "超出这一页");
    assert!(session.composing());

    let (mut session, all) = default_paged();
    type_keys(&mut session, "<down>");
    assert_eq!(session.select(13).commit.as_deref(), Some(all[13].as_str()));
}

#[test]
fn semicolon_and_quote_select() {
    // key_harness：「分号选第 2 个、引号选第 3 个」
    use wowtype_core::session::SelectKeys;
    let (mut session, all) = paged(Config {
        select_keys: SelectKeys::SemicolonQuote,
        ..Config::default()
    });
    assert_eq!(
        type_keys(&mut session, ";").commit.as_deref(),
        Some(all[1].as_str())
    );
    let (mut session, all) = paged(Config {
        select_keys: SelectKeys::SemicolonQuote,
        ..Config::default()
    });
    assert_eq!(
        type_keys(&mut session, "'").commit.as_deref(),
        Some(all[2].as_str())
    );

    // 只开分号：引号还是隔音符
    let (mut session, _) = paged(Config {
        select_keys: SelectKeys::Semicolon,
        ..Config::default()
    });
    assert_eq!(type_keys(&mut session, "'").preedit, "shi'");

    // 不开：分号交给外壳（当标点）
    let (mut session, _) = default_paged();
    assert!(!type_keys(&mut session, ";").handled);

    // 第 2 个以后是第二页的：按这一页数
    let (mut session, all) = paged(Config {
        select_keys: SelectKeys::Semicolon,
        ..Config::default()
    });
    type_keys(&mut session, "<pgdn>");
    assert_eq!(
        type_keys(&mut session, ";").commit.as_deref(),
        Some(all[6].as_str())
    );
}

#[test]
fn shuangpin_semicolon_is_a_key_on_odd_count() {
    // key_harness：「微软双拼分号是韵母键」：等第二个键时分号进编辑区；
    // 打满一对以后分号不进编辑区（开了选词键时选第 2 个也不行：分号是 ing 键）
    use wowtype_core::session::SelectKeys;
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(MANY).unwrap());
    let mut engine = Engine::new(lexicon, Arc::new(BigramLm::default()));
    engine.set_shuangpin(true);
    engine.set_shuangpin_scheme(wowtype_core::shuangpin::Scheme::Microsoft);
    let engine: &'static Engine = Box::leak(Box::new(engine));
    let mut session = with(engine);
    session.configure(Config {
        select_keys: SelectKeys::Semicolon,
        ..Config::default()
    });
    type_keys(&mut session, "x");
    let frame = type_keys(&mut session, ";");
    assert!(frame.handled);
    assert_eq!(session.input(), "x;");
    let frame = type_keys(&mut session, ";");
    assert!(!frame.handled, "成对了：分号交给外壳");
    assert_eq!(session.input(), "x;");
}

#[test]
fn brackets_pick_first_or_last_char() {
    // 以词定字（#83）：[ 取光标处候选的第一个字，] 取最后一个字
    let mut session = with(engine());
    type_keys(&mut session, "nihao");
    assert_eq!(texts(&mut session)[0], "你好");
    let frame = type_keys(&mut session, "[");
    assert!(frame.handled);
    assert_eq!(frame.commit.as_deref(), Some("你"));
    assert!(frame.preedit.is_empty());

    let mut session = with(engine());
    assert_eq!(
        type_keys(&mut session, "kaixin]").commit.as_deref(),
        Some("心")
    );

    // 光标挪到第二个候选：取它的字
    let mut session = with(engine());
    type_keys(&mut session, "kaixin<right>");
    assert_eq!(type_keys(&mut session, "]").commit.as_deref(), Some("欣"));

    // 句首词：取的字进已选栈，剩下的拼音接着选
    let mut session = with(engine());
    type_keys(&mut session, "nihaozhongguo");
    let prefix = session
        .candidates()
        .iter()
        .position(|c| c.text == "你好")
        .expect("有句首词你好");
    for _ in 0..prefix {
        type_keys(&mut session, "<right>");
    }
    let frame = type_keys(&mut session, "]");
    assert_eq!(frame.commit, None);
    assert_eq!(session.selected_text(), "好");
    assert_eq!(
        type_keys(&mut session, "<sp>").commit.as_deref(),
        Some("好中国")
    );

    // 取出来的字不记选词：你好没学过，不会因为按了 [ 变成用户词
    let engine = engine();
    let mut session = with(engine);
    type_keys(&mut session, "nihao[");
    assert!(
        engine.user_words().is_empty(),
        "以词定字不学词：{:?}",
        engine.user_words()
    );
}

#[test]
fn brackets_fall_through_when_nothing_to_pick() {
    // 单字候选：交给外壳（外壳先上屏再出【】）
    let mut session = with(engine());
    type_keys(&mut session, "ba");
    assert_eq!(texts(&mut session)[0], "吧");
    assert!(!type_keys(&mut session, "[").handled);

    // 关掉以词定字，或者 [ ] 是翻页键：不取字
    let mut session = with(engine());
    session.configure(Config {
        pick_char: false,
        ..Config::default()
    });
    type_keys(&mut session, "nihao");
    assert!(!type_keys(&mut session, "[").handled);

    let mut session = with(engine());
    session.configure(Config {
        page_keys: wowtype_core::session::page_keys::BRACKET,
        ..Config::default()
    });
    type_keys(&mut session, "nihao");
    assert_eq!(type_keys(&mut session, "[").commit, None);
}
