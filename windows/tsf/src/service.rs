//! TSF 文本服务：收按键、问服务进程、按应答组字 / 上屏 / 画候选窗。
//!
//! 每个用到输入法的线程有一个实例。按键流程：
//!
//! 1. `OnTestKeyDown`：把键翻成键码，没在组字时读光标前文字，带 `test` 问服务进程要不要；
//! 2. `OnKeyDown`：真正按下，复用预询问的上文（直接按下时才读取），拿到应答后开一个
//!    同步编辑会话改文档（上屏、更新编辑区、光标），再按编辑区的位置摆候选窗；
//! 3. 单按 Shift（按下到松开之间没有别的键，没和 Ctrl/Alt/Win 一起按）切换中英文：系统第一次
//!    报这次松开时就切换（`OnTestKeyUp` 或 `OnKeyUp`，见 [`key_released`]），不吃这个键。
//! 4. Caps Lock：按下、松开时看大写锁定变没变，变了就告诉服务进程（[`caps_changed`]）。开着时
//!    服务进程一律按英文处理，关掉后回到原来的中英文状态；这个键本身不吃。
//! 5. `Ctrl+空格` 是简体中文的系统输入法开关键：系统可能吃掉按下、只把系统的「键盘打开 / 关闭」状态改成「关闭」。
//!    设置里选了它作中英文切换键时，按键回调和这个状态变化（[`ctrl_space_closed`]）都能切换，
//!    同一次按下只切一次。
//!
//! 中英文状态只由 FreeType 自己管（服务进程里），不看系统的「键盘打开 / 关闭」状态：有的应用自己
//! 建 TSF 线程时这个状态是「关闭」，`Ctrl+空格` 也会把它关掉，以前因此会一声不响地只出字母。
//! 激活和换输入框时把它设回「打开」；只有应用明说这里不用输入法（密码框等）时才不接管按键。
//!
//! 服务进程连不上时通常不吃键（并记进 `tsf.log`）；已经显示的联想仍可用 Esc 本地关闭。

use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::rc::{Rc, Weak};

use freetype_protocol::{keysym, modifier, Command, Frame, Key, Mode, PairEdit, Request};
use unicode_segmentation::UnicodeSegmentation;
use windows::core::{implement, IUnknownImpl, Interface, Ref, Result, BOOL, GUID};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::Input::Ime::{IMR_DOCUMENTFEED, RECONVERTSTRING};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetFocus, GetKeyState, GetKeyboardState, SendInput, ToUnicode, INPUT, INPUT_0, INPUT_KEYBOARD,
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_LEFT,
};
use windows::Win32::UI::TextServices::{
    ITfCompartmentEventSink, ITfCompartmentEventSink_Impl, ITfCompartmentMgr, ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext,
    ITfContextComposition, ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection,
    ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfLangBarItemMgr, ITfRange, ITfSource,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl, ITfTextLayoutSink,
    ITfTextLayoutSink_Impl, ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, ITfContextView,
    TfLayoutCode, GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED,
    GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT, TF_SS_TRANSITORY,
    GUID_PROP_INPUTSCOPE, ITfInputScope, IS_PASSWORD, IS_NUMERIC_PASSWORD, IS_NUMERIC_PIN,
    IS_ALPHANUMERIC_PIN, IS_ALPHANUMERIC_PIN_SET,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_NONE, TF_ANCHOR_END,
    TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_ES_ASYNCDONTCARE, TF_ES_READ, TF_ES_READWRITE, TF_ES_SYNC,
    TF_GRAVITY_FORWARD, TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetGUIThreadInfo, GetMessageExtraInfo, GetMessageTime, GetWindowRect,
    EndMenu, PostMessageW, SendMessageW, TrackPopupMenu, GUITHREADINFO, MF_SEPARATOR, MF_STRING, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_IME_REQUEST,
};

use crate::client::Client;
use crate::keys::{self, Moment, ShiftTap};
use crate::langbar::{self, ButtonClick, ButtonHandle};
use crate::ui::{self, CandidateWindow, Click};
use crate::DllRef;

#[path = "pair.rs"]
mod pair;
#[path = "focus_notice.rs"]
mod focus_notice;

/// 光标前读多少个字。
const BEFORE_CHARS: i32 = 20;

struct CommitWatch {
    context: ITfContext,
    before: String,
    shortened: String,
    armed: bool,
}

#[derive(Clone)]
struct PredictionSpace {
    context: ITfContext,
    revision: u64,
    /// 第一次空格由输入法拥有；第二次选词在这个编辑区里替换，绝不删除应用正文。
    composition: ITfComposition,
    /// Windows 消息时间：重复预询问和正式处理共享同一时间，不依赖宿主转发松开事件。
    pressed_ms: u32,
    deadline_ms: u64,
}

struct KeyPreview {
    key: Key,
    context: ITfContext,
    before: Option<String>,
}

struct State {
    /// 激活代次：停用后排队中的编辑会话不能继续操作文档或报告旧焦点。
    generation: u64,
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    thread_sink_cookie: Option<u32>,
    thread_focus_cookie: Option<u32>,
    focus_notice: focus_notice::Notice,
    /// 订了线程「键盘打开 / 关闭」状态的 cookie（接住被系统吃掉的 Ctrl+空格）。
    open_close_cookie: Option<u32>,
    /// 已处理的 Ctrl+空格 消息时间；宿主可能不报松开，不能把旧标记带到下一次按下。
    ctrl_space_toggled: Option<u32>,
    sink: Option<ITfCompositionSink>,
    layout_sink: Option<ITfTextLayoutSink>,
    /// 订了排版变化的文档和 cookie。
    layout_cookie: Option<(ITfContext, u32)>,
    composition: Option<ITfComposition>,
    /// 最近一次按键、得到焦点的文档（候选窗点击时用）。
    context: Option<ITfContext>,
    /// 预询问与正式按下共用同一份上文；临时输入框上屏后可能已清空 TSF 文档。
    key_preview: Option<KeyPreview>,
    client: Client,
    shift: ShiftTap,
    /// 成对标点上屏后还没挪的光标（等用户松开 Shift）。
    caret_move: Option<keys::CaretMove>,
    pair_delivery: pair::Delivery,
    sensitive: bool,
    /// Only an observed password/PIN scope is cached, and only for its context.
    /// A temporary EMPTYCONTEXT/KEYBOARD_DISABLED flag is never sticky.
    sensitive_context: Option<ITfContext>,
    undo: Option<CommitWatch>,
    predict_double_space: bool,
    space: Option<PredictionSpace>,
    pairs: Vec<pair::Pair>,
    pair_enabled: bool,
    window: Option<CandidateWindow>,
    caret: Option<RECT>,
    /// 上一次拿到的光标位置（应用一时给不出位置时用）。
    last_caret: Option<RECT>,
    button: Option<ButtonHandle>,
    language_menu: MenuState,
    mode: Mode,
    /// 上一次报给服务进程的大写锁定状态（变了才再报）。
    caps: bool,
    /// 服务进程要光标前的字（自动加空格打开了）。
    want_before: bool,
    /// 记录联想接管的这一次 Esc，直到松开；不影响下一次按下。
    prediction_escape: PredictionEscape,
    /// 服务进程那边在组字、开着面板（键要先问它）。
    busy: bool,
    /// 候选窗里是上屏联想：不算在组字（照常读光标前的字，不认的键交给应用）。
    predicting: bool,
    /// 上一次成功上屏的末尾；核对联想选词位置时不使用可能滞后的正文快照。
    prediction_caret: Option<ITfRange>,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
enum MenuState {
    #[default]
    Closed,
    Queued,
    Open,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum PredictionEscape {
    #[default]
    Free,
    Preview,
    Held,
}

#[implement(
    ITfTextInputProcessorEx,
    ITfKeyEventSink,
    ITfThreadMgrEventSink,
    windows::Win32::UI::TextServices::ITfThreadFocusSink,
    ITfCompositionSink,
    ITfTextLayoutSink,
    ITfCompartmentEventSink
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
        s.composition.is_none() && !s.busy && s.caret_move.is_none() && s.space.is_none()
            && s.language_menu == MenuState::Closed
    }

    pub fn new(instance: HINSTANCE) -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                generation: 0,
                thread_mgr: None,
                client_id: 0,
                thread_sink_cookie: None,
                thread_focus_cookie: None,
                focus_notice: Default::default(),
                open_close_cookie: None,
                ctrl_space_toggled: None,
                sink: None,
                layout_sink: None,
                layout_cookie: None,
                composition: None,
                context: None,
                key_preview: None,
                client: Client::new(),
                shift: ShiftTap::default(),
                caret_move: None,
                pair_delivery: Default::default(),
                sensitive: false,
                sensitive_context: None,
                undo: None,
                predict_double_space: false,
                space: None,
                pairs: Vec::new(),
                pair_enabled: false,
                window: None,
                caret: None,
                last_caret: None,
                button: None,
                language_menu: MenuState::Closed,
                mode: Mode::Chinese,
                caps: false,
                want_before: false,
                prediction_escape: PredictionEscape::Free,
                busy: false,
                predicting: false,
                prediction_caret: None,
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
    let _ = request_edit(context, client_id, flags, action);
}

fn request_edit(
    context: &ITfContext,
    client_id: u32,
    flags: windows::Win32::UI::TextServices::TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    action: impl FnOnce(u32) -> Result<()> + 'static,
) -> Result<()> {
    let session: ITfEditSession = EditSession {
        action: RefCell::new(Some(Box::new(action))),
        _dll: DllRef::new(),
    }
    .into();
    unsafe { context.RequestEditSession(client_id, &session, flags)?.ok() }
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

/// CUAS 文档管理器上未公开的标记：值的最低位为 1 表示它替 IMM32 应用模拟 TSF 文档。
/// 来源：Mozc `src/win32/tip/tip_transitory_extension.cc` 的 `kTsfEmulatedDocumentMgrGuid`。
const GUID_COMPARTMENT_EMULATED_DOCUMENTMGR: GUID = GUID::from_u128(0xa94c5fd2_c471_4031_9546_709c17300cb9);

/// 光标前文字从哪里读。照搬 Mozc 的 `TipTransitoryExtension::AsFullContext` 与
/// `TipSurroundingText::Get`（`src/win32/tip/`）的判断：
///
/// - 非临时文档（Word、WPF、Firefox 等）：直接读这份 TSF 文档；
/// - 临时文档带完整的父文档（Edit、RichEdit 的 Transitory Extensions）：读父文档；
///   父文档也是临时的，或拿不到父文档的顶层上下文：按 IMM32 应用处理；
/// - 临时文档带 CUAS 模拟标记（IMM32 应用，如用 mygo 写的 FreeMD）：TSF 文档只装编辑区、
///   上屏后即清空，只能用 IMR_DOCUMENTFEED 向应用窗口要；
/// - 其余临时文档（Chromium 系）：TSF 文档里有正文，直接读。
///
/// 真机核对（Windows 11）：CUAS 文档的父文档隔间也是 VT_UNKNOWN，但它不是
/// ITfDocumentMgr，必须按 Mozc 的做法转换接口后再判断，不能只看是否非空。
enum Surrounding {
    Tsf(ITfContext),
    Imm32,
}

fn transitory(context: &ITfContext) -> bool {
    unsafe { context.GetStatus() }.is_ok_and(|status| status.dwStaticFlags & TF_SS_TRANSITORY != 0)
}

fn view_window(context: &ITfContext) -> Option<HWND> {
    unsafe { context.GetActiveView().and_then(|view| view.GetWnd()).ok() }.filter(|window| !window.is_invalid())
}

fn surrounding(context: &ITfContext) -> Surrounding {
    unsafe {
        let Ok(status) = context.GetStatus() else { return Surrounding::Imm32; };
        if status.dwStaticFlags & TF_SS_TRANSITORY == 0 { return Surrounding::Tsf(context.clone()); }
        let Ok(doc) = context.GetDocumentMgr() else { return Surrounding::Imm32; };
        let Ok(mgr) = doc.cast::<ITfCompartmentMgr>() else { return Surrounding::Tsf(context.clone()); };
        if let Some(parent) = transitory_parent(&mgr).filter(|parent| !same_object(parent, &doc)) {
            return match parent.GetTop() {
                Ok(top) if top.GetStatus().is_ok_and(|s| s.dwStaticFlags & TF_SS_TRANSITORY == 0) => Surrounding::Tsf(top),
                _ => Surrounding::Imm32,
            };
        }
        if compartment_value(&mgr, &GUID_COMPARTMENT_EMULATED_DOCUMENTMGR).is_some_and(|v| v & 1 != 0) {
            Surrounding::Imm32
        } else {
            Surrounding::Tsf(context.clone())
        }
    }
}

/// Transitory Extensions 的父文档；隔间里的对象不是文档管理器时视为没有。
fn transitory_parent(mgr: &ITfCompartmentMgr) -> Option<ITfDocumentMgr> {
    unsafe {
        let mut value = mgr.GetCompartment(&GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT).ok()?.GetValue().ok()?;
        let inner = &value.Anonymous.Anonymous;
        let parent = if inner.vt == VT_UNKNOWN {
            inner.Anonymous.punkVal.as_ref().and_then(|unknown| unknown.cast::<ITfDocumentMgr>().ok())
        } else { None };
        let _ = VariantClear(&mut value);
        parent
    }
}

/// IMM32 应用的光标前文字：向文档所在窗口要；拿不到窗口或应用不答都算不知道。
fn imm32_before(context: &ITfContext) -> Option<String> {
    unsafe { context.GetActiveView().and_then(|view| view.GetWnd()) }.ok()
        .filter(|window| !window.is_invalid())
        .and_then(document_feed)
}

/// 向 IMM32 应用要光标前最多 [`BEFORE_CHARS`] 个字（IMR_DOCUMENTFEED）。应用不答时返回 None：
/// 那是不知道上文，不是空文档，服务进程会沿用自己记下的上屏文字。
fn document_feed(window: HWND) -> Option<String> {
    let request = |data: LPARAM| unsafe {
        SendMessageW(window, WM_IME_REQUEST, Some(WPARAM(IMR_DOCUMENTFEED as usize)), Some(data)).0
    };
    let head = std::mem::size_of::<RECONVERTSTRING>();
    let size = usize::try_from(request(LPARAM(0))).ok().filter(|&size| size >= head)?;
    // RECONVERTSTRING 只含 DWORD，按 u32 分配即可对齐
    let mut buffer = vec![0u32; size.div_ceil(4)];
    let header = buffer.as_mut_ptr().cast::<RECONVERTSTRING>();
    unsafe { (*header).dwSize = size as u32; }
    if request(LPARAM(header as isize)) == 0 { return None; }
    let info = unsafe { *header };
    // 长度按 UTF-16 计，偏移按字节计；应用填的值越界就当没答
    let (start, len, caret) = (info.dwStrOffset as usize, info.dwStrLen as usize, info.dwCompStrOffset as usize / 2);
    if start % 2 != 0 || start.checked_add(len * 2)? > size || caret > len { return None; }
    let units = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(start).cast::<u16>(), caret) };
    let before: Vec<char> = char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
    Some(before[before.len().saturating_sub(BEFORE_CHARS as usize)..].iter().collect())
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
        // InputScope belongs to the host's ITextStoreACP/ITfContextOwner;
        // Microsoft specifies GetAppProperty, not the service-owned GetProperty.
        let Ok(property) = context.GetAppProperty(&GUID_PROP_INPUTSCOPE) else { return false; };
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
    if active && state.borrow().caret_move.is_some() { pair::finish(state, false); }
    if active { finish_prediction_space(state, true, false); }
    if std::mem::replace(&mut state.borrow_mut().sensitive, active) == active { return; }
    if let Some(frame) = call(state, &Request::SensitiveFocus { active }) {
        note_frame(state, &frame);
    }
    if active {
        {
            let mut state = state.borrow_mut();
            state.key_preview = None;
            state.undo = None;
            state.space = None;
            state.prediction_escape = PredictionEscape::Free;
            state.pairs.clear();
        }
        with_window(state, |window| window.hide());
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
        if let Some(active) = flag.get() {
            state.borrow_mut().sensitive_context = active.then(|| context.clone());
        }
    }
    // Like SampleIME/Weasel/msime, disabled compartments describe this key's
    // current context. Failure to obtain an optional read lock must not keep
    // an earlier non-editing surface disabled forever (RichEdit/CUAS).
    // A known password/PIN remains protected if its own read lock is unavailable.
    let active = state.borrow().sensitive_context.as_ref().is_some_and(|owner|
        context.is_some_and(|context| same_object(owner, context)));
    set_sensitive(state, active);
    active
}

fn replace_prediction_space(state: &Rc<RefCell<State>>, context: &ITfContext, ec: u32, text: &[u16]) -> Result<ITfRange> {
    let pending = state.borrow().space.clone();
    let failed = || windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL);
    let pending = pending.filter(|pending| same_object(&pending.context, context)).ok_or_else(failed)?;
    let selection = selection_range(context, ec).ok_or_else(failed)?;
    unsafe {
        let range = pending.composition.GetRange()?;
        if !selection.IsEmpty(ec)?.as_bool() || selection.CompareStart(ec, &range, TF_ANCHOR_END)? != 0 {
            return Err(failed());
        }
        range.SetText(ec, 0, text)?;
        range.Collapse(ec, TF_ANCHOR_END)?;
        take_prediction_space(state);
        pending.composition.EndComposition(ec)?;
        Ok(range)
    }
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
        if let Err(error) = compartment.SetValue(client_id, &value) {
            crate::log::write("恢复键盘打开状态失败", &error.to_string());
        }
    }
}

/// 系统把「键盘打开 / 关闭」改成关闭，且此刻正按着 Ctrl+空格：这是系统输入法开关键吃掉了按键。
/// 当成一次 Ctrl+空格 发给服务进程，由它按设置决定切不切换。
/// OnChange 中不能 SetValue（TSF 返回 E_UNEXPECTED）；排队到回调结束后再打开，
/// 否则下一次 Ctrl+空格 只会重新打开系统状态，表现为按两次才切换。
fn ctrl_space_closed(state: &Rc<RefCell<State>>) {
    let ms = unsafe { GetMessageTime() } as u32;
    if !ctrl_space_handled(state, ms, false) {
        // 带上大写锁定的状态：服务进程按每个键里的这一位同步大写锁定
        let caps = keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) }) & modifier::CAPS;
        let key = Key { keysym: 0x20, modifiers: modifier::CTRL | caps, test: false };
        let Some(frame) = call(state, &Request::PhysicalKey { key, before: None, scan: 0, shown_revision: None }) else { return };
        if !frame.handled {
            return;
        }
        state.borrow_mut().ctrl_space_toggled = Some(ms);
        let context = state.borrow().context.clone();
        apply(state, context.as_ref(), frame, false);
    }
    if let Some(window) = &state.borrow().window {
        if let Err(error) = unsafe { PostMessageW(Some(window.hwnd()), ui::WM_RESTORE_KEYBOARD_OPEN, WPARAM(0), LPARAM(0)) } {
            crate::log::write("安排恢复键盘打开状态失败", &error.to_string());
        }
    }
}

/// 同一条按键消息的预询问、正式处理和系统通知只切换一次；长按重复也不再切换。
fn ctrl_space_handled(state: &Rc<RefCell<State>>, ms: u32, repeat: bool) -> bool {
    let mut s = state.borrow_mut();
    if s.ctrl_space_toggled.is_some_and(|previous| previous == ms || repeat) {
        s.ctrl_space_toggled = Some(ms);
        true
    } else {
        false
    }
}

fn open_close_source(thread_mgr: &ITfThreadMgr) -> Option<ITfSource> {
    let mgr = thread_mgr.cast::<ITfCompartmentMgr>().ok()?;
    unsafe { mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE).ok()?.cast().ok() }
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
                    replace_prediction_space(state, context, ec, &text)?
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
                // Chromium 系与 CUAS 的临时文档由宿主自己把光标放在上屏文字末尾。它们的
                // TSF 选区可能落后于网页里的真实光标（如 Codex 编辑已发送消息时，成对标点
                // 后模拟的 ← 没同步到 TSF）；再按旧位置设选区，会把光标拉到右半边标点外。
                // 联想上屏仍要靠这次选区定位下一次空格，照旧设置。
                if frame.replace_space || frame.predict || !transitory(context) {
                    set_selection(context, ec, &range)?;
                }
                if frame.predict {
                    // Chromium may replay the renderer's committed suffix after
                    // the edit lock ends. Follow its final insertion to the right;
                    // the next first-space key creates its own replacement range.
                    let anchor = range.Clone().and_then(|caret| {
                        caret.SetGravity(ec, TF_GRAVITY_FORWARD, TF_GRAVITY_FORWARD)?;
                        Ok(caret)
                    });
                    let mut s = state.borrow_mut();
                    s.prediction_caret = anchor.ok();
                }
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
            Ok(if wants_window || frame.caret_back != 0 { selection_rect(context, ec) } else { None })
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
    if !s.predicting { s.prediction_caret = None; }
    s.pair_enabled = frame.pair_enabled;
    if !frame.pair_enabled { s.pairs.clear(); }
    s.busy = !s.predicting && (!frame.preedit.is_empty() || !frame.candidates.is_empty() || frame.title.is_some());
}

fn clear_prediction(state: &Rc<RefCell<State>>) -> bool {
    finish_prediction_space(state, true, false);
    let mut s = state.borrow_mut();
    if !std::mem::take(&mut s.predicting) {
        return false;
    }
    s.predict_double_space = false;
    s.prediction_caret = None;
    drop(s);
    with_window(state, |window| window.hide());
    true
}

/// 服务进程那边也要收起，不然看不见的联想词还会吃掉数字键和空格。
fn dismiss_prediction(state: &Rc<RefCell<State>>) {
    if clear_prediction(state) {
        if let Some(frame) = call(state, &Request::Reset { commit: false }) {
            note_frame(state, &frame);
        }
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

/// 候选窗的 Win32 调用（摆位置、显示、隐藏、改所属窗口）会同步发消息给宿主窗口，
/// 宿主（如记事本的 RichEdit）可能当场回调 TSF。调用期间不持有 State 借用：先取出
/// 窗口，用完放回；期间重入的回调看到「没有候选窗」，不会重复借用。
fn with_window<R>(state: &Rc<RefCell<State>>, f: impl FnOnce(&mut CandidateWindow) -> R) -> Option<R> {
    let (mut window, generation) = {
        let Ok(mut s) = state.try_borrow_mut() else {
            crate::log::write("候选窗调用重入", "已跳过");
            return None;
        };
        (s.window.take()?, s.generation)
    };
    let result = f(&mut window);
    let stale = {
        let mut s = state.borrow_mut();
        if s.generation == generation && s.window.is_none() {
            s.window = Some(window);
            None
        } else {
            Some(window)
        }
    };
    // 期间停用或重建了候选窗：旧窗口在借用之外销毁。
    drop(stale);
    Some(result)
}

/// 候选窗按应答更新；应用给不出位置时用上一次的或系统记的插入符。
fn show_window(state: &Rc<RefCell<State>>, frame: &Frame) {
    let wants = frame.hint.is_some() || !frame.candidates.is_empty() || frame.title.is_some();
    // GetWnd belongs to the host and can reenter TSF: release our state borrow
    // before calling it. Microsoft IME requirements specify GetFocus as fallback.
    // https://learn.microsoft.com/windows/apps/develop/input/input-method-editor-requirements#owned-window
    let context = state.borrow().context.clone();
    let owner = wants.then(|| unsafe {
        context.as_ref().and_then(|context| context.GetActiveView().and_then(|view| view.GetWnd()).ok())
            .filter(|window| !window.is_invalid()).unwrap_or_else(|| GetFocus())
    });
    let caret = {
        let s = state.borrow();
        if wants { s.caret.or(s.last_caret).or_else(fallback_caret) } else { s.caret }
    };
    let shown = with_window(state, |window| {
        if let Some(owner) = owner {
            if let Err(error) = window.set_owner(owner) {
                crate::log::write("关联候选窗失败", &error.to_string());
            }
        }
        window.update(frame, caret);
    });
    let unseen = wants && frame.predict && (caret.is_none() || shown.is_none());
    if unseen {
        // 不知道光标在哪、联想词摆不出来：不留看不见的联想
        dismiss_prediction(state);
    }
}

/// 应答落到文档和候选窗上。
fn apply(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, frame: Frame, sync: bool) {
    let expired = {
        let service = state.borrow();
        service.caret_move.is_some_and(|motion| service.window.as_ref().is_none_or(|window| window.hwnd().is_invalid())
            || motion.step(now_ms(), true, false) == keys::CaretStep::Drop)
    };
    if expired { pair::finish(state, true); }
    let deferred = {
        let mut service = state.borrow_mut();
        service.caret_move.is_some() && context.is_some() && service.pair_delivery.defer(&frame)
    };
    if deferred {
        note_frame(state, &frame);
        show_window(state, &frame);
        return;
    }
    // 鼠标选词、切换模式等也先落实单次空格；只有双击选词替换这个编辑区。
    if !frame.replace_space { finish_prediction_space(state, true, sync); }
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
    if let Some(pair) = pair::prepare_commit(state, context, &shared, client_id) {
        pair::commit_prepared(state, pair, shared, client_id);
        return;
    }
    if sync {
        let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
        edit(context, client_id, true, move |ec| {
            let result = apply_document(&state2, &context2, &frame2, ec);
            if let Err(error) = &result {
                crate::log::write("更新文档失败", &error.to_string());
                if frame2.undo_learning { let _ = call(&state2, &Request::UndoCommitLearning); }
            } else {
                caret_back(&state2, &frame2);
            }
            result
        });
        show_window(state, &shared);
    } else {
        // 异步：编辑会话里改完文档再摆候选窗
        let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
        let generation = state.borrow().generation;
        edit(context, client_id, false, move |ec| {
            if state2.borrow().generation != generation { return Ok(()); }
            let result = apply_document(&state2, &context2, &frame2, ec);
            if result.is_ok() { caret_back(&state2, &frame2); }
            show_window(&state2, &frame2);
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
    if s.window.as_ref().is_some_and(CandidateWindow::request_caret_tick) {
        s.caret_move = Some(keys::CaretMove::new(frame.caret_back, now_ms()));
        s.pair_delivery.sent = false;
        s.pair_delivery.released = false;
    } else {
        crate::log::write("发送辅助按键失败", "无法安排成对标点的光标移动");
    }
}

/// 等着的 [`keys::CaretMove`]：能挪就挪。返回 true 表示修饰键还按着，过一会儿再来。
///
/// 两处会叫它：候选窗的消息和定时器（[`ui::set_caret_handler`]），以及修饰键松开的时候
/// （[`key_released`]：用户一松开 Shift 就发，赶在下一个字前面）。
fn caret_tick(state: &Rc<RefCell<State>>) -> bool {
    let manager = state.borrow().thread_mgr.clone();
    // 焦点不在这个线程了：← 会落到别的程序里，不能发
    let focused = manager
        .as_ref()
        .is_some_and(|mgr| unsafe { mgr.IsThreadFocus() }.is_ok_and(|focus| focus.as_bool()));
    let mut s = state.borrow_mut();
    let Some(pending) = s.caret_move else { return false; };
    // 此刻键盘上的状态：消息里记的状态这时已经过时了
    let held = [0x10, 0x11, 0x12, 0x5b, 0x5c]
        .into_iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk) } < 0);
    let steps = match pending.step(now_ms(), focused, held) {
        keys::CaretStep::Wait => return true,
        keys::CaretStep::Drop => {
            drop(s);
            pair::finish(state, focused);
            return false;
        }
        keys::CaretStep::Send(_) if s.pair_delivery.sent => {
            let released = s.pair_delivery.released;
            drop(s);
            let placement = pair::placement(state);
            if placement == pair::Placement::Inside || (released && placement != pair::Placement::AfterTerminal) { pair::finish(state, true); return false; }
            return true;
        }
        keys::CaretStep::Send(steps) => steps,
    };
    s.pair_delivery.sent = true;
    drop(s);
    // 只为成对标点挪光标；联想选词始终通过 TSF 编辑区完成，不发送辅助键。
    let key = |up: bool, last: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_LEFT,
                wScan: 0x4b,
                dwFlags: if up { KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP } else { KEYEVENTF_EXTENDEDKEY },
                dwExtraInfo: if last { keys::CARET_END_TAG } else { keys::OWN_INPUT_TAG },
                ..Default::default()
            },
        },
    };
    let inputs: Vec<INPUT> = (0..steps).flat_map(|index| [key(false, false), key(true, index + 1 == steps)]).collect();
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        crate::log::write("发送辅助按键失败", &format!("{sent}/{}", inputs.len()));
        pair::finish(state, true);
        return false;
    }
    true
}

/// 正在处理的按键是不是 FreeType 为成对标点移动光标发出的按键。
fn own_input() -> bool {
    matches!(unsafe { GetMessageExtraInfo() }.0 as usize, keys::OWN_INPUT_TAG | keys::CARET_END_TAG)
}

fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// 只读一下光标前的字（同步）。
fn read_before(state: &Rc<RefCell<State>>, context: &ITfContext) -> Option<String> {
    let target = match surrounding(context) {
        Surrounding::Imm32 => return imm32_before(context),
        Surrounding::Tsf(target) => target,
    };
    let client_id = state.borrow().client_id;
    let out: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let (out2, context2) = (out.clone(), target.clone());
    edit_with(&target, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
        *out2.borrow_mut() = before_text(&context2, ec);
        Ok(())
    });
    let text = out.borrow_mut().take();
    text
}

fn key_before(state: &Rc<RefCell<State>>, context: &ITfContext) -> Option<String> {
    let wants = !composing(state) && {
        let s = state.borrow();
        s.mode == Mode::Chinese || s.want_before
    };
    if wants { read_before(state, context) } else { None }
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

fn plain_key(key: Key) -> bool {
    key.modifiers & (modifier::SHIFT | modifier::CTRL | modifier::ALT | modifier::SUPER) == 0
}

fn prediction_in(s: &State, context: &ITfContext) -> bool {
    s.predicting && s.context.as_ref().is_some_and(|owner| same_object(owner, context))
}

fn double_space_wait() -> u32 {
    unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }.clamp(200, 700)
}

/// 从第一次按下起拥有空格的编辑区，CUAS 和原生 TSF 使用完全相同的事务。
fn begin_prediction_space(state: &Rc<RefCell<State>>, context: &ITfContext, revision: u64, ms: u32) -> bool {
    if state.borrow().space.as_ref().is_some_and(|pending| pending.pressed_ms == ms) { return true; }
    finish_prediction_space(state, true, true);
    let (client_id, sink) = { let s = state.borrow(); (s.client_id, s.sink.clone()) };
    let done = Rc::new(std::cell::Cell::new(false));
    let (done2, state2, doc) = (done.clone(), state.clone(), context.clone());
    let requested = request_edit(context, client_id, TF_ES_SYNC | TF_ES_READWRITE, move |ec| {
        let result = (|| -> Result<()> { unsafe {
            let insert: ITfInsertAtSelection = doc.cast()?;
            let range = insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
            let owner: ITfContextComposition = doc.cast()?;
            let composition = owner.StartComposition(ec, &range, sink.as_ref())?;
            let result = (|| -> Result<ITfRange> {
                let range = composition.GetRange()?;
                range.SetText(ec, 0, &[u16::from(b' ')])?;
                let caret = range.Clone()?;
                caret.Collapse(ec, TF_ANCHOR_END)?;
                set_selection(&doc, ec, &caret)?;
                Ok(caret)
            })();
            let caret = match result {
                Ok(caret) => caret,
                Err(error) => {
                    let _ = range.SetText(ec, 0, &[]);
                    let _ = composition.EndComposition(ec);
                    return Err(error);
                }
            };
            let wait = double_space_wait();
            // GetTextExt 由应用实现，调用它时不持有可重入文本服务的状态借用。
            let rect = text_rect(&doc, ec, &caret);
            let at = {
                let mut s = state2.borrow_mut();
                s.space = Some(PredictionSpace {
                    context: doc.clone(), composition, revision, pressed_ms: ms,
                    deadline_ms: now_ms() + u64::from(wait),
                });
                s.undo = None;
                s.caret = rect.or(s.caret);
                s.caret
            };
            with_window(&state2, |window| {
                window.set_prediction_space_timeout(Some(wait));
                if let Some(at) = at { window.reposition(at); }
            });
            done2.set(true);
            Ok(())
        }})();
        if let Err(error) = &result { crate::log::write("保留联想空格失败", &error.to_string()); }
        result
    });
    if let Err(error) = requested { crate::log::write("保留联想空格失败", &error.to_string()); }
    done.get()
}

fn take_prediction_space(state: &Rc<RefCell<State>>) -> Option<PredictionSpace> {
    let mut s = state.borrow_mut();
    let pending = s.space.take()?;
    if let Some(window) = &s.window { window.set_prediction_space_timeout(None); }
    Some(pending)
}

/// 只结束这个空格拥有的编辑区。异步焦点清理也不会把旧文字写到新的输入框。
fn finish_prediction_space(state: &Rc<RefCell<State>>, commit: bool, sync: bool) {
    let Some(pending) = take_prediction_space(state) else { return; };
    let (client_id, state2) = (state.borrow().client_id, state.clone());
    let context = pending.context.clone();
    let flags = if sync { TF_ES_SYNC | TF_ES_READWRITE } else { TF_ES_ASYNCDONTCARE | TF_ES_READWRITE };
    let requested = request_edit(&context.clone(), client_id, flags, move |ec| {
        let result = (|| -> Result<()> { unsafe {
            let range = pending.composition.GetRange()?;
            if !commit { range.SetText(ec, 0, &[])?; }
            let caret = range.Clone()?;
            caret.Collapse(ec, TF_ANCHOR_END)?;
            caret.SetGravity(ec, TF_GRAVITY_FORWARD, TF_GRAVITY_FORWARD)?;
            pending.composition.EndComposition(ec)?;
            if !commit { set_selection(&context, ec, &caret)?; }
            let mut s = state2.borrow_mut();
            if prediction_in(&s, &context) { s.prediction_caret = Some(caret); }
            Ok(())
        }})();
        if let Err(error) = &result { crate::log::write("结束联想空格失败", &error.to_string()); }
        result
    });
    if let Err(error) = requested { crate::log::write("结束联想空格失败", &error.to_string()); }
}

fn prediction_space_timeout(state: &Rc<RefCell<State>>) {
    let remaining = state.borrow().space.as_ref().map(|pending| pending.deadline_ms.saturating_sub(now_ms()));
    match remaining {
        Some(0) => finish_prediction_space(state, true, false),
        Some(ms) => {
            if let Some(window) = &state.borrow().window { window.set_prediction_space_timeout(Some(ms as u32)); }
        }
        None => {}
    }
}

/// Both TSF callbacks use the same candidate page and caret. A fresh text
/// snapshot can lag behind the commit in a rich editor, so do not send it as
/// `before` for this space. A verified move still invalidates the prediction.
fn prediction_space_key(
    state: &Rc<RefCell<State>>, context: &ITfContext, key: Key, repeat: bool, revision: u64, ms: u32,
) -> (Key, bool) {
    if key.keysym != 0x20 || !plain_key(key) || repeat || revision == 0 {
        return (key, false);
    }
    let (client_id, anchor, pending, double) = {
        let s = state.borrow();
        if !prediction_in(&s, context) { return (key, false); }
        (s.client_id, s.prediction_caret.clone(), s.space.clone(), s.predict_double_space)
    };
    let pending = pending.filter(|p| p.revision == revision && same_object(&p.context, context));
    let second = double && pending.as_ref().is_some_and(|pending| {
        let elapsed = ms.wrapping_sub(pending.pressed_ms);
        elapsed > 0 && elapsed <= double_space_wait()
    });
    let result = Rc::new(std::cell::Cell::new(false));
    let (result2, doc) = (result.clone(), context.clone());
    edit_with(context, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
        let Some(caret) = selection_range(&doc, ec) else { return Ok(()); };
        unsafe {
            if !caret.IsEmpty(ec)?.as_bool() {
                result2.set(true);
                return Ok(());
            }
            if let Some(pending) = pending {
                let range = pending.composition.GetRange()?;
                result2.set(caret.CompareStart(ec, &range, TF_ANCHOR_END)? != 0);
            } else if let Some(anchor) = anchor {
                result2.set(caret.CompareStart(ec, &anchor, TF_ANCHOR_END)? != 0);
            }
        }
        Ok(())
    });
    if result.get() {
        dismiss_prediction(state);
        return (key, false);
    }
    if second { return (Key { keysym: keysym::SPACE_DOUBLE, ..key }, true); }
    if double && !key.test && !begin_prediction_space(state, context, revision, ms) {
        dismiss_prediction(state);
        return (key, false);
    }
    (key, true)
}

/// 虚拟键翻成键码；翻不出来（非 ASCII 字符、功能键）返回 None。
fn translate(wparam: WPARAM, lparam: LPARAM, test: bool) -> Option<Key> {
    let vk = wparam.0 as u16;
    let modifiers = keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) })
        | if lparam.0 & (1 << 30) != 0 { modifier::REPEAT } else { 0 };
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
        Click::LanguageMenu(x, y) => {
            let (context, generation) = {
                let mut s = state.borrow_mut();
                if s.language_menu != MenuState::Queued { return; }
                s.language_menu = MenuState::Open;
                (s.context.clone(), s.generation)
            };
            // Like Weasel, anchor the native menu to the focused document. Its lifetime
            // is independent of the candidate and hint windows and their hide timers.
            let owner = unsafe {
                context.and_then(|c| c.GetActiveView().and_then(|v| v.GetWnd()).ok())
                    .filter(|w| !w.is_invalid())
                    .unwrap_or_else(|| windows::Win32::UI::Input::KeyboardAndMouse::GetFocus())
            };
            let _dll = DllRef::new();
            let selected = button_menu(owner, x, y);
            let mut s = state.borrow_mut();
            if s.generation != generation { return; }
            s.language_menu = MenuState::Closed;
            let Some(request) = selected else { return; };
            request
        }
        Click::Candidate { revision, action } => Request::CandidateAction { revision, action },
        Click::PinChoices { .. } => return,
        Click::Dismiss => return dismiss_prediction(&state),
        Click::PredictionSpaceTimeout => return prediction_space_timeout(&state),
        Click::FocusNotice => return focus_notice::on_timer(&state),
        Click::RestoreKeyboardOpen => {
            let (manager, client_id) = {
                let s = state.borrow();
                (s.thread_mgr.clone(), s.client_id)
            };
            if let Some(manager) = manager { set_keyboard_open(&manager, client_id); }
            return;
        }
        Click::Settings => Request::Command {
            command: Command::OpenSettings,
        },
    };
    let Some(frame) = call(&state, &request) else {
        return;
    };
    if matches!(request, Request::Command { command: Command::OpenSettings }) {
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
        ButtonClick::Menu(x, y) => {
            // Return to the language bar immediately. Coalesce clicks even while
            // Windows dispatches messages inside TrackPopupMenu.
            let mut s = state.borrow_mut();
            if s.language_menu == MenuState::Closed {
                if let Some(window) = &s.window {
                    if unsafe { PostMessageW(Some(window.hwnd()), ui::WM_LANGUAGE_MENU,
                        WPARAM(x as usize), LPARAM(y as isize)) }.is_ok() {
                        s.language_menu = MenuState::Queued;
                    }
                }
            }
            return;
        }
    };
    let Some(frame) = call(&state, &request) else {
        return;
    };
    let context = state.borrow().context.clone();
    apply(&state, context.as_ref(), frame, false);
}

fn button_menu(owner: HWND, x: i32, y: i32) -> Option<Request> {
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_EX_TOOLWINDOW, WS_POPUP};
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let items: [(u32, &str); 7] = [
            (1, "切换中英文"),
            (2, "中文 / 英文标点"),
            (3, "简体 / 繁体"),
            (4, "表情与符号"),
            (5, "工具栏"),
            (7, "英文候选"),
            (8, "拆字查字"),
        ];
        for (id, text) in items {
            let _ = AppendMenuW(menu, MF_STRING, id as usize, &windows::core::HSTRING::from(text));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 6, windows::core::w!("FreeType 设置"));
        // The shell can deliver the click without a focused edit control. A separate
        // owner in that case survives every hide/update of the candidate window.
        let temporary = if owner.is_invalid() {
            CreateWindowExW(WS_EX_TOOLWINDOW, windows::core::w!("STATIC"), None, WS_POPUP,
                x, y, 0, 0, None, None, None, None).ok()
        } else { None };
        let owner = temporary.unwrap_or(owner);
        let cmd = if owner.is_invalid() { BOOL(0) } else {
            TrackPopupMenu(menu, TPM_NONOTIFY | TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, owner, None)
        };
        let _ = DestroyMenu(menu);
        if let Some(hwnd) = temporary { let _ = DestroyWindow(hwnd); }
        let command = match cmd.0 {
            1 => return Some(Request::ToggleMode { shift: None }),
            2 => Command::TogglePunct,
            3 => Command::ToggleTraditional,
            4 => Command::EmojiPanel,
            5 => Command::Toolbar,
            6 => Command::OpenSettings,
            7 => Command::EnglishCandidates,
            8 => Command::RadicalLookup,
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
    state.borrow_mut().key_preview = None;
    let context = focus.and_then(|doc| unsafe { doc.GetTop().ok() });
    focus_notice::observe(state, context.as_ref());
    // A rich editor can reassert the same focus during the commit, before
    // the prediction has even painted. Don't clear the new page or its range.
    if context.as_ref().is_some_and(|context| prediction_in(&state.borrow(), context)) {
        return;
    }
    let composition = state.borrow().composition.clone();
    if let Some(context) = composing_context(composition.as_ref(), focus) {
        resume_composition(state, context);
        return;
    }
    dismiss_prediction(state);
    let previous = state.borrow().context.clone();
    let same_context = previous.as_ref().zip(context.as_ref()).is_some_and(|(old, new)| same_object(old, new));
    let keep_move = same_context || previous.as_ref().zip(context.as_ref()).is_some_and(|(old, new)| {
        let (suffix, client) = {
            let service = state.borrow();
            (service.caret_move.and_then(|_| service.pairs.last().map(pair::Pair::suffix)), service.client_id)
        };
        let Some(suffix) = suffix else { return false; };
        if view_window(old).is_none_or(|window| Some(window) != view_window(new)) || disabled(Some(new)) { return false; }
        let verified = Rc::new(std::cell::Cell::new(false));
        let (result, target) = (verified.clone(), new.clone());
        edit_with(new, client, TF_ES_SYNC | TF_ES_READ, move |cookie| {
            result.set(!sensitive_scope(&target, cookie) && before_text(&target, cookie).is_some_and(|text| text.ends_with(&suffix)));
            Ok(())
        });
        verified.get()
    });
    if !keep_move && state.borrow().caret_move.is_some() { pair::finish(state, false); }
    {
        let mut s = state.borrow_mut();
        s.shift.cancel();
        s.prediction_escape = PredictionEscape::Free;
        if !same_context { s.pairs.clear(); }
        s.undo = None;
        s.last_caret = None;
    }
    with_window(state, |window| window.hide());
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
        state2.borrow_mut().sensitive_context = sensitive.then(|| context2.clone());
        set_sensitive(&state2, sensitive);
        if sensitive { return Ok(()); }
        // A queued focus read can arrive after the first commit in this field.
        if prediction_in(&state2.borrow(), &context2) { return Ok(()); }
        let before = match surrounding(&context2) {
            Surrounding::Tsf(target) if same_object(&target, &context2) => before_text(&context2, ec),
            // 父文档要另开读锁；这里先报不知道，下一个键会照常读到
            Surrounding::Tsf(_) => None,
            Surrounding::Imm32 => imm32_before(&context2),
        };
        // 大写锁定可能在别的窗口里开关过：先对齐，下面拿到的中英文状态才是对的
        let on = caps_on();
        state2.borrow_mut().caps = on;
        let _ = call(&state2, &Request::CapsLock { on });
        let Some(frame) = call(&state2, &Request::Focus { active: true, before }) else {
            return Ok(());
        };
        note_frame(&state2, &frame);
        focus_notice::capture(&state2, frame);
        Ok(())
    });
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        pair::finish(&self.state, false);
        finish_prediction_space(&self.state, true, false);
        let menu = std::mem::take(&mut self.state.borrow_mut().language_menu);
        if menu == MenuState::Open { unsafe { let _ = EndMenu(); } }
        ui::set_click_handler(None);
        ui::set_caret_handler(None);
        self.state.borrow_mut().focus_notice = Default::default();
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
                if let (Some(cookie), Ok(source)) = (s.thread_focus_cookie.take(), thread_mgr.cast::<ITfSource>()) {
                    let _ = source.UnadviseSink(cookie);
                }
                if let (Some(cookie), Some(source)) = (s.open_close_cookie.take(), open_close_source(&thread_mgr)) {
                    let _ = source.UnadviseSink(cookie);
                }
            }
        }
        s.composition = None;
        s.context = None;
        s.sensitive_context = None;
        s.key_preview = None;
        s.ctrl_space_toggled = None;
        s.busy = false;
        s.predicting = false;
        s.prediction_caret = None;
        s.space = None;
        s.prediction_escape = PredictionEscape::Free;
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
            let focus_sink: windows::Win32::UI::TextServices::ITfThreadFocusSink = self.to_interface();
            match source.AdviseSink(&windows::Win32::UI::TextServices::ITfThreadFocusSink::IID, &focus_sink) {
                Ok(cookie) => self.state.borrow_mut().thread_focus_cookie = Some(cookie),
                Err(error) => crate::log::write("订阅线程焦点失败", &error.to_string()),
            }
            let open_close_sink: ITfCompartmentEventSink = self.to_interface();
            let cookie = open_close_source(&thread_mgr).and_then(|source| source.AdviseSink(&ITfCompartmentEventSink::IID, &open_close_sink).ok());
            self.state.borrow_mut().open_close_cookie = cookie;
        }
        let weak = Rc::downgrade(&self.state);
        let button = langbar::create(Rc::new(move |click| on_button(&weak, click)));
        if let Err(error) = button.watch_brand(&thread_mgr) {
            crate::log::write("语言栏图标监视启动失败", &error.to_string());
        }
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
/// 导航、删除或快捷键可能换了插入点；普通字母的预询问不表示光标已移动。
fn note_key_down(state: &Rc<RefCell<State>>, wparam: WPARAM, lparam: LPARAM) {
    let vk = wparam.0 as u16;
    let repeat = lparam.0 & (1 << 30) != 0;
    let mut s = state.borrow_mut();
    let modifiers = moment();
    s.shift.key_down(vk, modifiers, repeat);
    if !keys::is_modifier(vk) { s.focus_notice.cancel(); }
    if matches!(vk, 0x08 | 0x09 | 0x0d | 0x1b | 0x21..=0x28 | 0x2d | 0x2e)
        || (!keys::is_modifier(vk) && modifiers.other_modifiers) {
        let pending = s.caret_move.is_some();
        drop(s);
        if pending { pair::finish(state, true); }
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
    let vk = wparam.0 as u16;
    if own_input() {
        if vk == VK_LEFT.0 && unsafe { GetMessageExtraInfo() }.0 as usize == keys::CARET_END_TAG
            && state.borrow().pair_delivery.sent {
            state.borrow_mut().pair_delivery.released = true;
            if pair::placement(state) != pair::Placement::AfterTerminal { pair::finish(state, true); }
        }
        return;
    }
    state.borrow_mut().key_preview = None;
    if vk == 0x20 { state.borrow_mut().ctrl_space_toggled = None; }
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

/// A visible prediction owns plain Esc until release, including repeats
/// after the window closes. A fresh press can still go to the application.
fn prediction_escape_down(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, key: Key, repeat: bool) -> bool {
    if key.keysym != keysym::ESCAPE { return false; }
    let (eat, close) = {
        let mut s = state.borrow_mut();
        if repeat && s.prediction_escape == PredictionEscape::Held { return true; }
        let visible = context.is_some_and(|context| prediction_in(&s, context))
            && s.window.as_ref().is_some_and(|window| window.visible());
        let claimed = visible && plain_key(key);
        s.prediction_escape = match (claimed, key.test) {
            (false, _) => PredictionEscape::Free,
            (true, true) => PredictionEscape::Preview,
            (true, false) => PredictionEscape::Held,
        };
        (claimed, claimed && !key.test)
    };
    if close { dismiss_prediction(state); }
    eat
}

fn prediction_escape_up(state: &Rc<RefCell<State>>, wparam: WPARAM, test: bool) -> bool {
    if own_input() || wparam.0 as u16 != 0x1b { return false; }
    let claimed = {
        let mut s = state.borrow_mut();
        let claimed = s.prediction_escape;
        if claimed == PredictionEscape::Free { return false; }
        s.prediction_escape = if test { PredictionEscape::Held } else { PredictionEscape::Free };
        claimed
    };
    // Do not forward an unmatched release. If the host omitted KeyDown
    // after a handled test, this still closes the window without editing text.
    if claimed == PredictionEscape::Preview { dismiss_prediction(state); }
    true
}

fn discard_prediction_from_other_context(state: &Rc<RefCell<State>>, context: &ITfContext) {
    let stale = {
        let s = state.borrow();
        s.predicting && !prediction_in(&s, context)
    };
    if stale { dismiss_prediction(state); }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> Result<()> {
        if !foreground.as_bool() {
            pair::finish(&self.state, false);
            self.state.borrow_mut().pairs.clear();
            dismiss_prediction(&self.state);
            self.state.borrow_mut().key_preview = None;
            self.state.borrow_mut().prediction_escape = PredictionEscape::Free;
            self.state.borrow_mut().ctrl_space_toggled = None;
            with_window(&self.state, |window| window.hide());
        }
        Ok(())
    }

    fn OnTestKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        self.state.borrow_mut().key_preview = None;
        if own_input() {
            return Ok((!self.state.borrow().pair_delivery.sent).into());
        }
        let ms = unsafe { GetMessageTime() } as u32;
        let vk = wparam.0 as u16;
        note_key_down(&self.state, wparam, lparam);
        // 先取得正式按下回调，再同步落实预编辑空格；预询问不是应用已处理按键的屏障。
        if vk != 0x1b && self.state.borrow().space.is_some() { return Ok(true.into()); }
        if vk == keys::VK_CAPITAL {
            caps_changed(&self.state, pic.as_ref());
        }
        if keys::is_modifier(vk) || check_sensitive(&self.state, pic.as_ref()) {
            return Ok(false.into());
        }
        if let Some(context) = pic.as_ref() { discard_prediction_from_other_context(&self.state, context); }
        watch_backspace(&self.state, pic.as_ref(), vk);
        let Some(key) = translate(wparam, lparam, true) else {
            dismiss_prediction(&self.state);
            // 组字时不认识的键也吃掉，免得漏给应用
            return Ok(composing(&self.state).into());
        };
        let repeat = lparam.0 & (1 << 30) != 0;
        // 同一消息已由按键回调或系统状态变化处理过；新按下不依赖上一回是否收到松开。
        if keys::is_ctrl_space(&key) && ctrl_space_handled(&self.state, ms, repeat) { return Ok(true.into()); }
        if prediction_escape_down(&self.state, pic.as_ref(), key, repeat)
            || verified_pair_key(&self.state, pic.as_ref(), key).is_some() {
            return Ok(true.into());
        }
        let (key, prediction_space) = pic.as_ref().map_or((key, false), |context| {
            prediction_space_key(&self.state, context, key, repeat, shown_revision(&self.state), ms)
        });
        if prediction_space && key.keysym == 0x20 && self.state.borrow().predict_double_space { return Ok(true.into()); }
        // 在决定接管按键前读取上文。若等到 OnKeyDown 才读，CUAS 的空文档会让
        // 自动空格从“接管并上屏”变成“放行”，重命名等输入框会丢掉这一次按键。
        let before = pic.as_ref().filter(|_| !prediction_space).and_then(|context| key_before(&self.state, context));
        let revision = shown_revision(&self.state);
        let frame = call(&self.state, &Request::PhysicalKey { key, before: before.clone(), scan: 0, shown_revision: Some(revision) });
        if let Some(frame) = &frame {
            note_preview(&self.state, frame);
            if frame.handled {
                if let Some(context) = pic.as_ref() {
                    self.state.borrow_mut().key_preview = Some(KeyPreview {
                        key: Key { test: false, ..key }, context: context.clone(), before,
                    });
                }
            }
            // A passed-through key won't get OnKeyDown. The server has
            // already put the prediction aside; don't leave the old popup.
            if !frame.handled && !frame.predict { clear_prediction(&self.state); }
        }
        Ok(frame.is_some_and(|f| f.handled).into())
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let preview = self.state.borrow_mut().key_preview.take();
        if own_input() {
            return Ok((!self.state.borrow().pair_delivery.sent).into());
        }
        let ms = unsafe { GetMessageTime() } as u32;
        let vk = wparam.0 as u16;
        note_key_down(&self.state, wparam, lparam);
        let repeat = lparam.0 & (1 << 30) != 0;
        let plain = plain_key(Key { modifiers: keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) }), ..Key::default() });
        if self.state.borrow().space.is_some() && (vk != 0x20 || repeat || !plain) {
            let erase = vk == 0x08 && plain;
            finish_prediction_space(&self.state, !erase, true);
            if erase { dismiss_prediction(&self.state); return Ok(true.into()); }
        }
        if vk == keys::VK_CAPITAL {
            caps_changed(&self.state, pic.as_ref());
        }
        if keys::is_modifier(vk) || check_sensitive(&self.state, pic.as_ref()) {
            return Ok(false.into());
        }
        watch_backspace(&self.state, pic.as_ref(), vk);
        let context = pic.ok()?.clone();
        discard_prediction_from_other_context(&self.state, &context);
        self.state.borrow_mut().context = Some(context.clone());
        let Some(key) = translate(wparam, lparam, false) else {
            // 翻不成键码的键（功能键等）交给应用；显示着的联想词先收起
            dismiss_prediction(&self.state);
            return Ok(composing(&self.state).into());
        };
        if keys::is_ctrl_space(&key) && ctrl_space_handled(&self.state, ms, repeat) { return Ok(true.into()); }
        if prediction_escape_down(&self.state, Some(&context), key, repeat) { return Ok(true.into()); }
        if let Some(action) = verified_pair_key(&self.state, Some(&context), key) {
            if let Some(mut frame) = call(&self.state, &Request::ClosePair) {
                frame.pair_edit = Some(action);
                apply(&self.state, Some(&context), frame, true);
                return Ok(true.into());
            }
        }
        let (key, prediction_space) = prediction_space_key(&self.state, &context, key, repeat, shown_revision(&self.state), ms);
        if prediction_space && key.keysym == 0x20 && self.state.borrow().predict_double_space { return Ok(true.into()); }
        let before = if prediction_space { None } else {
            preview.filter(|preview| preview.key == key && same_object(&preview.context, &context))
                .map(|preview| preview.before)
                .unwrap_or_else(|| key_before(&self.state, &context))
        };
        let scan = ((lparam.0 >> 16) & 0x1ff) as u16;
        let revision = shown_revision(&self.state);
        let frame = match call(&self.state, &Request::PhysicalKey { key, before, scan, shown_revision: Some(revision) }) {
            Some(frame) => frame,
            None => {
                // 服务进程没了：结束编辑区，键交给应用
                finish_prediction_space(&self.state, true, true);
                if composing(&self.state) {
                    apply(&self.state, Some(&context), Frame::default(), true);
                }
                return Ok(false.into());
            }
        };
        let handled = frame.handled;
        if handled && keys::is_ctrl_space(&key) { self.state.borrow_mut().ctrl_space_toggled = Some(ms); }
        let visible = self.state.borrow().window.as_ref().is_some_and(|w| w.visible());
        if handled || frame.commit.is_some() || composing(&self.state) || visible {
            apply(&self.state, Some(&context), frame, true);
        } else {
            note_frame(&self.state, &frame);
        }
        Ok(handled.into())
    }

    fn OnTestKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        if prediction_escape_up(&self.state, wparam, true) { return Ok(true.into()); }
        key_released(&self.state, pic.as_ref(), wparam, lparam);
        Ok(false.into())
    }

    fn OnKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        if prediction_escape_up(&self.state, wparam, false) { return Ok(true.into()); }
        key_released(&self.state, pic.as_ref(), wparam, lparam);
        Ok(false.into())
    }

    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(false.into())
    }
}

impl ITfCompartmentEventSink_Impl for TextService_Impl {
    fn OnChange(&self, rguid: *const GUID) -> Result<()> {
        if unsafe { rguid.as_ref() } != Some(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) { return Ok(()); }
        // 应用或系统改这个状态的时机不受我们控制；正在改 State 时不处理，避免在宿主进程里 panic
        let Ok(thread_mgr) = self.state.try_borrow().map(|s| s.thread_mgr.clone()) else { return Ok(()) };
        let closed = thread_mgr.and_then(|t| t.cast::<ITfCompartmentMgr>().ok())
            .is_some_and(|mgr| compartment_value(&mgr, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) == Some(0));
        let held = unsafe {
            let down = |vk: i32| GetAsyncKeyState(vk) < 0;
            down(0x11) && down(0x20) && !down(0x10) && !down(0x12) && !down(0x5b) && !down(0x5c)
        };
        if closed && held {
            ctrl_space_closed(&self.state);
        }
        Ok(())
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
    fn OnCompositionTerminated(&self, _ec: u32, composition: Ref<ITfComposition>) -> Result<()> {
        let Some(composition) = composition.as_ref() else { return Ok(()); };
        let pending = self.state.borrow().space.as_ref().is_some_and(|space| same_object(&space.composition, composition));
        if pending {
            take_prediction_space(&self.state);
            clear_prediction(&self.state);
        } else if !self.state.borrow().composition.as_ref().is_some_and(|current| same_object(current, composition)) {
            // 输入法主动结束的旧编辑区，不能清掉已开始的下一次输入。
            return Ok(());
        }
        self.state.borrow_mut().composition = None;
        with_window(&self.state, |window| window.hide());
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
        // RichEdit 等宿主会在我们改状态或摆候选窗时同步发来排版通知。这时跳过，
        // 编辑区下一次排版或下一个键会再摆一次；在宿主进程里 panic 会直接让应用崩溃。
        let Ok((composition, client_id, generation)) = self.state.try_borrow().map(|s| (
            s.composition.clone().or_else(|| s.space.as_ref().map(|space| space.composition.clone())),
            s.client_id, s.generation,
        )) else {
            return Ok(());
        };
        let Some(composition) = composition else {
            return Ok(());
        };
        let state = self.state.clone();
        let context2 = context.clone();
        edit_with(&context, client_id, TF_ES_ASYNCDONTCARE | TF_ES_READ, move |ec| {
            if state.try_borrow().map_or(true, |s| s.generation != generation) { return Ok(()); }
            let rect = unsafe { composition.GetRange() }
                .ok()
                .and_then(|range| text_rect(&context2, ec, &range));
            if let Some(rect) = rect {
                {
                    let Ok(mut s) = state.try_borrow_mut() else { return Ok(()); };
                    s.caret = Some(rect);
                    s.last_caret = Some(rect);
                }
                with_window(&state, |window| window.reposition(rect));
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

#[cfg(test)]
#[path = "service_prediction_tests.rs"]
mod prediction_tests;

#[cfg(test)]
#[path = "service_richedit_tests.rs"]
mod richedit_tests;
