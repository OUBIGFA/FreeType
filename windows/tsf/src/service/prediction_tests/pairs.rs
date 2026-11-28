//! 成对标点：补全、跳过、延迟左移与快速连打。
use super::*;

#[test]
fn inline_commit_before_a_terminal_pair_finishes_preedit_before_inserting_the_word() {
    for (transient, suffix, split) in [
        (true, "", true),
        (true, "\n\n", true),
        (true, "乙", false),
        (true, "\n乙", false),
        (false, "", false),
    ] {
        in_editor(|editor, state, _| {
            editor.store.transitory.set(transient);
            editor.store.app_insert(&format!("甲{suffix}"));
            editor.store.move_caret(1);
            apply(
                &state,
                Some(&editor.context),
                Frame {
                    handled: true,
                    commit: Some("「」".into()),
                    caret_back: 1,
                    pair_enabled: true,
                    ..Frame::default()
                },
                true,
            );
            editor.store.move_caret(2);
            pair::finish(&state, true);
            apply(
                &state,
                Some(&editor.context),
                Frame {
                    handled: true,
                    preedit: "shen me".into(),
                    preedit_cursor: 7,
                    pair_enabled: true,
                    ..Frame::default()
                },
                true,
            );
            assert!(editor.store.composing.get());
            *editor.store.completed_writes.borrow_mut() = Some(Vec::new());
            apply(
                &state,
                Some(&editor.context),
                Frame {
                    handled: true,
                    commit: Some("什么".into()),
                    candidates: vec!["意思".into()],
                    predict: true,
                    pair_enabled: true,
                    ..Frame::default()
                },
                true,
            );
            let writes = editor.store.completed_writes.borrow_mut().take().unwrap();
            let base = format!("甲「」{suffix}");
            assert_eq!(writes.iter().any(|(text, composing)| text == &base && !composing), split,
                "only a composing transient editor at its terminal closing mark needs separate host writes: {writes:?}");
            assert_eq!(
                String::from_utf16_lossy(&editor.store.text.borrow()),
                format!("甲「什么」{suffix}")
            );
            assert_eq!(editor.store.selection.get().acpStart, 4);
            assert!(state.borrow().predicting && !editor.store.composing.get());
            editor.store.app_insert("好");
            assert_eq!(
                String::from_utf16_lossy(&editor.store.text.borrow()),
                format!("甲「什么好」{suffix}")
            );
        });
    }
}

#[test]
fn paired_quote_survives_same_field_refocus_and_the_next_letter_preview() {
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        editor.store.transitory.set(true);
        editor.store.app_insert("前文后文");
        editor.store.move_caret(2);
        apply(
            &state,
            Some(&editor.context),
            Frame {
                handled: true,
                commit: Some("“”".into()),
                caret_back: 1,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        editor.store.move_caret(4);
        on_focus(&state, Some(&editor.document));
        assert!(
            state.borrow().caret_move.is_some(),
            "the same field did not lose focus"
        );
        assert!(sink
            .OnTestKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
            .unwrap()
            .as_bool());
        let pending = state
            .borrow_mut()
            .caret_move
            .take()
            .expect("a preview must not cancel the posted arrow");
        assert_eq!(
            pending.step(now_ms(), true, false),
            keys::CaretStep::Send(1)
        );
        editor
            .store
            .move_caret(editor.store.selection.get().acpStart - 1);
        assert!(sink
            .OnKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
            .unwrap()
            .as_bool());
        for vk in [0x45, 0x53, 0x48, 0x49, 0x20] {
            assert!(sink
                .OnTestKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
        }
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "前文“测试”后文"
        );
    });
}

#[test]
fn prepared_terminal_commit_survives_a_lock_refusal_without_writing_at_a_moved_caret() {
    in_editor(|editor, state, _| {
        editor.store.transitory.set(true);
        editor.store.app_insert("甲");
        apply(
            &state,
            Some(&editor.context),
            Frame {
                commit: Some("「」".into()),
                caret_back: 1,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        editor.store.move_caret(2);
        pair::finish(&state, true);
        apply(
            &state,
            Some(&editor.context),
            Frame {
                preedit: "shen me".into(),
                preedit_cursor: 7,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        let frame = Rc::new(Frame {
            handled: true,
            commit: Some("什么".into()),
            pair_enabled: true,
            ..Frame::default()
        });
        let pair = pair::prepare_commit(&state, &editor.context, &frame, editor.client).unwrap();
        editor.store.move_caret(0);
        editor.store.deny_next_write.set(true);
        pair::commit_prepared(&state, pair, frame, editor.client);
        assert!(!editor.store.deny_next_write.get());
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "甲「什么」"
        );
        assert_eq!(editor.store.selection.get().acpStart, 0);
    });
}

#[test]
fn terminal_pair_release_waits_if_the_text_store_still_places_the_caret_outside() {
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        editor.store.transitory.set(true);
        editor.store.app_insert("甲");
        apply(
            &state,
            Some(&editor.context),
            Frame {
                commit: Some("「」".into()),
                caret_back: 1,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        editor.store.move_caret(3);
        assert!(sink
            .OnTestKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
            .unwrap()
            .as_bool());
        assert!(sink
            .OnKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
            .unwrap()
            .as_bool());
        state.borrow_mut().pair_delivery.sent = true;
        let release = || {
            let old = windows::Win32::UI::WindowsAndMessaging::SetMessageExtraInfo(LPARAM(
                keys::CARET_END_TAG as isize,
            ));
            let result = sink.OnTestKeyUp(&editor.context, WPARAM(0x25), LPARAM(0));
            windows::Win32::UI::WindowsAndMessaging::SetMessageExtraInfo(old);
            assert!(!result.unwrap().as_bool());
        };
        release();
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "甲「」",
            "KeyUp is not a renderer acknowledgement"
        );
        editor.store.move_caret(2);
        release();
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "甲「c」"
        );
    });
}

#[test]
fn fast_typing_cannot_write_preedit_or_commit_ahead_of_the_pair_arrow() {
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        editor.store.transitory.set(true);
        editor.store.app_insert("前文后文");
        editor.store.move_caret(2);
        apply(
            &state,
            Some(&editor.context),
            Frame {
                commit: Some("“”".into()),
                caret_back: 1,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        editor.store.move_caret(4);
        for vk in [0x43, 0x45, 0x53, 0x48, 0x49, 0x20] {
            assert!(sink
                .OnTestKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
            assert_eq!(
                String::from_utf16_lossy(&editor.store.text.borrow()),
                "前文“”后文",
                "the renderer has not received Left yet"
            );
            if vk == 0x43 {
                assert!(
                    state.borrow().window.as_ref().unwrap().visible(),
                    "candidates must not wait for Left"
                );
            }
        }
        editor.store.move_caret(3);
        assert!(
            pair::placement(&state) == pair::Placement::Inside,
            "a renderer that omits KeyUp can acknowledge through its actual caret"
        );
        state.borrow_mut().pair_delivery.sent = true;
        let previous = windows::Win32::UI::WindowsAndMessaging::SetMessageExtraInfo(LPARAM(
            keys::CARET_END_TAG as isize,
        ));
        let released = sink.OnTestKeyUp(&editor.context, WPARAM(0x25), LPARAM(0));
        windows::Win32::UI::WindowsAndMessaging::SetMessageExtraInfo(previous);
        assert!(!released.unwrap().as_bool());
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "前文“测试”后文"
        );
    });
}

#[test]
fn pending_pair_movement_never_follows_navigation_or_a_lost_thread() {
    in_editor(|editor, state, sink| unsafe {
        for leave in [false, true] {
            apply(
                &state,
                Some(&editor.context),
                Frame {
                    handled: true,
                    commit: Some("（）".into()),
                    caret_back: 1,
                    pair_enabled: true,
                    ..Frame::default()
                },
                true,
            );
            assert!(state.borrow().caret_move.is_some());
            if leave {
                sink.OnSetFocus(false).unwrap();
            } else {
                note_key_down(&state, WPARAM(0x23), LPARAM(0));
            }
            assert!(
                state.borrow().caret_move.is_none(),
                "no arrow may land at a different caret"
            );
        }
    });
}

#[test]
fn deferred_pair_commit_survives_a_host_that_refuses_synchronous_writes() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, MSG, PM_REMOVE};
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        apply(
            &state,
            Some(&editor.context),
            Frame {
                commit: Some("（）".into()),
                caret_back: 1,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        for vk in [0x43, 0x45, 0x53, 0x48, 0x49, 0x20] {
            assert!(sink
                .OnTestKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
        }
        editor.store.move_caret(1);
        // Timer/KeyUp delivery is outside the synchronous OnKeyDown guarantee.
        editor.store.deny_sync_write.set(true);
        pair::finish(&state, true);
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            DispatchMessageW(&message);
        }
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "（测试）",
            "already accepted keys must not disappear when the host requires asynchronous delivery"
        );
    });
}

#[test]
fn passed_through_typing_cancels_the_pending_pair_arrow() {
    for preview in [true, false] {
        in_editor(|editor, state, sink| unsafe {
            use_test_host(&state);
            apply(
                &state,
                Some(&editor.context),
                Frame {
                    commit: Some("（）".into()),
                    caret_back: 1,
                    pair_enabled: true,
                    ..Frame::default()
                },
                true,
            );
            assert!(state.borrow().caret_move.is_some());
            let handled = if preview {
                sink.OnTestKeyDown(&editor.context, WPARAM(0x31), LPARAM(0))
            } else {
                sink.OnKeyDown(&editor.context, WPARAM(0x31), LPARAM(0))
            };
            assert!(
                !handled.unwrap().as_bool(),
                "a plain digit outside composition belongs to the application"
            );
            editor.store.app_insert("1");
            assert!(
                state.borrow().caret_move.is_none(),
                "a delayed Left must not move the caret after passed-through text"
            );
            assert_eq!(
                String::from_utf16_lossy(&editor.store.text.borrow()),
                "（）1"
            );
        });
    }
}

/// Issue #13：习惯连打左右两半的用户，在补全的左移键送达前就按下右半标点。
/// 这时光标还在补全的右标点后面：右键只被吞掉，不插入第二个右标点；
/// 待发的左移键照常保留，光标最后仍回到两半之间。同一次补全只吞一次右键。
#[test]
fn closing_key_before_the_pair_arrow_lands_keeps_a_single_closing_mark() {
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        apply(
            &state,
            Some(&editor.context),
            Frame {
                commit: Some("【】".into()),
                caret_back: 1,
                pair_enabled: true,
                pair_absorb_closing: true,
                ..Frame::default()
            },
            true,
        );
        assert!(state.borrow().caret_move.is_some());
        assert_eq!(
            editor.store.selection.get().acpStart,
            2,
            "Left not delivered yet"
        );
        let tested = sink
            .OnTestKeyDown(&editor.context, WPARAM(0xDD), LPARAM(0))
            .unwrap()
            .as_bool();
        let handled = sink
            .OnKeyDown(&editor.context, WPARAM(0xDD), LPARAM(0))
            .unwrap()
            .as_bool();
        assert!(
            tested && handled,
            "the closing key must not reach the application"
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "【】"
        );
        assert!(
            state.borrow().caret_move.is_some(),
            "the pending Left must still bring the caret between the halves"
        );
        let closing = Key {
            keysym: u32::from(b']'),
            ..Key::default()
        };
        assert!(
            verified_pair_key(&state, Some(&editor.context), closing).is_none(),
            "a second closing key is ordinary input"
        );
        // 左移键送达：光标回到两半之间，成对标点的位置仍在，之后的右键照常跳过。
        editor.store.move_caret(1);
        pair::finish(&state, true);
        assert!(sink
            .OnKeyDown(&editor.context, WPARAM(0xDD), LPARAM(0))
            .unwrap()
            .as_bool());
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "【】"
        );
        assert_eq!(editor.store.selection.get().acpStart, 2);
    });
}

/// 左移键已送达、光标在两半之间时，按右半标点只跳过已有的右标点。
#[test]
fn closing_key_inside_a_pair_skips_the_existing_closing_mark() {
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        apply(
            &state,
            Some(&editor.context),
            Frame {
                commit: Some("【】".into()),
                caret_back: 1,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        editor.store.move_caret(1);
        pair::finish(&state, true);
        assert!(sink
            .OnTestKeyDown(&editor.context, WPARAM(0xDD), LPARAM(0))
            .unwrap()
            .as_bool());
        assert!(sink
            .OnKeyDown(&editor.context, WPARAM(0xDD), LPARAM(0))
            .unwrap()
            .as_bool());
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "【】"
        );
        assert_eq!(editor.store.selection.get().acpStart, 2);
    });
}

/// 「连打两半时不重复右标点」默认关闭：关闭时右键照常交给服务进程，不撤销待发的左移键。
#[test]
fn closing_key_before_the_pair_arrow_is_ordinary_input_when_the_switch_is_off() {
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        assert!(!freetype_protocol::Config::default().auto_pair_absorb_closing);
        apply(
            &state,
            Some(&editor.context),
            Frame {
                commit: Some("【】".into()),
                caret_back: 1,
                pair_enabled: true,
                ..Frame::default()
            },
            true,
        );
        let _ = sink
            .OnTestKeyDown(&editor.context, WPARAM(0xDD), LPARAM(0))
            .unwrap();
        let _ = sink
            .OnKeyDown(&editor.context, WPARAM(0xDD), LPARAM(0))
            .unwrap();
        assert!(
            state.borrow().caret_move.is_some(),
            "without the switch the pending Left stays scheduled"
        );
    });
}
