//! Real Windows edit sessions and ranges, with a disposable app-side text store.
use super::*;
use std::cell::Cell;
use windows::core::{ComObject, ComObjectInner, IUnknown, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{E_FAIL, E_NOTIMPL, S_OK};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDataObject, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, FORMATETC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};
use windows::Win32::Foundation::LRESULT;

fn unused<T>() -> Result<T> { Err(E_NOTIMPL.into()) }

#[derive(Default)]
#[implement(ITextStoreACP, ITfContextOwnerCompositionSink)]
struct Store {
    text: RefCell<Vec<u16>>,
    selection: Cell<TS_SELECTION_ACP>,
    sink: RefCell<Option<ITextStoreACPSink>>,
    /// Chromium and CUAS both advertise this; it does not describe text visibility.
    transitory: Cell<bool>,
    deny_read: Cell<bool>,
    /// CUAS forwards committed text to the app, then empties its document.
    commit_only: Cell<bool>,
    app_text: RefCell<Vec<u16>>,
    composing: Cell<bool>,
    /// The app window that CUAS reports for its emulated document.
    window: Cell<HWND>,
    fail_window: Cell<bool>,
}

impl Store {
    fn replace(&self, start: i32, end: i32, text: &[u16]) -> TS_TEXTCHANGE {
        self.text.borrow_mut().splice(start as usize..end as usize, text.iter().copied());
        TS_TEXTCHANGE { acpStart: start, acpOldEnd: end, acpNewEnd: start + text.len() as i32 }
    }

    fn move_caret(&self, at: i32) {
        self.selection.set(TS_SELECTION_ACP { acpStart: at, acpEnd: at, ..Default::default() });
        let sink = self.sink.borrow().clone().unwrap();
        unsafe { sink.OnSelectionChange().unwrap(); }
    }

    fn app_insert(&self, text: &str) {
        let selection = self.selection.get();
        self.app_replace(selection.acpStart, selection.acpEnd, text);
    }

    fn app_replace(&self, start: i32, end: i32, text: &str) {
        let change = self.replace(start, end, &wide(text));
        let sink = self.sink.borrow().clone().unwrap();
        unsafe { sink.OnTextChange(TEXT_STORE_TEXT_CHANGE_FLAGS(0), &change).unwrap(); }
        self.move_caret(change.acpNewEnd);
    }
}

impl ITextStoreACP_Impl for Store_Impl {
    fn AdviseSink(&self, _: *const GUID, sink: Ref<IUnknown>, _: u32) -> Result<()> {
        *self.sink.borrow_mut() = Some(sink.ok()?.cast()?);
        Ok(())
    }
    fn UnadviseSink(&self, _: Ref<IUnknown>) -> Result<()> { self.sink.borrow_mut().take(); Ok(()) }
    fn RequestLock(&self, flags: u32) -> Result<HRESULT> {
        if self.deny_read.get() && flags & TS_LF_READWRITE.0 == TS_LF_READ.0 {
            return Ok(TS_E_SYNCHRONOUS);
        }
        let sink = self.sink.borrow().clone().ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
        let result = unsafe { sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(flags)) }.err().map_or(S_OK, |e| e.code());
        if self.commit_only.get() && !self.composing.get() && flags & TS_LF_READWRITE.0 == TS_LF_READWRITE.0 && result.is_ok() {
            let size = self.text.borrow().len() as i32;
            if size > 0 {
                self.app_text.borrow_mut().extend_from_slice(&self.text.borrow());
                self.app_replace(0, size, "");
            }
        }
        Ok(result)
    }
    fn GetStatus(&self) -> Result<TS_STATUS> {
        Ok(TS_STATUS { dwStaticFlags: if self.transitory.get() { TS_SS_TRANSITORY } else { 0 }, ..Default::default() })
    }
    fn QueryInsert(&self, start: i32, end: i32, _: u32, out_start: *mut i32, out_end: *mut i32) -> Result<()> {
        unsafe { *out_start = start; *out_end = end; }
        Ok(())
    }
    fn GetSelection(&self, _: u32, count: u32, out: *mut TS_SELECTION_ACP, fetched: *mut u32) -> Result<()> {
        assert!(count > 0);
        unsafe { *out = self.selection.get(); *fetched = 1; }
        Ok(())
    }
    fn SetSelection(&self, count: u32, selection: *const TS_SELECTION_ACP) -> Result<()> {
        assert_eq!(count, 1);
        self.selection.set(unsafe { *selection });
        Ok(())
    }
    fn GetText(&self, start: i32, end: i32, text: PWSTR, capacity: u32, read: *mut u32,
        runs: *mut TS_RUNINFO, run_capacity: u32, run_count: *mut u32, next: *mut i32) -> Result<()> {
        let buffer = self.text.borrow();
        let end = if end < 0 { buffer.len() } else { end as usize };
        let slice = &buffer[start as usize..end];
        let size = slice.len().min(capacity as usize);
        unsafe {
            if size != 0 { std::ptr::copy_nonoverlapping(slice.as_ptr(), text.0, size); }
            *read = size as u32;
            *run_count = u32::from(run_capacity > 0 && !slice.is_empty());
            if *run_count > 0 { *runs = TS_RUNINFO { uCount: slice.len() as u32, r#type: TS_RT_PLAIN }; }
            *next = start + if capacity == 0 { slice.len() } else { size } as i32;
        }
        Ok(())
    }
    fn SetText(&self, _: u32, start: i32, end: i32, text: &PCWSTR, count: u32) -> Result<TS_TEXTCHANGE> {
        let text = if count == 0 { &[] } else { unsafe { std::slice::from_raw_parts(text.0, count as usize) } };
        Ok(self.replace(start, end, text))
    }
    fn GetFormattedText(&self, _: i32, _: i32) -> Result<IDataObject> { unused() }
    fn GetEmbedded(&self, _: i32, _: *const GUID, _: *const GUID) -> Result<IUnknown> { unused() }
    fn QueryInsertEmbedded(&self, _: *const GUID, _: *const FORMATETC) -> Result<BOOL> { unused() }
    fn InsertEmbedded(&self, _: u32, _: i32, _: i32, _: Ref<IDataObject>) -> Result<TS_TEXTCHANGE> { unused() }
    fn InsertTextAtSelection(&self, flags: u32, text: &PCWSTR, count: u32, start: *mut i32,
        end: *mut i32, change: *mut TS_TEXTCHANGE) -> Result<()> {
        let at = self.selection.get();
        unsafe {
            if !start.is_null() { *start = at.acpStart; }
            if flags & TS_IAS_QUERYONLY == 0 {
                let edit = self.SetText(0, at.acpStart, at.acpEnd, text, count)?;
                if !end.is_null() { *end = edit.acpNewEnd; }
                if !change.is_null() { *change = edit; }
            } else if !end.is_null() { *end = at.acpEnd; }
        }
        Ok(())
    }
    fn InsertEmbeddedAtSelection(&self, _: u32, _: Ref<IDataObject>, _: *mut i32, _: *mut i32, _: *mut TS_TEXTCHANGE) -> Result<()> { unused() }
    fn RequestSupportedAttrs(&self, _: u32, _: u32, _: *const GUID) -> Result<()> { unused() }
    fn RequestAttrsAtPosition(&self, _: i32, _: u32, _: *const GUID, _: u32) -> Result<()> { unused() }
    fn RequestAttrsTransitioningAtPosition(&self, _: i32, _: u32, _: *const GUID, _: u32) -> Result<()> { unused() }
    fn FindNextAttrTransition(&self, _: i32, _: i32, _: u32, _: *const GUID, _: u32, _: *mut i32, _: *mut BOOL, _: *mut i32) -> Result<()> { unused() }
    fn RetrieveRequestedAttrs(&self, _: u32, _: *mut TS_ATTRVAL, _: *mut u32) -> Result<()> { unused() }
    fn GetEndACP(&self) -> Result<i32> { Ok(self.text.borrow().len() as i32) }
    fn GetActiveView(&self) -> Result<u32> { Ok(0) }
    fn GetACPFromPoint(&self, _: u32, _: *const POINT, _: u32) -> Result<i32> { unused() }
    fn GetTextExt(&self, _: u32, _: i32, _: i32, at: *mut RECT, clipped: *mut BOOL) -> Result<()> {
        unsafe { *at = RECT { left: 100, top: 100, right: 102, bottom: 120 }; *clipped = false.into(); }
        Ok(())
    }
    fn GetScreenExt(&self, _: u32) -> Result<RECT> { unused() }
    fn GetWnd(&self, _: u32) -> Result<HWND> {
        if self.fail_window.get() { return Err(E_FAIL.into()); }
        Ok(self.window.get())
    }
}

impl ITfContextOwnerCompositionSink_Impl for Store_Impl {
    fn OnStartComposition(&self, _: Ref<ITfCompositionView>) -> Result<BOOL> {
        self.composing.set(true);
        Ok(true.into())
    }
    fn OnUpdateComposition(&self, _: Ref<ITfCompositionView>, _: Ref<ITfRange>) -> Result<()> { Ok(()) }
    fn OnEndComposition(&self, _: Ref<ITfCompositionView>) -> Result<()> {
        self.composing.set(false);
        Ok(())
    }
}

struct Editor {
    manager: ITfThreadMgrEx,
    document: ITfDocumentMgr,
    context: ITfContext,
    client: u32,
    store: ComObject<Store>,
}

impl Editor {
    unsafe fn new() -> Self {
        let manager: ITfThreadMgrEx = CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).unwrap();
        let mut client = 0;
        manager.ActivateEx(&mut client, TF_TMAE_NOACTIVATETIP | TF_TMAE_NOACTIVATEKEYBOARDLAYOUT).unwrap();
        let document = manager.CreateDocumentMgr().unwrap();
        let store = Store::default().into_object();
        let (mut context, mut cookie) = (None, 0);
        let text_store: IUnknown = store.as_interface::<ITextStoreACP>().cast().unwrap();
        document.CreateContext(client, 0, &text_store, &mut context, &mut cookie).unwrap();
        let context = context.unwrap();
        document.Push(&context).unwrap();
        Self { manager, document, context, client, store }
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe { self.document.Pop(TF_POPF_ALL).unwrap(); self.manager.Deactivate().unwrap(); }
    }
}

fn in_editor(test: impl FnOnce(&Editor, Rc<RefCell<State>>, ITfKeyEventSink)) {
    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) { unsafe { CoUninitialize(); } }
    }
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap(); }
    let _apartment = Apartment;
    let editor = unsafe { Editor::new() };
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let service = TextService::new(instance).into_object();
    let state = service.state.clone();
    {
        let mut s = state.borrow_mut();
        s.client = Client::without_server();
        s.window = CandidateWindow::new(instance);
        s.context = Some(editor.context.clone());
        s.client_id = editor.client;
        s.sink = Some(service.to_interface::<ITfCompositionSink>());
    }
    let weak = Rc::downgrade(&state);
    ui::set_click_handler(Some(Box::new(move |click| on_click(&weak, click))));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        test(&editor, state.clone(), service.to_interface::<ITfKeyEventSink>());
    }));
    // Release real compositions and their sink cycle even when an assertion fails.
    finish_prediction_space(&state, false, true);
    apply(&state, Some(&editor.context), Frame::default(), true);
    state.borrow_mut().sink = None;
    ui::set_click_handler(None);
    if let Err(error) = result { std::panic::resume_unwind(error); }
}

#[test]
fn candidate_window_belongs_to_the_current_text_view() {
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_OWNER, WS_POPUP};
    in_editor(|editor, state, _| unsafe {
        // Real HWNDs and TSF views exercise the ownership contract required by
        // Windows shell input fields, without touching the user's desktop.
        let first = CreateWindowExW(WINDOW_EX_STYLE::default(), windows::core::w!("STATIC"),
            None, WS_POPUP, 40, 40, 500, 100, None, None, None, None).unwrap();
        let second = CreateWindowExW(WINDOW_EX_STYLE::default(), windows::core::w!("STATIC"),
            None, WS_POPUP, 40, 180, 500, 100, None, None, None, None).unwrap();
        let frame = Frame { candidates: vec!["测试".into()], cursor: Some(0), font_size: 18, ..Frame::default() };
        state.borrow_mut().caret = Some(RECT { left: 50, top: 50, right: 52, bottom: 70 });
        editor.store.window.set(first);
        show_window(&state, &frame);
        let candidate = state.borrow().window.as_ref().unwrap().hwnd();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_eq!(GetWindow(candidate, GW_OWNER).unwrap_or_default(), first,
                "候选窗必须关联到 TSF 当前输入窗口，才能显示在开始菜单等界面上方");
            show_window(&state, &frame);
            assert_eq!(state.borrow().window.as_ref().unwrap().hwnd(), candidate,
                "同一输入窗口连续刷新不能重建候选窗");
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
            assert!(GetWindow(candidate, GW_OWNER).is_err(), "隐藏后解除宿主关联");
        }));
        // Detach before destroying test owners; Windows destroys owned popups too.
        editor.store.window.set(HWND::default());
        show_window(&state, &Frame::default());
        DestroyWindow(first).unwrap();
        DestroyWindow(second).unwrap();
        if let Err(error) = outcome { std::panic::resume_unwind(error); }
    });
}

#[test]
fn candidate_window_recovers_after_its_input_host_is_destroyed() {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, IsWindow, GW_OWNER, WS_POPUP};
    in_editor(|editor, state, _| unsafe {
        let host = CreateWindowExW(WINDOW_EX_STYLE::default(), windows::core::w!("STATIC"),
            None, WS_POPUP, 40, 40, 500, 100, None, None, None, None).unwrap();
        editor.store.window.set(host);
        state.borrow_mut().caret = Some(RECT { left: 50, top: 50, right: 52, bottom: 70 });
        let frame = Frame { candidates: vec!["测试".into()], cursor: Some(0), font_size: 18, ..Frame::default() };
        show_window(&state, &frame);
        let candidate = state.borrow().window.as_ref().unwrap().hwnd();
        DestroyWindow(host).unwrap();
        assert!(!IsWindow(Some(candidate)).as_bool(), "Windows 会连带销毁所属弹窗");
        let next = CreateWindowExW(WINDOW_EX_STYLE::default(), windows::core::w!("STATIC"),
            None, WS_POPUP, 40, 40, 500, 100, None, None, None, None).unwrap();
        editor.store.window.set(next);
        show_window(&state, &frame);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let s = state.borrow();
            let window = s.window.as_ref().unwrap();
            assert!(window.visible(), "输入窗口关闭后，在另一个窗口继续输入仍须显示候选");
            assert_eq!(GetWindow(window.hwnd(), GW_OWNER).unwrap(), next);
        }));
        show_window(&state, &Frame::default());
        DestroyWindow(next).unwrap();
        if let Err(error) = outcome { std::panic::resume_unwind(error); }
    });
}

fn open_prediction(editor: &Editor, state: &Rc<RefCell<State>>, double: bool) -> u64 {
    let frame = Frame {
        handled: true, commit: Some("今天".into()), candidates: vec!["下雨".into()], cursor: Some(0),
        predict: true, predict_double_space: double, page_revision: 1,
        font_family: "Microsoft YaHei".into(), font_size: 18, ..Frame::default()
    };
    apply(state, Some(&editor.context), frame, true);
    assert_eq!(shown_revision(state), 1);
    assert!(state.borrow().prediction_caret.is_some(), "commit anchored at the real TSF caret");
    1
}

fn use_test_host(state: &Rc<RefCell<State>>) {
    let data = std::env::temp_dir().join(format!("freetype-digit-{}-{}", std::process::id(), unsafe { GetCurrentThreadId() }));
    let mut host = freetype_server::host::Host::new(freetype_server::store::Store::new(data));
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str("ce shi\t测试\t1000\n").unwrap();
    host.install_engine(Ok(wowtype_core::Engine::new(
        std::sync::Arc::new(lexicon), std::sync::Arc::new(wowtype_core::BigramLm::default()))));
    host.connect(1, "explorer.exe".into());
    assert!(!host.config().predict_next, "此回归不依赖上屏联想");
    state.borrow_mut().client = Client::with_test_host(host);
}

/// Only this private desktop's test thread sees the synthetic keyboard state.
struct TestKeyboard([u8; 256]);

impl TestKeyboard {
    fn ctrl_space() -> Self {
        let mut previous = [0; 256];
        unsafe { GetKeyboardState(&mut previous).unwrap(); }
        let mut pressed = [0; 256];
        pressed[0x11] = 0x80;
        pressed[0x20] = 0x80;
        unsafe { windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&pressed).unwrap(); }
        Self(previous)
    }
}

impl Drop for TestKeyboard {
    fn drop(&mut self) {
        unsafe { windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&self.0).unwrap(); }
    }
}

fn enable_ctrl_space(state: &Rc<RefCell<State>>) {
    use_test_host(state);
    let mut s = state.borrow_mut();
    let mut config = s.client.call(&Request::GetConfig).unwrap().config.unwrap();
    config.hotkey_mode = "ctrl+space".into();
    assert!(s.client.call(&Request::SetConfig { config }).unwrap().ok);
}

/// Give each press a real Windows message time; repeated previews share that time.
fn next_key_message(previous: Option<u32>) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, PostThreadMessageW, MSG, PM_REMOVE, WM_APP};
    let mut message = MSG::default();
    loop {
        unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_APP + 90, WPARAM(0), LPARAM(0)).unwrap();
            assert!(PeekMessageW(&mut message, None, WM_APP + 90, WM_APP + 90, PM_REMOVE).as_bool());
        }
        let ms = unsafe { GetMessageTime() } as u32;
        if Some(ms) != previous { return ms; }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn ctrl_space_next_press_switches_even_when_the_host_omits_key_up() {
    in_editor(|editor, state, sink| unsafe {
        enable_ctrl_space(&state);
        let space = WPARAM(0x20);
        let mut previous = None;
        for expected in [Mode::English, Mode::Chinese, Mode::English, Mode::Chinese] {
            previous = Some(next_key_message(previous));
            let _pressed = TestKeyboard::ctrl_space();
            assert!(sink.OnTestKeyDown(&editor.context, space, LPARAM(0)).unwrap().as_bool());
            assert!(sink.OnTestKeyDown(&editor.context, space, LPARAM(0)).unwrap().as_bool());
            assert!(sink.OnKeyDown(&editor.context, space, LPARAM(0)).unwrap().as_bool());
            assert_eq!(state.borrow().mode, expected, "a new press must not inherit the previous press's duplicate guard");
            assert!(sink.OnKeyDown(&editor.context, space, LPARAM(0)).unwrap().as_bool());
            assert_eq!(state.borrow().mode, expected, "duplicate callbacks must not switch back");
            previous = Some(next_key_message(previous));
            assert!(sink.OnKeyDown(&editor.context, space, LPARAM(1 << 30)).unwrap().as_bool());
            assert_eq!(state.borrow().mode, expected, "holding Space must not keep switching");
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
            if compartment_value(&self.compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) == Some(0) {
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
        let open = compartments.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE).unwrap();
        let source: ITfSource = open.cast().unwrap();
        let observer: ITfCompartmentEventSink = SystemCtrlSpace { state: state.clone(), compartments: compartments.clone() }.into();
        let cookie = source.AdviseSink(&ITfCompartmentEventSink::IID, &observer).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut previous = None;
            for (key_first, expected) in [(false, Mode::English), (true, Mode::Chinese), (false, Mode::English), (true, Mode::Chinese)] {
                previous = Some(next_key_message(previous));
                let _pressed = TestKeyboard::ctrl_space();
                if key_first {
                    assert!(sink.OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
                    assert!(sink.OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
                    assert_eq!(state.borrow().mode, expected);
                }
                open.SetValue(editor.client, &VARIANT::from(0i32)).unwrap();
                assert_eq!(state.borrow().mode, expected, "the system-consumed press must switch exactly once");
                assert_eq!(compartment_value(&compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE), Some(0),
                    "restoration must wait until the notification returns");
                assert!(sink.OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
                assert!(sink.OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
                assert_eq!(state.borrow().mode, expected, "a later key callback must not switch back");
                let hwnd = state.borrow().window.as_ref().unwrap().hwnd();
                let mut message = MSG::default();
                while PeekMessageW(&mut message, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                }
                assert_eq!(compartment_value(&compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE), Some(1),
                    "the keyboard must reopen outside OnChange so the next press is not wasted reopening it");
                assert_eq!(state.borrow().mode, expected, "restoring the system state must not switch modes");
            }
            let mut config = state.borrow_mut().client.call(&Request::GetConfig).unwrap().config.unwrap();
            config.hotkey_mode.clear();
            assert!(state.borrow_mut().client.call(&Request::SetConfig { config }).unwrap().ok);
            next_key_message(previous);
            open.SetValue(editor.client, &VARIANT::from(0i32)).unwrap();
            let hwnd = state.borrow().window.as_ref().unwrap().hwnd();
            let mut message = MSG::default();
            while PeekMessageW(&mut message, Some(hwnd), 0, 0, PM_REMOVE).as_bool() { DispatchMessageW(&message); }
            assert_eq!(compartment_value(&compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE), Some(0),
                "a disabled shortcut must not take over the system toggle");
            assert_eq!(state.borrow().mode, Mode::Chinese);
        }));
        source.UnadviseSink(cookie).unwrap();
        state.borrow_mut().thread_mgr = None;
        if let Err(error) = result { std::panic::resume_unwind(error); }
    });
}

fn commit_test_word(editor: &Editor, state: &Rc<RefCell<State>>, sink: &ITfKeyEventSink) {
    use_test_host(state);
    for vk in [b'C', b'E', b'S', b'H', b'I', b' '] {
        let key = WPARAM(usize::from(vk));
        unsafe {
            assert!(sink.OnTestKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool());
            assert!(sink.OnKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool());
        }
    }
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
                let buffer = if composition_only { &editor.store.app_text } else { &editor.store.text };
                assert_eq!(String::from_utf16_lossy(&buffer.borrow()), "测试");
                let digit = char::from(b'0' + if vk >= 0x60 { vk - 0x60 } else { vk - 0x30 });
                for count in 1..=2 {
                    let key = WPARAM(usize::from(vk));
                    let preview = sink.OnTestKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool();
                    assert_eq!(sink.OnTestKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool(), preview,
                        "宿主重复预询问不能改变接管结果");
                    if preview {
                        assert!(sink.OnKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool(),
                            "{digit} 预询问已接管，正式按下却放行；composition_only={composition_only}");
                    } else if composition_only {
                        // CUAS keeps the app's committed text outside its temporary TSF document.
                        editor.store.app_text.borrow_mut().extend(wide(&digit.to_string()));
                    } else {
                        editor.store.app_insert(&digit.to_string());
                    }
                    let gap = if composition_only { "" } else { " " };
                    assert_eq!(String::from_utf16_lossy(&buffer.borrow()), format!("测试{gap}{}", digit.to_string().repeat(count)));
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
        assert!(sink.OnTestKeyDown(&editor.context, zero, LPARAM(0)).unwrap().as_bool());
        editor.store.deny_read.set(false);
        assert!(sink.OnKeyDown(&editor.context, zero, LPARAM(0)).unwrap().as_bool(),
            "预询问读不到正文时，正式按下仍须保留已接管的按键");
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "测试 0");
    });
}

thread_local! {
    /// The app text that the test IMM32 window reports, with its caret at the end.
    static FEED: RefCell<Option<ComObject<Store>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn feed_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message != WM_IME_REQUEST || wparam.0 != IMR_DOCUMENTFEED as usize {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    FEED.with_borrow(|store| {
        let Some(store) = store else { return LRESULT(0) };
        let text = store.app_text.borrow();
        let head = std::mem::size_of::<RECONVERTSTRING>() as u32;
        let (len, size) = (text.len() as u32, head + text.len() as u32 * 2);
        let info = lparam.0 as *mut RECONVERTSTRING;
        if info.is_null() { return LRESULT(size as isize); }
        unsafe {
            if (*info).dwSize < size { return LRESULT(0); }
            *info = RECONVERTSTRING { dwSize: size, dwVersion: 0, dwStrLen: len, dwStrOffset: head,
                dwCompStrLen: 0, dwCompStrOffset: len * 2, dwTargetStrLen: 0, dwTargetStrOffset: len * 2 };
            std::ptr::copy_nonoverlapping(text.as_ptr(), info.cast::<u8>().add(head as usize).cast::<u16>(), text.len());
        }
        LRESULT(size as isize)
    })
}

fn set_compartment(editor: &Editor, guid: &GUID, value: &VARIANT) {
    let mgr: ITfCompartmentMgr = editor.document.cast().unwrap();
    unsafe { mgr.GetCompartment(guid).unwrap().SetValue(editor.client, value).unwrap(); }
}

/// Point the editor's Transitory Extensions compartment at `parent`, as Windows does.
fn set_parent(editor: &Editor, parent: &IUnknown) {
    set_compartment(editor, &GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT, &VARIANT::from(parent.clone()));
}

/// Turn the editor into what CUAS gives an IMM32 app such as FreeMD, as read on
/// Windows 11: a transitory document marked as emulated, whose parent compartment
/// holds an object that is not a document manager, emptied after each commit,
/// with the app's own window.
struct Imm32Host(Option<HWND>);

impl Imm32Host {
    unsafe fn new(editor: &Editor, answers_feed: bool) -> Self {
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        set_compartment(editor, &GUID_COMPARTMENT_EMULATED_DOCUMENTMGR, &VARIANT::from(1i32));
        set_parent(editor, &editor.store.as_interface::<ITextStoreACP>().cast().unwrap());
        unsafe { Self::window(editor, answers_feed) }
    }

    /// Only the app window that answers IMR_DOCUMENTFEED, for hosts set up by the test.
    unsafe fn window(editor: &Editor, answers_feed: bool) -> Self {
        if !answers_feed { return Self(None); }
        let class = windows::core::w!("FreeTypeImm32FeedTest");
        let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
        let window = unsafe {
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(feed_proc), hInstance: instance, lpszClassName: class, ..Default::default() });
            CreateWindowExW(WINDOW_EX_STYLE(0), class, PCWSTR::null(), WINDOW_STYLE(0), 0, 0, 0, 0,
                Some(HWND_MESSAGE), None, Some(instance), None).unwrap()
        };
        editor.store.window.set(window);
        FEED.set(Some(editor.store.clone()));
        Self(Some(window))
    }
}

impl Drop for Imm32Host {
    fn drop(&mut self) {
        FEED.set(None);
        if let Some(window) = self.0 { unsafe { let _ = DestroyWindow(window); } }
    }
}

fn press_digit(editor: &Editor, sink: &ITfKeyEventSink, digit: u8) -> bool {
    let key = WPARAM(usize::from(b'0' + digit));
    let preview = unsafe { sink.OnTestKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool() };
    if preview {
        assert!(unsafe { sink.OnKeyDown(&editor.context, key, LPARAM(0)).unwrap().as_bool() }, "预询问已接管，正式按下不能放行");
    }
    preview
}

#[test]
fn autospace_reads_imm32_text_through_document_feed() {
    in_editor(|editor, state, sink| unsafe {
        let _host = Imm32Host::new(editor, true);
        use_test_host(&state);
        // Text the app already had; FreeType never committed it.
        editor.store.app_text.borrow_mut().extend(wide("你好"));
        assert!(press_digit(editor, &sink, 5), "光标前是汉字，数字前要先补空格");
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "你好 5");
    });
}

/// A second TSF document, as the full parent of a Transitory Extensions document.
struct ParentDocument {
    document: ITfDocumentMgr,
    _store: ComObject<Store>,
}

impl ParentDocument {
    unsafe fn new(editor: &Editor, transitory: bool, text: &str) -> Self {
        let store = Store::default().into_object();
        store.transitory.set(transitory);
        store.text.borrow_mut().extend(wide(text));
        store.selection.set(TS_SELECTION_ACP { acpStart: text.encode_utf16().count() as i32, acpEnd: text.encode_utf16().count() as i32, ..Default::default() });
        let document = unsafe { editor.manager.CreateDocumentMgr().unwrap() };
        let (mut context, mut cookie) = (None, 0);
        let unknown: IUnknown = store.as_interface::<ITextStoreACP>().cast().unwrap();
        unsafe {
            document.CreateContext(editor.client, 0, &unknown, &mut context, &mut cookie).unwrap();
            document.Push(&context.unwrap()).unwrap();
        }
        set_parent(editor, &document.cast().unwrap());
        Self { document, _store: store }
    }
}

impl Drop for ParentDocument {
    fn drop(&mut self) { unsafe { let _ = self.document.Pop(TF_POPF_ALL); } }
}

#[test]
fn autospace_reads_the_full_parent_of_a_transitory_extensions_document() {
    in_editor(|editor, state, sink| unsafe {
        // Edit and RichEdit: the transitory document is empty, the parent has the text.
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        let _parent = ParentDocument::new(editor, false, "你好");
        use_test_host(&state);
        assert!(press_digit(editor, &sink, 5), "父文档光标前是汉字，数字前要先补空格");
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), " 5");
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
        assert!(press_digit(editor, &sink, 5), "父文档也是临时文档时照 Mozc 改问应用窗口");
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "你好 5");
    });
}

#[test]
fn autospace_keeps_committed_text_when_an_imm32_app_has_no_document_feed() {
    in_editor(|editor, state, sink| unsafe {
        let _host = Imm32Host::new(editor, false);
        commit_test_word(editor, &state, &sink);
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "测试");
        assert!(press_digit(editor, &sink, 0), "读不到正文时沿用刚上屏的字，而不是当成空文档");
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "测试 0");
    });
}

fn press_space(editor: &Editor, state: &Rc<RefCell<State>>, ms: u32) -> Key {
    let key = Key { keysym: 0x20, ..Key::default() };
    let revision = shown_revision(state);
    let (key, belongs) = prediction_space_key(state, &editor.context, key, false, revision, ms);
    assert!(belongs);
    key
}

fn choose_space(editor: &Editor, state: &Rc<RefCell<State>>, ms: u32, word: &str, next: Option<&str>) {
    assert_eq!(press_space(editor, state, ms).keysym, keysym::SPACE_DOUBLE);
    let revision = shown_revision(state) + 1;
    apply(state, Some(&editor.context), Frame {
        handled: true, replace_space: true, commit: Some(word.into()),
        candidates: next.into_iter().map(str::to_owned).collect(),
        predict: next.is_some(), predict_double_space: next.is_some(), cursor: next.map(|_| 0), page_revision: revision,
        font_family: "Microsoft YaHei".into(), font_size: 18, ..Frame::default()
    }, true);
}

#[test]
fn prediction_first_space_is_owned_before_any_application_key_is_dispatched() {
    in_editor(|editor, state, sink| unsafe {
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        // Both callbacks are production entry points. The host must never receive
        // the first Space, so it has no raw character or Backspace to dispatch later.
        for _ in 0..2 {
            assert!(sink.OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
        }
        assert!(editor.store.text.borrow().is_empty(), "previews cannot open a composition");
        assert!(sink.OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool());
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天");
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), " ");
        assert!(editor.store.composing.get(), "the real TSF owner must receive OnStartComposition");
        assert!(!composing(&state), "the engine is still showing predictions, not editing pinyin");
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
            assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天");
            assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), " ");
            choose_space(editor, &state, 1100, word, None);
            assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), format!("今天{word}"));
            assert!(editor.store.text.borrow().is_empty());
            assert!(!editor.store.composing.get(), "the result is committed only by OnEndComposition");
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
                let buffer = if composition_only { &editor.store.app_text } else { &editor.store.text };
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
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天", "an early/stale timer cannot end the double-tap window");
        state.borrow_mut().space.as_mut().unwrap().deadline_ms = 0;
        prediction_space_timeout(&state);
        prediction_space_timeout(&state);
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天 ");
        assert!(!editor.store.composing.get());
    });
}

#[test]
fn prediction_space_window_timer_reaches_the_text_service() {
    use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, DispatchMessageW, MSG, PM_REMOVE, WM_TIMER};
    in_editor(|editor, state, _| unsafe {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        state.borrow_mut().space.as_mut().unwrap().deadline_ms = 0;
        let hwnd = state.borrow().window.as_ref().unwrap().hwnd();
        state.borrow().window.as_ref().unwrap().set_prediction_space_timeout(Some(1));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.borrow().space.is_some() && std::time::Instant::now() < end {
            let mut message = MSG::default();
            if PeekMessageW(&mut message, Some(hwnd), WM_TIMER, WM_TIMER, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天 ");
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
        assert!(!service.can_reload(), "an owned space must finish before replacing the DLL");
        let owner: ITfContextOwnerCompositionServices = editor.context.cast().unwrap();
        owner.TerminateComposition(None::<&ITfCompositionView>).unwrap();
        prediction_space_timeout(&state);
        assert!(state.borrow().space.is_none());
        assert!(!state.borrow().predicting);
        assert_eq!(shown_revision(&state), 0);
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天 ");
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
        assert!(sink.OnTestKeyDown(&editor.context, WPARAM(0x20), repeat).unwrap().as_bool());
        assert!(!sink.OnKeyDown(&editor.context, WPARAM(0x20), repeat).unwrap().as_bool());
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天 ");
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
        assert!(sink.OnTestKeyDown(&editor.context, WPARAM(0x08), LPARAM(0)).unwrap().as_bool());
        assert!(sink.OnKeyDown(&editor.context, WPARAM(0x08), LPARAM(0)).unwrap().as_bool());
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天");
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
        assert!(sink.OnTestKeyDown(&editor.context, WPARAM(0x4e), LPARAM(0)).unwrap().as_bool());
        // With a disconnected test server, the next key is passed back after
        // the pending space has been committed. A real response can then compose.
        assert!(!sink.OnKeyDown(&editor.context, WPARAM(0x4e), LPARAM(0)).unwrap().as_bool());
        apply(&state, Some(&editor.context), Frame {
            handled: true, preedit: "n".into(), preedit_cursor: 1, ..Frame::default()
        }, true);
        prediction_space_timeout(&state);
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天 ");
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
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天好运来");
    });
}

#[test]
fn prediction_double_space_respects_message_timing_and_duplicate_previews() {
    in_editor(|editor, state, _| {
        editor.store.commit_only.set(true);
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        let preview = Key { keysym: 0x20, test: true, ..Key::default() };
        assert_eq!(prediction_space_key(&state, &editor.context, preview, false, 1, 1000).0.keysym, 0x20);
        press_space(editor, &state, 1000);
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), " ", "a duplicate press cannot create two spaces");
        assert_eq!(press_space(editor, &state, 2000).keysym, 0x20, "a slow tap commits the old space and starts a new pair");
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天 ");
        choose_space(editor, &state, 2100, "好", Some("天气"));
        press_space(editor, &state, u32::MAX - 50);
        choose_space(editor, &state, 49, "天气", None);
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天 好天气");
    });
}

#[test]
fn prediction_single_space_uses_the_page_only_while_the_caret_matches() {
    in_editor(|editor, state, _| {
        let revision = open_prediction(&editor, &state, false);
        let space = Key { keysym: 0x20, ..Key::default() };
        assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, 1000), (space, true));
        editor.store.move_caret(1);
        assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, 1100), (space, false));
        assert_eq!(shown_revision(&state), 0);
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "今天");
    });
}

#[test]
fn prediction_double_space_does_not_replace_text_after_a_caret_move() {
    in_editor(|editor, state, _| {
        open_prediction(editor, &state, true);
        press_space(editor, &state, 1000);
        editor.store.move_caret(1);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        assert_eq!(prediction_space_key(&state, &editor.context, space, false, 1, 1100), (space, false));
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "今天 ");
        assert_eq!(editor.store.selection.get().acpStart, 1, "ending a space must not move the user's caret back");
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
        other.CreateContext(editor.client, 0, None::<&IUnknown>, &mut context, &mut cookie).unwrap();
        let context = context.unwrap();
        other.Push(&context).unwrap();
        on_focus(&state, Some(&other));
        let closed = shown_revision(&state) == 0;
        let esc_reaches_app = !sink.OnTestKeyDown(&context, WPARAM(0x1b), LPARAM(0)).unwrap().as_bool();
        other.Pop(TF_POPF_ALL).unwrap();
        assert!(closed && esc_reaches_app);
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "今天 ");
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
            let modified = Key { keysym: keysym::ESCAPE, modifiers: modifier::SHIFT, test: true };
            assert!(!prediction_escape_down(&state, Some(&editor.context), modified, false));
            if preview {
                assert!(sink.OnTestKeyDown(&editor.context, esc, down).unwrap().as_bool());
                assert!(sink.OnTestKeyDown(&editor.context, esc, down).unwrap().as_bool());
            }
            if actual {
                assert!(sink.OnKeyDown(&editor.context, esc, down).unwrap().as_bool());
                assert_eq!(shown_revision(&state), 0);
                assert!(sink.OnKeyDown(&editor.context, esc, repeat).unwrap().as_bool());
            }
            assert!(sink.OnTestKeyUp(&editor.context, esc, up).unwrap().as_bool());
            assert!(sink.OnTestKeyUp(&editor.context, esc, up).unwrap().as_bool());
            assert!(sink.OnKeyUp(&editor.context, esc, up).unwrap().as_bool());
            assert_eq!(shown_revision(&state), 0);
            assert!(!sink.OnTestKeyDown(&editor.context, esc, down).unwrap().as_bool());
            assert!(!sink.OnTestKeyUp(&editor.context, esc, up).unwrap().as_bool());
        }
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "今天今天今天");
    });
}
