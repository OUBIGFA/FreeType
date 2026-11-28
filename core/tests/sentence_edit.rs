//! Real engine/session contracts for local sentence edits; no model download needed.
use std::sync::Arc;
use wowtype_core::sentence::Action;
use wowtype_core::session::{Key, Session};
use wowtype_core::{BigramLm, Engine, MemoryLexicon};

fn engine() -> Engine {
    Engine::new(Arc::new(MemoryLexicon::from_tsv_str(
        "wo\t我\t90000\nxi huan\t喜欢\t70000\nxi\t喜\t30000\nhuan\t欢\t30000\nni\t你\t90000\nni\t泥\t10000\nhao\t好\t80000\n"
    ).unwrap()), Arc::new(BigramLm::default()))
}
fn type_text(s: &mut Session, text: &str) {
    for ch in text.chars() {
        s.key(&Key::char(ch));
    }
}

#[test]
fn ordinary_input_does_not_enable_sentence_editor_by_default() {
    let e = engine();
    let mut s = Session::new();
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    assert!(s.sentence_view().is_none(), "整句编辑区默认关闭");
    let before = s.candidates().to_vec();
    s.sentence_action(Action::Focus { at: 9 });
    assert!(s.sentence_view().is_none());
    assert_eq!(s.candidates(), before);
    let frame = s.key(&Key::char('1'));
    assert_eq!(frame.commit.as_deref(), Some("我喜欢你"));
}

#[test]
fn disabling_editor_drops_local_focus_and_restores_digit_selection() {
    let e = engine();
    let mut s = Session::with_engine(&e);
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    type_text(&mut s, "woxihuanni");
    s.sentence_action(Action::Focus { at: 9 });
    assert!(s.sentence_view().unwrap().cursor.is_some());
    s.configure(Default::default());
    assert!(s.sentence_view().is_none());
    assert_eq!(s.key(&Key::char('1')).commit.as_deref(), Some("我喜欢你"));
}

#[test]
fn ordinary_candidate_forget_does_not_delete_the_focused_editor_block() {
    let e = engine();
    e.add_user_word("wo", "我", 90000.0).unwrap();
    e.add_user_word("ni", "你", 90000.0).unwrap();
    let mut s = Session::with_engine(&e);
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    type_text(&mut s, "woxihuanni");
    s.sentence_action(Action::Focus { at: 9 });
    assert_eq!(s.sentence_view().unwrap().blocks.iter().find(|b| b.focused).unwrap().text, "你");
    let index = s.candidates().iter().position(|c| c.text == "我").unwrap();
    assert_eq!(s.forget(index).as_deref(), Some("我"));
    let words = e.user_words();
    assert!(words.iter().any(|(_, word, _)| word == "你"), "普通候选菜单不能删编辑区焦点词");
    assert!(words.iter().all(|(_, word, _)| word != "我"));
}

#[test]
fn local_selection_keeps_the_rest_and_commits_once() {
    let e = engine();
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    assert_eq!(s.sentence_view().unwrap().text, "我喜欢你");
    s.sentence_action(Action::Focus { at: 9 });
    let alternatives = s.sentence_view().unwrap().alternatives.clone();
    let i = alternatives.iter().position(|x| x.text == "泥").unwrap();
    s.sentence_action(Action::Choose { index: i });
    assert_eq!(s.sentence_view().unwrap().text, "我喜欢泥");
    assert_eq!(s.key(&Key::char(' ')).commit.as_deref(), Some("我喜欢泥"));
    assert!(!s.composing());
}

#[test]
fn split_and_merge_change_boundaries_without_changing_text() {
    let e = engine();
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    s.sentence_action(Action::Focus { at: 8 });
    s.sentence_action(Action::Resize { expand: false });
    let v = s.sentence_view().unwrap();
    assert_eq!(v.text, "我喜欢你");
    assert!(v.blocks.iter().any(|b| b.text == "喜"));
    s.sentence_action(Action::Resize { expand: true });
    assert!(s
        .sentence_view()
        .unwrap()
        .blocks
        .iter()
        .any(|b| b.text == "喜欢"));
}

#[test]
fn cancelled_sentence_does_not_learn_a_boundary() {
    let e = engine();
    let before = e.export_learning().unwrap();
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    s.sentence_action(Action::Focus { at: 8 });
    s.sentence_action(Action::Resize { expand: false });
    s.reset(false);
    assert_eq!(e.export_learning().unwrap(), before);
}

#[test]
fn boundary_survives_backup_and_repeated_merge_without_learning_on_cancel() {
    let e = engine();
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    s.sentence_action(Action::Focus { at: 8 });
    s.sentence_action(Action::Resize { expand: false });
    s.key(&Key::char(' '));
    let saved = e.export_learning().unwrap();
    let restored = engine();
    restored.import_learning(&saved).unwrap();
    restored.import_learning(&saved).unwrap();
    let mut next = Session::new();
    next.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    next.set_engine(Some(&restored));
    type_text(&mut next, "woxihuanni");
    assert!(next
        .sentence_view()
        .unwrap()
        .blocks
        .iter()
        .any(|b| b.text == "喜"));
}

#[test]
fn windows_history_overwrite_keeps_latest_boundaries() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../_temp/sentence-tests")
        .join(format!("freetype-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("history.tsv");
    let e = engine();
    e.remember_boundary(
        "xi huan ni",
        vec![
            ("喜".into(), "xi".into()),
            ("欢你".into(), "huan ni".into()),
        ],
    );
    e.save_history(&file).unwrap();
    e.remember_boundary(
        "wo xi",
        vec![("我".into(), "wo".into()), ("喜".into(), "xi".into())],
    );
    e.save_history(&file).unwrap();
    let restored = engine();
    restored.load_history(&file).unwrap();
    assert_eq!(restored.sentence_boundaries().len(), 2);
}

#[test]
fn half_syllable_stays_after_committing_complete_prefix() {
    let e = engine();
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "wox");
    let frame = s.key(&Key::char(' '));
    assert_eq!(frame.commit.as_deref(), Some("我"));
    assert_eq!(s.input(), "x");
}

#[test]
fn changing_letters_preserves_nonoverlapping_confirmation() {
    let e = engine();
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    s.sentence_action(Action::Focus { at: 9 });
    let i = s
        .sentence_view()
        .unwrap()
        .alternatives
        .iter()
        .position(|a| a.text == "泥")
        .unwrap();
    s.sentence_action(Action::Choose { index: i });
    s.sentence_action(Action::Caret { at: 0 });
    s.key(&Key::char('n'));
    s.key(&Key::char('i'));
    assert!(s.sentence_view().unwrap().text.ends_with("泥"));
}

#[test]
fn merged_name_needs_second_confirmation_even_after_backup() {
    let e = engine();
    e.record_merged_word("我喜欢你", "wo xi huan ni");
    assert!(e.user_words().iter().all(|(_, w, _)| w != "我喜欢你"));
    let backup = e.export_learning().unwrap();
    let restored = engine();
    restored.import_learning(&backup).unwrap();
    restored.import_learning(&backup).unwrap();
    assert!(restored
        .user_words()
        .iter()
        .all(|(_, w, _)| w != "我喜欢你"));
    restored.record_merged_word("我喜欢你", "wo xi huan ni");
    assert!(restored
        .user_words()
        .iter()
        .any(|(_, w, _)| w == "我喜欢你"));
}

#[test]
fn learned_local_boundary_survives_an_added_suffix() {
    let e = engine();
    e.remember_boundary(
        "xi huan ni",
        vec![
            ("喜".into(), "xi".into()),
            ("欢你".into(), "huan ni".into()),
        ],
    );
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    assert!(s
        .sentence_view()
        .unwrap()
        .blocks
        .iter()
        .any(|b| b.text == "喜"));
    type_text(&mut s, "hao");
    assert!(s
        .sentence_view()
        .unwrap()
        .blocks
        .iter()
        .any(|b| b.text == "喜"));
    assert!(s.sentence_view().unwrap().text.ends_with("好"));
}

#[test]
fn cycling_back_restores_original_without_learning() {
    let e = engine();
    let before = e.export_learning().unwrap();
    let mut s = Session::new();
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    s.set_engine(Some(&e));
    type_text(&mut s, "woxihuanni");
    s.sentence_action(Action::Focus { at: 9 });
    s.sentence_action(Action::Cycle { backward: false });
    assert!(s.sentence_view().unwrap().text.ends_with("泥"));
    s.sentence_action(Action::Cycle { backward: true });
    assert_eq!(s.sentence_view().unwrap().text, "我喜欢你");
    s.reset(false);
    assert_eq!(e.export_learning().unwrap(), before);
}
