//! 上屏学习与删除学习。
use super::*;

#[test]
fn whole_sentence_is_learned() {
    // 分几次选出来的整句也记一次选词；选够 3 次（auto_learn_threshold）就收进用户词库
    let engine = engine();
    for round in 0..3 {
        let mut session = with(engine);
        type_keys(&mut session, "nihaozhongguo");
        choose_text(&mut session, "你好");
        assert_eq!(
            choose_text(&mut session, "中国").commit.as_deref(),
            Some("你好中国"),
            "第 {round} 次"
        );
    }
    let learned = engine.user_words();
    assert!(
        learned
            .iter()
            .any(|(pinyin, word, _)| word == "你好中国" && pinyin == "ni hao zhong guo"),
        "整句带拼音收进用户词库：{learned:?}"
    );
}

#[test]
fn single_word_is_learned_with_pinyin() {
    let engine = engine();
    for _ in 0..3 {
        let mut session = with(engine);
        type_keys(&mut session, "kaixin");
        assert_eq!(
            choose_text(&mut session, "开新").commit.as_deref(),
            Some("开新")
        );
    }
    assert!(engine
        .user_words()
        .iter()
        .any(|(pinyin, word, _)| word == "开新" && pinyin == "kai xin"));
}

#[test]
fn traditional_learns_the_lexicon_form() {
    // 开着繁体学到的词要按词库的写法（简体）收：关掉繁体照样能用，开着时只转一次。
    // 「萬里」再转一次会变成「萬裏」（里 -> 裏），收成繁体的话就会显示错。
    // 词库里 万、里 都不是首选，没学之前 wanli 出的是「晚理」
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str("wan\t晚\t9000\nwan\t万\t100\nli\t理\t9000\nli\t里\t100\n")
            .unwrap(),
    );
    let mut engine = Engine::new(lexicon, Arc::new(BigramLm::default()));
    engine.install_converter(Converter::from_tsv_str("万\t萬\n里\t裏\n万里\t萬里\n").unwrap());
    engine.set_traditional(true);
    let shown = |engine: &Engine| {
        let mut session = with_engine_ref(engine);
        type_keys(&mut session, "wanli");
        texts(&mut session)
    };
    assert_eq!(shown(&engine)[0], "晚理");
    for _ in 0..2 {
        // 连续两次分段选中后才正式收进用户词库。
        let mut session = with_engine_ref(&engine);
        type_keys(&mut session, "wanli");
        choose_text(&mut session, "萬");
        let frame = choose_text(&mut session, "裏");
        assert_eq!(
            frame.commit.as_deref(),
            Some("萬裏"),
            "上屏的是候选显示的字"
        );
    }
    let learned: Vec<String> = engine.user_words().into_iter().map(|(_, w, _)| w).collect();
    assert_eq!(learned, ["万里"], "学的是词库里的写法");

    // 开着繁体：学到的词排第一，转一次显示（萬里），没有再转一次的「萬裏」
    let candidates = shown(&engine);
    assert_eq!(candidates[0], "萬里", "{candidates:?}");
    assert!(!candidates.contains(&"萬裏".to_string()), "{candidates:?}");

    // 关掉繁体：学到的词照样排第一，是简体
    engine.set_traditional(false);
    let candidates = shown(&engine);
    assert_eq!(candidates[0], "万里", "{candidates:?}");
    assert!(
        candidates.iter().all(|c| !c.contains('萬')),
        "{candidates:?}"
    );
}

#[test]
fn emoji_and_plain_commit_without_learning() {
    // emoji 推荐、日期时间：带上已选的词一起上屏，不进用户词库
    let engine = engine();
    for _ in 0..3 {
        let mut session = with(engine);
        type_keys(&mut session, "nihaokaixin");
        choose_text(&mut session, "你好");
        assert_eq!(
            choose_text(&mut session, "😊").commit.as_deref(),
            Some("你好😊")
        );

        let mut session = with(engine);
        type_keys(&mut session, "riqi");
        assert_eq!(
            choose_text(&mut session, "2026-11-11").commit.as_deref(),
            Some("2026-11-11")
        );
        assert!(!session.composing());
    }
    assert!(engine
        .user_words()
        .iter()
        .all(|(_, word, _)| !word.contains('😊') && !word.contains("2026")));
}

#[test]
fn keycap_and_calculator_commit() {
    let mut session = with(engine());
    type_keys(&mut session, "ba");
    assert_eq!(
        choose_text(&mut session, "8️⃣").commit.as_deref(),
        Some("8️⃣")
    );

    let mut session = with(engine());
    type_keys(&mut session, "v");
    session.append("1+2*3");
    assert_eq!(session.choose(1).commit.as_deref(), Some("1+2*3=7"));
    assert!(!session.composing());
}

/// 分几次选出的词，首次候选收录，第二次进入词库。
#[test]
fn composed_word_is_learned_after_second_use() {
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(concat!(
            "shuang\t双\t9000\n",
            "shuang\t霜\t500\n",
            "pin\t拼\t9000\n",
            "pin\t频\t500\n",
            "ge\t个\t9000\n",
            "ge\t阁\t500\n",
            "wo\t我\t9000\n",
            "wo\t窝\t500\n",
            "men\t们\t9000\n",
            "men\t门\t500\n",
            "qu\t去\t9000\n",
            "qu\t区\t500\n",
        ))
        .unwrap(),
    );
    let engine: &'static Engine = Box::leak(Box::new(Engine::new(
        lexicon,
        Arc::new(BigramLm::default()),
    )));

    let mut session = with(engine);
    type_keys(&mut session, "shuangpinge");
    assert_ne!(
        texts(&mut session).first().map(String::as_str),
        Some("霜频阁")
    );
    for use_no in 1..=2 {
        if use_no == 2 {
            type_keys(&mut session, "shuangpinge");
        }
        choose_text(&mut session, "霜");
        choose_text(&mut session, "频");
        let frame = choose_text(&mut session, "阁");
        assert_eq!(frame.commit.as_deref(), Some("霜频阁"));
        assert_eq!(
            engine.user_words().iter().any(|(_, w, _)| w == "霜频阁"),
            use_no == 2
        );
    }

    let mut again = with(engine);
    type_keys(&mut again, "shuangpinge");
    assert_eq!(
        texts(&mut again).first().map(String::as_str),
        Some("霜频阁")
    );

    // 超过四个字的是整句，不会一次就进词库
    let mut long = with(engine);
    type_keys(&mut long, "womenquwomen");
    choose_text(&mut long, "窝");
    choose_text(&mut long, "门");
    choose_text(&mut long, "区");
    choose_text(&mut long, "窝");
    let frame = choose_text(&mut long, "门");
    assert_eq!(frame.commit.as_deref(), Some("窝门区窝门"));
    assert!(
        !engine
            .user_words()
            .iter()
            .any(|(_, w, _)| w == "窝门区窝门"),
        "整句不该一次就进词库：{:?}",
        engine.user_words()
    );
}

/// 学错的词 Ctrl+Delete 忘掉（#102）：用户词库、选词热度都去掉，编辑区不动，候选原地重算。
#[test]
fn ctrl_delete_forgets_learned_word() {
    let engine = forget_engine();
    let mut session = with(engine);
    for _ in 0..2 {
        type_keys(&mut session, "shuangpinge");
        choose_text(&mut session, "霜");
        choose_text(&mut session, "频");
        choose_text(&mut session, "阁");
    }
    assert!(engine.user_words().iter().any(|(_, w, _)| w == "霜频阁"));

    let mut again = with(engine);
    let before = type_keys(&mut again, "shuangpinge");
    assert_eq!(
        texts(&mut again).first().map(String::as_str),
        Some("霜频阁")
    );
    let frame = again.key(&ctrl(keysym::DELETE));
    assert!(frame.handled);
    assert!(frame.forgot, "忘掉了要告诉外壳存盘");
    assert_eq!(frame.commit, None);
    assert_eq!(frame.preedit, before.preedit, "编辑区不动");
    assert_eq!(again.input(), "shuangpinge");
    assert!(frame.cand_count > 0);
    assert!(
        !engine.user_words().iter().any(|(_, w, _)| w == "霜频阁"),
        "{:?}",
        engine.user_words()
    );
    assert_eq!(engine.history_count("霜频阁"), 0);
    assert_ne!(
        texts(&mut again).first().map(String::as_str),
        Some("霜频阁")
    );
    // 松开不算
    let release = Key {
        release: true,
        ..ctrl(keysym::DELETE)
    };
    assert!(!again.key(&release).handled);

    // 重新打一遍也不是第一个了，接着照常打字
    let mut retype = with(engine);
    type_keys(&mut retype, "shuangpinge");
    assert_ne!(
        texts(&mut retype).first().map(String::as_str),
        Some("霜频阁")
    );
    let frame = type_keys(&mut retype, "<sp>");
    assert!(frame.commit.is_some());
}

/// 词库里本来就有的词：删不掉，学到的加分去掉，回到原来的位置。光标停在原处。
#[test]
fn ctrl_delete_resets_system_word_and_keeps_cursor() {
    let engine = forget_engine();
    for _ in 0..3 {
        let mut session = with(engine);
        type_keys(&mut session, "shuang");
        choose_text(&mut session, "霜");
    }
    assert!(engine.history_count("霜") >= 3);
    let mut session = with(engine);
    type_keys(&mut session, "shuang");
    assert_eq!(texts(&mut session)[0], "霜", "选多了排到前面");
    // 光标挪到第二个（双），它没学过：什么也不做，但键吃掉
    let frame = type_keys(&mut session, "<right>");
    assert_eq!(frame.cand_cursor, Some(1));
    let frame = session.key(&ctrl(keysym::DELETE));
    assert!(frame.handled);
    assert!(!frame.forgot);
    assert_eq!(frame.cand_cursor, Some(1));
    assert_eq!(texts(&mut session)[0], "霜");
    // Ctrl+退格（Mac 笔记本没有向前删除键）忘掉首选
    type_keys(&mut session, "<left>");
    let frame = session.key(&ctrl(keysym::BACKSPACE));
    assert!(frame.handled && frame.forgot);
    assert_eq!(session.input(), "shuang", "Ctrl+退格不删字母");
    assert_eq!(texts(&mut session)[0], "双", "回到原来的位置");
    assert!(
        texts(&mut session).contains(&"霜".to_string()),
        "词库里的词还在"
    );
    assert_eq!(engine.history_count("霜"), 0);
    assert!(!engine.user_words().iter().any(|(_, w, _)| w == "霜"));
}

/// 自定义词不让 Ctrl+Delete 删；没在打字时这些键交给应用。
#[test]
fn ctrl_delete_keeps_custom_words_and_idle_is_not_ours() {
    let engine = forget_engine();
    engine.lexicon().set_custom_tsv("shuang\t爽\n");
    let mut session = with(engine);
    type_keys(&mut session, "shuang");
    assert_eq!(texts(&mut session)[0], "爽");
    let frame = session.key(&ctrl(keysym::DELETE));
    assert!(frame.handled && !frame.forgot);
    assert_eq!(texts(&mut session)[0], "爽");
    // Shift+Delete 也认，同样不动自定义词
    let shift_delete = Key {
        keysym: keysym::DELETE,
        modifiers: modifier::SHIFT,
        ..Key::default()
    };
    assert!(session.key(&shift_delete).handled);
    assert_eq!(texts(&mut session)[0], "爽");
    // 带 Alt 的不是
    let ctrl_alt = Key {
        keysym: keysym::DELETE,
        modifiers: modifier::CTRL | modifier::ALT,
        ..Key::default()
    };
    assert!(!session.key(&ctrl_alt).handled);

    let mut idle = with(engine);
    assert!(!idle.key(&ctrl(keysym::DELETE)).handled);
    assert!(!idle.key(&ctrl(keysym::BACKSPACE)).handled);
}

/// 鼠标长按候选忘词：按显示的序号忘，不看光标；没学过的、序号越界的返回 None。
#[test]
fn forget_by_shown_index() {
    let engine = forget_engine();
    for _ in 0..3 {
        let mut session = with(engine);
        type_keys(&mut session, "pin");
        choose_text(&mut session, "频");
    }
    let mut session = with(engine);
    type_keys(&mut session, "pin");
    assert_eq!(texts(&mut session)[0], "频");
    let at = texts(&mut session).iter().position(|t| t == "拼").unwrap();
    assert_eq!(session.forget(at), None, "没学过的词不动");
    assert_eq!(session.forget(99), None);
    assert_eq!(session.forget(0).as_deref(), Some("频"));
    assert_eq!(session.input(), "pin", "编辑区不动");
    assert_eq!(texts(&mut session)[0], "拼");
    let frame = session.frame(true);
    assert!(frame.forgot, "要告诉外壳存盘");
    assert_eq!(frame.commit, None);
}
