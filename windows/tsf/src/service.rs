//! TSF 文本服务：收按键、问服务进程、按应答组字 / 上屏 / 画候选窗。
//!
//! 每个用到输入法的线程有一个实例。按键流程：
//!
//! 1. `OnTestKeyDown`：把键翻成键码，带 `test` 问服务进程要不要；
//! 2. `OnKeyDown`：真正按下，拿到应答后开一个同步编辑会话改文档（上屏、更新编辑区、光标），
//!    再按编辑区的位置摆候选窗；
//! 3. 单按 Shift（按下到松开之间没有别的键）在 `OnKeyUp` 里切换中英文。
//!
//! 服务进程连不上时一律不吃键，应用照常收到按键。

use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::rc::{Rc, Weak};

use freetype_protocol::{Frame, Key, Request};
use windows::core::{implement, IUnknownImpl, Interface, Ref, Result, BOOL, GUID};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, RECT, WPARAM};
use windows::Win32::System::Variant::VT_I4;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetKeyboardState, ToUnicode};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection, ITfKeyEventSink,
    ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfRange, ITfSource, ITfTextInputProcessorEx,
    ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl, ITfThreadMgr, ITfThreadMgrEventSink,
    ITfThreadMgrEventSink_Impl, ITfCompartmentMgr, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_NONE, TF_ANCHOR_END, TF_ANCHOR_START, TF_DEFAULT_SELECTION,
    TF_ES_ASYNCDONTCARE, TF_ES_READWRITE, TF_ES_SYNC, TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE,
};

use crate::client::Client;
use crate::keys::{self, ShiftTap};
use crate::ui::{self, CandidateWindow, Click};
use crate::DllRef;

struct State {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    thread_sink_cookie: Option<u32>,
    sink: Option<ITfCompositionSink>,
    composition: Option<ITfComposition>,
    /// 最近一次按键所在的文档（候选窗点击时用）。
    context: Option<ITfContext>,
    client: Client,
    shift: ShiftTap,
    /// OnTestKeyUp 认出来的单按 Shift，等 OnKeyUp 去切换。
    shift_tapped: bool,
    window: Option<CandidateWindow>,
    caret: Option<RECT>,
}

#[implement(ITfTextInputProcessorEx, ITfKeyEventSink, ITfThreadMgrEventSink, ITfCompositionSink)]
pub struct TextService {
    state: Rc<RefCell<State>>,
    instance: HINSTANCE,
    _dll: DllRef,
}

impl TextService {
    pub fn new(instance: HINSTANCE) -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                thread_mgr: None,
                client_id: 0,
                thread_sink_cookie: None,
                sink: None,
                composition: None,
                context: None,
                client: Client::new(),
                shift: ShiftTap::default(),
                shift_tapped: false,
                window: None,
                caret: None,
            })),
            instance,
            _dll: DllRef::new(),
        }
    }
}

fn now_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// 编辑会话：把一段代码交给 TSF，在它给的编辑 cookie 下执行。
#[implement(ITfEditSession)]
struct EditSession {
    action: RefCell<Option<Box<dyn FnOnce(u32) -> Result<()>>>>,
    _dll: DllRef,
}

impl ITfEditSession_Impl for EditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        match self.action.borrow_mut().take() {
            Some(action) => action(ec),
            None => Ok(()),
        }
    }
}

fn edit(context: &ITfContext, client_id: u32, sync: bool, action: impl FnOnce(u32) -> Result<()> + 'static) {
    let session: ITfEditSession = EditSession {
        action: RefCell::new(Some(Box::new(action))),
        _dll: DllRef::new(),
    }
    .into();
    let flags = if sync {
        TF_ES_SYNC | TF_ES_READWRITE
    } else {
        TF_ES_ASYNCDONTCARE | TF_ES_READWRITE
    };
    unsafe {
        let _ = context.RequestEditSession(client_id, &session, flags);
    }
}

fn set_selection(context: &ITfContext, ec: u32, range: &ITfRange) -> Result<()> {
    let selection = TF_SELECTION {
        range: ManuallyDrop::new(Some(range.clone())),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_NONE,
            fInterimChar: false.into(),
        },
    };
    let result = unsafe { context.SetSelection(ec, &[selection.clone()]) };
    drop(ManuallyDrop::into_inner(selection.range));
    result
}

fn text_rect(context: &ITfContext, ec: u32, range: &ITfRange) -> Option<RECT> {
    unsafe {
        let view = context.GetActiveView().ok()?;
        let mut rect = RECT::default();
        let mut clipped = BOOL::default();
        view.GetTextExt(ec, range, &mut rect, &mut clipped).ok()?;
        (rect.right > rect.left || rect.bottom > rect.top || rect.left != 0).then_some(rect)
    }
}

fn selection_rect(context: &ITfContext, ec: u32) -> Option<RECT> {
    unsafe {
        let mut selection = [TF_SELECTION::default()];
        let mut fetched = 0u32;
        context
            .GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)
            .ok()?;
        let range = ManuallyDrop::into_inner(std::mem::take(&mut selection[0].range))?;
        (fetched > 0).then(|| text_rect(context, ec, &range)).flatten()
    }
}

/// 在编辑会话里按应答改文档：先上屏，再更新编辑区。
fn apply_document(state: &Rc<RefCell<State>>, context: &ITfContext, frame: &Frame, ec: u32) -> Result<()> {
    let (mut composition, sink) = {
        let s = state.borrow();
        (s.composition.clone(), s.sink.clone())
    };
    let result = (|| -> Result<Option<RECT>> {
        unsafe {
            if let Some(text) = &frame.commit {
                let text = wide(text);
                if let Some(comp) = composition.take() {
                    let range = comp.GetRange()?;
                    range.SetText(ec, 0, &text)?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    set_selection(context, ec, &range)?;
                    comp.EndComposition(ec)?;
                } else {
                    let insert: ITfInsertAtSelection = context.cast()?;
                    let range = insert.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &text)?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    set_selection(context, ec, &range)?;
                }
            }
            if !frame.preedit.is_empty() {
                let comp = match composition.take() {
                    Some(comp) => comp,
                    None => {
                        let insert: ITfInsertAtSelection = context.cast()?;
                        let range = insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
                        let start: ITfContextComposition = context.cast()?;
                        start.StartComposition(ec, &range, sink.as_ref())?
                    }
                };
                let range = comp.GetRange()?;
                range.SetText(ec, 0, &wide(&frame.preedit))?;
                let caret = range.Clone()?;
                caret.Collapse(ec, TF_ANCHOR_START)?;
                let mut moved = 0;
                let shift = frame.preedit_cursor as i32;
                caret.ShiftEnd(ec, shift, &mut moved, std::ptr::null())?;
                caret.ShiftStart(ec, shift, &mut moved, std::ptr::null())?;
                set_selection(context, ec, &caret)?;
                let rect = text_rect(context, ec, &range);
                composition = Some(comp);
                return Ok(rect.or_else(|| selection_rect(context, ec)));
            }
            if let Some(comp) = composition.take() {
                // 编辑区清空了（Esc、退格删完）
                let range = comp.GetRange()?;
                range.SetText(ec, 0, &[])?;
                comp.EndComposition(ec)?;
            }
            Ok(frame.hint.as_ref().and_then(|_| selection_rect(context, ec)))
        }
    })();
    let mut s = state.borrow_mut();
    s.composition = composition;
    match result {
        Ok(rect) => {
            s.caret = rect;
            Ok(())
        }
        Err(e) => {
            s.caret = None;
            Err(e)
        }
    }
}

/// 应答落到文档和候选窗上。
fn apply(state: &Rc<RefCell<State>>, context: &ITfContext, frame: Frame, sync: bool) {
    let client_id = state.borrow().client_id;
    let shared = Rc::new(frame);
    let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
    edit(context, client_id, sync, move |ec| apply_document(&state2, &context2, &frame2, ec));
    let s = state.borrow();
    if let Some(window) = &s.window {
        window.update(&shared, s.caret);
    }
}

fn keyboard_open(thread_mgr: &ITfThreadMgr) -> bool {
    unsafe {
        let Ok(compartments) = thread_mgr.cast::<ITfCompartmentMgr>() else {
            return true;
        };
        let Ok(compartment) = compartments.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) else {
            return true;
        };
        let Ok(value) = compartment.GetValue() else {
            return true;
        };
        let inner = &value.Anonymous.Anonymous;
        inner.vt != VT_I4 || inner.Anonymous.lVal != 0
    }
}

/// 虚拟键翻成键码；翻不出来（非 ASCII 字符、功能键）返回 None。
fn translate(wparam: WPARAM, lparam: LPARAM, test: bool) -> Option<Key> {
    let vk = wparam.0 as u16;
    let modifiers = keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) });
    if let Some(keysym) = keys::named_keysym(vk) {
        return Some(Key {
            keysym,
            modifiers,
            test,
        });
    }
    unsafe {
        let mut state = [0u8; 256];
        GetKeyboardState(&mut state).ok()?;
        // Ctrl、Alt 按着时也要拿到字母本身（Ctrl+C 是 c，不是控制字符）
        for vk in [0x11, 0x12, 0xa2, 0xa3, 0xa4, 0xa5] {
            state[vk] = 0;
        }
        let scan = ((lparam.0 >> 16) & 0xff) as u32;
        let mut buf = [0u16; 8];
        // 4：不改动键盘状态（不打断死键）
        let n = ToUnicode(u32::from(vk), scan, Some(&state), &mut buf, 4);
        if n != 1 {
            return None;
        }
        let keysym = char::from_u32(u32::from(buf[0])).and_then(keys::char_keysym)?;
        Some(Key {
            keysym,
            modifiers,
            test,
        })
    }
}

fn call(state: &Rc<RefCell<State>>, request: &Request) -> Option<Frame> {
    state.borrow_mut().client.call(request)?.frame
}

fn composing(state: &Rc<RefCell<State>>) -> bool {
    state.borrow().composition.is_some()
}

fn open(state: &Rc<RefCell<State>>) -> bool {
    state.borrow().thread_mgr.as_ref().is_some_and(keyboard_open)
}

fn on_click(state: &Weak<RefCell<State>>, click: Click) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let request = match click {
        Click::Select(index) => Request::Select { index },
        Click::Page(next) => Request::Page { next },
    };
    let Some(frame) = call(&state, &request) else {
        return;
    };
    let context = state.borrow().context.clone();
    if let Some(context) = context {
        apply(&state, &context, frame, false);
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        ui::set_click_handler(None);
        let mut s = self.state.borrow_mut();
        if let Some(thread_mgr) = s.thread_mgr.take() {
            unsafe {
                if let Ok(keystrokes) = thread_mgr.cast::<ITfKeystrokeMgr>() {
                    let _ = keystrokes.UnadviseKeyEventSink(s.client_id);
                }
                if let (Some(cookie), Ok(source)) = (s.thread_sink_cookie.take(), thread_mgr.cast::<ITfSource>()) {
                    let _ = source.UnadviseSink(cookie);
                }
            }
        }
        s.composition = None;
        s.context = None;
        s.sink = None;
        s.window = None;
        s.client.disconnect();
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, ptim: Ref<ITfThreadMgr>, tid: u32, _flags: u32) -> Result<()> {
        let thread_mgr = ptim.ok()?.clone();
        let key_sink: ITfKeyEventSink = self.to_interface();
        let thread_sink: ITfThreadMgrEventSink = self.to_interface();
        let composition_sink: ITfCompositionSink = self.to_interface();
        unsafe {
            let keystrokes: ITfKeystrokeMgr = thread_mgr.cast()?;
            keystrokes.AdviseKeyEventSink(tid, &key_sink, true)?;
            let source: ITfSource = thread_mgr.cast()?;
            let cookie = source.AdviseSink(&ITfThreadMgrEventSink::IID, &thread_sink).ok();
            let mut s = self.state.borrow_mut();
            s.thread_sink_cookie = cookie;
        }
        {
            let mut s = self.state.borrow_mut();
            s.thread_mgr = Some(thread_mgr);
            s.client_id = tid;
            s.sink = Some(composition_sink);
            s.window = CandidateWindow::new(self.instance);
        }
        let weak = Rc::downgrade(&self.state);
        ui::set_click_handler(Some(Box::new(move |click| on_click(&weak, click))));
        Ok(())
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> Result<()> {
        if !foreground.as_bool() {
            if let Some(window) = &self.state.borrow().window {
                window.hide();
            }
        }
        Ok(())
    }

    fn OnTestKeyDown(&self, _pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let vk = wparam.0 as u16;
        let repeat = lparam.0 & (1 << 30) != 0;
        self.state.borrow_mut().shift.key_down(vk, now_ms(), repeat);
        if keys::is_modifier(vk) || !open(&self.state) {
            return Ok(false.into());
        }
        let Some(key) = translate(wparam, lparam, true) else {
            // 组字时不认识的键也吃掉，免得漏给应用
            return Ok(composing(&self.state).into());
        };
        let handled = call(&self.state, &Request::Key { key }).is_some_and(|f| f.handled);
        Ok(handled.into())
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let vk = wparam.0 as u16;
        if keys::is_modifier(vk) || !open(&self.state) {
            return Ok(false.into());
        }
        let context = pic.ok()?.clone();
        self.state.borrow_mut().context = Some(context.clone());
        let Some(key) = translate(wparam, lparam, false) else {
            return Ok(composing(&self.state).into());
        };
        let frame = match call(&self.state, &Request::Key { key }) {
            Some(frame) => frame,
            None => {
                // 服务进程没了：结束编辑区，键交给应用
                if composing(&self.state) {
                    apply(&self.state, &context, Frame::default(), true);
                }
                return Ok(false.into());
            }
        };
        let handled = frame.handled;
        if handled || frame.commit.is_some() || composing(&self.state) {
            apply(&self.state, &context, frame, true);
        }
        Ok(handled.into())
    }

    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        let mut s = self.state.borrow_mut();
        let tapped = s.shift.key_up(wparam.0 as u16, now_ms());
        s.shift_tapped = tapped;
        Ok(tapped.into())
    }

    fn OnKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        let tapped = {
            let mut s = self.state.borrow_mut();
            let tapped = std::mem::take(&mut s.shift_tapped) || s.shift.key_up(wparam.0 as u16, now_ms());
            tapped && wparam.0 as u16 != 0
        };
        if !tapped || !open(&self.state) {
            return Ok(false.into());
        }
        let context = pic.ok()?.clone();
        if let Some(frame) = call(&self.state, &Request::ToggleMode) {
            apply(&self.state, &context, frame, true);
        }
        Ok(true.into())
    }

    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(false.into())
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnSetFocus(&self, _focus: Ref<ITfDocumentMgr>, _previous: Ref<ITfDocumentMgr>) -> Result<()> {
        let mut s = self.state.borrow_mut();
        s.shift.cancel();
        if let Some(window) = &s.window {
            window.hide();
        }
        Ok(())
    }

    fn OnPushContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    /// 应用结束了编辑区（点了别处、切了输入框）：编辑区的字留在文档里，会话清空。
    fn OnCompositionTerminated(&self, _ec: u32, _composition: Ref<ITfComposition>) -> Result<()> {
        {
            let mut s = self.state.borrow_mut();
            s.composition = None;
            if let Some(window) = &s.window {
                window.hide();
            }
        }
        let _ = call(&self.state, &Request::Reset { commit: false });
        Ok(())
    }
}
