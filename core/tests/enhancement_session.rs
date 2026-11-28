//! The asynchronous boundary must honor input identity and explicit choices.
use std::sync::{Arc, Mutex};
use wowtype_core::enhancement::{Answer, Backend, Query};
use wowtype_core::session::{Key, Session};
use wowtype_core::{BigramLm, Engine, MemoryLexicon};
#[derive(Default)]
struct Delayed {
    query: Mutex<Option<Query>>,
    answer: Mutex<Option<Answer>>,
}
impl Backend for Delayed {
    fn submit(&self, q: Query) {
        *self.query.lock().unwrap() = Some(q);
    }
    fn answer(&self, key: &str) -> Option<Answer> {
        self.answer
            .lock()
            .unwrap()
            .as_ref()
            .filter(|a| a.key == key)
            .cloned()
    }
}
fn setup() -> (Engine, Arc<Delayed>) {
    let e=Engine::new(Arc::new(MemoryLexicon::from_tsv_str("ni hao\t你好\t10000\nni\t你\t9000\nni\t泥\t1000\nhao\t好\t8000\nhao\t号\t1000\nma\t吗\t1000\n").unwrap()),Arc::new(BigramLm::default()));
    let d = Arc::new(Delayed::default());
    e.set_enhancement(Some(d.clone()));
    (e, d)
}
#[test]
fn stale_answer_cannot_change_new_input() {
    let (e, d) = setup();
    let mut s = Session::new();
    s.set_engine(Some(&e));
    for c in "nihao".chars() {
        s.key(&Key::char(c));
    }
    let q = d.query.lock().unwrap().clone().unwrap();
    s.key(&Key::char('m'));
    s.key(&Key::char('a'));
    *d.answer.lock().unwrap() = Some(Answer {
        uncertain:Vec::new(),
        key: q.key,
        text: "泥号".into(),
        pinyin: "ni hao".into(),
        words: vec![("泥号".into(), "ni hao".into())],
    });
    assert!(!s.refresh_enhancement());
    assert_ne!(s.candidates()[0].text, "泥号");
}
#[test]
fn ready_result_updates_sentence_and_commits_exactly_displayed_text() {
    let (e, d) = setup();
    let mut s = Session::new();
    s.set_engine(Some(&e));
    for c in "nihao".chars() {
        s.key(&Key::char(c));
    }
    let q = d.query.lock().unwrap().clone().unwrap();
    *d.answer.lock().unwrap() = Some(Answer {
        uncertain:Vec::new(),
        key: q.key,
        text: "泥号".into(),
        pinyin: "ni hao".into(),
        words: vec![("泥".into(), "ni".into()), ("号".into(), "hao".into())],
    });
    assert!(s.refresh_enhancement());
    assert!(s.sentence_view().is_none(), "模型增强不依赖编辑区开启");
    assert_eq!(s.candidates()[0].text, "泥号");
    assert_eq!(s.key(&Key::char(' ')).commit.as_deref(), Some("泥号"));
}

#[test]
fn cached_result_still_requests_display_refresh_on_repeated_input(){
    let(e,d)=setup();let mut s=Session::with_engine(&e);
    for c in "nihao".chars(){s.key(&Key::char(c));}
    let q=d.query.lock().unwrap().clone().unwrap();
    *d.answer.lock().unwrap()=Some(Answer{key:q.key,text:"泥号".into(),pinyin:"ni hao".into(),words:vec![("泥号".into(),"ni hao".into())],uncertain:Vec::new()});
    assert!(s.refresh_enhancement());s.reset(false);
    for c in "nihao".chars(){s.key(&Key::char(c));}
    assert!(s.enhancement_pending(),"ready cached answer must still reach the visible frame");
    assert!(s.refresh_enhancement());assert_eq!(s.candidates()[0].text,"泥号");
    assert!(!s.enhancement_pending());
}
#[test]
fn preview_does_not_launch_inference_but_accepted_key_does() {
    let (e, d) = setup();
    let mut s = Session::new();
    s.set_engine(Some(&e));
    for c in "niha".chars() {
        s.key(&Key::char(c));
    }
    *d.query.lock().unwrap() = None;
    let mut preview = s.preview();
    preview.key(&Key::char('o'));
    assert!(d.query.lock().unwrap().is_none());
    assert!(preview.accept_preview());
    assert!(d.query.lock().unwrap().is_some());
}
