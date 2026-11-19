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

fn unused<T>() -> Result<T> { Err(E_NOTIMPL.into()) }

#[derive(Default)]
#[implement(ITextStoreACP)]
struct Store {
    text: RefCell<Vec<u16>>,
    selection: Cell<TS_SELECTION_ACP>,
    sink: RefCell<Option<ITextStoreACPSink>>,
    /// Like CUAS for IMM32 apps: only composition text lives here, app text never does.
    transitory: Cell<bool>,
    deny_read: Cell<bool>,
    /// CUAS forwards committed text to the app, then empties its document.
    commit_only: Cell<bool>,
    app_text: RefCell<Vec<u16>>,
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
        if self.commit_only.get() && flags & TS_LF_READWRITE.0 == TS_LF_READWRITE.0 && result.is_ok() {
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
    fn GetWnd(&self, _: u32) -> Result<HWND> { unused() }
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

fn in_editor(test: impl FnOnce(Editor, Rc<RefCell<State>>, ITfKeyEventSink)) {
    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) { unsafe { CoUninitialize(); } }
    }
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap(); }
    let _apartment = Apartment;
    let editor = unsafe { Editor::new() };
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let service = TextService::new(instance);
    let state = service.state.clone();
    {
        let mut s = state.borrow_mut();
        s.client = Client::without_server();
        s.window = CandidateWindow::new(instance);
        s.context = Some(editor.context.clone());
        s.client_id = editor.client;
    }
    test(editor, state, service.into());
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

#[test]
fn prediction_double_space_uses_the_app_inserted_range_without_key_up() {
    in_editor(|editor, state, _| {
        // Chromium/Devin text stores advertise TS_SS_TRANSITORY but still expose
        // the application's committed space through the text store.
        editor.store.transitory.set(true);
        let revision = open_prediction(&editor, &state, true);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        let classify = |key, repeat, ms| prediction_space_key(&state, &editor.context, key, repeat, revision, ms);
        // Repeated previews are not taps. The app receives the first space and reports a text change.
        assert_eq!(classify(space, false, 1000), (space, true));
        assert_eq!(classify(space, false, 1000), (space, true));
        on_focus(&state, Some(&editor.document));
        assert_eq!(shown_revision(&state), revision, "same input context keeps the visible page");
        editor.store.app_insert(" ");
        assert_eq!(classify(space, true, 1050), (space, true), "held Space is not a double tap");
        assert_eq!(classify(space, false, 1100).0.keysym, keysym::SPACE_DOUBLE);
        assert_eq!(classify(Key { test: false, ..space }, false, 1100).0.keysym, keysym::SPACE_DOUBLE);
        apply(&state, Some(&editor.context), Frame {
            handled: true, replace_space: true, commit: Some("下雨".into()), ..Frame::default()
        }, true);
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "今天下雨");
    });
}

#[test]
fn prediction_double_space_continues_after_delayed_committed_text_notifications() {
    in_editor(|editor, state, _| {
        editor.store.transitory.set(true);
        open_prediction(&editor, &state, true);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        for (revision, word, next, expected) in [
            (1, "天气", "不错", "今天天气"),
            (2, "不错", "适合", "今天天气不错"),
            (3, "适合", "散步", "今天天气不错适合"),
        ] {
            let start = editor.store.text.borrow().len() as i32;
            let ms = revision as u32 * 1000;
            assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, ms), (space, true));
            editor.store.app_insert(" ");
            assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, ms + 100).0.keysym, keysym::SPACE_DOUBLE);
            apply(&state, Some(&editor.context), Frame {
                handled: true, replace_space: true, commit: Some(word.into()), candidates: vec![next.into()],
                predict: true, predict_double_space: true, cursor: Some(0), page_revision: revision + 1,
                font_family: "Microsoft YaHei".into(), font_size: 18, ..Frame::default()
            }, true);
            assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), expected);
            assert!(state.borrow().space_replace.is_none(), "a readable app space must use the TSF range, not injected backspace");
            // Chromium updates its text store again after OnLockGranted. A stale
            // renderer snapshot followed by the committed suffix must not strand
            // the next prediction's caret at the beginning of that suffix.
            editor.store.app_replace(start, start + 2, "");
            editor.store.app_insert(word);
            assert_eq!(shown_revision(&state), revision + 1);
        }
    });
}

#[test]
fn prediction_double_space_in_a_transitory_document_uses_distinct_down_messages() {
    in_editor(|editor, state, _| {
        editor.store.transitory.set(true);
        let revision = open_prediction(&editor, &state, true);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        let classify = |key, ms| prediction_space_key(&state, &editor.context, key, false, revision, ms);
        // The app types the first space itself; the document never shows it.
        assert_eq!(classify(space, 1000), (space, true));
        assert_eq!(classify(space, 1000), (space, true), "a repeated preview of the same press is not a double tap");
        assert_eq!(classify(space, 1100).0.keysym, keysym::SPACE_DOUBLE);
        assert_eq!(classify(Key { test: false, ..space }, 1100).0.keysym, keysym::SPACE_DOUBLE);
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "今天", "classification cannot change the document");
    });
}

#[test]
fn prediction_transitory_commit_does_not_require_a_backspace_key_up_callback() {
    use windows::Win32::UI::WindowsAndMessaging::SetMessageExtraInfo;
    in_editor(|editor, state, sink| unsafe {
        editor.store.transitory.set(true);
        editor.store.commit_only.set(true);
        open_prediction(&editor, &state, true);
        editor.store.app_text.borrow_mut().push(u16::from(b' '));
        defer_space_replace(&state, &editor.context, Frame {
            handled: true, replace_space: true, commit: Some("下雨".into()), ..Frame::default()
        });
        let previous = SetMessageExtraInfo(LPARAM(keys::OWN_INPUT_TAG as isize));
        // IMM32 hosts need not offer key-up events. An owned Space after the
        // Backspace pair completes the commit through normal key-down delivery.
        let back = sink.OnTestKeyDown(&editor.context, WPARAM(0x08), LPARAM(0)).unwrap().as_bool();
        let back_down = sink.OnKeyDown(&editor.context, WPARAM(0x08), LPARAM(0)).unwrap().as_bool();
        // The app, not the composition-only document, handles the passed key.
        if !back && !back_down { editor.store.app_text.borrow_mut().pop(); }
        let preview = sink.OnTestKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool();
        let before = String::from_utf16_lossy(&editor.store.app_text.borrow());
        let handled = sink.OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool();
        let duplicate = sink.OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool();
        SetMessageExtraInfo(previous);
        assert!(!back && !back_down, "the app must receive the backspace before the commit marker");
        assert!(preview && handled && duplicate, "the owned commit marker must not become an app space");
        assert_eq!(before, "今天", "preview must not commit ahead of the app's backspace");
        assert_eq!(String::from_utf16_lossy(&editor.store.app_text.borrow()), "今天下雨");
        assert!(editor.store.text.borrow().is_empty(), "committed app text is absent from the CUAS document");
    });
}

#[test]
fn prediction_transitory_double_space_survives_unavailable_read_sessions() {
    in_editor(|editor, state, _| {
        editor.store.transitory.set(true);
        let revision = open_prediction(&editor, &state, true);
        editor.store.deny_read.set(true);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, 1000), (space, true));
        assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, 1100).0.keysym, keysym::SPACE_DOUBLE);
    });
}

#[test]
fn prediction_transitory_pending_commit_cannot_overwrite_new_input() {
    use windows::Win32::UI::WindowsAndMessaging::SetMessageExtraInfo;
    in_editor(|editor, state, sink| unsafe {
        open_prediction(&editor, &state, true);
        defer_space_replace(&state, &editor.context, Frame {
            handled: true, replace_space: true, commit: Some("下雨".into()), ..Frame::default()
        });
        note_key_down(&state, WPARAM(0x4e), LPARAM(0));
        state.borrow_mut().sink = Some(sink.cast().unwrap());
        apply(&state, Some(&editor.context), Frame {
            handled: true, preedit: "n".into(), preedit_cursor: 1, ..Frame::default()
        }, true);
        let previous = SetMessageExtraInfo(LPARAM(keys::OWN_INPUT_TAG as isize));
        let back = sink.OnTestKeyDown(&editor.context, WPARAM(0x08), LPARAM(0)).unwrap().as_bool();
        let marker = sink.OnKeyDown(&editor.context, WPARAM(0x20), LPARAM(0)).unwrap().as_bool();
        SetMessageExtraInfo(previous);
        let text = String::from_utf16_lossy(&editor.store.text.borrow());
        let composing = composing(&state);
        apply(&state, Some(&editor.context), Frame::default(), true);
        state.borrow_mut().sink = None;
        assert!(back && marker, "cancelled helper keys cannot reach the new input");
        assert_eq!(text, "今天n");
        assert!(composing, "the user's new composition must survive");
    });
}

#[test]
fn prediction_double_space_respects_message_timing_and_ignores_previews() {
    in_editor(|editor, state, _| {
        editor.store.transitory.set(true);
        let revision = open_prediction(&editor, &state, true);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        let classify = |ms, repeat| prediction_space_key(&state, &editor.context, space, repeat, revision, ms).0.keysym;
        assert_eq!(classify(1000, false), 0x20);
        assert_eq!(classify(1000, false), 0x20, "duplicate preview cannot count as another tap");
        assert_eq!(classify(2000, false), 0x20, "a slow second tap begins a new pair");
        assert_eq!(classify(2100, true), 0x20, "holding Space cannot select");
        assert_eq!(classify(2100, false), keysym::SPACE_DOUBLE);
        state.borrow_mut().space = None;
        assert_eq!(classify(u32::MAX - 50, false), 0x20);
        assert_eq!(classify(49, false), keysym::SPACE_DOUBLE, "Windows message time may wrap");
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
        let revision = open_prediction(&editor, &state, true);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, 1000).0, space);
        editor.store.app_insert(" ");
        assert_eq!(prediction_space_key(&state, &editor.context, space, false, revision, 1100).0.keysym, keysym::SPACE_DOUBLE);
        editor.store.move_caret(1);
        let doc = editor.context.clone();
        let verdict = Rc::new(Cell::new(None));
        let result = verdict.clone();
        edit(&editor.context, editor.client, true, move |ec| {
            result.set(Some(replace_verified_space(&state, &doc, ec, &wide("下雨")).is_err()));
            Ok(())
        });
        assert_eq!(verdict.get(), Some(true));
        assert_eq!(String::from_utf16_lossy(&editor.store.text.borrow()), "今天 ");
    });
}

#[test]
fn prediction_does_not_follow_focus_to_another_input_context() {
    in_editor(|editor, state, sink| unsafe {
        let revision = open_prediction(&editor, &state, true);
        let space = Key { keysym: 0x20, test: true, ..Key::default() };
        prediction_space_key(&state, &editor.context, space, false, revision, 1000);
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
