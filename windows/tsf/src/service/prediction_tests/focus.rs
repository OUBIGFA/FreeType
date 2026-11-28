//! 焦点、候选窗归属、焦点提示与敏感输入框。
use super::*;

#[test]
fn candidate_window_belongs_to_the_current_text_view() {
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_OWNER, WS_POPUP};
    in_editor(|editor, state, _| unsafe {
        // Real HWNDs and TSF views exercise the ownership contract required by
        // Windows shell input fields, without touching the user's desktop.
        let first = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("STATIC"),
            None,
            WS_POPUP,
            40,
            40,
            500,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let second = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("STATIC"),
            None,
            WS_POPUP,
            40,
            180,
            500,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let frame = Frame {
            candidates: vec!["测试".into()],
            cursor: Some(0),
            font_size: 18,
            ..Frame::default()
        };
        state.borrow_mut().caret = Some(RECT {
            left: 50,
            top: 50,
            right: 52,
            bottom: 70,
        });
        editor.store.window.set(first);
        show_window(&state, &frame);
        let candidate = state.borrow().window.as_ref().unwrap().hwnd();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_eq!(
                GetWindow(candidate, GW_OWNER).unwrap_or_default(),
                first,
                "候选窗必须关联到 TSF 当前输入窗口，才能显示在开始菜单等界面上方"
            );
            show_window(&state, &frame);
            assert_eq!(
                state.borrow().window.as_ref().unwrap().hwnd(),
                candidate,
                "同一输入窗口连续刷新不能重建候选窗"
            );
            editor.store.window.set(second);
            show_window(&state, &frame);
            assert_eq!(GetWindow(candidate, GW_OWNER).unwrap(), second);
            // Null and failed GetWnd both fall back to this thread's focus,
            // never to an unrelated foreground application.
            let _ = SetFocus(Some(first));
            assert_eq!(GetFocus(), first);
            for fails in [false, true] {
                editor.store.window.set(HWND::default());
                editor.store.fail_window.set(fails);
                show_window(&state, &frame);
                assert_eq!(GetWindow(candidate, GW_OWNER).unwrap(), first);
                assert_eq!(GetFocus(), first, "候选窗显示不能抢走输入焦点");
            }
            show_window(&state, &Frame::default());
            assert!(!state.borrow().window.as_ref().unwrap().visible());
            assert!(
                GetWindow(candidate, GW_OWNER).is_err(),
                "隐藏后解除宿主关联"
            );
        }));
        // Detach before destroying test owners; Windows destroys owned popups too.
        editor.store.window.set(HWND::default());
        show_window(&state, &Frame::default());
        DestroyWindow(first).unwrap();
        DestroyWindow(second).unwrap();
        if let Err(error) = outcome {
            std::panic::resume_unwind(error);
        }
    });
}

#[test]
fn candidate_window_recovers_after_its_input_host_is_destroyed() {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, IsWindow, GW_OWNER, WS_POPUP};
    in_editor(|editor, state, _| unsafe {
        let host = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("STATIC"),
            None,
            WS_POPUP,
            40,
            40,
            500,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        editor.store.window.set(host);
        state.borrow_mut().caret = Some(RECT {
            left: 50,
            top: 50,
            right: 52,
            bottom: 70,
        });
        let frame = Frame {
            candidates: vec!["测试".into()],
            cursor: Some(0),
            font_size: 18,
            ..Frame::default()
        };
        show_window(&state, &frame);
        let candidate = state.borrow().window.as_ref().unwrap().hwnd();
        DestroyWindow(host).unwrap();
        assert!(
            !IsWindow(Some(candidate)).as_bool(),
            "Windows 会连带销毁所属弹窗"
        );
        let next = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("STATIC"),
            None,
            WS_POPUP,
            40,
            40,
            500,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        editor.store.window.set(next);
        show_window(&state, &frame);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let s = state.borrow();
            let window = s.window.as_ref().unwrap();
            assert!(
                window.visible(),
                "输入窗口关闭后，在另一个窗口继续输入仍须显示候选"
            );
            assert_eq!(GetWindow(window.hwnd(), GW_OWNER).unwrap(), next);
        }));
        show_window(&state, &Frame::default());
        DestroyWindow(next).unwrap();
        if let Err(error) = outcome {
            std::panic::resume_unwind(error);
        }
    });
}

#[test]
fn enabled_transitory_editor_does_not_inherit_disabled_focus() {
    for flag in [
        GUID_COMPARTMENT_EMPTYCONTEXT,
        GUID_COMPARTMENT_KEYBOARD_DISABLED,
    ] {
        in_editor(|editor, state, sink| unsafe {
            use_test_host(&state);
            editor.store.transitory.set(true);
            editor.store.commit_only.set(true);
            // Model an enabled edit context following a disabled/non-editing
            // surface, including the same temporary-context identity.
            let compartments: ITfCompartmentMgr = editor.context.cast().unwrap();
            let empty = compartments.GetCompartment(&flag).unwrap();
            empty.SetValue(editor.client, &VARIANT::from(1i32)).unwrap();
            assert!(!sink
                .OnTestKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
                .unwrap()
                .as_bool());
            empty.SetValue(editor.client, &VARIANT::from(0i32)).unwrap();
            editor.store.deny_read.set(true);
            for vk in [0x43, 0x45, 0x53, 0x48, 0x49, 0x20] {
                assert!(sink.OnTestKeyDown(&editor.context, WPARAM(vk), LPARAM(0)).unwrap().as_bool(),
                "enabled RichEdit/IMM32 text must resume Chinese input even without a read-only lock");
                assert!(sink
                    .OnKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                    .unwrap()
                    .as_bool());
            }
            assert_eq!(
                String::from_utf16_lossy(&editor.store.app_text.borrow()),
                "测试"
            );
        });
    }
}

#[test]
fn transitory_hosts_place_their_own_caret_after_a_commit() {
    // Codex/Chromium message editing: after a paired quote, the web caret sits
    // between the halves while the TSF selection can still be at the end.
    // Selecting the commit end would move the caret outside the closing half.
    for transitory in [true, false] {
        in_editor(|editor, state, sink| unsafe {
            use_test_host(&state);
            editor.store.transitory.set(transitory);
            for vk in [0x43, 0x45, 0x53, 0x48, 0x49] {
                assert!(sink
                    .OnTestKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                    .unwrap()
                    .as_bool());
                assert!(sink
                    .OnKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                    .unwrap()
                    .as_bool());
            }
            editor.store.selection_sets.set(0);
            assert!(sink
                .OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(String::from_utf16_lossy(&editor.store.text.borrow()).contains("测试"));
            assert_eq!(
                editor.store.selection_sets.get() == 0,
                transitory,
                "only transitory hosts keep their own caret after a plain commit"
            );
        });
    }
}

#[test]
fn focus_hint_distinguishes_documents_sharing_a_window() {
    in_editor(|editor, state, _| unsafe {
        use_test_host(&state);
        let mut config = state
            .borrow_mut()
            .client
            .call(&Request::GetConfig)
            .unwrap()
            .config
            .unwrap();
        config.hint_on_focus = true;
        assert!(
            state
                .borrow_mut()
                .client
                .call(&Request::SetConfig { config })
                .unwrap()
                .ok
        );
        on_focus(&state, Some(&editor.document));
        focus_notice::present(&state, Some(&editor.context));
        assert!(state.borrow().window.as_ref().unwrap().visible());
        let other = Editor::new();
        on_focus(&state, Some(&other.document));
        focus_notice::present(&state, Some(&other.context));
        assert!(
            state.borrow().window.as_ref().unwrap().visible(),
            "another text document needs its own focus hint even in the same host window"
        );
        with_window(&state, |window| window.hide());
        on_focus(&state, Some(&other.document));
        focus_notice::present(&state, Some(&other.context));
        assert!(
            !state.borrow().window.as_ref().unwrap().visible(),
            "repeated focus in that document must still be quiet"
        );
        state.borrow_mut().context = Some(editor.context.clone());
    });
}

#[test]
fn refocusing_uses_the_current_insertion_point_and_never_the_old_hint_position() {
    in_editor(|editor, state, _| unsafe {
        use_test_host(&state);
        let mut config = state
            .borrow_mut()
            .client
            .call(&Request::GetConfig)
            .unwrap()
            .config
            .unwrap();
        config.hint_on_focus = true;
        config.hint_position = "top-left".into();
        assert!(
            state
                .borrow_mut()
                .client
                .call(&Request::SetConfig { config })
                .unwrap()
                .ok
        );
        on_focus(&state, None);
        on_focus(&state, Some(&editor.document));
        let actual = RECT {
            left: 415,
            top: 320,
            right: 416,
            bottom: 343,
        };
        editor.store.extent.set(Some(actual));
        focus_notice::present(&state, Some(&editor.context));
        assert_eq!(
            state.borrow().caret,
            Some(actual),
            "the caret moved after focus arrived"
        );
        let hwnd = state.borrow().window.as_ref().unwrap().hwnd();
        let mut rect = RECT::default();
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect).unwrap();
        assert!(
            rect.right <= actual.left && rect.bottom <= actual.top,
            "configured top-left: {rect:?}"
        );
        on_focus(&state, None);
        on_focus(&state, Some(&editor.document));
        editor.store.clipped.set(true);
        editor.store.extent.set(Some(RECT::default()));
        focus_notice::present(&state, Some(&editor.context));
        assert!(
            !state.borrow().window.as_ref().unwrap().visible(),
            "no visible caret: do not guess a position"
        );
    });
}

#[test]
fn focus_hint_is_not_shown_immediately_and_recognizes_thread_focus_return() {
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        let mut config = state
            .borrow_mut()
            .client
            .call(&Request::GetConfig)
            .unwrap()
            .config
            .unwrap();
        config.hint_on_focus = true;
        assert!(
            state
                .borrow_mut()
                .client
                .call(&Request::SetConfig { config })
                .unwrap()
                .ok
        );
        on_focus(&state, Some(&editor.document));
        assert!(
            !state.borrow().window.as_ref().unwrap().visible(),
            "wait for the switch to settle"
        );
        let thread: ITfThreadFocusSink = sink
            .cast()
            .expect("some window switches report thread focus only");
        let source: ITfSource = editor.manager.cast().unwrap();
        let cookie = source
            .AdviseSink(&ITfThreadFocusSink::IID, &thread)
            .unwrap();
        thread.OnKillThreadFocus().unwrap();
        thread.OnSetThreadFocus().unwrap();
        assert!(!state.borrow().window.as_ref().unwrap().visible());
        focus_notice::present(&state, Some(&editor.context));
        assert!(
            state.borrow().window.as_ref().unwrap().visible(),
            "a thread-only return must show the enabled hint"
        );
        with_window(&state, |window| window.hide());
        on_focus(&state, Some(&editor.document));
        focus_notice::present(&state, Some(&editor.context));
        assert!(
            !state.borrow().window.as_ref().unwrap().visible(),
            "the same field must not announce on every edit"
        );
        thread.OnKillThreadFocus().unwrap();
        thread.OnSetThreadFocus().unwrap();
        assert!(sink
            .OnTestKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
            .unwrap()
            .as_bool());
        assert!(sink
            .OnKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
            .unwrap()
            .as_bool());
        focus_notice::present(&state, Some(&editor.context));
        assert!(
            state.borrow().composition.is_some()
                && state.borrow().window.as_ref().unwrap().visible()
        );
        source.UnadviseSink(cookie).unwrap();
    });
}

#[test]
fn focus_hint_requires_a_visible_insertion_point() {
    in_editor(|editor, state, _| {
        use_test_host(&state);
        let mut config = state
            .borrow_mut()
            .client
            .call(&Request::GetConfig)
            .unwrap()
            .config
            .unwrap();
        config.hint_on_focus = true;
        assert!(
            state
                .borrow_mut()
                .client
                .call(&Request::SetConfig { config })
                .unwrap()
                .ok
        );
        let caret = RECT {
            left: 100,
            top: 100,
            right: 101,
            bottom: 120,
        };
        for (extent, clipped, visible) in [
            (caret, true, false),
            (RECT::default(), false, false),
            (caret, false, true),
        ] {
            on_focus(&state, None);
            on_focus(&state, Some(&editor.document));
            editor.store.extent.set(Some(extent));
            editor.store.clipped.set(clipped);
            focus_notice::present(&state, Some(&editor.context));
            assert_eq!(
                state.borrow().window.as_ref().unwrap().visible(),
                visible,
                "a clipped or missing caret must not trigger a focus hint"
            );
        }
        on_focus(&state, None);
        focus_notice::present(&state, None);
        assert!(
            !state.borrow().window.as_ref().unwrap().visible(),
            "leaving the editor must hide its hint"
        );
    });
}

#[test]
fn focus_hint_does_not_announce_a_retained_editor_while_a_popup_menu_is_open() {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EndMenu, KillTimer, SetTimer, GUITHREADINFO, GUI_INMENUMODE, WS_POPUP, WS_VISIBLE,
    };
    thread_local! {
        static INSPECT: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    }
    unsafe extern "system" fn inspect(_: HWND, _: u32, timer: usize, _: u32) {
        let _ = KillTimer(None, timer);
        INSPECT.with(|slot| {
            if let Some(inspect) = slot.borrow_mut().take() {
                inspect();
            }
        });
        let _ = EndMenu();
    }

    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        let mut config = state
            .borrow_mut()
            .client
            .call(&Request::GetConfig)
            .unwrap()
            .config
            .unwrap();
        config.hint_on_focus = true;
        assert!(
            state
                .borrow_mut()
                .client
                .call(&Request::SetConfig { config })
                .unwrap()
                .ok
        );
        let thread: ITfThreadFocusSink = sink.cast().unwrap();
        let owner = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("STATIC"),
            None,
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            100,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let menu = CreatePopupMenu().unwrap();
        AppendMenuW(menu, MF_STRING, 1, windows::core::w!("Test action")).unwrap();
        let mut results = Vec::new();
        // A tray menu can retain both the old TSF context and a valid text extent.
        // Exercise a captured focus response as well as a thread-only return.
        for captured in [true, false] {
            with_window(&state, |window| window.hide());
            on_focus(&state, None);
            if captured {
                on_focus(&state, Some(&editor.document));
            } else {
                state.borrow_mut().context = Some(editor.context.clone());
                thread.OnKillThreadFocus().unwrap();
                thread.OnSetThreadFocus().unwrap();
            }
            let outcome = Rc::new(Cell::new(None));
            let (result, shared, context) =
                (outcome.clone(), state.clone(), editor.context.clone());
            INSPECT.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    let mut info = GUITHREADINFO {
                        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                        ..Default::default()
                    };
                    let menu_open = GetGUIThreadInfo(GetCurrentThreadId(), &mut info).is_ok()
                        && info.flags.contains(GUI_INMENUMODE);
                    focus_notice::present(&shared, Some(&context));
                    result.set(Some((
                        menu_open,
                        shared.borrow().window.as_ref().unwrap().visible(),
                    )));
                }))
            });
            let timer = SetTimer(None, 0, 50, Some(inspect));
            assert_ne!(timer, 0);
            let _ = TrackPopupMenu(
                menu,
                TPM_NONOTIFY | TPM_RETURNCMD | TPM_RIGHTBUTTON,
                20,
                20,
                None,
                owner,
                None,
            );
            let _ = KillTimer(None, timer);
            INSPECT.with(|slot| *slot.borrow_mut() = None);
            results.push(outcome.get());
        }
        DestroyMenu(menu).unwrap();
        DestroyWindow(owner).unwrap();
        assert_eq!(
            results,
            [Some((true, false)), Some((true, false))],
            "a real popup menu must suppress the hint even if the old editor still answers"
        );
        thread.OnKillThreadFocus().unwrap();
        thread.OnSetThreadFocus().unwrap();
        focus_notice::present(&state, Some(&editor.context));
        assert!(
            state.borrow().window.as_ref().unwrap().visible(),
            "returning to the editor must still show the hint"
        );
    });
}

#[test]
fn layout_notice_reentering_during_state_or_window_update_does_not_panic() {
    // Notepad's RichEdit answers candidate-window SetWindowPos with a synchronous
    // OnLayoutChange. A RefCell panic there aborts the host process.
    in_editor(|editor, state, sink| unsafe {
        use_test_host(&state);
        for vk in [0x43, 0x45] {
            assert!(sink
                .OnTestKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnKeyDown(&editor.context, WPARAM(vk), LPARAM(0))
                .unwrap()
                .as_bool());
        }
        assert!(state.borrow().composition.is_some());
        let layout: ITfTextLayoutSink = sink.cast().unwrap();
        {
            let _held = state.borrow_mut();
            layout
                .OnLayoutChange(&editor.context, TF_LC_CHANGE, None)
                .unwrap();
        }
        let reentered = with_window(&state, |_| {
            layout
                .OnLayoutChange(&editor.context, TF_LC_CHANGE, None)
                .unwrap();
            state.borrow_mut().caret = Some(RECT {
                left: 10,
                top: 10,
                right: 12,
                bottom: 30,
            });
        });
        assert!(
            reentered.is_some(),
            "the candidate window must be returned after the call"
        );
        assert!(state.borrow().window.is_some());
    });
}

#[test]
fn password_scope_survives_read_lock_failure_but_not_a_different_editor() {
    for scope in [
        IS_PASSWORD,
        IS_NUMERIC_PASSWORD,
        IS_NUMERIC_PIN,
        IS_ALPHANUMERIC_PIN,
        IS_ALPHANUMERIC_PIN_SET,
    ] {
        in_editor(|editor, state, sink| unsafe {
            use_test_host(&state);
            editor.store.scope.set(Some(scope));
            assert!(!sink
                .OnTestKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(
                state.borrow().sensitive_context.is_some(),
                "password scope must be read through the real TSF property"
            );
            editor.store.deny_read.set(true);
            assert!(
                !sink
                    .OnTestKeyDown(&editor.context, WPARAM(0x43), LPARAM(0))
                    .unwrap()
                    .as_bool(),
                "a password remains protected when its read lock is denied"
            );
            let other = Editor::new();
            other.store.deny_read.set(true);
            assert!(
                sink.OnTestKeyDown(&other.context, WPARAM(0x43), LPARAM(0))
                    .unwrap()
                    .as_bool(),
                "a different enabled editor cannot inherit another field's password scope"
            );
        });
    }
}
