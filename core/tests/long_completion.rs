//! 长词补全通过真实词库和输入会话验证；不能把完整词学成只输入的前缀读音。

#[test]
fn exact_homophones_stay_ahead_of_unlearned_extensions() {
    let engine = |completion| {
        let mut e = wowtype_core::Engine::new(std::sync::Arc::new(wowtype_core::MemoryLexicon::from_tsv_str(
            "zhi ding\t指定\t90000\nzhi ding\t置顶\t80000\nzhi ding\t制定\t70000\nzhi ding\t制订\t60000\nzhi ding de\t指定的\t95000\nzhi ding le\t指定了\t94000\nzhi ding chu\t指定出\t93000\n"
        ).unwrap()), std::sync::Arc::new(wowtype_core::BigramLm::default()));
        e.set_long_word_completion(completion);
        e
    };
    let baseline = engine(false).input("zhiding");
    let e = engine(true);
    let result = e.input("zhiding");
    let exact: Vec<_> = baseline
        .iter()
        .filter(|c| c.pinyin == "zhi ding" && c.consumed == 7)
        .take(4)
        .map(|c| c.text.as_str())
        .collect();
    assert_eq!(
        result
            .iter()
            .take(exact.len())
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        exact
    );
    assert!(result.iter().any(|c| c.long_completion), "补全仍应可选");
}
use std::sync::Arc;

use wowtype_core::session::{Key, Session};
use wowtype_core::{BigramLm, Converter, Engine, Lexicon, MemoryLexicon};

#[test]
fn completion_model_selects_a_real_extension_instead_of_a_frequent_fragment() {
    struct Vocabulary;
    impl wowtype_core::LanguageModel for Vocabulary {
        fn score(&self, _: Option<&str>, _: &str) -> f64 {
            -1.0
        }
        fn knows_word(&self, word: &str) -> Option<bool> {
            Some(matches!(word, "人工智能" | "人工智能技术"))
        }
    }
    let words = "ren gong zhi neng\t人工智能\t100000\nren gong zhi neng neng\t人工智能能\t90000\nren gong zhi neng ji shu\t人工智能技术\t1000\n";
    let mut engine = Engine::new(
        Arc::new(MemoryLexicon::from_tsv_str(words).unwrap()),
        Arc::new(Vocabulary),
    );
    engine.set_long_word_completion(true);
    let result = engine.input("rengongzhineng");
    assert_eq!(result[0].text, "人工智能");
    assert_eq!(
        result.iter().find(|c| c.long_completion).unwrap().text,
        "人工智能技术"
    );
    // A sole dictionary extension is still useful for names/poetry absent from the model.
    let words = "hai nei cun zhi\t海内存知\t1000\nhai nei cun zhi ji\t海内存知己\t10\n";
    let mut engine = Engine::new(
        Arc::new(MemoryLexicon::from_tsv_str(words).unwrap()),
        Arc::new(Vocabulary),
    );
    engine.set_long_word_completion(true);
    assert!(engine
        .input("haineicunzhi")
        .iter()
        .any(|c| c.long_completion && c.text == "海内存知己"));
}

const WORDS: &str = "zhong hua\t中华\t4000\n\
    zhong hua ren min\t中华人民\t3000\n\
    zhong hua ren min\t中华仁民\t1000\n\
    zhong hua ren min gong he guo\t中华人民共和国\t6000\n\
    zhong hua ren min gong he guo xian fa\t中华人民共和国宪法\t500\n\
    chuang qian\t窗前\t1000\nming yue\t明月\t1000\n\
    chuang qian ming yue guang\t床前明月光\t800\n";

fn engine() -> Engine {
    let mut engine = Engine::new(
        Arc::new(MemoryLexicon::from_tsv_str(WORDS).unwrap()),
        Arc::new(BigramLm::default()),
    );
    engine.set_long_word_completion(true);
    engine
}

#[test]
fn long_completion_preserves_first_choice_and_completes_a_dictionary_word() {
    let engine = engine();
    let candidates = engine.input("zhonghuarenmin");
    assert_eq!(candidates[0].text, "中华人民");
    assert_eq!(candidates[1].text, "中华人民共和国");
    assert_eq!(candidates[1].pinyin, "zhong hua ren min gong he guo");
    assert_eq!(candidates[1].consumed, "zhonghuarenmin".len());
}

#[test]
fn long_completion_commits_once_in_full_and_shuangpin_sessions() {
    for (shuangpin, input) in [(false, "zhonghuarenmin"), (true, "vshxrfmb")] {
        let mut engine = engine();
        engine.set_shuangpin(shuangpin);
        engine.set_shuangpin_scheme(wowtype_core::shuangpin::Scheme::Xiaohe);
        let mut session = Session::with_engine(&engine);
        for key in input.chars() {
            assert!(session.key(&Key::char(key)).commit.is_none());
        }
        let index = session
            .candidates()
            .iter()
            .position(|c| c.text == "中华人民共和国")
            .expect("完整长词应当出现在前缀候选中");
        assert_eq!(
            session.choose(index).commit.as_deref(),
            Some("中华人民共和国")
        );
        assert!(!session.composing());
        assert!(
            session.frame(false).commit.is_none(),
            "同一次选词不能再次上屏"
        );
    }
}

#[test]
fn long_completion_requires_complete_syllables_and_can_be_disabled() {
    let mut engine = engine();
    for input in ["zhong", "zhrm", "zhonghuarenmi", "zhonghuarenmni", "hello"] {
        assert!(
            !engine.input(input).iter().any(|c| c.long_completion),
            "{input}"
        );
    }
    engine.set_shuangpin(true);
    for input in ["vshxrfm", "vshxrfmbv"] {
        assert!(
            !engine.input(input).iter().any(|c| c.long_completion),
            "odd shuangpin {input}"
        );
    }
    engine.set_shuangpin(false);
    engine.set_long_word_completion(false);
    let ordinary = engine.input("zhonghuarenmin");
    engine.set_long_word_completion(true);
    let completed = engine.input("zhonghuarenmin");
    assert_eq!(completed.iter().filter(|c| c.long_completion).count(), 1);
    assert_eq!(completed[0].text, ordinary[0].text);
    engine.set_long_word_completion(false);
    assert_eq!(
        engine
            .input("zhonghuarenmin")
            .iter()
            .map(|c| &c.text)
            .collect::<Vec<_>>(),
        ordinary.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
    engine.set_long_word_completion(true);
    engine.set_max_candidates(1);
    assert_eq!(engine.input("zhonghuarenmin").len(), 1);
    assert_eq!(engine.input("zhonghuarenmin")[0].text, "中华人民");
}

#[test]
fn long_completion_uses_reading_for_guessed_sentences_and_converts_the_whole_word() {
    let mut engine = engine();
    let candidates = engine.input("chuangqianmingyue");
    assert_eq!(candidates[0].text, "窗前明月");
    assert_eq!(candidates[1].text, "床前明月光");
    engine.install_converter(Converter::from_tsv_str("华\t華\n国\t國\n").unwrap());
    engine.set_traditional(true);
    assert_eq!(engine.input("zhonghuarenmin")[1].text, "中華人民共和國");
}

#[test]
fn long_completion_learning_keeps_the_full_reading_after_cancellation_and_reuse() {
    for (shuangpin, input) in [(false, "zhonghuarenmin"), (true, "vshxrfmb")] {
        let mut engine = engine();
        engine.set_shuangpin(shuangpin);
        engine.set_shuangpin_scheme(wowtype_core::shuangpin::Scheme::Xiaohe);
        let mut session = Session::with_engine(&engine);
        for key in input.chars() {
            session.key(&Key::char(key));
        }
        assert!(session
            .key(&Key::press(wowtype_core::session::keysym::ESCAPE))
            .commit
            .is_none());
        assert!(engine.user_words().is_empty());
        for _ in 0..3 {
            for key in input.chars() {
                session.key(&Key::char(key));
            }
            let index = session
                .candidates()
                .iter()
                .position(|c| c.long_completion)
                .unwrap();
            assert_eq!(
                session.choose(index).commit.as_deref(),
                Some("中华人民共和国")
            );
        }
        let words = engine.user_words();
        let learned = words
            .iter()
            .find(|(_, word, _)| word == "中华人民共和国")
            .unwrap();
        assert_eq!(learned.0, "zhong hua ren min gong he guo");
        assert_eq!(
            engine.input(input)[0].text,
            "中华人民",
            "选补全词不替换前缀首选"
        );
    }
}

#[test]
fn long_completion_respects_manual_positions_and_is_not_pinned_under_the_prefix() {
    let engine = engine();
    engine.set_manual_pins(std::collections::BTreeMap::from([(
        "quanpin:zhonghuarenmin".into(),
        std::collections::BTreeMap::from([
            (1, "zhong hua ren min\t中华仁民".into()),
            (2, "zhong hua ren min\t中华人民".into()),
        ]),
    )]));
    let mut session = Session::with_engine(&engine);
    for key in "zhonghuarenmin".chars() {
        session.key(&Key::char(key));
    }
    assert_eq!(session.candidates()[0].text, "中华仁民");
    assert_eq!(session.candidates()[1].text, "中华人民");
    let index = session
        .candidates()
        .iter()
        .position(|c| c.long_completion)
        .unwrap();
    assert!(index >= 2);
    assert!(session.pin_target(index).is_none());
}

#[test]
fn long_completion_tracks_imported_and_personal_dictionary_changes() {
    let mut engine = Engine::new(
        Arc::new(MemoryLexicon::from_tsv_str("zhong hua ren min\t中华人民\t3000\n").unwrap()),
        Arc::new(BigramLm::default()),
    );
    engine.set_long_word_completion(true);
    let code = "zhonghuarenmin";
    assert!(!engine.input(code).iter().any(|c| c.long_completion));
    engine.lexicon().set_extra_words(vec![(
        "zhong hua ren min gong he guo".into(),
        "中华人民共和国".into(),
        6000.0,
    )]);
    assert_eq!(engine.input(code)[1].text, "中华人民共和国");
    engine.lexicon().set_extra_words(Vec::new());
    assert!(!engine.input(code).iter().any(|c| c.long_completion));
    engine
        .add_user_word("zhong hua ren min gong he guo", "中华人民共和国", 6000.0)
        .unwrap();
    assert_eq!(engine.input(code)[1].text, "中华人民共和国");
    engine.forget_word("中华人民共和国");
    assert!(!engine.input(code).iter().any(|c| c.long_completion));
}

#[test]
fn long_completion_compiled_and_cached_dictionaries_match_and_filter_by_word() {
    let root = std::env::var_os("FREETYPE_TEST_TEMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../_temp/long-completion")
        })
        .join(format!("compiled-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("lexicon.tsv");
    let index = root.join("lexicon.ftd");
    std::fs::write(&source, WORDS).unwrap();
    wowtype_core::lexicon::MappedLexicon::compile(&source, &index).unwrap();
    let memory = MemoryLexicon::from_tsv_path_cached(&source).unwrap();
    let cached = MemoryLexicon::from_tsv_path_cached(&source).unwrap();
    let mapped = wowtype_core::lexicon::MappedLexicon::open(&source, &index).unwrap();
    let prefix = wowtype_core::lexicon::parse_pinyin_keys("zhong hua ren min").unwrap();
    for lexicon in [&memory as &dyn Lexicon, &cached, &mapped] {
        let multiple = lexicon.long_completions(&prefix, "中华人民", 16);
        assert!(multiple.iter().any(|entry| entry.word == "中华人民共和国"));
        assert!(multiple
            .iter()
            .any(|entry| entry.word == "中华人民共和国宪法"));
        assert!(multiple.len() <= 16);
        assert_eq!(
            lexicon
                .complete_long_word(&prefix, "中华人民")
                .unwrap()
                .word,
            "中华人民共和国"
        );
        assert!(lexicon.complete_long_word(&prefix, "中华仁民").is_none());
        assert!(lexicon.complete_long_word(&prefix[..1], "").is_none());
        assert!(lexicon.complete_long_word(&prefix[..2], "").is_some());
        let longer =
            wowtype_core::lexicon::parse_pinyin_keys("zhong hua ren min gong he guo").unwrap();
        assert_eq!(
            lexicon.complete_long_word(&longer, "").unwrap().word,
            "中华人民共和国宪法"
        );
    }
    let mut engine = Engine::new(Arc::new(mapped), Arc::new(BigramLm::default()));
    engine.set_long_word_completion(true);
    assert_eq!(engine.input("zhonghuarenmin")[1].text, "中华人民共和国");
}

#[test]
fn long_completion_keeps_the_raw_candidate_when_the_list_is_full() {
    let mut engine = engine();
    engine.load_english_str("zhonghuarenmin\n");
    engine.set_max_candidates(3);
    let candidates = engine.input("zhonghuarenmin");
    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0].text, "中华人民");
    assert!(candidates.iter().any(|c| c.text == "zhonghuarenmin"));
    engine.set_max_candidates(2);
    let candidates = engine.input("zhonghuarenmin");
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].text, "中华人民");
    assert!(candidates.iter().any(|c| c.text == "zhonghuarenmin"));
}
