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
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::Input::Ime::{IMR_DOCUMENTFEED, RECONVERTSTRING};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetFocus, GetKeyState, GetKeyboardState, SendInput, ToUnicode, INPUT,
    INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_LEFT,
};
use windows::Win32::UI::TextServices::{
    ITfCompartmentEventSink, ITfCompartmentEventSink_Impl, ITfCompartmentMgr, ITfComposition,
    ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition, ITfContextView,
    ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInputScope, ITfInsertAtSelection,
    ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfLangBarItemMgr, ITfRange, ITfSource,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl,
    ITfTextLayoutSink, ITfTextLayoutSink_Impl, ITfThreadMgr, ITfThreadMgrEventSink,
    ITfThreadMgrEventSink_Impl, TfLayoutCode, GUID_COMPARTMENT_EMPTYCONTEXT,
    GUID_COMPARTMENT_KEYBOARD_DISABLED, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT, GUID_PROP_INPUTSCOPE,
    INSERT_TEXT_AT_SELECTION_FLAGS, IS_ALPHANUMERIC_PIN, IS_ALPHANUMERIC_PIN_SET,
    IS_NUMERIC_PASSWORD, IS_NUMERIC_PIN, IS_PASSWORD, TF_AE_NONE, TF_ANCHOR_END, TF_ANCHOR_START,
    TF_DEFAULT_SELECTION, TF_ES_ASYNCDONTCARE, TF_ES_READ, TF_ES_READWRITE, TF_ES_SYNC,
    TF_GRAVITY_FORWARD, TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE, TF_SS_TRANSITORY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, EndMenu, GetGUIThreadInfo, GetMessageExtraInfo,
    GetMessageTime, GetWindowRect, PostMessageW, SendMessageW, TrackPopupMenu, GUITHREADINFO,
    MF_SEPARATOR, MF_STRING, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_IME_REQUEST,
};

use crate::client::Client;
use crate::keys::{self, Moment, ShiftTap};
use crate::langbar::{self, ButtonClick, ButtonHandle};
use crate::ui::{self, CandidateWindow, Click};
use crate::DllRef;

mod document;
#[path = "focus_notice.rs"]
mod focus_notice;
#[path = "pair.rs"]
mod pair;
use document::*;
mod keyboard_state;
use keyboard_state::*;
mod frames;
use frames::*;
mod key_input;
use key_input::*;
mod prediction;
use prediction::*;
mod ui_events;
use ui_events::*;
mod focus;
use focus::*;
mod sinks;

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
    /// 设置允许：补全的左移键发出前按下的右半标点只跳过，不再插入。
    pair_absorb_closing: bool,
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
        s.composition.is_none()
            && !s.busy
            && s.caret_move.is_none()
            && s.space.is_none()
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
                pair_absorb_closing: false,
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

fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

fn call(state: &Rc<RefCell<State>>, request: &Request) -> Option<Frame> {
    state.borrow_mut().client.call(request)?.frame
}

fn shown_revision(state: &Rc<RefCell<State>>) -> u64 {
    state
        .borrow()
        .window
        .as_ref()
        .map_or(0, CandidateWindow::painted_revision)
}

fn composing(state: &Rc<RefCell<State>>) -> bool {
    let s = state.borrow();
    s.composition.is_some() || s.busy
}

/// 两个接口指针是不是同一个 COM 对象（按 COM 的规定比 `IUnknown`，不比接口指针本身）。
fn same_object<A: Interface, B: Interface>(a: &A, b: &B) -> bool {
    match (
        a.cast::<windows::core::IUnknown>(),
        b.cast::<windows::core::IUnknown>(),
    ) {
        (Ok(a), Ok(b)) => a.as_raw() == b.as_raw(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_handoff_waits_for_input_and_caret_completion() {
        let service = TextService::new(HINSTANCE::default());
        assert!(service.can_reload());
        note_frame(
            &service.state,
            &Frame {
                preedit: "ni".into(),
                handled: true,
                ..Frame::default()
            },
        );
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
        let predicted = Frame {
            handled: true,
            preedit: "ni".into(),
            ..Frame::default()
        };
        note_preview(&service.state, &predicted);
        assert!(
            !composing(&service.state),
            "尚未接受的按键不能使 DLL 提前进入组字状态"
        );
        note_frame(&service.state, &predicted);
        assert!(composing(&service.state));
        let predicted_commit = Frame {
            handled: true,
            commit: Some("你".into()),
            ..Frame::default()
        };
        note_preview(&service.state, &predicted_commit);
        assert!(
            composing(&service.state),
            "预判上屏也不能提前结束正在进行的组字"
        );
    }

    #[test]
    fn next_word_predictions_are_not_a_composition() {
        let service = TextService::new(HINSTANCE::default());
        let frame = Frame {
            handled: true,
            commit: Some("今天".into()),
            candidates: vec!["下雨".into()],
            predict: true,
            ..Frame::default()
        };
        note_frame(&service.state, &frame);
        assert!(
            !composing(&service.state),
            "联想词显示着时不认的键要交给应用，还要照常读光标前的字"
        );
        assert!(service.state.borrow().predicting);
        note_frame(&service.state, &Frame::default());
        assert!(!service.state.borrow().predicting);
    }
}

#[cfg(test)]
#[path = "service_focus_tests.rs"]
mod focus_tests;

#[cfg(test)]
mod prediction_tests;

#[cfg(test)]
#[path = "service_richedit_tests.rs"]
mod richedit_tests;
