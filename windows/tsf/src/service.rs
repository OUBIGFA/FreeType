//! TSF 文本服务：收按键、问服务进程、按应答组字 / 上屏 / 画候选窗。
//!
//! 每个用到输入法的线程有一个实例。按键流程：
//!
//! 1. `OnTestKeyDown`：把键翻成键码，带 `test` 问服务进程要不要；
//! 2. `OnKeyDown`：真正按下（没在组字时先读光标前的几个字一起发过去），拿到应答后开一个
//!    同步编辑会话改文档（上屏、更新编辑区、光标），再按编辑区的位置摆候选窗；
//! 3. 单按 Shift（按下到松开之间没有别的键，没和 Ctrl/Alt/Win 一起按）在 `OnKeyUp` 里切换中英文。
//!
//! 中英文状态只由 FreeType 自己管（服务进程里），不看系统的「键盘打开 / 关闭」状态：有的应用自己
//! 建 TSF 线程时这个状态是「关闭」，`Ctrl+空格` 也会把它关掉，以前因此会一声不响地只出字母。
//! 激活和换输入框时把它设回「打开」；只有应用明说这里不用输入法（密码框等）时才不接管按键。
//!
//! 服务进程连不上时一律不吃键，应用照常收到按键（并记进 `tsf.log`）。

use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::rc::{Rc, Weak};

use freetype_protocol::{Command, Frame, Key, Mode, Request};
use windows::core::{implement, IUnknownImpl, Interface, Ref, Result, BOOL, GUID};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetKeyboardState, ToUnicode};
use windows::Win32::UI::TextServices::{
    ITfCompartmentMgr, ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext,
    ITfContextComposition, ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection,
    ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfLangBarItemMgr, ITfRange, ITfSource,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl, ITfTextLayoutSink,
    ITfTextLayoutSink_Impl, ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, ITfContextView,
    TfLayoutCode, GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_NONE, TF_ANCHOR_END,
    TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_ES_ASYNCDONTCARE, TF_ES_READ, TF_ES_READWRITE, TF_ES_SYNC,
    TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetGUIThreadInfo, GetWindowRect, SetForegroundWindow,
    TrackPopupMenu, GUITHREADINFO, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

use crate::client::Client;
use crate::keys::{self, Moment, ShiftTap, Side};
use crate::langbar::{self, ButtonClick, ButtonHandle};
use crate::ui::{self, CandidateWindow, Click};
use crate::DllRef;

/// 光标前读多少个字。
const BEFORE_CHARS: i32 = 20;

struct State {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    thread_sink_cookie: Option<u32>,
    sink: Option<ITfCompositionSink>,
    layout_sink: Option<ITfTextLayoutSink>,
    /// 订了排版变化的文档和 cookie。
    layout_cookie: Option<(ITfContext, u32)>,
    composition: Option<ITfComposition>,
    /// 最近一次按键、得到焦点的文档（候选窗点击时用）。
    context: Option<ITfContext>,
    client: Client,
    shift: ShiftTap,
    /// OnTestKeyUp 认出来的单按 Shift，等 OnKeyUp 去切换。
    shift_tapped: Option<Side>,
    window: Option<CandidateWindow>,
    caret: Option<RECT>,
    /// 上一次拿到的光标位置（应用一时给不出位置时用）。
    last_caret: Option<RECT>,
    button: Option<ButtonHandle>,
    mode: Mode,
    /// 服务进程要光标前的字（自动加空格打开了）。
    want_before: bool,
    /// 服务进程那边在组字、开着面板（键要先问它）。
    busy: bool,
}

#[implement(
    ITfTextInputProcessorEx,
    ITfKeyEventSink,
    ITfThreadMgrEventSink,
    ITfCompositionSink,
    ITfTextLayoutSink
)]
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
                layout_sink: None,
                layout_cookie: None,
                composition: None,
                context: None,
                client: Client::new(),
                shift: ShiftTap::default(),
                shift_tapped: None,
                window: None,
                caret: None,
                last_caret: None,
                button: None,
                mode: Mode::Chinese,
                want_before: false,
                busy: false,
            })),
            instance,
            _dll: DllRef::new(),
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// 现在 Ctrl、Alt、Win 有没有按着（单按 Shift 的判定用）。
fn moment() -> Moment {
    unsafe {
        let down = |vk: i32| GetKeyState(vk) < 0;
        Moment {
            other_modifiers: down(0x11) || down(0x12) || down(0x5b) || down(0x5c),
        }
    }
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

fn edit_with(
    context: &ITfContext,
    client_id: u32,
    flags: windows::Win32::UI::TextServices::TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    action: impl FnOnce(u32) -> Result<()> + 'static,
) {
    let session: ITfEditSession = EditSession {
        action: RefCell::new(Some(Box::new(action))),
        _dll: DllRef::new(),
    }
    .into();
    unsafe {
        let _ = context.RequestEditSession(client_id, &session, flags);
    }
}

fn edit(context: &ITfContext, client_id: u32, sync: bool, action: impl FnOnce(u32) -> Result<()> + 'static) {
    let flags = if sync {
        TF_ES_SYNC | TF_ES_READWRITE
    } else {
        TF_ES_ASYNCDONTCARE | TF_ES_READWRITE
    };
    edit_with(context, client_id, flags, action);
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

fn selection_range(context: &ITfContext, ec: u32) -> Option<ITfRange> {
    unsafe {
        let mut selection = [TF_SELECTION::default()];
        let mut fetched = 0u32;
        context
            .GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)
            .ok()?;
        let range = ManuallyDrop::into_inner(std::mem::take(&mut selection[0].range))?;
        (fetched > 0).then_some(range)
    }
}

fn selection_rect(context: &ITfContext, ec: u32) -> Option<RECT> {
    selection_range(context, ec).and_then(|range| text_rect(context, ec, &range))
}

/// 光标前最多 [`BEFORE_CHARS`] 个字（只读）。
fn before_text(context: &ITfContext, ec: u32) -> Option<String> {
    unsafe {
        let range = selection_range(context, ec)?;
        range.Collapse(ec, TF_ANCHOR_START).ok()?;
        let mut moved = 0;
        range.ShiftStart(ec, -BEFORE_CHARS, &mut moved, std::ptr::null()).ok()?;
        let mut buf = [0u16; BEFORE_CHARS as usize * 2];
        let mut len = 0u32;
        range.GetText(ec, 0, &mut buf, &mut len).ok()?;
        Some(String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]))
    }
}

/// 应用给不出光标位置时：系统记的插入符，再不行用输入框窗口的左下角。
fn fallback_caret() -> Option<RECT> {
    unsafe {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(GetCurrentThreadId(), &mut info).ok()?;
        if !info.hwndCaret.is_invalid() {
            let r = info.rcCaret;
            let mut pts = [POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom }];
            MapWindowPoints(Some(info.hwndCaret), None, &mut pts);
            return Some(RECT {
                left: pts[0].x,
                top: pts[0].y,
                right: pts[1].x,
                bottom: pts[1].y,
            });
        }
        let focus = if info.hwndFocus.is_invalid() { info.hwndActive } else { info.hwndFocus };
        if focus.is_invalid() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(focus, &mut r).ok()?;
        Some(RECT {
            left: r.left + 8,
            top: r.bottom - 24,
            right: r.left + 9,
            bottom: r.bottom - 4,
        })
    }
}

fn compartment_value(mgr: &ITfCompartmentMgr, guid: &GUID) -> Option<i32> {
    unsafe {
        let compartment = mgr.GetCompartment(guid).ok()?;
        let value = compartment.GetValue().ok()?;
        let inner = &value.Anonymous.Anonymous;
        (inner.vt == VT_I4).then(|| inner.Anonymous.lVal)
    }
}

/// 应用说这里不用输入法（密码框、禁用输入法的窗口）。
fn disabled(context: Option<&ITfContext>) -> bool {
    let Some(mgr) = context.and_then(|c| c.cast::<ITfCompartmentMgr>().ok()) else {
        return false;
    };
    compartment_value(&mgr, &GUID_COMPARTMENT_KEYBOARD_DISABLED).is_some_and(|v| v != 0)
        || compartment_value(&mgr, &GUID_COMPARTMENT_EMPTYCONTEXT).is_some_and(|v| v != 0)
}

/// 把系统的「键盘打开」设成打开（中英文由 FreeType 自己管）。
fn set_keyboard_open(thread_mgr: &ITfThreadMgr, client_id: u32) {
    let Ok(mgr) = thread_mgr.cast::<ITfCompartmentMgr>() else {
        return;
    };
    if compartment_value(&mgr, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) == Some(1) {
        return;
    }
    unsafe {
        let Ok(compartment) = mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) else {
            return;
        };
        let mut value = VARIANT::default();
        (*value.Anonymous.Anonymous).vt = VT_I4;
        (*value.Anonymous.Anonymous).Anonymous.lVal = 1;
        let _ = compartment.SetValue(client_id, &value);
    }
}

/// 订这个文档的排版变化（浏览器类应用打字时还没排好版，排好了再把候选窗挪过去）。
fn watch_layout(state: &Rc<RefCell<State>>, context: &ITfContext) {
    let (current, sink) = {
        let s = state.borrow();
        (s.layout_cookie.as_ref().map(|(c, _)| c.clone()), s.layout_sink.clone())
    };
    if current.as_ref() == Some(context) {
        return;
    }
    unwatch_layout(state);
    let Some(sink) = sink else {
        return;
    };
    unsafe {
        if let Ok(source) = context.cast::<ITfSource>() {
            if let Ok(cookie) = source.AdviseSink(&ITfTextLayoutSink::IID, &sink) {
                state.borrow_mut().layout_cookie = Some((context.clone(), cookie));
            }
        }
    }
}

fn unwatch_layout(state: &Rc<RefCell<State>>) {
    let old = state.borrow_mut().layout_cookie.take();
    if let Some((context, cookie)) = old {
        unsafe {
            if let Ok(source) = context.cast::<ITfSource>() {
                let _ = source.UnadviseSink(cookie);
            }
        }
    }
}

/// 在编辑会话里按应答改文档：先上屏，再更新编辑区。返回候选窗该放的位置。
fn apply_document(state: &Rc<RefCell<State>>, context: &ITfContext, frame: &Frame, ec: u32) -> Result<()> {
    let (mut composition, sink) = {
        let s = state.borrow();
        (s.composition.clone(), s.sink.clone())
    };
    let wants_window = frame.hint.is_some() || !frame.candidates.is_empty() || frame.title.is_some();
    let result = (|| -> Result<Option<RECT>> {
        unsafe {
            if let Some(text) = &frame.commit {
                let text = wide(text);
                let range = if let Some(comp) = composition.take() {
                    let range = comp.GetRange()?;
                    range.SetText(ec, 0, &text)?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    comp.EndComposition(ec)?;
                    range
                } else {
                    let insert: ITfInsertAtSelection = context.cast()?;
                    let range = insert.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &text)?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    range
                };
                if frame.caret_back > 0 && frame.preedit.is_empty() {
                    // 成对标点：光标停在两半中间
                    let back = -(frame.caret_back as i32);
                    let mut moved = 0;
                    range.ShiftStart(ec, back, &mut moved, std::ptr::null())?;
                    range.Collapse(ec, TF_ANCHOR_START)?;
                }
                set_selection(context, ec, &range)?;
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
            Ok(if wants_window { selection_rect(context, ec) } else { None })
        }
    })();
    let mut s = state.borrow_mut();
    s.composition = composition;
    match result {
        Ok(rect) => {
            if rect.is_some() {
                s.last_caret = rect;
            }
            s.caret = rect;
            Ok(())
        }
        Err(e) => {
            s.caret = None;
            Err(e)
        }
    }
}

/// 记下应答里的状态（中英文、要不要光标前的字），更新任务栏按钮。
fn note_frame(state: &Rc<RefCell<State>>, frame: &Frame) {
    let button = {
        let mut s = state.borrow_mut();
        s.mode = frame.mode;
        s.want_before = frame.want_before;
        s.busy = !frame.preedit.is_empty() || !frame.candidates.is_empty() || frame.title.is_some();
        s.button.clone()
    };
    if let Some(button) = button {
        button.set_mode(frame.mode);
    }
}

/// 候选窗按应答更新；应用给不出位置时用上一次的或系统记的插入符。
fn show_window(state: &Rc<RefCell<State>>, frame: &Frame) {
    let s = state.borrow();
    let wants = frame.hint.is_some() || !frame.candidates.is_empty() || frame.title.is_some();
    let caret = if wants {
        s.caret.or(s.last_caret).or_else(fallback_caret)
    } else {
        s.caret
    };
    if let Some(window) = &s.window {
        window.update(frame, caret);
    }
}

/// 应答落到文档和候选窗上。
fn apply(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, frame: Frame, sync: bool) {
    note_frame(state, &frame);
    let Some(context) = context else {
        state.borrow_mut().caret = None;
        show_window(state, &frame);
        return;
    };
    if !frame.preedit.is_empty() {
        watch_layout(state, context);
    }
    let client_id = state.borrow().client_id;
    let shared = Rc::new(frame);
    if sync {
        let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
        edit(context, client_id, true, move |ec| apply_document(&state2, &context2, &frame2, ec));
        show_window(state, &shared);
    } else {
        // 异步：编辑会话里改完文档再摆候选窗
        let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
        edit(context, client_id, false, move |ec| {
            let result = apply_document(&state2, &context2, &frame2, ec);
            show_window(&state2, &frame2);
            result
        });
    }
}

/// 只读一下光标前的字（同步）。
fn read_before(state: &Rc<RefCell<State>>, context: &ITfContext) -> Option<String> {
    let client_id = state.borrow().client_id;
    let out: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let (out2, context2) = (out.clone(), context.clone());
    edit_with(context, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
        *out2.borrow_mut() = before_text(&context2, ec);
        Ok(())
    });
    let text = out.borrow_mut().take();
    text
}

/// 虚拟键翻成键码；翻不出来（非 ASCII 字符、功能键）返回 None。
fn translate(wparam: WPARAM, lparam: LPARAM, test: bool) -> Option<Key> {
    let vk = wparam.0 as u16;
    let modifiers = keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) });
    if let Some(keysym) = keys::named_keysym(vk) {
        return Some(Key { keysym, modifiers, test });
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
        Some(Key { keysym, modifiers, test })
    }
}

fn call(state: &Rc<RefCell<State>>, request: &Request) -> Option<Frame> {
    state.borrow_mut().client.call(request)?.frame
}

fn composing(state: &Rc<RefCell<State>>) -> bool {
    let s = state.borrow();
    s.composition.is_some() || s.busy
}

fn on_click(state: &Weak<RefCell<State>>, click: Click) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let request = match click {
        Click::Select(index) => Request::Select { index },
        Click::Page(next) => Request::Page { next },
        Click::Forget(index) => Request::Forget { index },
        Click::Settings => Request::Command {
            command: Command::OpenSettings,
        },
    };
    let Some(frame) = call(&state, &request) else {
        return;
    };
    if click == Click::Settings {
        return;
    }
    let context = state.borrow().context.clone();
    apply(&state, context.as_ref(), frame, false);
}

/// 任务栏按钮：左键切换中英文，右键菜单。
fn on_button(state: &Weak<RefCell<State>>, click: ButtonClick) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let request = match click {
        ButtonClick::Toggle => Request::ToggleMode { shift: None },
        ButtonClick::Menu(x, y) => match button_menu(&state, x, y) {
            Some(request) => request,
            None => return,
        },
    };
    let settings = matches!(request, Request::Command { command: Command::OpenSettings });
    let Some(frame) = call(&state, &request) else {
        return;
    };
    if settings {
        return;
    }
    let context = state.borrow().context.clone();
    apply(&state, context.as_ref(), frame, false);
}

fn button_menu(state: &Rc<RefCell<State>>, x: i32, y: i32) -> Option<Request> {
    let owner: HWND = state.borrow().window.as_ref().map(|w| w.hwnd())?;
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let items: [(u32, &str); 5] = [
            (1, "切换中 / 英文"),
            (2, "中文 / 英文标点"),
            (3, "简体 / 繁体"),
            (4, "表情与符号"),
            (5, "工具栏（剪贴板、常用语）"),
        ];
        for (id, text) in items {
            let _ = AppendMenuW(menu, MF_STRING, id as usize, &windows::core::HSTRING::from(text));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 6, windows::core::w!("FreeType 设置"));
        let _ = SetForegroundWindow(owner);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, owner, None);
        let _ = DestroyMenu(menu);
        let command = match cmd.0 {
            1 => return Some(Request::ToggleMode { shift: None }),
            2 => Command::TogglePunct,
            3 => Command::ToggleTraditional,
            4 => Command::EmojiPanel,
            5 => Command::Toolbar,
            6 => Command::OpenSettings,
            _ => return None,
        };
        Some(Request::Command { command })
    }
}

/// 换了输入框：设回「键盘打开」，告诉服务进程，按设置提示中英文状态。
fn on_focus(state: &Rc<RefCell<State>>, focus: Option<&ITfDocumentMgr>) {
    {
        let mut s = state.borrow_mut();
        s.shift.cancel();
        s.last_caret = None;
        if let Some(window) = &s.window {
            window.hide();
        }
    }
    let context = focus.and_then(|doc| unsafe { doc.GetTop().ok() });
    let Some(context) = context else {
        state.borrow_mut().context = None;
        if let Some(frame) = call(state, &Request::Focus { active: false, before: None }) {
            note_frame(state, &frame);
        }
        return;
    };
    let (thread_mgr, client_id) = {
        let mut s = state.borrow_mut();
        s.context = Some(context.clone());
        (s.thread_mgr.clone(), s.client_id)
    };
    if let Some(thread_mgr) = &thread_mgr {
        set_keyboard_open(thread_mgr, client_id);
    }
    if disabled(Some(&context)) {
        return;
    }
    // 读光标前的字和光标位置要编辑会话；这里不能同步要，异步做完再告诉服务进程
    let (state2, context2) = (state.clone(), context.clone());
    edit_with(&context, client_id, TF_ES_ASYNCDONTCARE | TF_ES_READ, move |ec| {
        let before = before_text(&context2, ec);
        let caret = selection_rect(&context2, ec);
        let Some(mut frame) = call(&state2, &Request::Focus { active: true, before }) else {
            return Ok(());
        };
        note_frame(&state2, &frame);
        if frame.hint.is_some() {
            state2.borrow_mut().caret = caret;
            frame.candidates.clear();
            frame.title = None;
            show_window(&state2, &frame);
        }
        Ok(())
    });
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        ui::set_click_handler(None);
        unwatch_layout(&self.state);
        let mut s = self.state.borrow_mut();
        if let Some(thread_mgr) = s.thread_mgr.take() {
            unsafe {
                if let (Some(button), Ok(items)) = (s.button.take(), thread_mgr.cast::<ITfLangBarItemMgr>()) {
                    button.detach();
                    if let Some(item) = button.as_item() {
                        let _ = items.RemoveItem(&item);
                    }
                }
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
        s.layout_sink = None;
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
        let layout_sink: ITfTextLayoutSink = self.to_interface();
        unsafe {
            let keystrokes: ITfKeystrokeMgr = thread_mgr.cast()?;
            keystrokes.AdviseKeyEventSink(tid, &key_sink, true)?;
            let source: ITfSource = thread_mgr.cast()?;
            let cookie = source.AdviseSink(&ITfThreadMgrEventSink::IID, &thread_sink).ok();
            self.state.borrow_mut().thread_sink_cookie = cookie;
        }
        let weak = Rc::downgrade(&self.state);
        let button = langbar::create(Rc::new(move |click| on_button(&weak, click)));
        unsafe {
            if let (Ok(items), Some(item)) = (thread_mgr.cast::<ITfLangBarItemMgr>(), button.as_item()) {
                let _ = items.AddItem(&item);
            }
        }
        set_keyboard_open(&thread_mgr, tid);
        let focus = unsafe { thread_mgr.GetFocus().ok() };
        {
            let mut s = self.state.borrow_mut();
            s.thread_mgr = Some(thread_mgr);
            s.client_id = tid;
            s.sink = Some(composition_sink);
            s.layout_sink = Some(layout_sink);
            s.window = CandidateWindow::new(self.instance);
            s.button = Some(button);
        }
        let weak = Rc::downgrade(&self.state);
        ui::set_click_handler(Some(Box::new(move |click| on_click(&weak, click))));
        if let Some(focus) = focus {
            on_focus(&self.state, Some(&focus));
        }
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

    fn OnTestKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let vk = wparam.0 as u16;
        let repeat = lparam.0 & (1 << 30) != 0;
        self.state.borrow_mut().shift.key_down(vk, moment(), repeat);
        if keys::is_modifier(vk) || disabled(pic.as_ref()) {
            return Ok(false.into());
        }
        let Some(key) = translate(wparam, lparam, true) else {
            // 组字时不认识的键也吃掉，免得漏给应用
            return Ok(composing(&self.state).into());
        };
        let frame = call(&self.state, &Request::Key { key, before: None });
        if let Some(frame) = &frame {
            note_frame(&self.state, frame);
        }
        Ok(frame.is_some_and(|f| f.handled).into())
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let vk = wparam.0 as u16;
        if keys::is_modifier(vk) || disabled(pic.as_ref()) {
            return Ok(false.into());
        }
        let context = pic.ok()?.clone();
        self.state.borrow_mut().context = Some(context.clone());
        let Some(key) = translate(wparam, lparam, false) else {
            return Ok(composing(&self.state).into());
        };
        // 没在组字时带上光标前的字（标点、自动加空格看它）
        let wants_before = {
            let s = self.state.borrow();
            !composing(&self.state) && (s.mode == Mode::Chinese || s.want_before)
        };
        let before = if wants_before { read_before(&self.state, &context) } else { None };
        let frame = match call(&self.state, &Request::Key { key, before }) {
            Some(frame) => frame,
            None => {
                // 服务进程没了：结束编辑区，键交给应用
                if composing(&self.state) {
                    apply(&self.state, Some(&context), Frame::default(), true);
                }
                return Ok(false.into());
            }
        };
        let handled = frame.handled;
        let visible = self.state.borrow().window.as_ref().is_some_and(|w| w.visible());
        if handled || frame.commit.is_some() || composing(&self.state) || visible {
            apply(&self.state, Some(&context), frame, true);
        } else {
            note_frame(&self.state, &frame);
        }
        Ok(handled.into())
    }

    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let mut s = self.state.borrow_mut();
        let vk = wparam.0 as u16;
        let tapped = s.shift.key_up(vk, moment());
        let scan = ((lparam.0 >> 16) & 0xff) as u32;
        s.shift_tapped = tapped.then(|| keys::shift_side(vk, scan));
        Ok(tapped.into())
    }

    fn OnKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let side = {
            let mut s = self.state.borrow_mut();
            let vk = wparam.0 as u16;
            let scan = ((lparam.0 >> 16) & 0xff) as u32;
            s.shift_tapped
                .take()
                .or_else(|| s.shift.key_up(vk, moment()).then(|| keys::shift_side(vk, scan)))
        };
        let Some(side) = side else {
            return Ok(false.into());
        };
        if disabled(pic.as_ref()) {
            return Ok(false.into());
        }
        let context = pic.ok()?.clone();
        let request = Request::ToggleMode {
            shift: Some(side.name().to_string()),
        };
        if let Some(frame) = call(&self.state, &request) {
            apply(&self.state, Some(&context), frame, true);
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

    fn OnSetFocus(&self, focus: Ref<ITfDocumentMgr>, _previous: Ref<ITfDocumentMgr>) -> Result<()> {
        on_focus(&self.state, focus.as_ref());
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
        if let Some(frame) = call(&self.state, &Request::Reset { commit: false }) {
            note_frame(&self.state, &frame);
        }
        Ok(())
    }
}

impl ITfTextLayoutSink_Impl for TextService_Impl {
    /// 应用排好了版（浏览器类应用打字后一会儿才给得出位置）：候选窗挪到编辑区下面。
    fn OnLayoutChange(&self, pic: Ref<ITfContext>, _lcode: TfLayoutCode, _view: Ref<ITfContextView>) -> Result<()> {
        let Some(context) = pic.as_ref().cloned() else {
            return Ok(());
        };
        let (composition, client_id) = {
            let s = self.state.borrow();
            (s.composition.clone(), s.client_id)
        };
        let Some(composition) = composition else {
            return Ok(());
        };
        let state = self.state.clone();
        let context2 = context.clone();
        edit_with(&context, client_id, TF_ES_ASYNCDONTCARE | TF_ES_READ, move |ec| {
            let rect = unsafe { composition.GetRange() }
                .ok()
                .and_then(|range| text_rect(&context2, ec, &range));
            if let Some(rect) = rect {
                let mut s = state.borrow_mut();
                s.caret = Some(rect);
                s.last_caret = Some(rect);
                if let Some(window) = &s.window {
                    window.reposition(rect);
                }
            }
            Ok(())
        });
        Ok(())
    }
}
