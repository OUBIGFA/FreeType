//! TSF 文本服务：收按键、问服务进程、按应答组字 / 上屏 / 画候选窗。
//!
//! 每个用到输入法的线程有一个实例。按键流程：
//!
//! 1. `OnTestKeyDown`：把键翻成键码，带 `test` 问服务进程要不要；
//! 2. `OnKeyDown`：真正按下（没在组字时先读光标前的几个字一起发过去），拿到应答后开一个
//!    同步编辑会话改文档（上屏、更新编辑区、光标），再按编辑区的位置摆候选窗；
//! 3. 单按 Shift（按下到松开之间没有别的键，没和 Ctrl/Alt/Win 一起按）切换中英文：系统第一次
//!    报这次松开时就切换（`OnTestKeyUp` 或 `OnKeyUp`，见 [`key_released`]），不吃这个键。
//! 4. Caps Lock：按下、松开时看大写锁定变没变，变了就告诉服务进程（[`caps_changed`]）。开着时
//!    服务进程一律按英文处理，关掉后回到原来的中英文状态；这个键本身不吃。
//!
//! 中英文状态只由 FreeType 自己管（服务进程里），不看系统的「键盘打开 / 关闭」状态：有的应用自己
//! 建 TSF 线程时这个状态是「关闭」，`Ctrl+空格` 也会把它关掉，以前因此会一声不响地只出字母。
//! 激活和换输入框时把它设回「打开」；只有应用明说这里不用输入法（密码框等）时才不接管按键。
//!
//! 服务进程连不上时一律不吃键，应用照常收到按键（并记进 `tsf.log`）。

use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::rc::{Rc, Weak};

use freetype_protocol::{Command, Frame, Key, Mode, PairEdit, Request};
use unicode_segmentation::UnicodeSegmentation;
use windows::core::{implement, IUnknownImpl, Interface, Ref, Result, BOOL, GUID};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardState, SendInput, ToUnicode, INPUT, INPUT_0, INPUT_KEYBOARD,
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_LEFT,
};
use windows::Win32::UI::TextServices::{
    ITfCompartmentMgr, ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext,
    ITfContextComposition, ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection,
    ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfLangBarItemMgr, ITfRange, ITfSource,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl, ITfTextLayoutSink,
    ITfTextLayoutSink_Impl, ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, ITfContextView,
    TfLayoutCode, GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED,
    GUID_PROP_INPUTSCOPE, ITfInputScope, IS_PASSWORD, IS_NUMERIC_PASSWORD, IS_NUMERIC_PIN,
    IS_ALPHANUMERIC_PIN, IS_ALPHANUMERIC_PIN_SET,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_NONE, TF_ANCHOR_END,
    TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_ES_ASYNCDONTCARE, TF_ES_READ, TF_ES_READWRITE, TF_ES_SYNC,
    TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetGUIThreadInfo, GetMessageExtraInfo, GetWindowRect,
    SetForegroundWindow, TrackPopupMenu, GUITHREADINFO, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

use crate::client::Client;
use crate::keys::{self, Moment, ShiftTap};
use crate::langbar::{self, ButtonClick, ButtonHandle};
use crate::ui::{self, CandidateWindow, Click};
use crate::DllRef;

#[path = "pair.rs"]
mod pair;

/// 光标前读多少个字。
const BEFORE_CHARS: i32 = 20;

struct CommitWatch {
    context: ITfContext,
    before: String,
    shortened: String,
    armed: bool,
}

struct SpaceWatch {
    context: ITfContext,
    revision: u64,
    expected: String,
    pressed_ms: u64,
    released: bool,
}

struct State {
    /// 激活代次：停用后排队中的编辑会话不能继续操作文档或报告旧焦点。
    generation: u64,
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
    /// 成对标点上屏后还没挪的光标（等用户松开 Shift）。
    caret_move: Option<keys::CaretMove>,
    sensitive: bool,
    undo: Option<CommitWatch>,
    predict_double_space: bool,
    space: Option<SpaceWatch>,
    pairs: Vec<pair::Pair>,
    pair_enabled: bool,
    window: Option<CandidateWindow>,
    caret: Option<RECT>,
    /// 上一次拿到的光标位置（应用一时给不出位置时用）。
    last_caret: Option<RECT>,
    button: Option<ButtonHandle>,
    mode: Mode,
    /// 上一次报给服务进程的大写锁定状态（变了才再报）。
    caps: bool,
    /// 服务进程要光标前的字（自动加空格打开了）。
    want_before: bool,
    /// 服务进程那边在组字、开着面板（键要先问它）。
    busy: bool,
    /// 候选窗里是上屏联想：不算在组字（照常读光标前的字，不认的键交给应用）。
    predicting: bool,
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
    /// 组字、候选选择和补全光标移动全部结束后才允许交接实现。
    pub(crate) fn can_reload(&self) -> bool {
        let s = self.state.borrow();
        s.composition.is_none() && !s.busy && s.caret_move.is_none()
    }

    pub fn new(instance: HINSTANCE) -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                generation: 0,
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
                caret_move: None,
                sensitive: false,
                undo: None,
                predict_double_space: false,
                space: None,
                pairs: Vec::new(),
                pair_enabled: false,
                window: None,
                caret: None,
                last_caret: None,
                button: None,
                mode: Mode::Chinese,
                caps: false,
                want_before: false,
                busy: false,
                predicting: false,
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
///
/// 看此刻键盘上的状态（GetAsyncKeyState），不看「处理到这条按键消息时」的状态（GetKeyState）：
/// 有的程序（Word）不在取消息的时候把按键报给输入法，那时消息里记的还是上一个键的状态，
/// 刚按过 Ctrl+S 再单按 Shift 就会被当成 Ctrl 还按着，切换不了。
fn moment() -> Moment {
    unsafe {
        let down = |vk: i32| GetAsyncKeyState(vk) < 0;
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

/// 输入范围由宿主声明；读取密码、数字密码及各类 PIN，只读取范围，不读取文字。
fn sensitive_scope(context: &ITfContext, ec: u32) -> bool {
    unsafe {
        let Some(selection) = selection_range(context, ec) else { return false; };
        let Ok(property) = context.GetProperty(&GUID_PROP_INPUTSCOPE) else { return false; };
        let Ok(mut value) = property.GetValue(ec, &selection) else { return false; };
        let inner = &*value.Anonymous.Anonymous;
        let scopes = if inner.vt == VT_UNKNOWN {
            inner.Anonymous.punkVal.as_ref().and_then(|value| value.cast::<ITfInputScope>().ok())
        } else { None };
        let _ = VariantClear(&mut value);
        let Some(scopes) = scopes else { return false; };
        let mut data = std::ptr::null_mut();
        let mut count = 0;
        if scopes.GetInputScopes(&mut data, &mut count).is_err() || data.is_null() { return false; }
        let sensitive = std::slice::from_raw_parts(data, count as usize).iter().any(|scope|
            matches!(*scope, IS_PASSWORD | IS_NUMERIC_PASSWORD | IS_NUMERIC_PIN | IS_ALPHANUMERIC_PIN | IS_ALPHANUMERIC_PIN_SET));
        CoTaskMemFree(Some(data.cast()));
        sensitive
    }
}

fn set_sensitive(state: &Rc<RefCell<State>>, active: bool) {
    if std::mem::replace(&mut state.borrow_mut().sensitive, active) == active { return; }
    if let Some(frame) = call(state, &Request::SensitiveFocus { active }) {
        note_frame(state, &frame);
    }
    if active {
        let mut state = state.borrow_mut();
        state.undo = None;
        state.space = None;
        state.pairs.clear();
        if let Some(window) = &state.window { window.hide(); }
    }
}

fn check_sensitive(state: &Rc<RefCell<State>>, context: Option<&ITfContext>) -> bool {
    if disabled(context) {
        set_sensitive(state, true);
        return true;
    }
    if let Some(context) = context {
        let client_id = state.borrow().client_id;
        let (flag, input) = (Rc::new(std::cell::Cell::new(None)), context.clone());
        let out = flag.clone();
        edit_with(context, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
            out.set(Some(sensitive_scope(&input, ec)));
            Ok(())
        });
        if let Some(active) = flag.get() { set_sensitive(state, active); }
    }
    state.borrow().sensitive
}

fn replace_verified_space(state: &Rc<RefCell<State>>, context: &ITfContext, ec: u32, text: &[u16]) -> Result<ITfRange> {
    let watch = state.borrow_mut().space.take();
    let failed = || windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL);
    let watch = watch.filter(|watch| same_object(&watch.context, context)).ok_or_else(failed)?;
    let selection = selection_range(context, ec).ok_or_else(failed)?;
    unsafe {
        if !selection.IsEmpty(ec)?.as_bool() || !before_text(context, ec).is_some_and(|before| before.ends_with(&watch.expected)) {
            return Err(failed());
        }
        let mut moved = 0;
        selection.ShiftStart(ec, -1, &mut moved, std::ptr::null())?;
        if moved != -1 { return Err(failed()); }
        selection.SetText(ec, 0, text)?;
        selection.Collapse(ec, TF_ANCHOR_END)?;
    }
    Ok(selection)
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
                let range = if frame.replace_space {
                    replace_verified_space(state, context, ec, &text)?
                } else if let Some(comp) = composition.take() {
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
                set_selection(context, ec, &range)?;
                if frame.caret_back == 1 && frame.pair_enabled {
                    if let Some(pair) = frame.commit.as_ref().and_then(|text| pair::Pair::after_commit(context, &range, ec, text)) {
                        let mut s = state.borrow_mut();
                        if s.pairs.len() == 16 { s.pairs.remove(0); }
                        s.pairs.push(pair);
                    }
                }
                // Record a short, transient suffix only after TSF accepted the commit.
                if frame.undo_learning && frame.caret_back == 0 {
                    let watch = before_text(context, ec).and_then(|before| {
                        let (start, _) = before.grapheme_indices(true).next_back()?;
                        (start > 0).then(|| CommitWatch {
                            context: context.clone(), shortened: before[..start].to_owned(), before, armed: false,
                        })
                    });
                    state.borrow_mut().undo = watch;
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
                let preedit = frame.inline_preedit.as_deref().unwrap_or(&frame.preedit);
                range.SetText(ec, 0, &wide(preedit))?;
                let caret = range.Clone()?;
                caret.Collapse(ec, TF_ANCHOR_START)?;
                let mut moved = 0;
                let shift = if frame.inline_preedit.is_some() { frame.inline_cursor } else { frame.preedit_cursor } as i32;
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
            if let Some(action) = frame.pair_edit {
                let pair = state.borrow_mut().pairs.pop();
                if let Some(pair) = pair { pair.apply(context, ec, action)?; }
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
    note_preview(state, frame);
    let mut s = state.borrow_mut();
    s.predicting = frame.predict && !frame.candidates.is_empty();
    s.predict_double_space = s.predicting && frame.predict_double_space;
    if !s.predict_double_space && !frame.replace_space { s.space = None; }
    s.pair_enabled = frame.pair_enabled;
    if !frame.pair_enabled { s.pairs.clear(); }
    s.busy = !s.predicting && (!frame.preedit.is_empty() || !frame.candidates.is_empty() || frame.title.is_some());
}

/// 收起上屏联想：一会儿没选、来了翻不成键码的键、候选窗摆不出来。
/// 服务进程那边也要收起，不然看不见的联想词还会吃掉数字键和空格。
fn dismiss_prediction(state: &Rc<RefCell<State>>) {
    if !state.borrow().predicting {
        return;
    }
    if let Some(frame) = call(state, &Request::Reset { commit: false }) {
        note_frame(state, &frame);
        show_window(state, &frame);
    }
}

/// OnTestKeyDown 只询问是否接收按键，预判结果不能提前改变真实组字状态。
fn note_preview(state: &Rc<RefCell<State>>, frame: &Frame) {
    let button = {
        let mut s = state.borrow_mut();
        s.mode = frame.mode;
        s.want_before = frame.want_before;
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
    let unseen = wants && frame.predict && (caret.is_none() || s.window.is_none());
    drop(s);
    if unseen {
        // 不知道光标在哪、联想词摆不出来：不留看不见的联想
        dismiss_prediction(state);
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
        edit(context, client_id, true, move |ec| {
            let result = apply_document(&state2, &context2, &frame2, ec);
            if let Err(error) = &result {
                crate::log::write("更新文档失败", &error.to_string());
                if frame2.undo_learning { let _ = call(&state2, &Request::UndoCommitLearning); }
            }
            result
        });
        show_window(state, &shared);
        caret_back(state, &shared);
    } else {
        // 异步：编辑会话里改完文档再摆候选窗
        let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
        let generation = state.borrow().generation;
        edit(context, client_id, false, move |ec| {
            if state2.borrow().generation != generation { return Ok(()); }
            let result = apply_document(&state2, &context2, &frame2, ec);
            show_window(&state2, &frame2);
            caret_back(&state2, &frame2);
            result
        });
    }
}

/// 成对标点：上屏以后让应用收到几下 ←，光标停在两半中间（和搜狗一样；
/// 直接改选区的话，有的应用会在编辑结束时把光标挪回末尾）。
///
/// 这里只记下来、请候选窗回到消息循环以后叫 [`caret_tick`]，不当场发：见 [`keys::CaretMove`]。
fn caret_back(state: &Rc<RefCell<State>>, frame: &Frame) {
    if frame.commit.is_none() || frame.caret_back == 0 || !frame.preedit.is_empty() {
        return;
    }
    let mut s = state.borrow_mut();
    s.caret_move = Some(keys::CaretMove::new(frame.caret_back, now_ms()));
    if let Some(window) = &s.window {
        window.request_caret_tick();
    }
}

/// 等着的 [`keys::CaretMove`]：能挪就挪。返回 true 表示修饰键还按着，过一会儿再来。
///
/// 两处会叫它：候选窗的消息和定时器（[`ui::set_caret_handler`]），以及修饰键松开的时候
/// （[`key_released`]：用户一松开 Shift 就发，赶在下一个字前面）。
fn caret_tick(state: &Rc<RefCell<State>>) -> bool {
    let mut s = state.borrow_mut();
    let Some(pending) = s.caret_move else {
        return false;
    };
    // 焦点不在这个线程了：← 会落到别的程序里，不能发
    let focused = s
        .thread_mgr
        .as_ref()
        .is_some_and(|mgr| unsafe { mgr.IsThreadFocus() }.is_ok_and(|focus| focus.as_bool()));
    // 此刻键盘上的状态：消息里记的状态这时已经过时了
    let held = [0x10, 0x11, 0x12, 0x5b, 0x5c]
        .into_iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk) } < 0);
    let steps = match pending.step(now_ms(), focused, held) {
        keys::CaretStep::Wait => return true,
        keys::CaretStep::Drop => {
            s.caret_move = None;
            return false;
        }
        keys::CaretStep::Send(steps) => steps,
    };
    s.caret_move = None;
    drop(s);
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_LEFT,
                // 带真键盘的扫描码：浏览器内核的程序按它认键（网页拿到的 `code`）
                wScan: 0x4b,
                dwFlags: if up { KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY } else { KEYEVENTF_EXTENDEDKEY },
                dwExtraInfo: keys::OWN_INPUT_TAG,
                ..Default::default()
            },
        },
    };
    let inputs: Vec<INPUT> = (0..steps).flat_map(|_| [key(false), key(true)]).collect();
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
    false
}

/// 正在处理的按键是不是 FreeType 自己发的（[`caret_tick`] 发的 ←）。
fn own_input() -> bool {
    unsafe { GetMessageExtraInfo() }.0 as usize == keys::OWN_INPUT_TAG
}

fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
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

fn watch_backspace(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, vk: u16) {
    if vk != 0x08 || moment().other_modifiers || unsafe { GetAsyncKeyState(0x10) } < 0 || composing(state) {
        state.borrow_mut().undo = None;
        return;
    }
    let Some(context) = context else { return; };
    let before = {
        let state = state.borrow();
        let Some(watch) = state.undo.as_ref().filter(|watch| same_object(&watch.context, context)) else { return; };
        watch.before.clone()
    };
    let valid = read_before(state, context).as_deref() == Some(before.as_str());
    let mut state = state.borrow_mut();
    if valid {
        if let Some(watch) = state.undo.as_mut() { watch.armed = true; }
    } else {
        state.undo = None;
    }
}

fn confirm_backspace(state: &Rc<RefCell<State>>, context: Option<&ITfContext>) {
    let Some(context) = context.filter(|context| !disabled(Some(context))) else { return; };
    let expected = {
        let state = state.borrow();
        let Some(watch) = state.undo.as_ref().filter(|watch| watch.armed && same_object(&watch.context, context)) else { return; };
        watch.shortened.clone()
    };
    if read_before(state, context).is_some_and(|before| before.ends_with(&expected)) {
        state.borrow_mut().undo = None;
        let _ = call(state, &Request::UndoCommitLearning);
    }
}

fn verified_pair_key(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, key: Key) -> Option<PairEdit> {
    let context = context?;
    let (action, composition, client_id) = {
        let s = state.borrow();
        if !s.pair_enabled || (s.busy && s.composition.is_none()) { return None; }
        let action = s.pairs.last()?.for_key(key, s.composition.is_some())?;
        (action, s.composition.clone(), s.client_id)
    };
    let accepted = Rc::new(std::cell::Cell::new(false));
    let (out, state, doc) = (accepted.clone(), state.clone(), context.clone());
    edit_with(context, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
        let caret = if let Some(comp) = composition {
            unsafe { comp.GetRange().and_then(|range| { range.Collapse(ec, TF_ANCHOR_END)?; Ok(range) }).ok() }
        } else { selection_range(&doc, ec) };
        let state = state.borrow();
        if let (Some(caret), Some(last)) = (caret, state.pairs.last()) {
            out.set(last.permits(&doc, ec, &caret, action));
        }
        Ok(())
    });
    accepted.get().then_some(action)
}

/// Preview and real key-down both see the same frame. Only a release between them can arm a second tap.
fn detect_double_space(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, key: Key, lparam: LPARAM) -> Key {
    use freetype_protocol::{keysym, modifier};
    let revision = shown_revision(state);
    let Some(context) = context else { return key; };
    if key.keysym != 0x20 || key.modifiers & (modifier::SHIFT | modifier::CTRL | modifier::ALT | modifier::SUPER) != 0
        || lparam.0 & (1 << 30) != 0 || revision == 0 || !state.borrow().predict_double_space {
        return key;
    }
    let ms = now_ms();
    let wait = u64::from(unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }).clamp(200, 700);
    {
        let held = state.borrow();
        if held.space.as_ref().is_some_and(|watch| !watch.released && watch.revision == revision) {
            return key;
        }
    }
    let Some(before) = read_before(state, context) else { return key; };
    let mut state = state.borrow_mut();
    if state.space.as_ref().is_some_and(|watch| watch.released && watch.revision == revision
        && ms.saturating_sub(watch.pressed_ms) <= wait && same_object(&watch.context, context)
        && before.ends_with(&watch.expected)) {
        return Key { keysym: keysym::SPACE_DOUBLE, ..key };
    }
    // Compare a suffix that stays inside the TSF read limit even after one extra character.
    let start = before.char_indices().rev().nth(7).map_or(0, |(i, _)| i);
    let expected = format!("{} ", &before[start..]);
    state.space = Some(SpaceWatch { context: context.clone(), revision, expected, pressed_ms: ms, released: false });
    key
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

fn shown_revision(state: &Rc<RefCell<State>>) -> u64 {
    state.borrow().window.as_ref().map_or(0, CandidateWindow::painted_revision)
}

fn composing(state: &Rc<RefCell<State>>) -> bool {
    let s = state.borrow();
    s.composition.is_some() || s.busy
}

/// 大写锁定现在开着没有（按处理到这条按键消息时的状态）。
fn caps_on() -> bool {
    unsafe { GetKeyState(i32::from(keys::VK_CAPITAL)) & 1 != 0 }
}

/// Caps Lock 按下、松开时：大写锁定变了就告诉服务进程，按应答上屏打到一半的字母、提示新状态。
///
/// 按下和松开都看一遍：浏览器内核的程序先看一眼消息再取走，按下那一刻状态可能还没更新；
/// 同一次变化只报一遍（`State::caps` 记着上次报的）。应用禁用输入法的地方（密码框）不提示，
/// 服务进程从之后每个按键带的修饰键状态里照样能跟上。
fn caps_changed(state: &Rc<RefCell<State>>, context: Option<&ITfContext>) {
    let on = caps_on();
    if std::mem::replace(&mut state.borrow_mut().caps, on) == on || disabled(context) {
        return;
    }
    if let Some(frame) = call(state, &Request::CapsLock { on }) {
        // 和单按 Shift 一样不强求同步：系统只是先问一声的时候不一定给同步的编辑会话
        apply(state, context, frame, false);
    }
}

fn on_click(state: &Weak<RefCell<State>>, click: Click) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let request = match click {
        Click::Candidate { revision, action } => Request::CandidateAction { revision, action },
        Click::PinChoices { .. } => return,
        Click::Dismiss => Request::Reset { commit: false },
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

/// 两个接口指针是不是同一个 COM 对象（按 COM 的规定比 `IUnknown`，不比接口指针本身）。
fn same_object<A: Interface, B: Interface>(a: &A, b: &B) -> bool {
    match (a.cast::<windows::core::IUnknown>(), b.cast::<windows::core::IUnknown>()) {
        (Ok(a), Ok(b)) => a.as_raw() == b.as_raw(),
        _ => false,
    }
}

/// 焦点通知说的就是正在组字的那个文档时，返回组字所在的上下文。
///
/// 应用会对同一个输入框再报一次焦点：资源管理器地址栏（XAML）点进去以后，要等第一个键
/// 打出来才报，这时编辑区和候选窗已经在了。这不是换输入框，见 [`on_focus`]。
fn composing_context(composition: Option<&ITfComposition>, focus: Option<&ITfDocumentMgr>) -> Option<ITfContext> {
    let context = unsafe { composition?.GetRange().and_then(|range| range.GetContext()) }.ok()?;
    let owner = unsafe { context.GetDocumentMgr() }.ok()?;
    same_object(&owner, focus?).then_some(context)
}

/// 焦点还在（或回到了）正在组字的输入框：编辑区留着，候选窗接着显示。
///
/// 做法照水杉输入法 Windows 版的 `ITfThreadMgrEventSink::OnSetFocus`：候选窗属于哪个文档，
/// 焦点落在那个文档上就显示，落在别处才收起。不提示中英文状态（用户正在打字），
/// 也不清掉记着的光标位置（候选窗靠它摆）。
fn resume_composition(state: &Rc<RefCell<State>>, context: ITfContext) {
    state.borrow_mut().context = Some(context);
    // 焦点中途丢过的话服务进程已经当这个输入框不在用了：重新报上，顺便拿回这一页候选
    let Some(mut frame) = call(state, &Request::Focus { active: true, before: None }) else {
        return;
    };
    frame.hint = None;
    note_frame(state, &frame);
    show_window(state, &frame);
}

/// 换了输入框：设回「键盘打开」，告诉服务进程，按设置提示中英文状态。
///
/// 焦点通知不一定是换了输入框：正在组字的文档自己又报一次焦点时，编辑区和候选窗原样保留
/// （[`resume_composition`]）。以前一律收起候选窗，第一个字母就像直接成了英文。
/// 焦点暂时落空（`focus` 是 None）时只收起候选窗，编辑区不动，回来以后同样接着显示。
fn on_focus(state: &Rc<RefCell<State>>, focus: Option<&ITfDocumentMgr>) {
    let composition = state.borrow().composition.clone();
    if let Some(context) = composing_context(composition.as_ref(), focus) {
        resume_composition(state, context);
        return;
    }
    {
        let mut s = state.borrow_mut();
        s.shift.cancel();
        s.space = None;
        s.pairs.clear();
        s.caret_move = None;
        s.undo = None;
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
        set_sensitive(state, true);
        return;
    }
    // 读光标前的字和光标位置要编辑会话；这里不能同步要，异步做完再告诉服务进程
    let (state2, context2) = (state.clone(), context.clone());
    let generation = state.borrow().generation;
    edit_with(&context, client_id, TF_ES_ASYNCDONTCARE | TF_ES_READ, move |ec| {
        if state2.borrow().generation != generation { return Ok(()); }
        if !state2.borrow().context.as_ref().is_some_and(|current| same_object(current, &context2)) { return Ok(()); }
        let sensitive = sensitive_scope(&context2, ec);
        set_sensitive(&state2, sensitive);
        if sensitive { return Ok(()); }
        let before = before_text(&context2, ec);
        let caret = selection_rect(&context2, ec);
        // 大写锁定可能在别的窗口里开关过：先对齐，下面拿到的中英文状态才是对的
        let on = caps_on();
        state2.borrow_mut().caps = on;
        let _ = call(&state2, &Request::CapsLock { on });
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
        ui::set_caret_handler(None);
        unwatch_layout(&self.state);
        let mut s = self.state.borrow_mut();
        s.generation = s.generation.wrapping_add(1);
        s.caret_move = None;
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
        s.busy = false;
        s.predicting = false;
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
            s.generation = s.generation.wrapping_add(1);
            s.thread_mgr = Some(thread_mgr);
            s.client_id = tid;
            s.sink = Some(composition_sink);
            s.layout_sink = Some(layout_sink);
            s.window = CandidateWindow::new(self.instance);
            s.button = Some(button);
        }
        let weak = Rc::downgrade(&self.state);
        ui::set_click_handler(Some(Box::new(move |click| on_click(&weak, click))));
        let weak = Rc::downgrade(&self.state);
        ui::set_caret_handler(Some(Box::new(move || weak.upgrade().is_some_and(|state| caret_tick(&state)))));
        if let Some(focus) = focus {
            on_focus(&self.state, Some(&focus));
        }
        Ok(())
    }
}

/// 有键按下（OnTestKeyDown、OnKeyDown 共用）：记给单按 Shift 的判定。
/// 有的应用不先问就直接报按下，所以两处都记；重复记同一下没有影响。
///
/// 成对标点还等着挪光标时用户又按了别的键（Shift、Ctrl 这些不算）：不挪了，
/// 光标已经不一定在那一对标点后面。
fn note_key_down(state: &Rc<RefCell<State>>, wparam: WPARAM, lparam: LPARAM) {
    let vk = wparam.0 as u16;
    let repeat = lparam.0 & (1 << 30) != 0;
    let mut s = state.borrow_mut();
    s.shift.key_down(vk, moment(), repeat);
    if vk != 0x20 || repeat { s.space = None; }
    if !keys::is_modifier(vk) {
        s.caret_move = None;
    }
}

/// 有键松开（OnTestKeyUp、OnKeyUp 共用）：是单按 Shift 就切换中英文。
///
/// 系统对同一次松开可能问几遍：浏览器内核的程序（Electron、Chrome）空闲时先看一眼消息再取走，
/// 两次都走到 OnTestKeyUp。所以第一次报到就切换，[`ShiftTap`] 保证同一次松开不切第二遍。
/// （以前 OnTestKeyUp 只记下结果、等 OnKeyUp 去切换；第二遍 OnTestKeyUp 把记下的结果清掉，
/// 并回答「不要这个键」，OnKeyUp 不再被调用，这类程序里就切换不了。）
///
/// 不吃这个键：应用照常收到 Shift 松开，不会以为 Shift 还按着。
///
/// 成对标点还等着挪光标的话，修饰键一松开就去挪（见 [`caret_tick`]）。
fn key_released(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, wparam: WPARAM, lparam: LPARAM) {
    if own_input() { return; }
    let vk = wparam.0 as u16;
    if vk == 0x20 {
        if let Some(watch) = state.borrow_mut().space.as_mut() { watch.released = true; }
    }
    if vk == 0x08 { confirm_backspace(state, context); }
    if keys::is_modifier(vk) && state.borrow().caret_move.is_some() {
        caret_tick(state);
    }
    if vk == keys::VK_CAPITAL {
        caps_changed(state, context);
        return;
    }
    let shift = state.borrow_mut().shift.key_up(vk, moment());
    if !shift || check_sensitive(state, context) {
        return;
    }
    state.borrow_mut().undo = None;
    let scan = ((lparam.0 >> 16) & 0xff) as u32;
    let request = Request::ToggleMode {
        shift: Some(keys::shift_side(vk, scan).name().to_string()),
    };
    if let Some(frame) = call(state, &request) {
        // 不强求同步：系统只是先问一声（OnTestKeyUp）的时候不一定给同步的编辑会话，
        // 这样写它能同步就同步、不能就排到后面，打到一半的字母照样能上屏
        apply(state, context, frame, false);
    }
}

/// FreeType 自己发的 ←（[`caret_tick`]）到了：交给应用。只有这时又在组字了才吃掉
/// （用户已经接着打字，← 会把编辑区打断）。
fn own_key_down(state: &Rc<RefCell<State>>) -> Result<BOOL> {
    Ok(composing(state).into())
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
        if own_input() {
            return own_key_down(&self.state);
        }
        let vk = wparam.0 as u16;
        note_key_down(&self.state, wparam, lparam);
        if vk == keys::VK_CAPITAL {
            caps_changed(&self.state, pic.as_ref());
        }
        if keys::is_modifier(vk) || check_sensitive(&self.state, pic.as_ref()) {
            return Ok(false.into());
        }
        watch_backspace(&self.state, pic.as_ref(), vk);
        let Some(key) = translate(wparam, lparam, true) else {
            // 组字时不认识的键也吃掉，免得漏给应用
            return Ok(composing(&self.state).into());
        };
        if verified_pair_key(&self.state, pic.as_ref(), key).is_some() { return Ok(true.into()); }
        let key = detect_double_space(&self.state, pic.as_ref(), key, lparam);
        let revision = shown_revision(&self.state);
        let frame = call(&self.state, &Request::PhysicalKey { key, before: None, scan: 0, shown_revision: Some(revision) });
        if let Some(frame) = &frame {
            note_preview(&self.state, frame);
        }
        Ok(frame.is_some_and(|f| f.handled).into())
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        if own_input() {
            return own_key_down(&self.state);
        }
        let vk = wparam.0 as u16;
        note_key_down(&self.state, wparam, lparam);
        if vk == keys::VK_CAPITAL {
            caps_changed(&self.state, pic.as_ref());
        }
        if keys::is_modifier(vk) || check_sensitive(&self.state, pic.as_ref()) {
            return Ok(false.into());
        }
        watch_backspace(&self.state, pic.as_ref(), vk);
        let context = pic.ok()?.clone();
        self.state.borrow_mut().context = Some(context.clone());
        let Some(key) = translate(wparam, lparam, false) else {
            // 翻不成键码的键（功能键等）交给应用；显示着的联想词先收起
            dismiss_prediction(&self.state);
            return Ok(composing(&self.state).into());
        };
        // 没在组字时带上光标前的字（标点、自动加空格看它）
        let wants_before = {
            let s = self.state.borrow();
            !composing(&self.state) && (s.mode == Mode::Chinese || s.want_before)
        };
        if let Some(action) = verified_pair_key(&self.state, Some(&context), key) {
            if let Some(mut frame) = call(&self.state, &Request::ClosePair) {
                frame.pair_edit = Some(action);
                apply(&self.state, Some(&context), frame, true);
                return Ok(true.into());
            }
        }
        let key = detect_double_space(&self.state, Some(&context), key, lparam);
        let before = if wants_before && key.keysym != freetype_protocol::keysym::SPACE_DOUBLE {
            read_before(&self.state, &context)
        } else { None };
        let scan = ((lparam.0 >> 16) & 0x1ff) as u16;
        let revision = shown_revision(&self.state);
        let frame = match call(&self.state, &Request::PhysicalKey { key, before, scan, shown_revision: Some(revision) }) {
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

    fn OnTestKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        key_released(&self.state, pic.as_ref(), wparam, lparam);
        Ok(false.into())
    }

    fn OnKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        key_released(&self.state, pic.as_ref(), wparam, lparam);
        Ok(false.into())
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
        let generation = state.borrow().generation;
        edit_with(&context, client_id, TF_ES_ASYNCDONTCARE | TF_ES_READ, move |ec| {
            if state.borrow().generation != generation { return Ok(()); }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_handoff_waits_for_input_and_caret_completion() {
        let service = TextService::new(HINSTANCE::default());
        assert!(service.can_reload());
        note_frame(&service.state, &Frame { preedit: "ni".into(), handled: true, ..Frame::default() });
        assert!(!service.can_reload(), "正在组字时不能替换实现");
        note_frame(&service.state, &Frame::default());
        service.state.borrow_mut().caret_move = Some(keys::CaretMove::new(1, 0));
        assert!(!service.can_reload(), "补全光标移动结束前不能替换实现");
        service.state.borrow_mut().caret_move = None;
        assert!(service.can_reload());
    }

    #[test]
    fn predicted_key_does_not_start_or_end_document_composition() {
        let service = TextService::new(HINSTANCE::default());
        let predicted = Frame { handled: true, preedit: "ni".into(), ..Frame::default() };
        note_preview(&service.state, &predicted);
        assert!(!composing(&service.state), "尚未接受的按键不能使 DLL 提前进入组字状态");
        note_frame(&service.state, &predicted);
        assert!(composing(&service.state));
        let predicted_commit = Frame { handled: true, commit: Some("你".into()), ..Frame::default() };
        note_preview(&service.state, &predicted_commit);
        assert!(composing(&service.state), "预判上屏也不能提前结束正在进行的组字");
    }

    #[test]
    fn next_word_predictions_are_not_a_composition() {
        let service = TextService::new(HINSTANCE::default());
        let frame = Frame { handled: true, commit: Some("今天".into()), candidates: vec!["下雨".into()], predict: true, ..Frame::default() };
        note_frame(&service.state, &frame);
        assert!(!composing(&service.state), "联想词显示着时不认的键要交给应用，还要照常读光标前的字");
        assert!(service.state.borrow().predicting);
        note_frame(&service.state, &Frame::default());
        assert!(!service.state.borrow().predicting);
    }
}

#[cfg(test)]
#[path = "service_focus_tests.rs"]
mod focus_tests;
