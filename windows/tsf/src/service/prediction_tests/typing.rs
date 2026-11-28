//! 数字占键、Ctrl+空格与中英文间自动空格。
use super::*;

#[test]
fn ctrl_space_next_press_switches_even_when_the_host_omits_key_up() {
    in_editor(|editor, state, sink| unsafe {
        enable_ctrl_space(&state);
        let space = WPARAM(0x20);
        let mut previous = None;
        for expected in [Mode::English, Mode::Chinese, Mode::English, Mode::Chinese] {
            previous = Some(next_key_message(previous));
            let _pressed = TestKeyboard::ctrl_space();
            assert!(sink
                .OnTestKeyDown(&editor.context, space, LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnTestKeyDown(&editor.context, space, LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnKeyDown(&editor.context, space, LPARAM(0))
                .unwrap()
                .as_bool());
            assert_eq!(
                state.borrow().mode,
                expected,
                "a new press must not inherit the previous press's duplicate guard"
            );
            assert!(sink
                .OnKeyDown(&editor.context, space, LPARAM(0))
                .unwrap()
                .as_bool());
            assert_eq!(
                state.borrow().mode,
                expected,
                "duplicate callbacks must not switch back"
            );
            previous = Some(next_key_message(previous));
            assert!(sink
                .OnKeyDown(&editor.context, space, LPARAM(1 << 30))
                .unwrap()
                .as_bool());
            assert_eq!(
                state.borrow().mode,
                expected,
                "holding Space must not keep switching"
            );
        }
    });
}

#[test]
fn ctrl_space_system_toggle_reopens_the_keyboard_after_notification() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, MSG, PM_REMOVE};
    // Inject only the physical Ctrl+Space observation: GetAsyncKeyState is unavailable
    // on a private desktop. The compartment notification, handler and server are real.
    #[implement(ITfCompartmentEventSink)]
    struct SystemCtrlSpace {
        state: Rc<RefCell<State>>,
        compartments: ITfCompartmentMgr,
    }
    impl ITfCompartmentEventSink_Impl for SystemCtrlSpace_Impl {
        fn OnChange(&self, _: *const GUID) -> Result<()> {
            if compartment_value(&self.compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
                == Some(0)
            {
                ctrl_space_closed(&self.state);
            }
            Ok(())
        }
    }
    in_editor(|editor, state, sink| unsafe {
        enable_ctrl_space(&state);
        let manager: ITfThreadMgr = editor.manager.cast().unwrap();
        state.borrow_mut().thread_mgr = Some(manager.clone());
        set_keyboard_open(&manager, editor.client);
        let compartments: ITfCompartmentMgr = manager.cast().unwrap();
        let open = compartments
            .GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
            .unwrap();
        let source: ITfSource = open.cast().unwrap();
        let observer: ITfCompartmentEventSink = SystemCtrlSpace {
            state: state.clone(),
            compartments: compartments.clone(),
        }
        .into();
        let cookie = source
            .AdviseSink(&ITfCompartmentEventSink::IID, &observer)
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut previous = None;
            for (key_first, expected) in [
                (false, Mode::English),
                (true, Mode::Chinese),
                (false, Mode::English),
                (true, Mode::Chinese),
            ] {
                previous = Some(next_key_message(previous));
                let _pressed = TestKeyboard::ctrl_space();
                if key_first {
                    assert!(sink
                        .OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
                        .unwrap()
                        .as_bool());
                    assert!(sink
                        .OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
                        .unwrap()
                        .as_bool());
                    assert_eq!(state.borrow().mode, expected);
                }
                open.SetValue(editor.client, &VARIANT::from(0i32)).unwrap();
                assert_eq!(
                    state.borrow().mode,
                    expected,
                    "the system-consumed press must switch exactly once"
                );
                assert_eq!(
                    compartment_value(&compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE),
                    Some(0),
                    "restoration must wait until the notification returns"
                );
                assert!(sink
                    .OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
                    .unwrap()
                    .as_bool());
                assert!(sink
                    .OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0))
                    .unwrap()
                    .as_bool());
                assert_eq!(
                    state.borrow().mode,
                    expected,
                    "a later key callback must not switch back"
                );
                let hwnd = state.borrow().window.as_ref().unwrap().hwnd();
                let mut message = MSG::default();
                while PeekMessageW(&mut message, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                }
                assert_eq!(compartment_value(&compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE), Some(1),
                    "the keyboard must reopen outside OnChange so the next press is not wasted reopening it");
                assert_eq!(
                    state.borrow().mode,
                    expected,
                    "restoring the system state must not switch modes"
                );
            }
            let mut config = state
                .borrow_mut()
                .client
                .call(&Request::GetConfig)
                .unwrap()
                .config
                .unwrap();
            config.hotkey_mode.clear();
            assert!(
                state
                    .borrow_mut()
                    .client
                    .call(&Request::SetConfig { config })
                    .unwrap()
                    .ok
            );
            next_key_message(previous);
            open.SetValue(editor.client, &VARIANT::from(0i32)).unwrap();
            let hwnd = state.borrow().window.as_ref().unwrap().hwnd();
            let mut message = MSG::default();
            while PeekMessageW(&mut message, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            }
            assert_eq!(
                compartment_value(&compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE),
                Some(0),
                "a disabled shortcut must not take over the system toggle"
            );
            assert_eq!(state.borrow().mode, Mode::Chinese);
        }));
        source.UnadviseSink(cookie).unwrap();
        state.borrow_mut().thread_mgr = None;
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    });
}

#[test]
fn first_digit_after_commit_keeps_the_key_claim_in_composition_only_hosts() {
    for composition_only in [false, true] {
        // Both the number row and keypad use the same host-context boundary.
        for vk in (0x30..=0x39).chain([0x60, 0x69]) {
            in_editor(|editor, state, sink| unsafe {
                editor.store.transitory.set(composition_only);
                editor.store.commit_only.set(composition_only);
                commit_test_word(editor, &state, &sink);
                let buffer = if composition_only {
                    &editor.store.app_text
                } else {
                    &editor.store.text
                };
                assert_eq!(String::from_utf16_lossy(&buffer.borrow()), "测试");
                let digit = char::from(b'0' + if vk >= 0x60 { vk - 0x60 } else { vk - 0x30 });
                for count in 1..=2 {
                    let key = WPARAM(usize::from(vk));
                    let preview = sink
                        .OnTestKeyDown(&editor.context, key, LPARAM(0))
                        .unwrap()
                        .as_bool();
                    assert_eq!(
                        sink.OnTestKeyDown(&editor.context, key, LPARAM(0))
                            .unwrap()
                            .as_bool(),
                        preview,
                        "宿主重复预询问不能改变接管结果"
                    );
                    if preview {
                        assert!(sink.OnKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool(),
                            "{digit} 预询问已接管，正式按下却放行；composition_only={composition_only}");
                    } else if composition_only {
                        // CUAS keeps the app's committed text outside its temporary TSF document.
                        editor
                            .store
                            .app_text
                            .borrow_mut()
                            .extend(wide(&digit.to_string()));
                    } else {
                        editor.store.app_insert(&digit.to_string());
                    }
                    let gap = if composition_only { "" } else { " " };
                    assert_eq!(
                        String::from_utf16_lossy(&buffer.borrow()),
                        format!("测试{gap}{}", digit.to_string().repeat(count))
                    );
                }
            });
        }
    }
}

#[test]
fn digit_claim_survives_a_host_that_allows_reading_only_after_preview() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.commit_only.set(true);
        commit_test_word(editor, &state, &sink);
        editor.store.deny_read.set(true);
        let zero = WPARAM(usize::from(b'0'));
        assert!(sink
            .OnTestKeyDown(&editor.context, zero, LPARAM(0))
            .unwrap()
            .as_bool());
        editor.store.deny_read.set(false);
        assert!(
            sink.OnKeyDown(&editor.context, zero, LPARAM(0))
                .unwrap()
                .as_bool(),
            "预询问读不到正文时，正式按下仍须保留已接管的按键"
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "测试 0"
        );
    });
}

#[test]
fn autospace_reads_imm32_text_through_document_feed() {
    in_editor(|editor, state, sink| unsafe {
        let _host = Imm32Host::new(editor, true);
        use_test_host(&state);
        // Text the app already had; FreeType never committed it.
        editor.store.app_text.borrow_mut().extend(wide("你好"));
        assert!(
            press_digit(editor, &sink, 5),
            "光标前是汉字，数字前要先补空格"
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "你好 5"
        );
    });
}

#[test]
fn autospace_reads_the_full_parent_of_a_transitory_extensions_document() {
    in_editor(|editor, state, sink| unsafe {
        // Edit and RichEdit: the transitory document is empty, the parent has the text.
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        let _parent = ParentDocument::new(editor, false, "你好");
        use_test_host(&state);
        assert!(
            press_digit(editor, &sink, 5),
            "父文档光标前是汉字，数字前要先补空格"
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            " 5"
        );
    });
}

#[test]
fn autospace_asks_the_app_when_the_parent_document_is_transitory_too() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        let _parent = ParentDocument::new(editor, true, "");
        let _window = Imm32Host::window(editor, true);
        use_test_host(&state);
        editor.store.app_text.borrow_mut().extend(wide("你好"));
        assert!(
            press_digit(editor, &sink, 5),
            "父文档也是临时文档时照 Mozc 改问应用窗口"
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "你好 5"
        );
    });
}

#[test]
fn autospace_keeps_committed_text_when_an_imm32_app_has_no_document_feed() {
    in_editor(|editor, state, sink| unsafe {
        let _host = Imm32Host::new(editor, false);
        commit_test_word(editor, &state, &sink);
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "测试"
        );
        assert!(
            press_digit(editor, &sink, 0),
            "读不到正文时沿用刚上屏的字，而不是当成空文档"
        );
        assert_eq!(
            String::from_utf16_lossy(&editor.store.app_text.borrow()),
            "测试 0"
        );
    });
}
