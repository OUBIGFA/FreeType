//! Real Windows edit sessions and ranges, with a disposable app-side text store.
use super::*;
use std::cell::Cell;
use windows::core::{ComObject, ComObjectInner, IUnknown, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::LRESULT;
use windows::Win32::Foundation::{E_FAIL, E_NOTIMPL, S_OK};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDataObject, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, FORMATETC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, HWND_MESSAGE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSW,
};

fn unused<T>() -> Result<T> {
    Err(E_NOTIMPL.into())
}

#[implement(ITfInputScope)]
struct Scope(InputScope);

impl ITfInputScope_Impl for Scope_Impl {
    fn GetInputScopes(&self, scopes: *mut *mut InputScope, count: *mut u32) -> Result<()> {
        unsafe {
            let value =
                windows::Win32::System::Com::CoTaskMemAlloc(std::mem::size_of::<InputScope>())
                    .cast::<InputScope>();
            assert!(!value.is_null());
            *value = self.0;
            *scopes = value;
            *count = 1;
        }
        Ok(())
    }
    fn GetPhrase(&self, _: *mut *mut windows::core::BSTR, _: *mut u32) -> Result<()> {
        unused()
    }
    fn GetRegularExpression(&self) -> Result<windows::core::BSTR> {
        unused()
    }
    fn GetSRGS(&self) -> Result<windows::core::BSTR> {
        unused()
    }
    fn GetXML(&self) -> Result<windows::core::BSTR> {
        unused()
    }
}

#[derive(Default)]
#[implement(ITextStoreACP, ITfContextOwnerCompositionSink)]
struct Store {
    text: RefCell<Vec<u16>>,
    selection: Cell<TS_SELECTION_ACP>,
    /// How often a text service moved the host's selection.
    selection_sets: Cell<u32>,
    completed_writes: RefCell<Option<Vec<(String, bool)>>>,
    sink: RefCell<Option<ITextStoreACPSink>>,
    /// Chromium and CUAS both advertise this; it does not describe text visibility.
    transitory: Cell<bool>,
    deny_read: Cell<bool>,
    deny_next_write: Cell<bool>,
    deny_sync_write: Cell<bool>,
    scope: Cell<Option<InputScope>>,
    scope_requested: Cell<bool>,
    /// CUAS forwards committed text to the app, then empties its document.
    commit_only: Cell<bool>,
    app_text: RefCell<Vec<u16>>,
    composing: Cell<bool>,
    /// The app window that CUAS reports for its emulated document.
    window: Cell<HWND>,
    fail_window: Cell<bool>,
    extent: Cell<Option<RECT>>,
    clipped: Cell<bool>,
}

impl Store {
    fn replace(&self, start: i32, end: i32, text: &[u16]) -> TS_TEXTCHANGE {
        self.text
            .borrow_mut()
            .splice(start as usize..end as usize, text.iter().copied());
        TS_TEXTCHANGE {
            acpStart: start,
            acpOldEnd: end,
            acpNewEnd: start + text.len() as i32,
        }
    }

    fn move_caret(&self, at: i32) {
        self.selection.set(TS_SELECTION_ACP {
            acpStart: at,
            acpEnd: at,
            ..Default::default()
        });
        let sink = self.sink.borrow().clone().unwrap();
        unsafe {
            sink.OnSelectionChange().unwrap();
        }
    }

    fn app_insert(&self, text: &str) {
        let selection = self.selection.get();
        self.app_replace(selection.acpStart, selection.acpEnd, text);
    }

    fn app_replace(&self, start: i32, end: i32, text: &str) {
        let change = self.replace(start, end, &wide(text));
        let sink = self.sink.borrow().clone().unwrap();
        unsafe {
            sink.OnTextChange(TEXT_STORE_TEXT_CHANGE_FLAGS(0), &change)
                .unwrap();
        }
        self.move_caret(change.acpNewEnd);
    }
}

impl ITextStoreACP_Impl for Store_Impl {
    fn AdviseSink(&self, _: *const GUID, sink: Ref<IUnknown>, _: u32) -> Result<()> {
        *self.sink.borrow_mut() = Some(sink.ok()?.cast()?);
        Ok(())
    }
    fn UnadviseSink(&self, _: Ref<IUnknown>) -> Result<()> {
        self.sink.borrow_mut().take();
        Ok(())
    }
    fn RequestLock(&self, flags: u32) -> Result<HRESULT> {
        if self.deny_sync_write.get()
            && flags & TS_LF_SYNC == TS_LF_SYNC
            && flags & TS_LF_READWRITE.0 == TS_LF_READWRITE.0
        {
            return Ok(TS_E_SYNCHRONOUS);
        }
        if flags & TS_LF_READWRITE.0 == TS_LF_READWRITE.0 && self.deny_next_write.replace(false) {
            return Ok(TS_E_SYNCHRONOUS);
        }
        if self.deny_read.get() && flags & TS_LF_READWRITE.0 == TS_LF_READ.0 {
            return Ok(TS_E_SYNCHRONOUS);
        }
        let sink = self
            .sink
            .borrow()
            .clone()
            .ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
        let result = unsafe { sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(flags)) }
            .err()
            .map_or(S_OK, |e| e.code());
        if self.commit_only.get()
            && !self.composing.get()
            && flags & TS_LF_READWRITE.0 == TS_LF_READWRITE.0
            && result.is_ok()
        {
            let size = self.text.borrow().len() as i32;
            if size > 0 {
                self.app_text
                    .borrow_mut()
                    .extend_from_slice(&self.text.borrow());
                self.app_replace(0, size, "");
            }
        }
        if flags & TS_LF_READWRITE.0 == TS_LF_READWRITE.0 && result.is_ok() {
            if let Some(writes) = self.completed_writes.borrow_mut().as_mut() {
                writes.push((
                    String::from_utf16_lossy(&self.text.borrow()),
                    self.composing.get(),
                ));
            }
        }
        Ok(result)
    }
    fn GetStatus(&self) -> Result<TS_STATUS> {
        Ok(TS_STATUS {
            dwStaticFlags: if self.transitory.get() {
                TS_SS_TRANSITORY
            } else {
                0
            },
            ..Default::default()
        })
    }
    fn QueryInsert(
        &self,
        start: i32,
        end: i32,
        _: u32,
        out_start: *mut i32,
        out_end: *mut i32,
    ) -> Result<()> {
        unsafe {
            *out_start = start;
            *out_end = end;
        }
        Ok(())
    }
    fn GetSelection(
        &self,
        _: u32,
        count: u32,
        out: *mut TS_SELECTION_ACP,
        fetched: *mut u32,
    ) -> Result<()> {
        assert!(count > 0);
        unsafe {
            *out = self.selection.get();
            *fetched = 1;
        }
        Ok(())
    }
    fn SetSelection(&self, count: u32, selection: *const TS_SELECTION_ACP) -> Result<()> {
        assert_eq!(count, 1);
        self.selection.set(unsafe { *selection });
        self.selection_sets.set(self.selection_sets.get() + 1);
        Ok(())
    }
    fn GetText(
        &self,
        start: i32,
        end: i32,
        text: PWSTR,
        capacity: u32,
        read: *mut u32,
        runs: *mut TS_RUNINFO,
        run_capacity: u32,
        run_count: *mut u32,
        next: *mut i32,
    ) -> Result<()> {
        let buffer = self.text.borrow();
        let end = if end < 0 { buffer.len() } else { end as usize };
        let slice = &buffer[start as usize..end];
        let size = slice.len().min(capacity as usize);
        unsafe {
            if size != 0 {
                std::ptr::copy_nonoverlapping(slice.as_ptr(), text.0, size);
            }
            *read = size as u32;
            *run_count = u32::from(run_capacity > 0 && !slice.is_empty());
            if *run_count > 0 {
                *runs = TS_RUNINFO {
                    uCount: slice.len() as u32,
                    r#type: TS_RT_PLAIN,
                };
            }
            *next = start + if capacity == 0 { slice.len() } else { size } as i32;
        }
        Ok(())
    }
    fn SetText(
        &self,
        _: u32,
        start: i32,
        end: i32,
        text: &PCWSTR,
        count: u32,
    ) -> Result<TS_TEXTCHANGE> {
        let text = if count == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(text.0, count as usize) }
        };
        Ok(self.replace(start, end, text))
    }
    fn GetFormattedText(&self, _: i32, _: i32) -> Result<IDataObject> {
        unused()
    }
    fn GetEmbedded(&self, _: i32, _: *const GUID, _: *const GUID) -> Result<IUnknown> {
        unused()
    }
    fn QueryInsertEmbedded(&self, _: *const GUID, _: *const FORMATETC) -> Result<BOOL> {
        unused()
    }
    fn InsertEmbedded(&self, _: u32, _: i32, _: i32, _: Ref<IDataObject>) -> Result<TS_TEXTCHANGE> {
        unused()
    }
    fn InsertTextAtSelection(
        &self,
        flags: u32,
        text: &PCWSTR,
        count: u32,
        start: *mut i32,
        end: *mut i32,
        change: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        let at = self.selection.get();
        unsafe {
            if !start.is_null() {
                *start = at.acpStart;
            }
            if flags & TS_IAS_QUERYONLY == 0 {
                let edit = self.SetText(0, at.acpStart, at.acpEnd, text, count)?;
                if !end.is_null() {
                    *end = edit.acpNewEnd;
                }
                if !change.is_null() {
                    *change = edit;
                }
            } else if !end.is_null() {
                *end = at.acpEnd;
            }
        }
        Ok(())
    }
    fn InsertEmbeddedAtSelection(
        &self,
        _: u32,
        _: Ref<IDataObject>,
        _: *mut i32,
        _: *mut i32,
        _: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        unused()
    }
    fn RequestSupportedAttrs(&self, _: u32, count: u32, ids: *const GUID) -> Result<()> {
        self.scope_requested.set(
            count > 0
                && unsafe { std::slice::from_raw_parts(ids, count as usize) }
                    .contains(&GUID_PROP_INPUTSCOPE),
        );
        Ok(())
    }
    fn RequestAttrsAtPosition(
        &self,
        _: i32,
        count: u32,
        ids: *const GUID,
        flags: u32,
    ) -> Result<()> {
        self.RequestSupportedAttrs(flags, count, ids)
    }
    fn RequestAttrsTransitioningAtPosition(
        &self,
        _: i32,
        _: u32,
        _: *const GUID,
        _: u32,
    ) -> Result<()> {
        unused()
    }
    fn FindNextAttrTransition(
        &self,
        _: i32,
        _: i32,
        _: u32,
        _: *const GUID,
        _: u32,
        _: *mut i32,
        _: *mut BOOL,
        _: *mut i32,
    ) -> Result<()> {
        unused()
    }
    fn RetrieveRequestedAttrs(
        &self,
        count: u32,
        attrs: *mut TS_ATTRVAL,
        fetched: *mut u32,
    ) -> Result<()> {
        unsafe {
            *fetched = 0;
        }
        if count > 0 && self.scope_requested.replace(false) {
            if let Some(scope) = self.scope.get() {
                let scope: ITfInputScope = Scope(scope).into();
                unsafe {
                    *attrs = TS_ATTRVAL {
                        idAttr: GUID_PROP_INPUTSCOPE,
                        dwOverlapId: 0,
                        varValue: VARIANT::from(scope.cast::<IUnknown>()?),
                    };
                    *fetched = 1;
                }
            }
        }
        Ok(())
    }
    fn GetEndACP(&self) -> Result<i32> {
        Ok(self.text.borrow().len() as i32)
    }
    fn GetActiveView(&self) -> Result<u32> {
        Ok(0)
    }
    fn GetACPFromPoint(&self, _: u32, _: *const POINT, _: u32) -> Result<i32> {
        unused()
    }
    fn GetTextExt(&self, _: u32, _: i32, _: i32, at: *mut RECT, clipped: *mut BOOL) -> Result<()> {
        unsafe {
            *at = self.extent.get().unwrap_or(RECT {
                left: 100,
                top: 100,
                right: 102,
                bottom: 120,
            });
            *clipped = self.clipped.get().into();
        }
        Ok(())
    }
    fn GetScreenExt(&self, _: u32) -> Result<RECT> {
        unused()
    }
    fn GetWnd(&self, _: u32) -> Result<HWND> {
        if self.fail_window.get() {
            return Err(E_FAIL.into());
        }
        Ok(self.window.get())
    }
}

impl ITfContextOwnerCompositionSink_Impl for Store_Impl {
    fn OnStartComposition(&self, _: Ref<ITfCompositionView>) -> Result<BOOL> {
        self.composing.set(true);
        Ok(true.into())
    }
    fn OnUpdateComposition(&self, _: Ref<ITfCompositionView>, _: Ref<ITfRange>) -> Result<()> {
        Ok(())
    }
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
        let manager: ITfThreadMgrEx =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).unwrap();
        let mut client = 0;
        manager
            .ActivateEx(
                &mut client,
                TF_TMAE_NOACTIVATETIP | TF_TMAE_NOACTIVATEKEYBOARDLAYOUT,
            )
            .unwrap();
        let document = manager.CreateDocumentMgr().unwrap();
        let store = Store::default().into_object();
        let (mut context, mut cookie) = (None, 0);
        let text_store: IUnknown = store.as_interface::<ITextStoreACP>().cast().unwrap();
        document
            .CreateContext(client, 0, &text_store, &mut context, &mut cookie)
            .unwrap();
        let context = context.unwrap();
        document.Push(&context).unwrap();
        Self {
            manager,
            document,
            context,
            client,
            store,
        }
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            self.document.Pop(TF_POPF_ALL).unwrap();
            self.manager.Deactivate().unwrap();
        }
    }
}

fn in_editor(test: impl FnOnce(&Editor, Rc<RefCell<State>>, ITfKeyEventSink)) {
    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
    }
    let _apartment = Apartment;
    let editor = unsafe { Editor::new() };
    let _keyboard = TestKeyboard::with_state([0; 256]);
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
        test(
            &editor,
            state.clone(),
            service.to_interface::<ITfKeyEventSink>(),
        );
    }));
    // Release real compositions and their sink cycle even when an assertion fails.
    finish_prediction_space(&state, false, true);
    apply(&state, Some(&editor.context), Frame::default(), true);
    state.borrow_mut().sink = None;
    ui::set_click_handler(None);
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

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

fn open_prediction(editor: &Editor, state: &Rc<RefCell<State>>, double: bool) -> u64 {
    let frame = Frame {
        handled: true,
        commit: Some("今天".into()),
        candidates: vec!["下雨".into()],
        cursor: Some(0),
        predict: true,
        predict_double_space: double,
        page_revision: 1,
        font_family: "Microsoft YaHei".into(),
        font_size: 18,
        ..Frame::default()
    };
    apply(state, Some(&editor.context), frame, true);
    assert_eq!(shown_revision(state), 1);
    assert!(
        state.borrow().prediction_caret.is_some(),
        "commit anchored at the real TSF caret"
    );
    1
}

pub(super) fn use_test_host(state: &Rc<RefCell<State>>) {
    let data = std::env::temp_dir().join(format!(
        "freetype-digit-{}-{}",
        std::process::id(),
        unsafe { GetCurrentThreadId() }
    ));
    let mut host = freetype_server::host::Host::new(freetype_server::store::Store::new(data));
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str("ce shi\t测试\t1000\n").unwrap();
    host.install_engine(Ok(wowtype_core::Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    )));
    host.connect(1, "explorer.exe".into());
    assert!(!host.config().predict_next, "此回归不依赖上屏联想");
    state.borrow_mut().client = Client::with_test_host(host);
}

/// Only this private desktop's test thread sees the synthetic keyboard state.
struct TestKeyboard([u8; 256]);

impl TestKeyboard {
    fn with_state(pressed: [u8; 256]) -> Self {
        let mut previous = [0; 256];
        unsafe {
            GetKeyboardState(&mut previous).unwrap();
        }
        unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&pressed).unwrap();
        }
        Self(previous)
    }

    fn ctrl_space() -> Self {
        let mut pressed = [0; 256];
        pressed[0x11] = 0x80;
        pressed[0x20] = 0x80;
        Self::with_state(pressed)
    }
}

impl Drop for TestKeyboard {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&self.0).unwrap();
        }
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
    use windows::Win32::UI::WindowsAndMessaging::{
        PeekMessageW, PostThreadMessageW, MSG, PM_REMOVE, WM_APP,
    };
    let mut message = MSG::default();
    loop {
        unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_APP + 90, WPARAM(0), LPARAM(0)).unwrap();
            assert!(
                PeekMessageW(&mut message, None, WM_APP + 90, WM_APP + 90, PM_REMOVE).as_bool()
            );
        }
        let ms = unsafe { GetMessageTime() } as u32;
        if Some(ms) != previous {
            return ms;
        }
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

fn commit_test_word(editor: &Editor, state: &Rc<RefCell<State>>, sink: &ITfKeyEventSink) {
    use_test_host(state);
    for vk in [b'C', b'E', b'S', b'H', b'I', b' '] {
        let key = WPARAM(usize::from(vk));
        unsafe {
            assert!(sink
                .OnTestKeyDown(&editor.context, key, LPARAM(0))
                .unwrap()
                .as_bool());
            assert!(sink
                .OnKeyDown(&editor.context, key, LPARAM(0))
                .unwrap()
                .as_bool());
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

thread_local! {
    /// The app text that the test IMM32 window reports, with its caret at the end.
    static FEED: RefCell<Option<ComObject<Store>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn feed_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message != WM_IME_REQUEST || wparam.0 != IMR_DOCUMENTFEED as usize {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    FEED.with_borrow(|store| {
        let Some(store) = store else {
            return LRESULT(0);
        };
        let text = store.app_text.borrow();
        let head = std::mem::size_of::<RECONVERTSTRING>() as u32;
        let (len, size) = (text.len() as u32, head + text.len() as u32 * 2);
        let info = lparam.0 as *mut RECONVERTSTRING;
        if info.is_null() {
            return LRESULT(size as isize);
        }
        unsafe {
            if (*info).dwSize < size {
                return LRESULT(0);
            }
            *info = RECONVERTSTRING {
                dwSize: size,
                dwVersion: 0,
                dwStrLen: len,
                dwStrOffset: head,
                dwCompStrLen: 0,
                dwCompStrOffset: len * 2,
                dwTargetStrLen: 0,
                dwTargetStrOffset: len * 2,
            };
            std::ptr::copy_nonoverlapping(
                text.as_ptr(),
                info.cast::<u8>().add(head as usize).cast::<u16>(),
                text.len(),
            );
        }
        LRESULT(size as isize)
    })
}

fn set_compartment(editor: &Editor, guid: &GUID, value: &VARIANT) {
    let mgr: ITfCompartmentMgr = editor.document.cast().unwrap();
    unsafe {
        mgr.GetCompartment(guid)
            .unwrap()
            .SetValue(editor.client, value)
            .unwrap();
    }
}

/// Point the editor's Transitory Extensions compartment at `parent`, as Windows does.
fn set_parent(editor: &Editor, parent: &IUnknown) {
    set_compartment(
        editor,
        &GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT,
        &VARIANT::from(parent.clone()),
    );
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
        set_compartment(
            editor,
            &GUID_COMPARTMENT_EMULATED_DOCUMENTMGR,
            &VARIANT::from(1i32),
        );
        set_parent(
            editor,
            &editor.store.as_interface::<ITextStoreACP>().cast().unwrap(),
        );
        unsafe { Self::window(editor, answers_feed) }
    }

    /// Only the app window that answers IMR_DOCUMENTFEED, for hosts set up by the test.
    unsafe fn window(editor: &Editor, answers_feed: bool) -> Self {
        if !answers_feed {
            return Self(None);
        }
        let class = windows::core::w!("FreeTypeImm32FeedTest");
        let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
        let window = unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(feed_proc),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            });
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )
            .unwrap()
        };
        editor.store.window.set(window);
        FEED.set(Some(editor.store.clone()));
        Self(Some(window))
    }
}

impl Drop for Imm32Host {
    fn drop(&mut self) {
        FEED.set(None);
        if let Some(window) = self.0 {
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
    }
}

fn press_digit(editor: &Editor, sink: &ITfKeyEventSink, digit: u8) -> bool {
    let key = WPARAM(usize::from(b'0' + digit));
    let preview = unsafe {
        sink.OnTestKeyDown(&editor.context, key, LPARAM(0))
            .unwrap()
            .as_bool()
    };
    if preview {
        assert!(
            unsafe {
                sink.OnKeyDown(&editor.context, key, LPARAM(0))
                    .unwrap()
                    .as_bool()
            },
            "预询问已接管，正式按下不能放行"
        );
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
        store.selection.set(TS_SELECTION_ACP {
            acpStart: text.encode_utf16().count() as i32,
            acpEnd: text.encode_utf16().count() as i32,
            ..Default::default()
        });
        let document = unsafe { editor.manager.CreateDocumentMgr().unwrap() };
        let (mut context, mut cookie) = (None, 0);
        let unknown: IUnknown = store.as_interface::<ITextStoreACP>().cast().unwrap();
        unsafe {
            document
                .CreateContext(editor.client, 0, &unknown, &mut context, &mut cookie)
                .unwrap();
            document.Push(&context.unwrap()).unwrap();
        }
        set_parent(editor, &document.cast().unwrap());
        Self {
            document,
            _store: store,
        }
    }
}

impl Drop for ParentDocument {
    fn drop(&mut self) {
        unsafe {
            let _ = self.document.Pop(TF_POPF_ALL);
        }
    }
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

fn press_space(editor: &Editor, state: &Rc<RefCell<State>>, ms: u32) -> Key {
    let key = Key {
        keysym: 0x20,
        ..Key::default()
    };
    let revision = shown_revision(state);
    let (key, belongs) = prediction_space_key(state, &editor.context, key, false, revision, ms);
    assert!(belongs);
    key
}

fn choose_space(
    editor: &Editor,
    state: &Rc<RefCell<State>>,
    ms: u32,
    word: &str,
    next: Option<&str>,
) {
    assert_eq!(press_space(editor, state, ms).keysym, keysym::SPACE_DOUBLE);
    let revision = shown_revision(state) + 1;
    apply(
        state,
        Some(&editor.context),
        Frame {
            handled: true,
            replace_space: true,
            commit: Some(word.into()),
            candidates: next.into_iter().map(str::to_owned).collect(),
            predict: next.is_some(),
            predict_double_space: next.is_some(),
            cursor: next.map(|_| 0),
            page_revision: revision,
            font_family: "Microsoft YaHei".into(),
            font_size: 18,
            ..Frame::default()
        },
        true,
    );
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

/// Issue #13：习惯连打左右两半的用户，在补全的左移键送达前就按下右半标点。
/// 这时光标还在补全的右标点后面，右键只应撤销待发的左移、不再插入第二个右标点。
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
        assert!(
            state.borrow().caret_move.is_none(),
            "the pending Left must be dropped"
        );
        let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
            &mut message,
            None,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
        )
        .as_bool()
        {
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
        }
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
