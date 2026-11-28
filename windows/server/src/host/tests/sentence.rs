//! 整句编辑区与可选增强模型的服务端行为。
use super::*;

#[test]
fn sentence_actions_require_current_display_and_preserve_session() {
    let mut host = host("sentence-actions");
    configure(&mut host, |c| c.sentence_editor = true);
    let frame = typing(&mut host, "nihaoshijie");
    let revision = frame.page_revision;
    let stale = host
        .handle(
            1,
            Request::CandidateAction {
                revision: revision.wrapping_add(1),
                action: CandidateAction::SentenceFocus { at: 5 },
            },
        )
        .frame
        .unwrap();
    assert!(stale.sentence.unwrap().cursor.is_none());
    let revision = stale.page_revision;
    let focus = host
        .handle(
            1,
            Request::CandidateAction {
                revision,
                action: CandidateAction::SentenceFocus { at: 5 },
            },
        )
        .frame
        .unwrap();
    assert_eq!(
        focus.candidates, frame.candidates,
        "词块焦点不能替换普通候选"
    );
    assert_eq!(focus.preedit, frame.preedit);
    assert_eq!(focus.title, frame.title);
    assert_eq!(focus.sentence.unwrap().cursor, Some(5));
    assert!(focus.commit.is_none());
    let reset = host
        .handle(1, Request::Reset { commit: false })
        .frame
        .unwrap();
    assert!(reset.sentence.is_none());
    let late = host.handle(
        1,
        Request::RefreshEnhancement {
            revision: focus.page_revision,
        },
    );
    assert!(late.frame.is_none());
    cleanup(&host);
}

#[test]
fn sentence_editor_defaults_off_and_disabling_restores_ordinary_selection() {
    let mut host = host("sentence-default-off");
    assert!(!host.config.sentence_editor);
    let frame = typing(&mut host, "nihaoshijie");
    assert!(frame.sentence.is_none());
    configure(&mut host, |c| c.sentence_editor = true);
    let frame = host
        .handle(
            1,
            Request::CandidateAction {
                revision: frame.page_revision,
                action: CandidateAction::SentenceFocus { at: 5 },
            },
        )
        .frame
        .unwrap();
    configure(&mut host, |c| c.sentence_editor = false);
    let current = host
        .handle(
            1,
            Request::CandidateAction {
                revision: frame.page_revision,
                action: CandidateAction::SentenceFocus { at: 5 },
            },
        )
        .frame
        .unwrap();
    assert!(current.sentence.is_none());
    let first = current.candidates[0].clone();
    let selected = press(&mut host, '1' as u32);
    assert_eq!(selected.commit.as_deref(), Some(first.as_str()));
    cleanup(&host);
}

#[test]
fn accepted_key_preview_exposes_new_model_job_to_the_candidate_window() {
    struct Delayed;
    impl wowtype_core::enhancement::Backend for Delayed {
        fn submit(&self, _: wowtype_core::enhancement::Query) {}
        fn answer(&self, _: &str) -> Option<wowtype_core::enhancement::Answer> {
            None
        }
    }
    let mut host = host("model-preview");
    host.engine
        .as_ref()
        .unwrap()
        .set_enhancement(Some(Arc::new(Delayed)));
    for c in "nihao".chars() {
        let preview = Key {
            keysym: c as u32,
            modifiers: 0,
            test: true,
        };
        host.handle(
            1,
            Request::Key {
                key: preview,
                before: None,
            },
        );
        let result = host
            .handle(
                1,
                Request::Key {
                    key: Key {
                        test: false,
                        ..preview
                    },
                    before: None,
                },
            )
            .frame
            .unwrap();
        if c == 'o' {
            assert!(
                result.enhancement_pending,
                "accepted preview must start polling"
            );
        }
    }
    cleanup(&host);
}

/// 增强结果没到时，轮询应答保持原页（未处理、页码不变），DLL 据此跳过重写编辑区和重画候选窗；
/// 结果到了才换新页码。
#[test]
fn enhancement_poll_keeps_the_page_until_a_result_arrives() {
    use wowtype_core::enhancement::{Answer, Backend, Query};
    #[derive(Default)]
    struct Slow(std::sync::Mutex<Option<Answer>>);
    impl Backend for Slow {
        fn submit(&self, _: Query) {}
        fn answer(&self, _: &str) -> Option<Answer> {
            self.0.lock().unwrap().clone()
        }
    }
    let mut host = host("model-poll");
    let backend = Arc::new(Slow::default());
    host.engine
        .as_ref()
        .unwrap()
        .set_enhancement(Some(backend.clone()));
    let typed = typing(&mut host, "nihao");
    assert!(typed.enhancement_pending);
    let poll = |host: &mut Host, revision| {
        host.handle(1, Request::RefreshEnhancement { revision })
            .frame
            .unwrap()
    };
    let idle = poll(&mut host, typed.page_revision);
    assert!(!idle.handled);
    assert_eq!(idle.page_revision, typed.page_revision);
    assert!(idle.enhancement_pending, "还没有结果时继续轮询");
    *backend.0.lock().unwrap() = Some(Answer {
        key: String::new(),
        text: "拟好".into(),
        pinyin: "ni hao".into(),
        words: vec![("拟好".into(), "ni hao".into())],
        uncertain: vec![false; 2],
    });
    let ready = poll(&mut host, typed.page_revision);
    assert!(ready.handled);
    assert_ne!(ready.page_revision, typed.page_revision);
    assert_eq!(ready.candidates.first().map(String::as_str), Some("拟好"));
    assert!(!ready.enhancement_pending);
    cleanup(&host);
}
