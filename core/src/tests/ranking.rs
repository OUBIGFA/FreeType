//! 候选排序：生僻字、分词读音、整句、英文、纠错与单音节。
use super::*;

#[test]
fn rare_chars_listed_last_and_never_in_sentences() {
    // 频次小于 1 的单字是大字表补进来的（tools/rime2tsv.py --extra-chars）：
    // 一个音节的字要全列出来，但只排在最后，也不拿来组句
    let mut e = engine(concat!(
        "shi\t是\t9000\nshi\t十\t8000\nshi\t时\t7000\n",
        "shi shi\t事实\t5000\n",
        "shi\t𠯜\t0.7\nshi\t䊓\t0.8\nshi\t戠\t0.9\n",
    ));
    e.set_max_candidates(1000);
    // 默认不列扩展 B 区以后的字（常见字体显示成方块），基本区、扩展 A 照常
    let texts: Vec<String> = e.input("shi").into_iter().map(|c| c.text).collect();
    assert!(!texts.iter().any(|t| t == "𠯜"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "䊓") && texts.iter().any(|t| t == "戠"),
        "{texts:?}"
    );

    e.set_rare_ext_chars(true);
    let list = e.input("shi");
    let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts.first(), Some(&"是"), "{texts:?}");
    let at = |t: &str| texts.iter().position(|x| *x == t);
    for ch in ["是", "十", "时", "戠", "䊓", "𠯜"] {
        assert!(at(ch).is_some(), "{ch} 要列出来：{texts:?}");
    }
    assert!(at("时") < at("戠"), "常用字在前：{texts:?}");
    assert!(
        at("戠") < at("䊓") && at("䊓") < at("𠯜"),
        "基本区、扩展 A、扩展 B：{texts:?}"
    );

    let list = e.input("shishi");
    let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts.first(), Some(&"事实"), "{texts:?}");
    let rare = ['𠯜', '䊓', '戠'];
    for c in &list {
        if c.text.chars().count() > 1 {
            assert!(
                !c.text.chars().any(|ch| rare.contains(&ch)),
                "整句不用生僻字：{texts:?}"
            );
        }
    }
    assert!(texts.contains(&"𠯜"), "第一个音节的字也要列全：{texts:?}");

    // 简拼不带生僻字
    let texts: Vec<String> = e.input("sh").into_iter().map(|c| c.text).collect();
    assert!(!texts.iter().any(|t| t == "𠯜"), "{texts:?}");
}

#[test]
fn ext_chars_hidden_by_char_not_by_freq() {
    // 通用规范汉字表里的扩展区字（𬭩）照常出；别的扩展区单字不管频次多少都藏（万象词库
    // 把它们记成普通单字）；含这些字的词不受影响
    let mut e = engine(concat!(
        "yang\t氧\t9000\nyang\t𬭩\t0.7\nyang\t𪦲\t600\n",
        "an kang\t𩽾𩾌\t50\n",
    ));
    e.set_max_candidates(100);
    let texts: Vec<String> = e.input("yang").into_iter().map(|c| c.text).collect();
    assert!(texts.iter().any(|t| t == "𬭩"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "𪦲"), "{texts:?}");
    let texts: Vec<String> = e.input("ankang").into_iter().map(|c| c.text).collect();
    assert_eq!(texts.first().map(String::as_str), Some("𩽾𩾌"), "{texts:?}");
    e.set_rare_ext_chars(true);
    let texts: Vec<String> = e.input("yang").into_iter().map(|c| c.text).collect();
    assert!(texts.iter().any(|t| t == "𪦲"), "{texts:?}");
}

#[test]
fn lookup_common_drops_rare_chars() {
    let lex =
        MemoryLexicon::from_tsv_str("shi\t是\t9000\nshi\t𬤊\t0.7\nshi shi\t事实\t0.5\n").unwrap();
    let shi = crate::syllable::id_of("shi").unwrap();
    let q = [SyllableQuery::Full {
        id: shi,
        erhua: false,
    }];
    let all: Vec<String> = lex.lookup(&q).into_iter().map(|e| e.word).collect();
    let common: Vec<String> = lex.lookup_common(&q).into_iter().map(|e| e.word).collect();
    assert_eq!(all, ["是", "𬤊"]);
    assert_eq!(common, ["是"]);
    // 只看单字：低频的词照样能组句
    let two: Vec<String> = lex
        .lookup_common(&[q[0], q[0]])
        .into_iter()
        .map(|e| e.word)
        .collect();
    assert_eq!(two, ["事实"]);
}

#[test]
fn other_split_heads_come_before_chars() {
    // houxuanci 也能切成 hou xu | an ci：「后续」排在单字前面
    let mut e = engine(concat!(
        "hou xuan ci\t候选词\t3000\nhou xuan\t候选\t4000\n",
        "hou xu\t后续\t3000\nan\t按\t5000\n",
        "hou\t后\t9000\nhou\t侯\t300\nhou\t㕈\t0.8\n",
        "xuan\t选\t8000\nci\t此\t90000\nci\t词\t70000\n",
    ));
    e.set_max_candidates(1000);
    let list = e.input("houxuanci");
    let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
    let at = |t: &str| texts.iter().position(|x| *x == t);
    assert!(at("后续").is_some(), "{texts:?}");
    assert!(at("后续") < at("㕈"), "{texts:?}");
    assert!(at("候选") < at("后续"), "正常切分的词在前：{texts:?}");
}

#[test]
fn split_reading_gets_first_page_slot() {
    let e = split_reading_engine();
    let list = e.input("yuan");
    let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(list[0].pinyin, "yuan", "整音节读法照旧排第一：{texts:?}");
    assert!(
        texts[1..SPLIT_READING_FIRST_PAGE].contains(&"预案"),
        "拆开读法的常见词留在第一页：{texts:?}"
    );
    assert!(
        list.windows(2).all(|w| w[0].score > w[1].score),
        "分数单调递减"
    );
    // 分数差超过 lift_split_word 的 3.5 但词频仍在 1/100 内时，由 promote_split_reading 提到第 2 位
    let e2 = engine(concat!(
        "yu an\t预案\t30000\n",
        "yuan\t原\t1400000\nyuan\t愿\t1350000\nyuan\t远\t1300000\n",
        "yuan\t元\t1250000\nyuan\t袁\t1200000\nyuan\t员\t1150000\n",
        "yu\t于\t900000\nan\t安\t500000\n",
    ));
    let list2 = e2.input("yuan");
    let texts2: Vec<&str> = list2.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts2[1], "预案", "{texts2:?}");
    assert!(
        list2.windows(2).all(|w| w[0].score > w[1].score),
        "分数单调递减"
    );
    // 打了隔音符就只有拆开的读法
    assert_eq!(e.input("yu'an")[0].text, "预案");
}

#[test]
fn rare_split_reading_stays_behind() {
    // 激昂 的词频不到 将 的 1/100，是罕见重码，不提
    let e = split_reading_engine();
    let texts: Vec<String> = e.input("jiang").into_iter().map(|c| c.text).collect();
    let at = texts.iter().position(|t| t == "激昂");
    assert!(
        at.map_or(true, |i| i >= SPLIT_READING_FIRST_PAGE),
        "{texts:?}"
    );
}

#[test]
fn long_input_keeps_few_sentences_then_words_and_chars() {
    let e = engine(concat!(
        "heng xiang\t横向\t5000\n",
        "hou xuan ci\t候选词\t3000\n",
        "hou xuan\t候选\t4000\n",
        "heng\t横\t3000\nheng\t恒\t2000\nheng\t衡\t1000\n",
        "ci\t此\t90000\nci\t次\t80000\nci\t词\t70000\nci\t刺\t20000\n",
        "xiang\t向\t5000\nhou\t后\t9000\nxuan\t选\t8000\n",
    ));
    let list = e.input("hengxianghouxuanci");
    let texts: Vec<&str> = list.iter().map(|c| c.text.as_str()).collect();
    let sentences = list
        .iter()
        .filter(|c| c.words.len() >= 2 && c.consumed == 18)
        .count();
    assert!(sentences <= 3, "整句最多 3 个：{texts:?}");
    // 只差最后一个字的整句不重复出现
    let tail_variants = texts
        .iter()
        .filter(|t| t.starts_with("横向候选") && t.chars().count() == 5)
        .count();
    assert_eq!(tail_variants, 1, "{texts:?}");
    assert!(texts.contains(&"横向"), "{texts:?}");
    assert!(texts.contains(&"横"), "后面要有单字：{texts:?}");
    let word = texts.iter().position(|t| *t == "横向").unwrap();
    let single = texts.iter().position(|t| *t == "横").unwrap();
    assert!(word < single, "词在单字前面：{texts:?}");
}

#[test]
fn sentence_head_is_offered_for_long_input() {
    let e = engine(concat!(
        "wo\t我\t900000\n",
        "jin tian\t今天\t100000\n",
        "wan shang\t晚上\t100000\n",
        "yao\t要\t500000\n",
        "jia ban\t加班\t20000\n",
        "jia ban\t甲板\t19000\n",
    ));
    let list = e.input("wojintianwanshangyaojiaban");
    let head = list
        .iter()
        .find(|c| c.text == "我今天晚上要")
        .unwrap_or_else(|| panic!("{:?}", list.iter().map(|c| &c.text).collect::<Vec<_>>()));
    assert_eq!(head.consumed, "wojintianwanshangyao".len());
}

#[test]
fn complete_syllable_survives_crowded_completions() {
    let mut tsv = String::from("lia\t俩\t99\nlia\t倆\t0.5\nli a\t理啊\t50\n");
    for (i, character) in ('\u{4e00}'..).take(60).enumerate() {
        let syllable = ["lian", "liang", "liao"][i % 3];
        tsv.push_str(&format!(
            "{syllable}\t{character}\t{}\n",
            100_000 - i * 1000
        ));
    }
    let lexicon = Arc::new(MemoryLexicon::from_tsv_str(&tsv).unwrap());
    let mut engine = Engine::new(lexicon, Arc::new(BigramLm::default()));
    engine.set_max_candidates(300);
    let candidates = engine.input("lia");
    assert_eq!(candidates.first().map(|c| c.text.as_str()), Some("俩"));
}

#[test]
fn duplicate_letter_typo() {
    // 音节中间同一个键多按一次（nihhao）也能纠成 ni hao
    let e = engine("ni hao\t你好\t10000\n");
    assert!(e.input("nihhao").iter().any(|c| c.text == "你好"));
}

#[test]
fn english_input_yields_raw_candidate() {
    let e = default_engine();
    let c = e.input("hello");
    assert_eq!(c.first().map(|x| x.text.as_str()), Some("hello"));
    assert!(c.iter().any(|x| x.text == "hello"));

    // 原始大小写要保留
    let c = e.input("Hello");
    assert_eq!(c[0].text, "Hello");

    // 只有未登录兜底时，原始串排最前
    let c = e.input("ok");
    assert_eq!(
        c[0].text,
        "ok",
        "{:?}",
        c.iter().map(|x| &x.text).collect::<Vec<_>>()
    );
}

#[test]
fn known_english_word_beats_typo_corrected_chinese() {
    let e = engine(concat!(
        "he\t和\t9000\n",
        "lo\t咯\t500\n",
        "she\t设\t5000\n",
        "shen\t神\t9000\n",
        "ni hao\t你好\t100000\n",
    ));
    assert_eq!(
        e.load_english_str("hello\thello\nshe\tshe\nGitHub\tGitHub\nJanuary\tJan\n"),
        3
    );
    let texts =
        |input: &str| -> Vec<String> { e.input(input).into_iter().map(|c| c.text).collect() };

    // 中文只能靠纠错凑出来时，已知英文词排第一
    assert_eq!(texts("hello")[0], "hello");
    // 全小写输入用词表里的写法，自己打了大写就照原样
    assert_eq!(texts("github")[0], "GitHub");
    assert_eq!(texts("Github")[0], "Github");
    // 本身就是完整拼音的英文词，中文在前，英文排第 6（候选不足时排最后）
    let she = texts("she");
    let pos = she.iter().position(|t| t == "she").expect("应当有 she");
    assert!(pos == 5 || (pos >= 1 && pos == she.len() - 1), "{she:?}");
    // 缩写编码（Jan -> January）不当作英文词
    assert!(!texts("jan").iter().any(|t| t == "January"));
}

#[test]
fn english_word_beats_chinese_sentence_made_of_abbreviations() {
    // ok -> 哦 | 可（o + k）是两个词硬凑的；bug -> 不过（bu + g）是一个词还没打完
    let e = engine("o\t哦\t900000\nke\t可\t900000\nbu guo\t不过\t500000\nbu\t不\t1000000\n");
    e.load_english_str("ok\nbug\n");
    let texts =
        |input: &str| -> Vec<String> { e.input(input).into_iter().map(|c| c.text).collect() };
    assert_eq!(
        texts("ok").first().map(String::as_str),
        Some("ok"),
        "{:?}",
        texts("ok")
    );
    let bug = texts("bug");
    assert_eq!(bug.first().map(String::as_str), Some("不过"), "{bug:?}");
    assert_eq!(bug.get(1).map(String::as_str), Some("bug"), "{bug:?}");
}

#[test]
fn junk_sentence_does_not_follow_a_clear_word() {
    // 你好比「泥 | 好」高得多：硬凑的整句不占第二位；英文词后面不出两处纠错硬解的中文
    let e = engine("ni hao\t你好\t1000000\nni\t泥\t10000\nhao\t好\t100000\n");
    let texts: Vec<String> = e.input("nihao").into_iter().map(|c| c.text).collect();
    assert_eq!(texts.first().map(String::as_str), Some("你好"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "泥好"), "{texts:?}");
}

#[test]
fn swapped_keys_beat_a_wrong_key() {
    // meiyuo：两键按反是「没有」，按错一个键（y 当 g）是「美国」；美国词频高一些，
    // 但打反比按错常见，没有排第一
    let e = engine("mei you\t没有\t500000\nmei guo\t美国\t900000\n");
    let texts: Vec<String> = e.input("meiyuo").into_iter().map(|c| c.text).collect();
    assert_eq!(texts.first().map(String::as_str), Some("没有"), "{texts:?}");
}

#[test]
fn one_typo_word_follows_a_sentence_of_single_characters() {
    // tianqu：第一名「天 | 去」是两个单字硬凑的，纠一处错（u 当 i）就是「天气」，排第二
    let e = engine("tian\t天\t100000\nqu\t去\t100000\ntian qi\t天气\t1000000\n");
    let texts: Vec<String> = e.input("tianqu").into_iter().map(|c| c.text).collect();
    assert_eq!(texts.first().map(String::as_str), Some("天去"), "{texts:?}");
    assert_eq!(texts.get(1).map(String::as_str), Some("天气"), "{texts:?}");
}

#[test]
fn single_syllable_candidates_follow_word_frequency() {
    // 打 he 时「和」比「何」常用，句首 LM 分不该把「何」抬上来。
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str("he\t和\t100000\nhe\t何\t10000\nhe\t合\t1000\n").unwrap(),
    );
    let e = Engine::new(lexicon, Arc::new(SentenceStartBoost));
    let texts: Vec<String> = e.input("he").into_iter().map(|c| c.text).collect();
    let at = |word: &str| texts.iter().position(|t| t == word);
    assert_eq!(texts.first().map(String::as_str), Some("和"), "{texts:?}");
    assert!(at("何") < at("合"), "{texts:?}");
}

#[test]
fn single_syllable_ranking_ignores_lm_weight() {
    // 句首分加的时候乘了 lm_weight，减的时候也要乘同一个数：不管权重多少，
    // 单音节的候选和分数都只看字频（#200 按长度调权重时减的是没乘系数的，shi 的「十」掉到第 9）
    let ranked = |lm_weight: f64| {
        let lexicon: Arc<dyn Lexicon> = Arc::new(
            MemoryLexicon::from_tsv_str("he\t和\t100000\nhe\t何\t10000\nhe\t合\t1000\n").unwrap(),
        );
        let mut config = EngineConfig::default();
        config.beam.lm_weight = lm_weight;
        Engine::new(lexicon, Arc::new(SentenceStartBoost))
            .with_config(config)
            .input("he")
            .into_iter()
            .map(|c| (c.text, c.score))
            .collect::<Vec<_>>()
    };
    let reference = ranked(1.0);
    for lm_weight in [0.5, 0.7, 1.5] {
        let got = ranked(lm_weight);
        let texts = |r: &[(String, f64)]| r.iter().map(|c| c.0.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&got), texts(&reference), "lm_weight {lm_weight}");
        for (a, b) in got.iter().zip(&reference) {
            assert!(
                (a.1 - b.1).abs() < 1e-9,
                "lm_weight {lm_weight}: {a:?} vs {b:?}"
            );
        }
    }
}

#[test]
fn complete_syllable_is_not_made_longer() {
    // 「想」字频高得多，但 xian 已经打全了：不把它接长成 xiang 的字（没开模糊音时那是读音不符的候选）；
    // 切成 xi'an 的整词「西安」分数差得不多，抬到第五个
    let mut tsv = String::from(
        "xiang\t想\t50000000\nxi an\t西安\t400000\nxiao\t笑\t100\nzhong\t中\t900000\n",
    );
    for (i, c) in "先现显线险仙咸鲜".chars().enumerate() {
        tsv.push_str(&format!("xian\t{c}\t{}\n", 1_000_000 - i * 100_000));
    }
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(&tsv).unwrap());
    let e = Engine::new(lexicon, Arc::new(BigramLm::default()));
    let texts =
        |input: &str| -> Vec<String> { e.input(input).into_iter().map(|c| c.text).collect() };
    let xian = texts("xian");
    let at = |w: &str| xian.iter().position(|t| t == w);
    assert_eq!(xian.first().map(String::as_str), Some("先"), "{xian:?}");
    assert_eq!(at("想"), None, "打全的音节被接长成了别的读音：{xian:?}");
    assert_eq!(at("西安"), Some(4), "{xian:?}");
    // xia 也是完整音节
    let xia = texts("xia");
    assert!(!xia.iter().any(|t| t == "想"), "{xia:?}");
    // 没打全的音节照样补全
    let zhon = texts("zhon");
    assert!(zhon.iter().any(|t| t == "中"), "{zhon:?}");
}

#[test]
fn exact_word_is_not_overridden_by_completion_or_typo() {
    // 重点的词频比种地高 20 倍，补全（zhongdi -> zhong dian）的分数会比精确词高
    let e = engine(concat!(
        "zhong dian\t重点\t500988\n",
        "zhong di\t种地\t25085\n",
        "zhong\t中\t900000\n",
        "di\t地\t800000\n",
        "ka\t卡\t402256\n",
        "kan\t看\t1500000\n",
        "zhong guo\t中国\t500000\n",
        "gu\t骨\t90000\n",
        "shi\t是\t9000000\n",
        "dan\t但\t900000\n",
        "shi dan\t实弹\t3175\n",
        "shi dang\t适当\t300000\n",
        "ta\t他\t3000000\n",
        "ta me\t他么\t333\n",
        "ta men\t他们\t3000000\n",
    ));
    let texts =
        |input: &str| -> Vec<String> { e.input(input).into_iter().map(|c| c.text).collect() };
    let zhongdi = texts("zhongdi");
    assert_eq!(zhongdi[0], "种地", "{zhongdi:?}");
    assert!(
        zhongdi.iter().any(|t| t == "重点"),
        "补全仍然在候选里：{zhongdi:?}"
    );
    assert_eq!(texts("ka")[0], "卡");
    // 整个输入是一个词时，单字凑出来的（是 | 但）也不能压过它
    assert_eq!(texts("shidan")[0], "实弹");
    // 精确结果只是单字拼凑（中 | 骨）时用户多半没打完，补全照常排第一
    assert_eq!(texts("zhonggu")[0], "中国");
    // 生僻词（词频低于下限）不受保护
    assert_eq!(texts("tame")[0], "他们");
}

#[test]
fn prefix_candidates_do_not_split_a_syllable() {
    let e = engine(concat!(
        "dan\t但\t900000\n",
        "da\t大\t900000\n",
        "ni hao\t你好\t100000\n",
        "ni\t你\t900000\n",
        "zhong guo\t中国\t500000\n",
    ));
    let dan: Vec<String> = e.input("dan").into_iter().map(|c| c.text).collect();
    assert!(!dan.iter().any(|t| t == "大"), "单音节不出句首词：{dan:?}");
    let long = e.input("nihaozhongguo");
    assert_eq!(long[0].text, "你好中国");
    let head = long
        .iter()
        .position(|c| c.text == "你好")
        .expect("应当有句首词 你好");
    assert!(
        head <= 1,
        "{:?}",
        long.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
    assert!(long.len() <= e.config.max_candidates.max(1));
}

#[test]
fn later_pages_list_every_homophone_and_head_word() {
    let mut tsv = String::new();
    for i in 0..40 {
        tsv.push_str(&format!("shi\t字{i}\t{}\n", 1000 - i));
    }
    tsv.push_str("shi jie\t世界\t9000\nshi jie\t视界\t50\njie\t界\t500\n");
    let mut e = engine(&tsv);
    e.set_max_candidates(300);
    let one: Vec<String> = e.input("shi").into_iter().map(|c| c.text).collect();
    for i in 0..40 {
        assert!(one.contains(&format!("字{i}")), "缺 字{i}: {one:?}");
    }
    let two = e.input("shijie");
    assert_eq!(two[0].text, "世界");
    let texts: Vec<&str> = two.iter().map(|c| c.text.as_str()).collect();
    assert!(texts.contains(&"视界"), "{texts:?}");
    // 翻到后面能选第一个音节的任意一个字，只用掉 shi
    let head = two.iter().find(|c| c.text == "字39").expect("单字在后面");
    assert_eq!(head.consumed, 3);

    let mut off = engine(&tsv);
    off.set_max_candidates(300);
    off.config.more_candidates = false;
    assert!(off.input("shi").len() < 40);
}

#[test]
fn raw_candidate_survives_a_full_list() {
    let mut e = engine(concat!(
        "he\t和\t9000\nhe\t何\t8000\nhe\t合\t7000\n",
        "lo\t咯\t500\nluo\t落\t400\n",
    ));
    e.set_max_candidates(3);
    // hrlo 不是拼音（r 是 e 的邻键），中文全靠纠错，列表还是满的
    let c = e.input("hrlo");
    assert!(
        c.iter().any(|x| x.text == "hrlo"),
        "原始串不该被截掉: {:?}",
        c.iter().map(|x| &x.text).collect::<Vec<_>>()
    );
}

#[test]
fn full_pinyin_does_not_add_raw_candidate() {
    let e = default_engine();
    assert!(
        !e.input("nihao").iter().any(|c| c.text == "nihao"),
        "完整覆盖时不应插入原始拼音串"
    );
}

#[test]
fn traditional_candidates() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("zhong guo\t中国\t8000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    e.install_converter(Converter::from_tsv_str("国\t國\n").unwrap());
    assert_eq!(e.input("zhongguo")[0].text, "中国");
    e.set_traditional(true);
    assert_eq!(e.input("zhongguo")[0].text, "中國");
    // 分词不转，还是词库里的写法（学词、记搭配用）
    assert_eq!(e.input("zhongguo")[0].words[0].text, "中国");
}

#[test]
fn traditional_phrase_candidates() {
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(
            "tou fa\t头发\t8000\ngan jing\t干净\t8000\nruan jian\t软件\t8000\n",
        )
        .unwrap(),
    );
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    e.install_converter(
        Converter::from_tsv_str(
            "发\t發\n干\t幹\n头\t頭\n净\t淨\n软\t軟\n头发\t頭髮\n干净\t乾淨\n软件\t軟體\n",
        )
        .unwrap(),
    );
    e.set_traditional(true);
    assert_eq!(e.input("toufa")[0].text, "頭髮");
    assert_eq!(e.input("ganjing")[0].text, "乾淨");
    // 词组可以换成台湾用语，字数也可以不同
    assert_eq!(e.input("ruanjian")[0].text, "軟體");
    assert_eq!(e.input("ruanjian")[0].words[0].text, "软件");
}

#[test]
fn ltr_valid_reflects_ranker() {
    struct NullLtr;
    impl crate::rank::LtrModel for NullLtr {
        fn score(&self, _feature: &Feature) -> f32 {
            0.0
        }
    }
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("xian\t先\t1000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let ranker = Arc::new(LinearRanker::default().with_ltr(Arc::new(NullLtr), 1.0));
    let e = Engine::new(lexicon, lm).with_ranker(ranker);
    let cands = e.input("xian");
    assert!(!cands.is_empty());
    assert_eq!(cands[0].features.ltr_valid, 1.0);
}
