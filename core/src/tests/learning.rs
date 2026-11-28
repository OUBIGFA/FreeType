//! 选词学习、个人词与上下文：引擎层的学习与排序回归。
use super::*;

#[test]
fn history_boosts_selection() {
    let e = engine("shi\t是\t900\nshi\t事\t800\n");
    let first = e.input_at("shi", 1_000_000);
    assert_eq!(first[0].text, "是", "默认按频次");

    for i in 0..5 {
        e.record_selection_at("事", 1_000_000 + i);
    }
    assert_eq!(e.history_count("事"), 5);

    let after = e.input_at("shi", 1_000_100);
    assert_eq!(
        after[0].text,
        "事",
        "选过多次的词应当排到前面: {:?}",
        after.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn custom_words_rank_first_and_reload_replaces() {
    let e = engine(concat!(
        "ni hao\t你好\t332885\n",
        "zhang san\t张三\t20000\n",
        "zhang\t张\t500000\n",
        "san\t三\t500000\n",
    ));
    e.lexicon()
        .set_custom_tsv("# 注释\nni hao\t妮好\nwo de you xiang\t我的邮箱\nbad xx\t坏\n");
    assert_eq!(e.lexicon().custom_words().len(), 2, "不合法的拼音跳过");
    assert_eq!(e.input_at("nihao", 1)[0].text, "妮好");
    assert_eq!(e.input_at("wodeyouxiang", 1)[0].text, "我的邮箱");
    // 自定义词不进用户词库，保存时不会写出去
    assert!(e.user_words().is_empty());

    e.lexicon().set_custom_tsv("zhang san\t章三\n");
    assert_eq!(e.input_at("nihao", 1)[0].text, "你好", "旧的自定义词去掉了");
    assert_eq!(e.input_at("zhangsan", 1)[0].text, "章三");
    e.lexicon().set_custom_tsv("");
    assert_eq!(e.input_at("zhangsan", 1)[0].text, "张三");
}

#[test]
fn learned_pair_fixes_homophone_inside_sentence() {
    let e = engine(concat!(
        "wo men\t我们\t900000\n",
        "zai\t在\t900000\n",
        "zai\t再\t300000\n",
        "tao lun\t讨论\t50000\n",
    ));
    let first = e.input_at("womenzaitaolun", 10);
    assert_eq!(first[0].text, "我们在讨论", "没学之前按词频");
    // 用户一段一段选出「我们再讨论」：我们 -> 再 -> 讨论
    e.set_context("");
    e.record_selection_at("我们", 10);
    e.set_context("我们");
    e.input_at("zaitaolun", 10);
    e.record_selection_at("再", 11);
    e.set_context("我们再");
    e.input_at("taolun", 11);
    e.record_selection_at("讨论", 12);
    e.set_context("");

    let after = e.input_at("womenzaitaolun", 20);
    assert_eq!(
        after[0].text,
        "我们再讨论",
        "{:?}",
        after.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
}

#[test]
fn previous_commit_is_context_for_next_word() {
    let e = engine("zai\t在\t900000\nzai\t再\t300000\nhui tou\t回头\t1000\nhui\t回\t1000\n");
    // 上一次上屏的是「回头」，这次打 zai：学过「回头 -> 再」以后排第一
    e.input_at("huitou", 1);
    e.record_selection_at("回头", 1);
    e.set_context("回头");
    e.input_at("zai", 1);
    e.record_selection_at("再", 2);

    e.set_context("回头");
    e.input_at("huitou", 3);
    e.record_selection_at("回头", 3);
    let with_context = e.input_at("zai", 4);
    assert_eq!(with_context[0].text, "再");

    // 光标前不是「回头」时不受影响（「再」本身选过一次，热度还在，所以换个更远的时间）
    e.set_context("别的");
    let without = e.input_at("zai", 4 + 365 * 24 * 3600 * 1000);
    assert_eq!(without[0].text, "在");
}

#[test]
fn history_survives_save_and_load() {
    let tsv = "shi\t是\t900\nshi\t事\t800\n";
    let e = engine(tsv);
    e.record_selection_at("事", now_ms());
    let dir = std::env::temp_dir().join(format!("wowtype-hist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.tsv");
    e.save_history(&path).unwrap();

    let fresh = engine(tsv);
    assert!(fresh.load_history(&path).unwrap() >= 1);
    assert_eq!(fresh.input("shi")[0].text, "事");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn context_bigram_boosts_first_word() {
    let mut lm = BigramLm::new().with_backoff(0.001);
    lm.add_unigram("今天", 10.0);
    lm.add_unigram("天", 5.0);
    lm.add_unigram("田", 5.0);
    lm.add_bigram("今天", "天", 8.0);

    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("tian\t田\t200\ntian\t天\t100\n").unwrap());
    let e = Engine::new(lexicon, Arc::new(lm));

    let before = e.input_at("tian", 1);
    assert_eq!(before[0].text, "田", "没有上下文时按频次");

    e.set_context("今天");
    let after = e.input_at("tian", 1);
    assert_eq!(
        after[0].text,
        "天",
        "上下文应当把 天 提上来: {:?}",
        after.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
    );
    let tian = after.iter().find(|c| c.text == "天").unwrap();
    let tian_farm = after.iter().find(|c| c.text == "田").unwrap();
    assert!(tian.features.with_context == 1.0);
    assert!(
        tian.features.context_bigram_score > tian_farm.features.context_bigram_score,
        "{} vs {}",
        tian.features.context_bigram_score,
        tian_farm.features.context_bigram_score
    );
}

#[test]
fn user_word_is_usable() {
    let e = engine("ni hao\t你好\t10\n");
    assert!(e.add_user_word("wo wo te", "我特", 5000.0).is_ok());
    assert_eq!(e.user_words().len(), 1);
    let cands = e.input("wowote");
    assert!(
        cands.iter().any(|c| c.text == "我特"),
        "用户词应当能输入: {:?}",
        cands.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
    );
    assert!(e.remove_user_word("我特"));
    let after = e.input("wowote");
    assert!(!after.iter().any(|c| c.text == "我特"));
}

#[test]
fn forget_word_clears_learning_and_saves_empty_dict() {
    let e = engine("ni hao\t你好\t10\n");
    e.add_user_word("wo wo te", "我特", 5000.0).unwrap();
    e.record_selection("我特");
    e.set_context("我特");
    e.record_selection("你好");
    assert_eq!(e.pair_count(), 1, "我特 -> 你好");
    assert!(e.forget_word("我特"));
    assert!(e.user_words().is_empty());
    assert_eq!(e.history_count("我特"), 0);
    assert_eq!(e.pair_count(), 0, "和它有关的搭配也去掉");
    assert!(!e.forget_word("我特"), "第二次没东西可忘");

    // 用户词全删光了也要存盘，不然旧的 user.tsv 留着，重启又回来
    let dir = std::env::temp_dir().join(format!("wowtype-forget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    std::fs::write(&path, "wo wo te\t我特\t5000\n").unwrap();
    e.save_user_dict(&path).unwrap();
    let e2 = engine("ni hao\t你好\t10\n");
    assert_eq!(e2.load_user_dict(&path).unwrap(), 0);
    std::fs::remove_dir_all(&dir).ok();

    // 自定义词不忘
    e.lexicon().set_custom_tsv("ni hao\t妮好\n");
    e.record_selection("妮好");
    assert!(!e.forget_word("妮好"));
    assert_eq!(e.history_count("妮好"), 1);
}

#[test]
fn invalid_user_pinyin_rejected() {
    let e = engine("ni hao\t你好\t10\n");
    assert!(e.add_user_word("zzz", "坏词", 1.0).is_err());
    assert!(e.user_words().is_empty());
}

#[test]
fn single_syllable_keeps_learned_pair_bonus() {
    // 去掉的只是基础模型的句首分：用户在「快」后面选过「乐」，这个搭配加成要留着。
    // 两个引擎都选过一次「乐」（热度一样），只有 a 是在「快」后面选的。
    let make = || {
        let lexicon: Arc<dyn Lexicon> =
            Arc::new(MemoryLexicon::from_tsv_str("le\t了\t100000\nle\t乐\t10000\n").unwrap());
        Engine::new(lexicon, Arc::new(SentenceStartBoost))
    };
    // 搭配只认「上一次选的词，且光标前正好以它结尾」，所以先选前一个字
    let pick = |e: &Engine, prev: &str, word: &str| {
        e.set_context("");
        e.record_selection(prev);
        e.set_context(prev);
        e.record_selection(word);
    };
    let (a, b) = (make(), make());
    pick(&a, "快", "乐");
    pick(&b, "慢", "乐");
    let score = |e: &Engine| {
        e.set_context("");
        e.record_selection("快");
        e.set_context("快");
        e.input("le")
            .into_iter()
            .find(|c| c.text == "乐")
            .map(|c| c.score)
    };
    let (sa, sb) = (score(&a).unwrap(), score(&b).unwrap());
    assert!(sa > sb, "学过的搭配没起作用：{sa} vs {sb}");
}

#[test]
fn context_merged_into_one_word_counts_as_common_pair() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("yao\t要\t1000000\nyao\t腰\t30000\n").unwrap());
    let e = Engine::new(lexicon, Arc::new(MergedPairLm));
    // 上文只有「我」和「今天我」（整段不是模型里的词）都要一样：取末尾模型认识的后缀
    for context in ["我", "今天我"] {
        e.set_context(context);
        let texts: Vec<String> = e.input("yao").into_iter().map(|c| c.text).collect();
        assert_eq!(
            texts.first().map(String::as_str),
            Some("要"),
            "{context}: {texts:?}"
        );
    }
}

#[test]
fn auto_learn_promotes_word() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let e = Engine::new(lexicon, lm);

    // 词库里没有「我特」，初始只能拿到原始串
    assert!(!e.input("wote").iter().any(|c| c.text == "我特"));

    // 选不够次数不学
    e.record_selection_with_pinyin("我特", Some("wo te"));
    e.record_selection_with_pinyin("我特", Some("wo te"));
    assert!(!e.input("wote").iter().any(|c| c.text == "我特"));

    // 第三次达到阈值
    e.record_selection_with_pinyin("我特", Some("wo te"));
    assert_eq!(e.user_words().len(), 1);
    assert!(
        e.input("wote").iter().any(|c| c.text == "我特"),
        "自动学习后应当能打出 我特"
    );
}

#[test]
fn composed_word_is_learned_on_second_use() {
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(
            "shuang\t双\t10000\nshuang\t霜\t500\npin\t拼\t8000\npin\t频\t500\n\
             ge\t个\t20000\nge\t阁\t300\nshuang pin\t双拼\t3000\n",
        )
        .unwrap(),
    );
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let e = Engine::new(lexicon, lm);
    assert!(!e.input("shuangpinge").iter().any(|c| c.text == "霜频阁"));

    e.record_composed_word("霜频阁", "shuang pin ge");
    assert!(e.user_words().is_empty());
    e.record_composed_word("霜频阁", "shuang pin ge");
    assert_eq!(e.user_words().len(), 1);
    assert_eq!(e.input("shuangpinge")[0].text, "霜频阁");
}

#[test]
fn long_composed_sentence_still_needs_repeats() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let e = Engine::new(lexicon, lm);
    // 先选句首词再选剩下的长句：不当造词，一次不进词库
    e.record_composed_word("我今天去学校了", "wo jin tian qu xue xiao le");
    assert!(e.user_words().is_empty());
    e.record_composed_word("我今天去学校了", "wo jin tian qu xue xiao le");
    e.record_composed_word("我今天去学校了", "wo jin tian qu xue xiao le");
    assert_eq!(e.user_words().len(), 1);
}

#[test]
fn composed_word_respects_auto_learn_off() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let config = EngineConfig {
        auto_learn: false,
        ..EngineConfig::default()
    };
    let e = Engine::new(lexicon, lm).with_config(config);
    e.record_composed_word("我特", "wo te");
    assert!(e.user_words().is_empty());
}

#[test]
fn auto_learn_can_be_disabled() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    let config = EngineConfig {
        auto_learn: false,
        ..EngineConfig::default()
    };
    let e = Engine::new(lexicon, lm).with_config(config);
    for _ in 0..5 {
        e.record_selection_with_pinyin("我特", Some("wo te"));
    }
    assert!(e.user_words().is_empty());
}

#[test]
fn context_disambiguates_homophones() {
    // 同一拼音，两个候选：先（高频）、西安（低频）
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("xian\t先\t50000\nxi an\t西安\t1000\n").unwrap());
    // TSV 加载器会把二元组计数也记进一元，所以这里额外给「先」补一条
    // 高频搭配，让它在**没有上下文**时仍然靠词频胜出。
    let lm: Arc<dyn LanguageModel> = Arc::new(
        BigramLm::from_bigram_tsv_str("我\t西安\t9990\n我\t先\t10\n先\t的\t100000\n").unwrap(),
    );
    let e = Engine::new(lexicon, lm);

    // 没有上下文时，词频决定：先
    assert_eq!(e.input("xian")[0].text, "先");

    // 有上下文「我」之后，语言模型应当把 西安 顶上来
    e.set_context("我");
    assert_eq!(e.input("xian")[0].text, "西安", "上下文应当消除同音词歧义");
}

#[test]
fn arpa_context_disambiguates() {
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("xian\t先\t50000\nxi an\t西安\t1000\n").unwrap());
    let arpa = "\\data\\\nngram 1=3\nngram 2=2\n\n\\1-grams:\n-1.0\t我\n-1.0\t先\n-1.0\t西安\n\n\\2-grams:\n-0.01\t我 西安\n-4.0\t我 先\n\n\\end\\\n";
    let lm: Arc<dyn LanguageModel> = Arc::new(ArpaLm::from_arpa_str(arpa).expect("ARPA 解析失败"));
    let e = Engine::new(lexicon, lm);
    assert_eq!(e.input("xian")[0].text, "先");
    e.set_context("我");
    assert_eq!(e.input("xian")[0].text, "西安");
}

#[test]
fn context_uses_last_character() {
    // 中文上下文通常没有空格，必须能退到「最后一个字」去查 bigram
    let lexicon: Arc<dyn Lexicon> =
        Arc::new(MemoryLexicon::from_tsv_str("shi\t是\t50000\nshi\t事\t30000\n").unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(
        BigramLm::from_bigram_tsv_str("做\t事\t9990\n做\t是\t10\n是\t的\t100000\n事\t的\t100000\n")
            .unwrap(),
    );
    let e = Engine::new(lexicon, lm);
    assert_eq!(e.input("shi")[0].text, "是");

    e.set_context("我今天做");
    assert_eq!(
        e.input("shi")[0].text,
        "事",
        "多字上下文应当用最后一个字查二元组"
    );
}
