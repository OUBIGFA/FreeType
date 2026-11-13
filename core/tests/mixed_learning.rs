//! Exercise the real session, user dictionary and learning persistence for mixed input.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use wowtype_core::session::{Key, Session};
use wowtype_core::{BigramLm, Engine, MemoryLexicon};

fn new_engine() -> Engine {
    let lexicon = MemoryLexicon::from_tsv_str(
        "da\t打\t1000\nda\t大\t800\nta\t他\t700\nge\t个\t600\nhao\t好\t900\nwo\t我\t800\npin\t频\t900\n",
    ).unwrap();
    let engine = Engine::new(Arc::new(lexicon), Arc::new(BigramLm::default()));
    engine.load_english_str("tag\nGitHub\nhello\n");
    engine
}

fn choose_mixed(engine: &Engine) {
    let mut session = Session::new();
    session.set_engine(Some(engine));
    for letter in "datag".chars() {
        session.key(&Key::char(letter));
    }
    let head = session
        .candidates()
        .iter()
        .position(|candidate| candidate.text == "打")
        .unwrap();
    assert!(session.choose(head).commit.is_none());
    let tail = session
        .candidates()
        .iter()
        .position(|candidate| candidate.text == "tag")
        .unwrap();
    assert_eq!(session.choose(tail).commit.as_deref(), Some("打tag"));
}

fn directory(name: &str) -> PathBuf {
    let root = std::env::var_os("FREETYPE_TEST_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("_temp/core-learning")
        });
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = root.join(format!("{name}-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn segmented_mixed_word_is_learned_under_the_complete_input() {
    let engine = new_engine();
    choose_mixed(&engine);
    assert!(
        engine
            .user_words()
            .iter()
            .any(|(input, word, _)| input == "datag" && word == "打tag"),
        "中英组词应记录整条实际输入码，不能丢掉英文部分"
    );
    assert_eq!(
        engine.input("datag")[0].text,
        "打tag",
        "以后完整输入应直接给出学到的词"
    );
    assert!(
        !engine
            .input("da")
            .iter()
            .any(|candidate| candidate.text == "打tag"),
        "不能错误地只绑定中文部分"
    );
    assert_eq!(engine.history_count("打tag"), 1);
}

#[test]
fn english_words_and_longer_mixed_words_use_the_real_user_dictionary() {
    let mut engine = new_engine();
    for _ in 0..3 {
        engine.record_selection_with_pinyin("tag", Some("tag"));
    }
    assert!(engine
        .user_words()
        .iter()
        .any(|(input, word, _)| input == "tag" && word == "tag"));
    engine.record_composed_word("安装FreeType", "an zhuang freetype");
    assert!(
        engine
            .user_words()
            .iter()
            .any(|(input, word, _)| input == "anzhuangfreetype" && word == "安装FreeType"),
        "英文名称长度不应让短中文混输词被当成长句"
    );
    assert_eq!(engine.input("anzhuangfreetype")[0].text, "安装FreeType");
    engine.set_shuangpin(true);
    for _ in 0..3 {
        engine.record_selection_with_pinyin("GitHub", Some("github"));
    }
    assert!(
        engine
            .user_words()
            .iter()
            .any(|(input, word, _)| input == "github" && word == "GitHub"),
        "双拼不能把英文输入码转换成另一串拼音"
    );
    assert_eq!(engine.input("github")[0].text, "GitHub");
}

#[test]
fn shuangpin_single_syllable_keeps_the_candidates_full_reading() {
    let mut engine = new_engine();
    engine.set_shuangpin(true);
    let mut session = Session::new();
    session.set_engine(Some(&engine));
    for _ in 0..3 {
        for letter in "pn".chars() {
            session.key(&Key::char(letter));
        }
        let index = session
            .candidates()
            .iter()
            .position(|candidate| candidate.text == "频")
            .unwrap();
        assert_eq!(session.choose(index).commit.as_deref(), Some("频"));
    }
    assert!(
        engine
            .user_words()
            .iter()
            .any(|(reading, word, _)| reading == "pin" && word == "频"),
        "候选已给出单音节全拼时不能再当成双拼键位转换"
    );
}

#[test]
fn mixed_words_history_and_pairs_survive_restart_and_backup() {
    let source = new_engine();
    choose_mixed(&source);
    let directory = directory("mixed-roundtrip");
    let words = directory.join("user.tsv");
    let history = directory.join("history.tsv");
    source.save_user_dict(&words).unwrap();
    source.save_history(&history).unwrap();
    let restarted = new_engine();
    restarted.load_user_dict(&words).unwrap();
    restarted.load_history(&history).unwrap();
    assert_eq!(restarted.input("datag")[0].text, "打tag");
    assert_eq!(restarted.history_count("打tag"), 1);
    assert!(restarted.pair_count() > 0);
    let restored = new_engine();
    restored
        .import_learning(&source.export_learning().unwrap())
        .unwrap();
    assert_eq!(restored.input("datag")[0].text, "打tag");
    assert_eq!(restored.history_count("tag"), 1);
    assert_eq!(restored.history_count("打tag"), 1);
    assert!(restored.forget_word("打tag"));
    assert!(!restored
        .input("datag")
        .iter()
        .any(|candidate| candidate.text == "打tag"));
    restarted.clear_learned();
    assert!(!restarted
        .input("datag")
        .iter()
        .any(|candidate| candidate.text == "打tag"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn automatic_spacing_does_not_break_learning_the_following_word() {
    let engine = new_engine();
    choose_mixed(&engine);
    engine.set_context("打 tag");
    engine.input("hao");
    engine.record_selection_with_pinyin("好", Some("hao"));
    let backup: serde_json::Value =
        serde_json::from_str(&engine.export_learning().unwrap()).unwrap();
    assert!(
        backup["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pair| pair[0] == "打tag" && pair[1] == "好"),
        "自动加空格后的光标前文字仍应对应刚选出的混输词"
    );
}

#[test]
fn context_matching_does_not_join_words_across_newlines() {
    let engine = new_engine();
    choose_mixed(&engine);
    engine.set_context("打\ntag");
    engine.input("hao");
    engine.record_selection_with_pinyin("好", Some("hao"));
    let backup: serde_json::Value =
        serde_json::from_str(&engine.export_learning().unwrap()).unwrap();
    assert!(!backup["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pair| pair[0] == "打tag" && pair[1] == "好"));
}

#[test]
fn mixed_learning_rejects_invalid_inputs_without_partial_updates() {
    let engine = new_engine();
    choose_mixed(&engine);
    let before = engine.user_words();
    for (input, word) in [
        ("da\ttag", "打tag"),
        ("../tag", "打tag"),
        ("notapinyin", "中文"),
        ("tag", "bad\nword"),
    ] {
        assert!(engine.add_user_word(input, word, 1000.0).is_err());
        assert_eq!(engine.user_words(), before);
    }
}

#[test]
fn learned_code_candidates_follow_usage_and_context() {
    let engine = new_engine();
    engine
        .import_learning(
            r#"{
        "format":"freetype-learning","version":1,
        "words":[["datag","打tag",3000],["datag","大tag",9000]],
        "history":[["打tag",1,1000],["大tag",5,1000]],
        "pairs":[["我","打tag",3,1000]]
    }"#,
        )
        .unwrap();
    assert_eq!(
        engine.input_at("datag", 1000)[0].text,
        "大tag",
        "无上文时优先常用词"
    );
    engine.record_selection_at("我", 1000);
    engine.set_context("我");
    assert_eq!(
        engine.input_at("datag", 1000)[0].text,
        "打tag",
        "有上文时应使用学到的搭配"
    );
}

#[test]
fn multiline_shortcuts_do_not_create_invalid_learning_records() {
    let engine = new_engine();
    engine
        .lexicon()
        .set_custom_tsv("ni\t您好\\n下一行\t10000\n");
    let mut session = Session::new();
    session.set_engine(Some(&engine));
    for letter in "ni".chars() {
        session.key(&Key::char(letter));
    }
    let index = session
        .candidates()
        .iter()
        .position(|candidate| candidate.text == "您好\n下一行")
        .unwrap();
    assert_eq!(
        session.choose(index).commit.as_deref(),
        Some("您好\n下一行")
    );
    let restored = new_engine();
    restored
        .import_learning(&engine.export_learning().unwrap())
        .unwrap();
    assert_eq!(
        restored.history_count("您好\n下一行"),
        0,
        "多行短语保存在快捷短语中，不应破坏单行词条记录"
    );
}
