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
        // 真实文本存储对越界位置返回 TS_E_INVALIDPOS；替身不能在 COM 回调里 panic，
        // 否则定时器在文本变短后读旧位置时会让整个测试进程中止。
        if start < 0 || start as usize > end || end > buffer.len() {
            return Err(TS_E_INVALIDPOS.into());
        }
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

fn press_space(editor: &Editor, state: &Rc<RefCell<State>>, ms: u32) -> Key {
    let key = Key {
        keysym: 0x20,
        ..Key::default()
    };
    let revision = shown_revision(state);
    let (key, belongs) = prediction_key(state, &editor.context, key, false, revision, ms);
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

mod focus;
mod pairs;

thread_local! {
    /// The app text that the test IMM32 window reports, with its caret at the end.
    static FEED: RefCell<Option<ComObject<Store>>> = const { RefCell::new(None) };
}

mod prediction;
mod typing;
