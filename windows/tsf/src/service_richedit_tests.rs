//! Use the Windows RichEdit control itself, on the existing private test desktop.
use super::*;
use windows::core::{ComObjectInner, w};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn pump() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[test]
fn native_richedit_accepts_composition_and_commit() {
    // RichEdit/CTF retains helper windows until this test thread exits.
    let _desktop = ManuallyDrop::new(crate::ui::render_tests::PrivateDesktop::new());
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        let manager: ITfThreadMgrEx = CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).unwrap();
        let mut client_id = 0;
        manager.ActivateEx(&mut client_id, TF_TMAE_NOACTIVATETIP | TF_TMAE_NOACTIVATEKEYBOARDLAYOUT).unwrap();
        // Match WinOne's Win32++ adapter: system Msftedit, plain multiline text.
        let _module = LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).unwrap();
        let parent = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("FreeType RichEdit test"),
            WS_POPUP | WS_VISIBLE, 30, 30, 500, 240, None, None, None, None).unwrap();
        let control = CreateWindowExW(WINDOW_EX_STYLE(0), w!("RICHEDIT50W"), w!(""),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32 | ES_NOHIDESEL as u32),
            10, 10, 450, 180, Some(parent), None, None, None).unwrap();
        // EM_SETTEXTMODE: TM_PLAINTEXT | TM_MULTILEVELUNDO | TM_MULTICODEPAGE.
        SendMessageW(control, WM_USER + 89, Some(WPARAM(1 | 8 | 32)), Some(LPARAM(0)));
        // Exercise native TSF without activating or registering an installed IME.
        // WinOne uses the default IMM bridge; its lock/focus contract is covered
        // separately by enabled_transitory_editor_does_not_inherit_disabled_focus.
        SendMessageW(control, WM_USER + 204, Some(WPARAM(0x10000)), Some(LPARAM(0x10000)));
        let _ = ShowWindow(control, SW_SHOW);
        SetFocus(Some(control)).unwrap();
        pump();
        let focused = manager.GetFocus();
        if focused.is_err() {
            DestroyWindow(parent).unwrap();
            manager.Deactivate().unwrap();
            drop(manager);
            CoUninitialize();
            panic!("RichEdit must supply its real TSF document: {focused:?}");
        }
        let document = focused.expect("RichEdit must supply its real TSF document");
        let context = document.GetTop().unwrap();
        assert_eq!(context.GetStatus().unwrap().dwStaticFlags & TF_SS_TRANSITORY, 0);
        let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
        let service = TextService::new(instance).into_object();
        let state = service.state.clone();
        let keys = service.to_interface::<ITfKeyEventSink>();
        {
            let mut s = state.borrow_mut();
            s.client = Client::without_server();
            s.client_id = client_id;
            s.context = Some(context.clone());
            s.sink = Some(service.to_interface::<ITfCompositionSink>());
            s.window = CandidateWindow::new(instance);
        }
        super::prediction_tests::use_test_host(&state);
        windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&[0; 256]).unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for vk in [0x43, 0x45, 0x53, 0x48, 0x49] {
                assert!(keys.OnTestKeyDown(&context, WPARAM(vk), LPARAM(0)).unwrap().as_bool());
                assert!(keys.OnKeyDown(&context, WPARAM(vk), LPARAM(0)).unwrap().as_bool());
                pump();
            }
            assert!(state.borrow().composition.is_some(), "RichEdit must receive the composition");
            assert!(state.borrow().window.as_ref().unwrap().visible(), "the real candidate window must be visible");
            assert!(keys.OnTestKeyDown(&context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
            assert!(keys.OnKeyDown(&context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
            pump();
            let mut text = [0u16; 128];
            let length = GetWindowTextW(control, &mut text);
            assert_eq!(String::from_utf16_lossy(&text[..length as usize]), "测试");
        }));
        apply(&state, Some(&context), Frame::default(), false);
        pump();
        state.borrow_mut().sink = None;
        DestroyWindow(parent).unwrap();
        manager.Deactivate().unwrap();
        drop((keys, service, state, context, document, manager));
        CoUninitialize();
        windows::Win32::Foundation::FreeLibrary(_module).unwrap();
        if let Err(error) = outcome { std::panic::resume_unwind(error); }
    }
}
