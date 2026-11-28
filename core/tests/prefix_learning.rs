//! Complete words stay selectable from their beginning; use changes their rank.
use std::sync::Arc;
use wowtype_core::session::{Key, Session};
use wowtype_core::{BigramLm, Engine, MemoryLexicon};
fn engine() -> Engine {
    let mut e=Engine::new(Arc::new(MemoryLexicon::from_tsv_str("zhe\t这\t9000\nshi\t是\t8000\nzhe shi\t这是\t10000\nzhe shi ge hao xi gua\t这是个好西瓜\t500\nzhe shi yi ge li zi\t这是一个例子\t1000\n").unwrap()),Arc::new(BigramLm::default()));
    e.set_long_word_completion(true);
    e.load_english_str("GitHub\ngitlab\ngit\n");
    e
}
#[test]
fn english_prefix_preserves_case_and_learns_full_word() {
    let e = engine();
    let mut s = Session::with_engine(&e);
    for _ in 0..8 {
        for c in "git".chars() {
            s.key(&Key::char(c));
        }
        let i = s
            .candidates()
            .iter()
            .position(|c| c.text == "GitHub")
            .expect("git must offer GitHub");
        assert_eq!(s.choose(i).commit.as_deref(), Some("GitHub"));
    }
    assert_eq!(e.input("git")[0].text, "GitHub");
    assert!(e.input("gi").iter().any(|c| c.text == "GitHub"));
    assert_eq!(e.input("gi")[0].text, "GitHub");
    for ch in "gi".chars() {
        s.key(&Key::char(ch));
    }
    assert_eq!(s.key(&Key::char(' ')).commit.as_deref(), Some("GitHub"));
    assert!(e
        .user_words()
        .iter()
        .any(|(p, w, _)| w == "GitHub" && p == "github"));
    let restored = engine();
    restored
        .import_learning(&e.export_learning().unwrap())
        .unwrap();
    assert_eq!(restored.input("git")[0].text, "GitHub");
    assert_eq!(restored.input("gi")[0].text, "GitHub");
}
#[test]
fn two_syllables_offer_dictionary_and_frequent_learned_phrase() {
    let e = engine();
    assert!(e.input("zheshi").iter().any(|c| c.text == "这是个好西瓜"));
    for _ in 0..8 {
        let mut s = Session::with_engine(&e);
        for c in "zheshi".chars() {
            s.key(&Key::char(c));
        }
        let i = s
            .candidates()
            .iter()
            .position(|c| c.text == "这是个好西瓜")
            .unwrap();
        assert_eq!(s.choose(i).commit.as_deref(), Some("这是个好西瓜"));
    }
    assert_eq!(e.input("zheshi")[0].text, "这是个好西瓜");
    let mut session = Session::with_engine(&e);
    for ch in "zheshi".chars() {
        session.key(&Key::char(ch));
    }
    assert_eq!(
        session.key(&Key::char(' ')).commit.as_deref(),
        Some("这是个好西瓜")
    );
    let restored = engine();
    restored
        .import_learning(&e.export_learning().unwrap())
        .unwrap();
    assert_eq!(restored.input("zheshi")[0].text, "这是个好西瓜");
}
#[test]
fn learned_word_outside_default_english_dictionary_completes() {
    let e = engine();
    for _ in 0..8 {
        e.record_selection_with_pinyin("MyProject", Some("myproject"));
    }
    assert!(e.input("myp").iter().any(|c| c.text == "MyProject"));
}

#[test]
fn personal_phrase_is_visible_despite_a_different_top_homophone() {
    let mut e = Engine::new(
        Arc::new(
            MemoryLexicon::from_tsv_str("zhe shi\t这时\t90000\nzhe\t这\t20000\nshi\t时\t10000\n")
                .unwrap(),
        ),
        Arc::new(BigramLm::default()),
    );
    e.set_long_word_completion(true);
    e.add_user_word("zhe shi ge hao xi gua", "这是个好西瓜", 1000.0)
        .unwrap();
    assert!(e.input("zheshi").iter().any(|c| c.text == "这是个好西瓜"));
}

#[test]
fn common_dictionary_extensions_do_not_hide_a_new_personal_phrase() {
    let mut e=Engine::new(Arc::new(MemoryLexicon::from_tsv_str("zhe shi\t这是\t90000\nzhe shi yi ge\t这是一个\t80000\nzhe shi wo\t这是我\t70000\nzhe shi ge\t这是个\t60000\n").unwrap()),Arc::new(BigramLm::default()));
    e.set_long_word_completion(true);
    e.add_user_word("zhe shi ge hao xi gua", "这是个好西瓜", 1000.0)
        .unwrap();
    assert!(e
        .input("zheshi")
        .iter()
        .take(4)
        .any(|c| c.text == "这是个好西瓜"));
}

#[test]
fn multiple_frequent_completions_keep_descending_usage_order() {
    let e = engine();
    for _ in 0..12 {
        e.record_selection_with_pinyin("GitHub", Some("github"));
    }
    for _ in 0..8 {
        e.record_selection_with_pinyin("gitlab", Some("gitlab"));
    }
    let result = e.input("gi");
    assert_eq!(result[0].text, "GitHub");
    assert_eq!(result[1].text, "gitlab");
}
