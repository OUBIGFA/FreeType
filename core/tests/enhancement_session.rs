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

#[test]
fn model_suggestions_preserve_original_candidates_and_remove_duplicates() {
    for suggested in ["泥号", "你好"] {
        let (e, d) = setup();
        let mut s = Session::with_engine(&e);
        for c in "nihao".chars() { s.key(&Key::char(c)); }
        let original = s.candidates().to_vec();
        // 模型生成的新整句不能删掉原来的整句或句首候选。
        let q = d.query.lock().unwrap().clone().unwrap();
        *d.answer.lock().unwrap() = Some(Answer { key: q.key, text: suggested.into(), pinyin: "ni hao".into(),
            words: vec![(suggested.into(), "ni hao".into())], uncertain: Vec::new() });
        s.refresh_enhancement();
        assert_eq!(s.candidates()[0].text, suggested);
        for candidate in &original {
            assert!(s.candidates().iter().any(|c| c.text == candidate.text && c.consumed == candidate.consumed),
                "模型建议不得删除普通候选：{}", candidate.text);
        }
        let all = s.candidates();
        let unique: std::collections::HashSet<_> = all.iter().map(|c| (&c.text, c.consumed)).collect();
        assert_eq!(unique.len(), all.len(), "相同文本与消费长度不能重复");
    }
}

#[test]
fn model_merge_preserves_candidates_across_readings_options_learning_and_pins() {
    use std::collections::{BTreeMap, HashSet};
    for (raw, reading, words, fresh) in [
        ("nihao", "ni hao", ["你好", "拟好", "泥号"], "泥好"),
        ("shijie", "shi jie", ["世界", "视界", "时节"], "世节"),
        ("gongshi", "gong shi", ["公式", "工时", "公事"], "工事"),
    ] {
        let mut dictionary=String::new();
        for (i, word) in words.iter().enumerate() {
            dictionary.push_str(&format!("{reading}\t{word}\t{}\n",90000-i*10000));
            for (ch, syllable) in word.chars().zip(reading.split_whitespace()) {
                dictionary.push_str(&format!("{syllable}\t{ch}\t10000\n"));
            }
        }
        for editor in [false,true] { for fuzzy in [false,true] { for learned in [false,true] {
            for pin in [0u8,1,3] { for suggested in [words[0],words[1],fresh] {
                let mut e=Engine::new(Arc::new(MemoryLexicon::from_tsv_str(&dictionary).unwrap()),Arc::new(BigramLm::default()));
                e.set_fuzzy(fuzzy);
                if learned { for _ in 0..3 { e.record_selection_with_pinyin(words[0],Some(reading)); } }
                let d=Arc::new(Delayed::default()); e.set_enhancement(Some(d.clone()));
                let mut s=Session::with_engine(&e);
                s.configure(wowtype_core::session::Config { sentence_editor:editor,..Default::default() });
                for c in raw.chars() { s.key(&Key::char(c)); }
                if pin>0 {
                    let (scope,key)=s.pin_target(0).unwrap();
                    e.set_manual_pins(BTreeMap::from([(scope,BTreeMap::from([(pin,key)]))]));
                    s.refresh_candidates();
                }
                let original=s.candidates().to_vec();
                let q=d.query.lock().unwrap().clone().unwrap();
                *d.answer.lock().unwrap()=Some(Answer { key:q.key,text:suggested.into(),pinyin:reading.into(),
                    words:vec![(suggested.into(),reading.into())],uncertain:Vec::new() });
                s.refresh_enhancement();
                let current=s.candidates();
                for old in &original {
                    assert!(current.iter().any(|c|c.text==old.text&&c.consumed==old.consumed),
                        "原候选消失：{raw} {suggested} editor={editor} fuzzy={fuzzy} learned={learned} pin={pin}: {}",old.text);
                }
                assert_eq!(current.iter().map(|c|(&c.text,c.consumed)).collect::<HashSet<_>>().len(),current.len());
                if pin>0 { assert_eq!(current[pin as usize-1].text,original[pin as usize-1].text); }
                let selected=current[0].text.clone();
                if editor { assert_eq!(s.sentence_view().unwrap().text,selected); }
                assert_eq!(s.key(&Key::char(' ')).commit.as_deref(),Some(selected.as_str()));
            } }
        } } }
    }
}

#[test]
fn model_promotes_an_existing_homophone_without_erasing_the_learned_first_choice() {
    let mut e=Engine::new(Arc::new(MemoryLexicon::from_tsv_str(
        "zhi ding\t置顶\t90000\nzhi ding\t指定\t80000\nzhi ding\t制定\t70000\nzhi\t置\t10000\nzhi\t指\t10000\nding\t顶\t10000\nding\t定\t10000\n"
    ).unwrap()),Arc::new(BigramLm::default()));
    e.set_fuzzy(true);
    let d=Arc::new(Delayed::default());e.set_enhancement(Some(d.clone()));
    let mut s=Session::with_engine(&e);
    s.configure(wowtype_core::session::Config { sentence_editor: true, ..Default::default() });
    for c in "zhiding".chars() { s.key(&Key::char(c)); }
    assert_eq!(s.candidates()[0].text,"置顶");
    let q=d.query.lock().unwrap().clone().unwrap();
    *d.answer.lock().unwrap()=Some(Answer { key:q.key,text:"指定".into(),pinyin:"zhi ding".into(),
        words:vec![("指定".into(),"zhi ding".into())],uncertain:Vec::new() });
    assert!(s.refresh_enhancement());
    assert_eq!(s.candidates().iter().filter(|c|c.text=="指定").count(),1);
    let at=s.candidates().iter().position(|c|c.text=="置顶").expect("原首选仍可选");
    assert_eq!(at,1);
    assert_eq!(s.select(at).commit.as_deref(),Some("置顶"));
}
