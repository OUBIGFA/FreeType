//! 模糊音与双拼。
use super::*;

#[test]
fn fuzzy_matches_zh_z() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("zhi dao\t知道\t5000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    // 纠错会把 zidao 当成漏按 h，这里只测模糊音开关；关着时的兜底另测
    e.set_typo_correction(false);
    e.set_fuzzy_fallback(false);
    assert!(
        !e.input("zidao").iter().any(|c| c.text == "知道"),
        "默认关闭模糊音时不应命中"
    );
    e.set_fuzzy(true);
    assert!(
        e.input("zidao").iter().any(|c| c.text == "知道"),
        "开启后 zi dao 应当匹配 zhi dao"
    );
}

#[test]
fn fuzzy_matches_nasal() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("shang dian\t商店\t5000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    e.set_fuzzy(true);
    assert!(
        e.input("sangdian").iter().any(|c| c.text == "商店"),
        "sang dian 应当匹配 shang dian"
    );
}

#[test]
fn fuzzy_groups_filter() {
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(
            "nan ren\t男人\t5000\nfu jian\t福建\t5000\nzhi dao\t知道\t5000\n",
        )
        .unwrap(),
    );
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    e.set_typo_correction(false);
    e.set_fuzzy(true);
    let has = |e: &Engine, input: &str, word: &str| e.input(input).iter().any(|c| c.text == word);
    // 默认只开平翘舌和前后鼻音
    assert!(has(&e, "zidao", "知道"));
    assert!(!has(&e, "lanren", "男人"), "n/l 默认不开");
    assert!(!has(&e, "hujian", "福建"), "f/h 默认不开");
    e.set_fuzzy_groups(crate::syllable::fuzzy::NL | crate::syllable::fuzzy::FH);
    assert!(has(&e, "lanren", "男人"));
    assert!(has(&e, "hujian", "福建"));
    assert!(!has(&e, "zidao", "知道"), "关掉平翘舌后 zi 不再等于 zhi");
    // n 和 r 都归到 l，但 n/r 本身不是一组
    e.set_fuzzy_groups(crate::syllable::fuzzy::ALL);
    assert!(!has(&e, "ranren", "男人"), "n 和 r 不该互相模糊");
    // 几个分组一起用：lan（n/l）+ leng（r/l、en/eng）
    assert!(has(&e, "lanleng", "男人"));
}

#[test]
fn fuzzy_new_groups() {
    use crate::syllable::fuzzy;
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(
            "dong xi\t东西\t5000\nguang zhou\t广州\t5000\nxiang fa\t想法\t5000\n",
        )
        .unwrap(),
    );
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    e.set_typo_correction(false);
    e.set_fuzzy(true);
    let has = |e: &Engine, input: &str, word: &str| e.input(input).iter().any(|c| c.text == word);
    // 默认：ian/iang、uan/uang 开，un/ong 不开
    assert!(has(&e, "guanzhou", "广州"));
    assert!(has(&e, "xianfa", "想法"));
    assert!(!has(&e, "dunxi", "东西"), "un/ong 默认不开");
    e.set_fuzzy_groups(fuzzy::UN);
    assert!(has(&e, "dunxi", "东西"));
    assert!(
        !has(&e, "guanzhou", "广州"),
        "关掉 uan/uang 后 guan 不再等于 guang"
    );
    // an/ang 和 ian/iang 分开：只开 an/ang 时 xian 不等于 xiang
    e.set_fuzzy_groups(fuzzy::AN);
    assert!(!has(&e, "xianfa", "想法"));
}

#[test]
fn fuzzy_pair_groups() {
    use crate::syllable::fuzzy;
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(
            "huang se\t黄色\t5000\njun\t军\t5000\nzhong\t中\t5000\ndan\t单\t5000\nyun\t云\t5000\n",
        )
        .unwrap(),
    );
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    e.set_typo_correction(false);
    e.set_fuzzy(true);
    e.set_fuzzy_fallback(false);
    let has = |e: &Engine, input: &str, word: &str| e.input(input).iter().any(|c| c.text == word);
    // 只开另外十二组时都对不上（yun 是 ün：yun = yong 算 un = iong，不算 un = ong）
    let new_groups = fuzzy::AN_AI | fuzzy::HUANG_WANG | fuzzy::UN_IONG | fuzzy::ENG_ONG;
    e.set_fuzzy_groups(fuzzy::ALL & !new_groups);
    let pairs = [
        ("wangse", "黄色"),
        ("jiong", "军"),
        ("zheng", "中"),
        ("dai", "单"),
        ("yong", "云"),
    ];
    for (input, word) in pairs {
        assert!(!has(&e, input, word), "{input} 不该出 {word}");
    }
    e.set_fuzzy_groups(fuzzy::HUANG_WANG);
    assert!(has(&e, "wangse", "黄色"));
    e.set_fuzzy_groups(fuzzy::UN_IONG);
    assert!(has(&e, "jiong", "军"));
    assert!(
        has(&e, "yong", "云"),
        "yun 是 ün，和 yong 是 un = iong，不是 un = ong"
    );
    e.set_fuzzy_groups(fuzzy::ENG_ONG);
    assert!(has(&e, "zheng", "中"));
    e.set_fuzzy_groups(fuzzy::AN_AI);
    assert!(has(&e, "dai", "单"));
    // 和换声母的分组叠着用：zeng 换声母 z -> zh、换韵母 eng -> ong
    e.set_fuzzy_groups(fuzzy::ENG_ONG);
    assert!(!has(&e, "zeng", "中"));
    e.set_fuzzy_groups(fuzzy::ENG_ONG | fuzzy::ZH);
    assert!(has(&e, "zeng", "中"));
    // 整个音节换完还能再换韵母：wan -> wang -> huang
    e.set_fuzzy_groups(fuzzy::HUANG_WANG | fuzzy::AN);
    assert!(has(&e, "wanse", "黄色"));
}

#[test]
fn fuzzy_off_still_offers_common_word_when_exact_reading_is_junk() {
    // 模糊音关着：yingai 精确读音只能凑出「因 | 该」，按模糊音是「应该」，给出来排第一；
    // 精确读音本身有词时（dansi -> 单丝）不兜底
    let mut e = Engine::new(
        Arc::new(
            MemoryLexicon::from_tsv_str(concat!(
                "yin\t因\t50000\n",
                "gai\t该\t50000\n",
                "ying gai\t应该\t5000000\n",
                "dan si\t单丝\t5\n",
                "dan shi\t但是\t3000000\n",
            ))
            .unwrap(),
        ),
        Arc::new(BigramLm::default()),
    );
    e.set_typo_correction(false);
    assert_eq!(position(&e, "yingai", "应该"), Some(0));
    assert_eq!(position(&e, "dansi", "但是"), None);
    e.set_fuzzy_fallback(false);
    assert_eq!(position(&e, "yingai", "应该"), None);
}

#[test]
fn fuzzy_competes_with_exact_reading() {
    let mut e = fuzzy_rank_engine();
    e.set_typo_correction(false);
    e.set_fuzzy_fallback(false);
    // 关掉模糊音（也不兜底）时照旧只有精确读音
    assert_eq!(position(&e, "zong", "中"), None);
    assert_eq!(position(&e, "si", "是"), None);
    assert_eq!(position(&e, "dansi", "但是"), None);
    assert_eq!(position(&e, "culai", "出来"), None);
    e.set_fuzzy(true);
    // 单音节：模糊音的字也列出来，常用的排在前面
    assert!(
        position(&e, "zong", "中").is_some_and(|p| p < 3),
        "zong 要有 中"
    );
    assert!(
        position(&e, "si", "是").is_some_and(|p| p < 3),
        "si 要有 是"
    );
    assert!(
        position(&e, "sou", "手").is_some_and(|p| p < 3),
        "sou 要有 手"
    );
    assert_eq!(e.input("zong")[0].text, "总", "差不多常用时精确读音在前");
    // 两个音节：常用词压过精确读音凑出来的冷门词
    assert_eq!(e.input("dansi")[0].text, "但是");
    assert_eq!(e.input("culai")[0].text, "出来");
    assert_eq!(e.input("souji")[0].text, "手机");
    // 精确读音的词还在
    assert!(position(&e, "dansi", "单丝").is_some());
    assert!(position(&e, "souji", "搜集").is_some());
}

#[test]
fn fuzzy_competes_with_exact_reading_shuangpin() {
    use crate::shuangpin::Scheme;
    let mut e = fuzzy_rank_engine();
    e.set_shuangpin(true);
    e.set_shuangpin_scheme(Scheme::Xiaohe);
    // 小鹤：zs = zong，sz = sou，si = si，culd = cu lai，djsi = dan si
    assert_eq!(position(&e, "zs", "中"), None);
    assert_eq!(position(&e, "djsi", "但是"), None);
    e.set_fuzzy(true);
    assert!(position(&e, "zs", "中").is_some_and(|p| p < 3));
    assert!(position(&e, "si", "是").is_some_and(|p| p < 3));
    assert!(position(&e, "sz", "手").is_some_and(|p| p < 3));
    assert_eq!(e.input("djsi")[0].text, "但是");
    assert_eq!(e.input("culd")[0].text, "出来");
    assert_eq!(e.input("szji")[0].text, "手机");
}

#[test]
fn fuzzy_penalty_counts_each_rule() {
    // 用的模糊音越多扣得越多：dan shi 一处，dang shi 两处
    let mut e = engine("dan shi\t但是\t1000\ndang shi\t当时\t1000\n");
    e.set_typo_correction(false);
    e.set_fuzzy(true);
    let c = e.input("dansi");
    let at = |w: &str| c.iter().position(|x| x.text == w);
    assert!(at("但是").unwrap() < at("当时").unwrap());
    // 同一个音节里声母、韵母各算一处：san -> shan 一处，san -> shang 两处
    let mut e = engine("dai san\t带伞\t1000\ndai shan\t带山\t1000\ndai shang\t带上\t1000\n");
    e.set_typo_correction(false);
    e.set_fuzzy(true);
    let c = e.input("daisan");
    let at = |w: &str| c.iter().position(|x| x.text == w);
    assert!(at("带伞").unwrap() < at("带山").unwrap());
    assert!(at("带山").unwrap() < at("带上").unwrap());
}

#[test]
fn fuzzy_groups_between_pairs() {
    use crate::syllable::{fuzzy, fuzzy_groups_between, id_of};
    let g = |a: &str, b: &str| fuzzy_groups_between(id_of(a).unwrap(), id_of(b).unwrap());
    assert_eq!(g("zhang", "zan"), Some(fuzzy::ZH | fuzzy::AN));
    assert_eq!(g("niang", "lian"), Some(fuzzy::NL | fuzzy::IAN));
    assert_eq!(g("guang", "guan"), Some(fuzzy::UAN));
    assert_eq!(g("dong", "dun"), Some(fuzzy::UN));
    assert_eq!(g("chong", "cun"), Some(fuzzy::CH | fuzzy::UN));
    assert_eq!(g("jiong", "jun"), Some(fuzzy::UN_IONG));
    assert_eq!(g("yong", "yun"), Some(fuzzy::UN_IONG), "yong 按 iong 算");
    assert_eq!(g("zheng", "zhong"), Some(fuzzy::ENG_ONG));
    assert_eq!(g("keng", "kong"), Some(fuzzy::ENG_ONG));
    assert_eq!(g("zeng", "zhong"), Some(fuzzy::ZH | fuzzy::ENG_ONG));
    assert_eq!(g("dai", "dan"), Some(fuzzy::AN_AI));
    assert_eq!(g("huang", "wang"), Some(fuzzy::HUANG_WANG));
    assert_eq!(g("wan", "huang"), Some(fuzzy::HUANG_WANG | fuzzy::AN));
    assert_eq!(g("wang", "huan"), Some(fuzzy::HUANG_WANG | fuzzy::UAN));
    // 韵母只能换一步：要换两步的不算
    assert_eq!(g("dai", "dang"), None);
    assert_eq!(g("zhen", "zhong"), None);
    assert_eq!(g("dun", "deng"), None);
    assert_eq!(g("wan", "huan"), None);
    assert_eq!(g("huang", "fang"), None, "uang 和 ang 不是一组");
    assert_eq!(g("ren", "leng"), Some(fuzzy::RL | fuzzy::EN));
    assert_eq!(g("nu", "ru"), None);
    assert_eq!(g("ying", "yin"), Some(fuzzy::IN));
    assert_eq!(g("shi", "shi"), Some(0));
}

#[test]
fn shuangpin_input() {
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str("zhong guo\t中国\t8000\nni hao\t你好\t10000\n").unwrap(),
    );
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    assert!(
        !e.input("vsgo").iter().any(|c| c.text == "中国"),
        "默认全拼模式下 vsgo 不应命中"
    );
    e.set_shuangpin(true);
    assert!(
        e.input("vsgo").iter().any(|c| c.text == "中国"),
        "双拼 vsgo 应当命中 中国"
    );
    assert!(
        e.input("nihk").iter().any(|c| c.text == "你好"),
        "双拼 nihk 应当命中 你好"
    );
}

#[test]
fn shuangpin_prefix_and_trailing_key() {
    use crate::shuangpin::Scheme;
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(
            "zhong guo\t中国\t8000\nren\t人\t9000\nzhong guo ren\t中国人\t3000\n\
             ni hao\t你好\t10000\nni\t你\t9000\nhao\t好\t9000\nzhong\t中\t9000\n",
        )
        .unwrap(),
    );
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let mut e = Engine::new(lexicon, lm);
    e.set_shuangpin(true);
    // 自然码 vs go rf：句首词「中」用掉前 2 个键
    let c = e.input("vsgorf");
    assert!(
        c.iter().any(|x| x.text == "中国人" && x.consumed == 6),
        "{c:?}"
    );
    assert!(c.iter().any(|x| x.text == "中" && x.consumed == 2), "{c:?}");
    // 句首词「你好」用掉前 4 个键（隔音符也算进去）
    let c = e.input("ni'hkvs");
    assert!(
        c.iter().any(|x| x.text == "你好" && x.consumed == 5),
        "{c:?}"
    );
    // 最后单一个键当声母
    assert!(e.input("vsg").iter().any(|x| x.text == "中国"));
    assert_eq!(e.shuangpin_display("vsgo").as_deref(), Some("zhong'guo"));
    // 换成小鹤：hao 是 hc
    e.set_shuangpin_scheme(Scheme::Xiaohe);
    assert!(e.input("nihc").iter().any(|x| x.text == "你好"));
    // 学习时传进来的拼音是全拼
    assert!(e.input("nihk").iter().all(|x| x.text != "你好"));
}
