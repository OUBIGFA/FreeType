use std::sync::Arc;
use wowtype_core::session::{keysym, Key, Session};
use wowtype_core::{BigramLm, Engine, MemoryLexicon};

fn engine() -> Engine {
    Engine::new(
        Arc::new(
            MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\nni hao\t拟好\t100\nma\t吗\t1000\n")
                .unwrap(),
        ),
        Arc::new(BigramLm::default()),
    )
}
fn typing(s: &mut Session<'_>, text: &str) {
    for c in text.chars() {
        s.key(&Key::char(c));
    }
}

#[test]
fn trace_is_opt_in_and_captures_pick_abandon_and_immediate_backspace() {
    let e = engine();
    let mut s = Session::with_engine(&e);
    typing(&mut s, "nihao ");
    assert!(s.take_trace().is_empty());
    s.set_trace(true);
    typing(&mut s, "nihao");
    s.choose(1);
    let event = s.take_trace().remove(0);
    assert_eq!(event["e"], "commit");
    assert_eq!(event["text"], "拟好");
    assert_eq!(event["steps"][0]["via"], "click");
    assert_eq!(event["steps"][0]["pick"], 2);
    s.key(&Key::press(keysym::BACKSPACE));
    assert_eq!(s.take_trace()[0]["e"], "backspace");
    typing(&mut s, "nihao");
    s.key(&Key::press(keysym::ESCAPE));
    let event = s.take_trace().remove(0);
    assert_eq!(event["e"], "abandon");
    assert_eq!(event["input"], "nihao");
    s.set_no_learn(true);
    typing(&mut s, "private ");
    assert!(s.take_trace().is_empty());
    s.set_no_learn(false);
    s.set_trace(false);
    typing(&mut s, "nihao ");
    assert!(s.take_trace().is_empty());
}

#[test]
fn undone_prefix_selection_is_not_exported_as_a_final_choice() {
    let e = engine();
    let mut s = Session::with_engine(&e);
    s.set_trace(true);
    typing(&mut s, "nihaoma");
    let head = s
        .candidates()
        .iter()
        .position(|c| c.text == "你好" && c.consumed < 8)
        .unwrap();
    s.choose(head);
    assert!(s.take_trace().is_empty());
    s.key(&Key::press(keysym::BACKSPACE));
    s.key(&Key::press(keysym::SPACE));
    let events = s.take_trace();
    let commit = events.iter().find(|e| e["e"] == "commit").unwrap();
    assert_eq!(commit["steps"].as_array().unwrap().len(), 1);
}
