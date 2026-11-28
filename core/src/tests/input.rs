//! 拼音解析与基础输入。
use super::*;

#[test]
fn old_selection_still_ranks_above_unused_word() {
    // 以前的历史特征按 ln(1+秒) 扣分：一天前选过的词反而排在没选过的后面
    let e = engine("shi\t是\t900\nshi\t事\t800\n");
    e.record_selection_at("事", 0);
    e.record_selection_at("事", 1);
    let day = 24 * 3600 * 1000;
    let after = e.input_at("shi", day);
    assert_eq!(
        after[0].text,
        "事",
        "{:?}",
        after.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
    // 半年不用就退回去
    let later = e.input_at("shi", 180 * day);
    assert_eq!(later[0].text, "是");
}

#[test]
fn partial_tail_yields_completions() {
    let e = default_engine();
    let c = e.input("nih");
    assert!(
        c.iter().any(|x| x.text == "你好"),
        "nih 应当补全出 你好: {:?}",
        c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>()
    );
    let c = e.input("zhongg");
    assert!(
        c.iter().any(|x| x.text == "中国"),
        "zhongg 应当补全出 中国: {:?}",
        c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn incomplete_last_syllable_prefers_real_word() {
    let e = default_engine();
    let c = e.input("niha");
    assert_eq!(c[0].text, "你好", "niha 应当优先给出 你好");
}

#[test]
fn custom_codes_and_newlines() {
    let e = default_engine();
    e.lexicon().set_custom_tsv(
        "mail\tme@example.com\nwdyx\t我的邮箱\ndi zhi\t四川省\\n宜宾市\nbad1\t不收\n",
    );
    let c = e.input("mail");
    assert_eq!(c[0].text, "me@example.com");
    assert_eq!(c[0].consumed, 4);
    assert_eq!(e.input("wdyx")[0].text, "我的邮箱");
    assert_eq!(
        e.input("dizhi")[0].text,
        "四川省\n宜宾市",
        "\\n 换成真的换行"
    );
    assert!(
        !e.input("mai").iter().any(|c| c.text == "me@example.com"),
        "要整个编码对上"
    );
    assert_eq!(unescape_custom("a\\tb\\\\c\\x"), "a\tb\\c\\x");
}

#[test]
fn completion_does_not_extend_unknown_head() {
    // shesh：只能是 she + sh*，不能把 shesh 整体当成能解析的头
    let e = engine("she shi\t设施\t500\nshe\t设\t100\n");
    assert!(e.input("shesh").iter().any(|c| c.text == "设施"));
}

#[test]
fn completion_extends_the_last_syllable() {
    // 「shesh」尾巴是 sh，应当能补成 设施(she shi)
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(
            "she shi\t设施\t9000\nshe sha\t射杀\t500\nshe shou\t射手\t400\n",
        )
        .unwrap(),
    );
    let e = Engine::new(lexicon, Arc::new(BigramLm::default()));
    let list = e.input("shesh");
    assert!(
        list.iter().take(3).any(|c| c.text == "设施"),
        "应当补出 设施: {:?}",
        list.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
}

#[test]
fn input_xian() {
    let e = default_engine();
    let cands = e.input("xian");
    let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
    assert!(texts.contains(&"先"), "{:?}", texts);
    assert!(texts.contains(&"西安"), "{:?}", texts);
}

#[test]
fn apostrophe_does_not_yield_xian_word() {
    let e = default_engine();
    let cands = e.input("xi'an");
    let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
    assert!(texts.contains(&"西安"), "{:?}", texts);
    // 强制切分后不应再出现单音节 xian 对应的 先。
    assert!(!texts.contains(&"先"), "{:?}", texts);
}

#[test]
fn jianpin_fallback() {
    let e = default_engine();
    let cands = e.input("zg");
    assert!(cands.iter().any(|c| c.text == "中国"), "{:?}", cands);
}

#[test]
fn single_initial_pins_the_common_char_first() {
    // #156：只打 l，词频更高的「来」「里」不能排在「了」前面；选过别的字也不换掉第一个
    let e = engine(concat!(
        "lai\t来\t14837007\n",
        "li\t里\t7329328\n",
        "le\t了\t3481243\n",
        "liao\t了\t999\n",
        "ba\t把\t3798288\n",
        "bu\t不\t2985809\n",
        "fa\t发\t3000000\n",
        "fang\t放\t4000000\n",
    ));
    let texts = |raw: &str| e.input(raw).into_iter().map(|c| c.text).collect::<Vec<_>>();
    assert_eq!(texts("l")[0], "了", "{:?}", texts("l"));
    assert_eq!(texts("b")[0], "不", "{:?}", texts("b"));
    // 没列的声母照词频
    assert_eq!(texts("f")[0], "放", "{:?}", texts("f"));
    // 打全的拼音不受影响
    assert_eq!(texts("lai")[0], "来");
    for _ in 0..5 {
        e.record_selection("来");
    }
    let l = texts("l");
    assert_eq!(&l[..2], ["了", "来"], "选过的字排第二，第一个不变：{l:?}");
}

#[test]
fn single_letters_and_dead_syllables_still_give_candidates() {
    // 以前这几种输入一个候选都没有：
    // - 单独的 r 被当成儿化标记，整个输入报错
    // - hm 能切成叹词音节，可词库里没有 hm 的词，也不去试简拼
    // - qv、xv、yv 是扩展音节，词库却记在 qu、xu、yu 下面
    let e = engine(concat!(
        "ren\t人\t50000\n",
        "ri\t日\t30000\n",
        "hou mian\t后面\t20000\n",
        "hao ma\t好吗\t10000\n",
        "qu\t去\t80000\n",
        "xu yao\t需要\t40000\n",
        "yu\t与\t30000\n",
        "ju zi\t句子\t9000\n",
        "quan bu\t全部\t9000\n",
        "hua\t花\t9000\n",
        "lv\t绿\t9000\n",
        "lu\t路\t9000\n",
    ));
    let has = |raw: &str, want: &str| {
        let c = e.input(raw);
        assert!(
            c.iter().any(|c| c.text == want),
            "{raw} 应有 {want}：{:?}",
            c.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
    };
    has("r", "人");
    has("r", "日");
    has("hm", "后面");
    has("hm", "好吗");
    has("qv", "去");
    has("xvyao", "需要");
    has("yv", "与");
    has("jvzi", "句子");
    has("qvanbu", "全部");
    // 儿化照旧
    has("hua'r", "花");
    // lv 还是绿，不变成路
    assert_eq!(e.input("lv").first().map(|c| c.text.as_str()), Some("绿"));
}

#[test]
fn mixed_full_and_initial_only_syllables() {
    // 有的音节只打声母（省略韵母）：k yi -> 可以，yong z wen -> 用中文
    let e = engine(concat!(
        "ke yi\t可以\t50000\n",
        "ke\t可\t20000\n",
        "kui\t亏\t3000\n",
        "yi\t一\t100000\n",
        "yong\t用\t30000\n",
        "zhong wen\t中文\t20000\n",
        "zheng wen\t正文\t2000\n",
        "zen\t怎\t5000\n",
        "wen\t文\t10000\n",
    ));
    assert_eq!(e.input("kyi")[0].text, "可以");
    assert_eq!(e.input("yongzwen")[0].text, "用中文");
    // 打全了照旧
    assert_eq!(e.input("keyi")[0].text, "可以");
    assert_eq!(e.input("yongzhongwen")[0].text, "用中文");
}

#[test]
fn stretched_mixed_parse_does_not_bury_raw_input() {
    // 没有英文词表时，Hello 被混简拼凑成「合理了哦」，原样输入不能垫底
    let e = engine(concat!(
        "he li\t合理\t50000\n",
        "le o\t了哦\t30000\n",
        "ke yi\t可以\t50000\n",
    ));
    assert_eq!(e.input("Hello")[0].text, "Hello");
    // 省略在词里面的照旧
    assert_eq!(e.input("kyi")[0].text, "可以");
}

#[test]
fn erhua() {
    let e = default_engine();
    let cands = e.input("huar");
    assert!(cands.iter().any(|c| c.text == "花儿"), "{:?}", cands);
}

#[test]
fn empty_input() {
    let e = default_engine();
    assert!(e.input("").is_empty());
    assert!(e.input("   ").is_empty());
}

#[test]
fn unknown_input_falls_back_to_pinyin() {
    let e = engine("");
    let cands = e.input("xian");
    assert!(!cands.is_empty());
    assert_eq!(cands[0].text, "xian");
}
