//! 候选来源：引擎、表情、键帽、日期时间、计算器与缓存。
use super::*;

#[test]
fn preedit_is_segmented() {
    // key_harness：「编辑区给拼音分音节」「编辑区 = 已选 + 剩余拼音（分好音节）」「退格先撤销选词」
    let mut session = with(engine());
    assert_eq!(type_keys(&mut session, "hengxiang").preedit, "heng'xiang");

    let mut session = with(engine());
    assert_eq!(
        type_keys(&mut session, "nihaozhongguo").preedit,
        "ni'hao'zhong'guo"
    );
    assert_eq!(choose_text(&mut session, "你好").preedit, "你好zhong'guo");
    assert_eq!(
        type_keys(&mut session, "<bs>").preedit,
        "ni'hao'zhong'guo",
        "退格撤销选词，分音节跟着回来"
    );
}

#[test]
fn candidates_come_from_the_engine() {
    let mut session = with(engine());
    type_keys(&mut session, "nihao");
    let all = session.candidates();
    assert_eq!(all[0].text, "你好");
    assert_eq!(all[0].pinyin, "ni hao");
    assert_eq!(all[0].consumed, 5);
    assert_eq!(all[0].kind, CandidateKind::Word);
    assert_eq!(all[0].commit, "你好");
}

#[test]
fn no_engine_yet_means_no_candidates() {
    // 词库在后台载入：载入好之前照样打字，编辑区是原始输入；载入好了候选马上出来
    let mut session = Session::new();
    type_keys(&mut session, "hengxiang");
    assert!(session.candidates().is_empty());
    assert_eq!(session.frame(false).preedit, "hengxiang");
    session.set_engine(Some(engine()));
    assert_eq!(texts(&mut session)[0], "横向");
    assert_eq!(session.frame(false).preedit, "heng'xiang");
}

#[test]
fn emoji_suggestion_is_third() {
    // key_harness：「打字时推荐 emoji：插在第三个候选」
    let mut session = with(engine());
    type_keys(&mut session, "kaixin");
    let all = session.candidates().to_vec();
    assert_eq!(all[0].text, "开心");
    assert_eq!(all[2].text, "😊");
    assert_eq!(all[2].kind, CandidateKind::Emoji);
    assert_eq!(all[2].commit, "😊");
    assert_eq!(all[2].emoji, "😊");

    // 关掉 emoji 推荐后没有 😊
    session.configure(Config {
        emoji_suggest: false,
        ..Config::default()
    });
    assert!(!texts(&mut session).contains(&"😊".to_string()));
}

#[test]
fn keycap_follows_number_word() {
    // key_harness：「ba：八 后面是 8️⃣」「拼音没打完的『八』不跟 8️⃣」
    let mut session = with(engine());
    type_keys(&mut session, "ba");
    let all = session.candidates().to_vec();
    let at = all.iter().position(|c| c.text == "8️⃣").expect("没有 8️⃣");
    assert_eq!(all[at - 1].text, "八");
    assert_eq!(all[at].kind, CandidateKind::Emoji);
    assert_eq!(all[at].commit, "8️⃣");
    assert_eq!(
        all.iter()
            .filter(|c| c.kind == CandidateKind::Emoji)
            .count(),
        1,
        "只跟一个"
    );

    // 第一页（page_size）之外的数字字不跟
    session.configure(Config {
        page_size: 1,
        ..Config::default()
    });
    assert!(!texts(&mut session).contains(&"8️⃣".to_string()));
}

#[test]
fn date_time_after_first() {
    // key_harness：「日期时间候选：格式从第二个开始」
    let mut session = with(engine());
    type_keys(&mut session, "riqi");
    let all = session.candidates().to_vec();
    assert_eq!(all[0].text, "日期");
    assert_eq!(all[1].text, "2026年11月11日");
    assert_eq!(all[1].kind, CandidateKind::Plain);
    assert_eq!(all[2].text, "2026-11-11");

    let mut session = with(engine());
    type_keys(&mut session, "shijian");
    assert_eq!(texts(&mut session)[..3], ["时间", "10:20", "10:20:30"]);

    let mut session = with(engine());
    type_keys(&mut session, "xingqi");
    assert_eq!(texts(&mut session)[1], "星期三");

    // 选过词以后不出
    let mut session = with(engine());
    type_keys(&mut session, "nihaoriqi");
    choose_text(&mut session, "你好");
    assert!(!texts(&mut session).iter().any(|t| t.contains('年')));

    // 关掉不出
    let mut session = with(engine());
    session.configure(Config {
        date_time: false,
        ..Config::default()
    });
    type_keys(&mut session, "riqi");
    assert!(!texts(&mut session).iter().any(|t| t.contains('年')));
}

#[test]
fn date_time_follows_trigger_word_even_when_it_is_not_first() {
    // 接着前文打（刚上屏一个日期）时，「事件」可能排在「时间」前面；时间的写法是给「时间」的，
    // 「时间」挪到第一，写法跟在它后面
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str("shi jian\t事件\t900000\nshi jian\t时间\t500000\n").unwrap(),
    );
    let engine: &'static Engine = Box::leak(Box::new(Engine::new(
        lexicon,
        Arc::new(BigramLm::default()),
    )));
    let mut session = with(engine);
    type_keys(&mut session, "shijian");
    assert_eq!(texts(&mut session)[..3], ["时间", "10:20", "10:20:30"]);
    assert!(texts(&mut session).contains(&"事件".to_string()));
}

#[test]
fn insertion_order_emoji_keycap_date_time() {
    // 外壳的顺序：emoji 推荐插第 3 个，然后在第一页里找数字字跟按键 emoji，
    // 最后日期时间插在首选后面（把前面插的往后挤）
    let mut session = with(engine());
    type_keys(&mut session, "riqi");
    let all = session.candidates().to_vec();
    let formats = all
        .iter()
        .skip(1)
        .take_while(|c| c.kind == CandidateKind::Plain)
        .count();
    assert!(formats >= 5, "日期格式紧跟首选：{:?}", texts(&mut session));
    assert!(
        all[1 + formats..]
            .iter()
            .all(|c| c.kind != CandidateKind::Plain),
        "日期格式是连着的"
    );
}

#[test]
fn shuangpin_display_and_date_key() {
    // key_harness：「双拼编辑区显示全拼」；双拼时日期时间看首选的拼音
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(LEXICON).unwrap());
    let mut engine = Engine::new(lexicon, Arc::new(BigramLm::default()));
    engine.set_shuangpin(true);
    let engine: &'static Engine = Box::leak(Box::new(engine));
    let mut session = with(engine);
    let frame = type_keys(&mut session, "vsgo");
    assert!(frame.preedit.contains("zhong"), "{}", frame.preedit);
    assert_eq!(texts(&mut session)[0], "中国");

    // #122：设置了显示双拼键时编辑区是打的键，候选照旧
    let mut session = with(engine);
    session.configure(Config {
        shuangpin_keys: true,
        ..Config::default()
    });
    let frame = type_keys(&mut session, "vsgo");
    assert_eq!(frame.preedit, "vsgo");
    assert_eq!(texts(&mut session)[0], "中国");

    let mut session = with(engine);
    type_keys(&mut session, "riqi");
    let all = texts(&mut session);
    assert_eq!(all[0], "日期");
    assert_eq!(all[1], "2026年11月11日");

    // 双拼时小写 v 是声母键：v1+2 不算
    let mut session = with(engine);
    type_keys(&mut session, "v");
    session.append("1+2");
    assert_ne!(texts(&mut session).first().map(String::as_str), Some("3"));

    // 双拼用大写 V 进计算器（#160）：按键进编辑区、算式原样显示、= 上屏结果
    let mut session = with(engine);
    let frame = type_keys(&mut session, "V1+2*3");
    assert_eq!(frame.preedit, "V1+2*3", "算式不当双拼键换成全拼");
    assert_eq!(texts(&mut session)[..2], ["7", "1+2*3=7"]);
    let frame = type_keys(&mut session, "=");
    assert_eq!(frame.commit.as_deref(), Some("7"));
    assert!(!session.composing());
    // 大写金额也一样
    let mut session = with(engine);
    type_keys(&mut session, "V12345.67");
    assert_eq!(texts(&mut session)[0], "壹万贰仟叁佰肆拾伍元陆角柒分");
}

#[test]
fn calculator() {
    // key_harness：「V 模式计算器」（v 后面的数字和运算符是外壳 append 进来的）
    let mut session = with(engine());
    type_keys(&mut session, "v");
    session.append("1+2*3");
    let all = session.candidates().to_vec();
    assert_eq!(all[0].text, "7");
    assert_eq!(all[0].kind, CandidateKind::Plain);
    assert_eq!(all[1].text, "1+2*3=7");
    assert_eq!(session.frame(false).preedit, "v1+2*3", "算式不分音节");

    let mut session = with(engine());
    type_keys(&mut session, "v");
    session.append("(1.5-0.5)/4");
    assert_eq!(texts(&mut session)[0], "0.25");

    // 只打一个数：大写金额、中文数字、数字本身（#92）
    let mut session = with(engine());
    type_keys(&mut session, "v");
    session.append("12345.67");
    assert_eq!(
        texts(&mut session),
        [
            "壹万贰仟叁佰肆拾伍元陆角柒分",
            "一万二千三百四十五点六七",
            "12345.67"
        ]
    );

    // 全拼时只认小写 v
    let mut session = with(engine());
    type_keys(&mut session, "V");
    session.append("1+2");
    assert_ne!(texts(&mut session).first().map(String::as_str), Some("3"));

    // 关掉计算器：v1+2 不算
    let mut session = with(engine());
    session.configure(Config {
        calculator: false,
        ..Config::default()
    });
    type_keys(&mut session, "v");
    session.append("1+2");
    assert_ne!(texts(&mut session).first().map(String::as_str), Some("3"));
}

#[test]
fn cache_follows_edits() {
    // 改了编辑区、上文、设置，候选跟着变；没改就不重算
    let mut session = with(engine());
    type_keys(&mut session, "ba");
    let first = texts(&mut session);
    assert_eq!(texts(&mut session), first);
    type_keys(&mut session, "<bs>");
    type_keys(&mut session, "<esc>shi");
    assert_eq!(texts(&mut session)[0], "是");
    session.set_context("我");
    assert_eq!(texts(&mut session)[0], "是");
    type_keys(&mut session, "<esc>");
    assert!(session.candidates().is_empty());
}
