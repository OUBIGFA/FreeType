//! 上屏联想的单次与双击空格在真实 TSF 编辑会话中的行为。
use super::*;

#[test]
fn prediction_first_space_is_owned_before_any_application_key_is_dispatched() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        // Both callbacks are production entry points. The host must never receive
        // the first Space, so it has no raw character or Backspace to dispatch later.
        for _ in 0..2 {
            assert!(sink
                .OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
                .unwrap()
                .as_bool());
        }
        assert!(
            editor.store.text.borrow().is_empty(),
            "previews cannot open a composition"
        );
        assert!(sink
            .OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
            .unwrap()
            .as_bool());
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天"
        );
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), " ");
        assert!(
            editor.store.composing.get(),
            "the real TSF owner must receive OnStartComposition"
        );
        assert!(
            !composing(&state),
            "the engine is still showing predictions, not editing pinyin"
        );
    });
}

#[test]
fn prediction_transitory_commit_keeps_every_character_without_a_deferred_backspace() {
    for word in ["好", "天气", "好运来", "𠮷", "👩‍💻"] {
        in_editor(|editor, state, _| {
            editor.store.transitory.set(true);
            editor.store.commit_only.set(true);
            open_prediction(editor, &state, true);
            assert_eq!(press_space(editor, &state, 1000).keysym, 0x20);
            assert_eq!(
                String::from_utf16_lossy(&editor.store.app_text.borrow()),
                "今天"
            );
            assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), " ");
            choose_space(editor, &state, 1100, word, None);
            assert_eq!(
                String::from_utf16_lossy(&editor.store.app_text.borrow()),
                format!("今天{word}")
            );
            assert!(editor.store.text.borrow().is_empty());
            assert!(
                !editor.store.composing.get(),
                "the result is committed only by OnEndComposition"
            );
            assert!(state.borrow().space.is_none());
        });
    }
}

#[test]
fn prediction_double_space_continues_across_native_and_composition_only_hosts() {
    for composition_only in [false, true] {
        in_editor(|editor, state, _| {
            editor.store.transitory.set(true);
            editor.store.commit_only.set(composition_only);
            open_prediction(editor, &state, true);
            for (ms, word, next, expected) in [
                (1000, "天气", "不错", "今天天气"),
                (2000, "不错", "适合", "今天天气不错"),
                (3000, "适合", "散步", "今天天气不错适合"),
            ] {
                let start = editor.store.text.borrow().len() as i32;
                assert_eq!(press_space(editor, &state, ms).keysym, 0x20);
                choose_space(editor, &state, ms + 100, word, Some(next));
                let buffer = if composition_only {
                    &editor.store.app_text
                } else {
                    &editor.store.text
                };
                assert_eq!(String::from_utf16_lossy(&buffer.borrow()), expected);
                if !composition_only {
                    // A Chromium renderer may replay the committed suffix after
                    // the edit session. Keep following its final caret position.
                    editor.store.app_replace(start, start + 2, "");
                    editor.store.app_insert(word);
                }
            }
        });
    }
}

#[test]
fn prediction_single_tap_timeout_commits_exactly_one_space() {
    in_editor(|editor, state, _| {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        prediction_space_timeout(&state);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天",
            "an early/stale timer cannot end the double-tap window"
        );
        state.borrow_mut().space.as_mut().unwrap().deadline_ms = 0;
        prediction_space_timeout(&state);
        prediction_space_timeout(&state);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天 "
        );
        assert!(!editor.store.composing.get());
    });
}

#[test]
fn prediction_space_window_timer_reaches_the_text_service() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, MSG, PM_REMOVE, WM_TIMER,
    };
    in_editor(|editor, state, _| unsafe {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        state.borrow_mut().space.as_mut().unwrap().deadline_ms = 0;
        let hwnd = state.borrow().window.as_ref().unwrap().hwnd();
        state
            .borrow()
            .window
            .as_ref()
            .unwrap()
            .set_prediction_space_timeout(Some(1));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.borrow().space.is_some() && std::time::Instant::now() < end {
            let mut message = MSG::default();
            if PeekMessageW(&mut message, Some(hwnd), WM_TIMER, WM_TIMER, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天 "
        );
        assert!(!editor.store.composing.get());
    });
}

#[test]
fn prediction_host_termination_finishes_the_owned_space_and_cancels_its_timer() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        let service = ComObject::<TextService>::cast_from(&sink).unwrap();
        press_space(editor, &state, 1000);
        assert!(
            !service.can_reload(),
            "an owned space must finish before replacing the DLL"
        );
        let owner: ITfContextOwnerCompositionServices = editor.context.cast().unwrap();
        owner
            .TerminateComposition(None::<&ITfCompositionView>)
            .unwrap();
        prediction_space_timeout(&state);
        assert!(state.borrow().space.is_none());
        assert!(!state.borrow().predicting);
        assert_eq!(shown_revision(&state), 0);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天 "
        );
        assert!(service.can_reload());
    });
}

#[test]
fn prediction_held_space_commits_the_first_space_and_passes_the_repeat() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        let repeat = LPARAM(1 << 30);
        assert!(sink
            .OnTestKeyDown(&editor.context, WPARAM(0x20), repeat)
            .unwrap()
            .as_bool());
        assert!(!sink
            .OnKeyDown(&editor.context, WPARAM(0x20), repeat)
            .unwrap()
            .as_bool());
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天 "
        );
        assert!(state.borrow().space.is_none());
        assert!(!editor.store.composing.get());
    });
}

#[test]
fn prediction_backspace_cancels_only_the_pending_space() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        assert!(sink
            .OnTestKeyDown(&editor.context, WPARAM(0x08), LPARAM(0))
            .unwrap()
            .as_bool());
        assert!(sink
            .OnKeyDown(&editor.context, WPARAM(0x08), LPARAM(0))
            .unwrap()
            .as_bool());
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天"
        );
        assert!(editor.store.text.borrow().is_empty());
        assert!(!editor.store.composing.get());
    });
}

#[test]
fn prediction_next_input_commits_the_space_without_overwriting_new_composition() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        assert!(sink
            .OnTestKeyDown(&editor.context, WPARAM(0x4e), LPARAM(0))
            .unwrap()
            .as_bool());
        // With a disconnected test server, the next key is passed back after
        // the pending space has been committed. A real response can then compose.
        assert!(!sink
            .OnKeyDown(&editor.context, WPARAM(0x4e), LPARAM(0))
            .unwrap()
            .as_bool());
        apply(
            &state,
            Some(&editor.context),
            Frame {
                handled: true,
                preedit: "n".into(),
                preedit_cursor: 1,
                ..Frame::default()
            },
            true,
        );
        prediction_space_timeout(&state);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天 "
        );
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "n");
        assert!(composing(&state));
    });
}

#[test]
fn prediction_double_space_owns_text_even_without_read_only_locks() {
    in_editor(|editor, state, _| {
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        editor.store.deny_read.set(true);
        press_space(editor, &state, 1000);
        choose_space(editor, &state, 1100, "好运来", None);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天好运来"
        );
    });
}

#[test]
fn prediction_double_space_respects_message_timing_and_duplicate_previews() {
    in_editor(|editor, state, _| {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        let preview = Key {
            keysym: 0x20,
            test: true,
            ..Key::default()
        };
        assert_eq!(
            prediction_space_key(&state, &editor.context, preview, false, 1, 1000)
                .0
                .keysym,
            0x20
        );
        press_space(editor, &state, 1000);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            " ",
            "a duplicate press cannot create two spaces"
        );
        assert_eq!(
            press_space(editor, &state, 2000).keysym,
            0x20,
            "a slow tap commits the old space and starts a new pair"
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天 "
        );
        choose_space(editor, &state, 2100, "好", Some("天气"));
        press_space(editor, &state, u32::MAX - 50);
        choose_space(editor, &state, 49, "天气", None);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "今天 好天气"
        );
    });
}

#[test]
fn prediction_single_space_uses_the_page_only_while_the_caret_matches() {
    in_editor(|editor, state, _| {
        let revision = open_prediction(&editor, &state, false);
        let space = Key {
            keysym: 0x20,
            ..Key::default()
        };
        assert_eq!(
            prediction_space_key(&state, &editor.context, space, false, revision, 1000),
            (space, true)
        );
        editor.store.move_caret(1);
        assert_eq!(
            prediction_space_key(&state, &editor.context, space, false, revision, 1100),
            (space, false)
        );
        assert_eq!(shown_revision(&state), 0);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "今天"
        );
    });
}

#[test]
fn prediction_double_space_does_not_replace_text_after_a_caret_move() {
    in_editor(|editor, state, _| {
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        editor.store.move_caret(1);
        let space = Key {
            keysym: 0x20,
            test: true,
            ..Key::default()
        };
        assert_eq!(
            prediction_space_key(&state, &editor.context, space, false, 1, 1100),
            (space, false)
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "今天 "
        );
        assert_eq!(
            editor.store.selection.get().acpStart,
            1,
            "ending a space must not move the user's caret back"
        );
        assert!(!editor.store.composing.get());
    });
}

#[test]
fn prediction_does_not_follow_focus_to_another_input_context() {
    in_editor(|editor, state, sink| unsafe {
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        let other = editor.manager.CreateDocumentMgr().unwrap();
        let mut context = None;
        let mut cookie = 0;
        other
            .CreateContext(
                editor.client,
                0,
                None::<&IUnknown>,
                &mut context,
                &mut cookie,
            )
            .unwrap();
        let context = context.unwrap();
        other.Push(&context).unwrap();
        on_focus(&state, Some(&other));
        let closed = shown_revision(&state) == 0;
        let esc_reaches_app = !sink
            .OnTestKeyDown(&context, WPARAM(0x1b), LPARAM(0))
            .unwrap()
            .as_bool();
        other.Pop(TF_POPF_ALL).unwrap();
        assert!(closed && esc_reaches_app);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "今天 "
        );
        assert!(state.borrow().space.is_none() && !editor.store.composing.get());
    });
}

#[test]
fn prediction_escape_owns_down_repeat_and_release_but_not_the_next_press() {
    in_editor(|editor, state, sink| unsafe {
        let esc = WPARAM(0x1b);
        let down = LPARAM(0);
        let repeat = LPARAM(1 << 30);
        let up = LPARAM((3u32 << 30) as isize);
        // Direct KeyDown, duplicate TestKeyDown, and TestKeyDown with the actual down omitted.
        for (preview, actual) in [(false, true), (true, true), (true, false)] {
            open_prediction(&editor, &state, false);
            let modified = Key {
                keysym: keysym::ESCAPE,
                modifiers: modifier::SHIFT,
                test: true,
            };
            assert!(!prediction_escape_down(
                &state,
                Some(&editor.context),
                modified,
                false
            ));
            if preview {
                assert!(sink
                    .OnTestKeyDown(&editor.context, esc, down)
                    .unwrap()
                    .as_bool());
                assert!(sink
                    .OnTestKeyDown(&editor.context, esc, down)
                    .unwrap()
                    .as_bool());
            }
            if actual {
                assert!(sink
                    .OnKeyDown(&editor.context, esc, down)
                    .unwrap()
                    .as_bool());
                assert_eq!(shown_revision(&state), 0);
                assert!(sink
                    .OnKeyDown(&editor.context, esc, repeat)
                    .unwrap()
                    .as_bool());
            }
            assert!(sink
                .OnTestKeyUp(&editor.context, esc, up)
                .unwrap()
                .as_bool());
            assert!(sink
                .OnTestKeyUp(&editor.context, esc, up)
                .unwrap()
                .as_bool());
            assert!(sink.OnKeyUp(&editor.context, esc, up).unwrap().as_bool());
            assert_eq!(shown_revision(&state), 0);
            assert!(!sink
                .OnTestKeyDown(&editor.context, esc, down)
                .unwrap()
                .as_bool());
            assert!(!sink
                .OnTestKeyUp(&editor.context, esc, up)
                .unwrap()
                .as_bool());
        }
        assert_eq!(
            String::from_utf16_lossy(&editor.store.text.borrow()),
            "今天今天今天"
        );
    });
}
