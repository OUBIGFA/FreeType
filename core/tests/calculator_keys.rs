use std::sync::Arc;
use wowtype_core::session::{keysym, Key, Session};
use wowtype_core::{BigramLm, Engine, MemoryLexicon};

fn engine() -> Engine {
    Engine::new(
        Arc::new(MemoryLexicon::default()),
        Arc::new(BigramLm::default()),
    )
}
fn type_text(session: &mut Session<'_>, text: &str) {
    for c in text.chars() {
        session.key(&Key::char(c));
    }
}

#[test]
fn calculator_letters_choose_and_enter_ignores_highlight() {
    let engine = engine();
    let mut session = Session::with_engine(&engine);
    type_text(&mut session, "v1+2");
    assert_eq!(
        session.key(&Key::char('b')).commit.as_deref(),
        Some("1+2=3")
    );
    type_text(&mut session, "v123.45");
    session.key(&Key::press(keysym::DOWN));
    assert_eq!(
        session.key(&Key::press(keysym::RETURN)).commit.as_deref(),
        Some("123.45")
    );
    type_text(&mut session, "v2**3**2");
    assert_eq!(
        session.key(&Key::press(keysym::KP_ENTER)).commit.as_deref(),
        Some("512")
    );
    type_text(&mut session, "v2^3");
    assert_eq!(session.key(&Key::char('=')).commit.as_deref(), Some("8"));
}

#[test]
fn calculator_currency_rounds_to_cents_and_preserves_invalid_input() {
    let engine = engine();
    let mut session = Session::with_engine(&engine);
    for symbol in ['$', '￥', '¥'] {
        type_text(&mut session, "v123.456");
        assert_eq!(
            session.key(&Key::char(symbol)).commit.as_deref(),
            Some("壹佰贰拾叁元肆角陆分")
        );
    }
    type_text(&mut session, "v1/0");
    assert!(session.key(&Key::char('$')).commit.is_none());
    assert_eq!(session.input(), "v1/0");
    session.reset(false);
    type_text(&mut session, "v1+");
    assert_eq!(
        session.key(&Key::press(keysym::RETURN)).commit.as_deref(),
        Some("v1+")
    );
}

#[test]
fn calculator_formats_are_stable_and_do_not_learn() {
    let engine = engine();
    let mut session = Session::with_engine(&engine);
    type_text(&mut session, "v1234");
    let texts: Vec<_> = session
        .candidates()
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert_eq!(
        &texts[..3],
        &["壹仟贰佰叁拾肆元整", "一千二百三十四", "1234"]
    );
    assert!(texts.contains(&"MCCXXXIV".into()));
    assert!(texts.contains(&"1,234".into()));
    assert!(texts.contains(&"壹仟贰佰叁拾肆".into()));
    assert_eq!(
        session.key(&Key::char('B')).commit.as_deref(),
        Some("一千二百三十四")
    );
    assert!(!session.take_learning_change());
}

#[test]
fn paged_calculator_letters_follow_visible_labels_and_digits_keep_typing() {
    let engine = engine();
    let mut s = Session::with_engine(&engine);
    let mut config = *s.config();
    config.page_size = 3;
    s.configure(config);
    type_text(&mut s, "v1234");
    let page = s.key(&Key::press(keysym::DOWN));
    assert_eq!(page.panel, wowtype_core::session::Panel::Normal);
    assert_eq!(s.key(&Key::char('b')).commit.as_deref(), Some("MCCXXXIV"));
    type_text(&mut s, "v1234");
    s.key(&Key::press(keysym::DOWN));
    assert!(s.key(&Key::char('5')).commit.is_none());
    assert_eq!(s.input(), "v12345");
}
