//! 候选窗：每个线程一个不抢焦点的弹出窗口，用 DirectWrite 排版、Direct2D 双缓冲画。
//!
//! - 横排：顶上是标题（工具栏、表情面板、邮箱补全的说明）和编辑区拼音（小字），下面是
//!   「1 候选 2 候选 …」和翻页箭头；竖排、列表每个候选一行；网格（展开面板、表情）按列排。
//! - 鼠标停在候选、箭头上是手形指针并有浅色底；点候选选词，点箭头翻页。
//! - 长按候选（半秒）或右键候选：菜单里能删除这个词的记忆；右键任何地方都有「打开设置」。
//! - 切换中英文等提示只显示一小块，一会儿自动收起，位置按设置放在光标的某一边。
//!
//! 颜色跟随系统的浅色 / 深色模式。字体、字号来自设置。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use freetype_protocol::{parse_hex_rgb, CandidateAction, CandidateSkin, Frame, Panel};
use windows::core::{w, Error, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, SetLastError, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT,
    WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, MonitorFromWindow, SelectObject,
    UpdateWindow, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

#[path = "ui_text.rs"]
mod text;
mod theme;
use theme::*;
mod layout;
use layout::*;
mod paint;
use paint::*;
mod window_proc;
use window_proc::*;
mod candidate_window;
mod editor;
use editor::*;

use text::{Fonts, Painter, Text};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos,
    GetWindowLongPtrW, GetWindowRect, IsWindowVisible, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassExW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW,
    ShowWindow, UnhookWindowsHookEx, CS_DROPSHADOW, CS_HREDRAW, CS_VREDRAW, GWLP_HWNDPARENT,
    GWLP_USERDATA, HC_ACTION, HHOOK, HWND_TOPMOST, IDC_ARROW, IDC_HAND, MA_NOACTIVATE,
    MOUSEHOOKSTRUCT, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WH_MOUSE, WM_ACTIVATEAPP, WM_APP,
    WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_NCMBUTTONDOWN,
    WM_NCRBUTTONDOWN, WM_NCXBUTTONDOWN, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
    WM_TIMER, WM_XBUTTONDOWN, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("FreeTypeCandidateWindow");
const EDITOR_CLASS: PCWSTR = w!("FreeTypeSentenceEditor");
const MENU_CLASS: PCWSTR = w!("FreeTypeCandidateMenu");
const HINT_TIMER: usize = 1;
const LONG_TIMER: usize = 2;
/// 成对标点等着挪光标（等用户松开 Shift）时隔一会儿再看一次。
const CARET_TIMER: usize = 3;
const CARET_RETRY_MS: u32 = 15;
/// 发给自己的消息：回到应用的消息循环以后，叫文本服务去挪光标（见 [`set_caret_handler`]）。
const WM_CARET_TICK: u32 = WM_APP + 1;
const WM_EDITOR_UNAVAILABLE: u32 = WM_APP + 5;
pub(crate) const WM_LANGUAGE_MENU: u32 = WM_APP + 2;
pub(crate) const WM_RESTORE_KEYBOARD_OPEN: u32 = WM_APP + 3;
const HINT_MS: u32 = 1000;
/// 上屏联想显示着没人选：过这么久自己收起（见 [`Click::Dismiss`]）。
const PREDICT_TIMER: usize = 4;
const PREDICT_MS: u32 = 6000;
/// 单次空格暂留在输入法编辑区，双击窗口结束后提交为普通空格。
const PREDICTION_SPACE_TIMER: usize = 5;
const FOCUS_NOTICE_TIMER: usize = 6;
const ENHANCEMENT_TIMER: usize = 7;
/// 按住多久算长按。
const LONG_PRESS_MS: u32 = 500;
/// 联想显示时，本线程里点到候选窗以外（见 [`watch_outside_clicks`]）。
const WM_OUTSIDE_CLICK: u32 = WM_APP + 4;
/// 鼠标离开窗口（TrackMouseEvent）。
const WM_MOUSELEAVE: u32 = 0x02a3;

/// 候选窗上的点击（交给文本服务处理）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    RefreshEnhancement(u64),
    LanguageMenu(i32, i32),
    Candidate {
        action: CandidateAction,
        revision: u64,
    },
    /// 展开固定排位子菜单，带原候选页的绘制编号。
    PinChoices {
        owner: HWND,
        index: usize,
        revision: u64,
    },
    Settings,
    /// 上屏联想一会儿没选：收起来（服务进程那边也结束联想，数字键、空格不再选词）。
    Dismiss,
    /// 双击间隔结束，请文本服务提交仍保留着的单次空格。
    PredictionSpaceTimeout,
    FocusNotice,
    /// 离开 TSF 状态通知后恢复键盘打开，避免在锁定的 compartment 内写回。
    RestoreKeyboardOpen,
}

thread_local! {
    /// 点击交给文本服务处理（每个线程一个文本服务）。
    static HANDLER: RefCell<Option<std::rc::Rc<dyn Fn(Click)>>> = const { RefCell::new(None) };
    /// 这个线程的右键 / 长按菜单窗口。
    static MENU: RefCell<Option<HWND>> = const { RefCell::new(None) };
    /// 文本服务挪光标的处理函数：返回 true 表示还没挪成，过一会儿再叫一次。
    static CARET_HANDLER: RefCell<Option<Box<dyn Fn() -> bool>>> = const { RefCell::new(None) };
    /// 联想显示期间装着的本线程鼠标钩子，和要通知的候选窗。
    static OUTSIDE: Cell<Option<(HHOOK, HWND)>> = const { Cell::new(None) };
}

pub fn set_click_handler(handler: Option<Box<dyn Fn(Click)>>) {
    HANDLER.with(|h| *h.borrow_mut() = handler.map(std::rc::Rc::from));
}

/// 候选窗是这个线程里输入法自己的窗口，文本服务借它「回到消息循环以后再做」和定时重试
/// （[`CandidateWindow::request_caret_tick`]）。
pub fn set_caret_handler(handler: Option<Box<dyn Fn() -> bool>>) {
    CARET_HANDLER.with(|h| *h.borrow_mut() = handler);
}

#[derive(Clone, Copy)]
struct Theme {
    background: COLORREF,
    border: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    hover: COLORREF,
    /// 选中候选的底色；None 是搜狗那样不画底色块，只把字换成 `accent_text`。
    accent: Option<COLORREF>,
    /// 选中候选的字（和序号）的颜色。
    accent_text: COLORREF,
    /// 切换提示（中 / 英）的字色：强调色在背景上看得清的那一档。
    emphasis: COLORREF,
}

/// 要画的东西。
#[derive(Clone, Default, PartialEq)]
struct View {
    sentence: Option<freetype_protocol::SentenceView>,
    editor_above: bool,
    title: Option<String>,
    preedit: String,
    candidates: Vec<String>,
    annotations: Vec<String>,
    badges: Vec<String>,
    labels: Vec<String>,
    cursor: Option<usize>,
    has_prev: bool,
    has_next: bool,
    hint: Option<String>,
    hint_position: String,
    font_size: u32,
    font_family: String,
    font_fallbacks: String,
    preedit_size: u32,
    lock_position: bool,
    wrap_width: i32,
    available_width: i32,
    wrap: bool,
    /// 候选强调色，见 [`theme`]。
    candidate_style: String,
    candidate_skin: Option<CandidateSkin>,
    candidate_wheel: bool,
    revision: u64,
    pin_slots: Vec<Option<u8>>,
    pin_limit: u8,
    vertical: bool,
    panel: Panel,
    columns: usize,
    forgettable: bool,
    prediction: bool,
}

impl View {
    /// 同一批联想的高亮和发布版本不参与几何计算；工作区宽度是首次排版的结果。
    fn same_prediction_page(&self, next: &Self) -> bool {
        if !self.prediction || !next.prediction {
            return false;
        }
        let mut current = self.clone();
        current.cursor = next.cursor;
        current.revision = next.revision;
        current.wrap_width = next.wrap_width;
        current.available_width = next.available_width;
        current == *next
    }

    fn label(&self, i: usize) -> String {
        if self.labels.is_empty() {
            format!("{}", (i + 1) % 10)
        } else {
            self.labels.get(i).cloned().unwrap_or_default()
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Sentence(usize),
    Letter,
    Title,
    Preedit,
    Label(usize),
    Candidate(usize),
    Hint,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hover {
    Sentence(usize),
    Letter(usize),
    Item(usize),
    Arrow(bool),
}

/// 一段摆好位置的文字。
struct Piece {
    /// 墨迹占的地方（窗口坐标）。只有渲染测试用它核对文字没有越出窗口。
    #[cfg_attr(not(test), allow(dead_code))]
    ink: RECT,
    part: Part,
    text: Text,
    /// 文字的框画在哪（[`Painter::text`]）。
    at: (f32, f32),
}

impl Piece {
    fn new(part: Part, text: Text, x: f32, y: f32) -> Self {
        Self {
            ink: text.ink_at(x, y),
            part,
            text,
            at: (x, y),
        }
    }
}

/// 排好的版：每一块的位置。
#[derive(Default)]
struct Layout {
    sentence_hits: Vec<(RECT, usize)>,
    letter_hits: Vec<(RECT, usize)>,
    width: i32,
    height: i32,
    parts: Vec<Piece>,
    /// 候选的底（点击区域）。
    items: Vec<RECT>,
    /// 翻页箭头（点击区域）。
    arrows: Vec<(RECT, bool, Text)>,
}

struct Data {
    // Windows destroys owned popups with their host; never reuse that stale HWND.
    alive: Rc<Cell<bool>>,
    view: View,
    layout: Layout,
    scale: f32,
    hovered: Option<Hover>,
    /// 松开时必须仍命中同一目标，避免内容更新或拖出后误选。
    pressed: Option<Hover>,
    /// 长按已经弹出菜单了（松开时不再选词）。
    long_fired: bool,
    // 精密滚轮可能分多条消息报告一格。
    wheel_delta: i32,
    painted_revision: u64,
    anchor: Option<RECT>,
}

impl Data {
    fn click(&self, action: CandidateAction) -> Click {
        Click::Candidate {
            action,
            revision: self.painted_revision,
        }
    }
}

pub struct CandidateWindow {
    hwnd: HWND,
    instance: HINSTANCE,
    alive: Rc<Cell<bool>>,
    // 辅助区拥有独立窗口与排版；不能把其尺寸加入候选窗的几何计算。
    editor: RefCell<Option<EditorWindow>>,
}

impl CandidateWindow {
    pub fn new(instance: HINSTANCE) -> Option<Self> {
        let hwnd = popup(instance, CLASS_NAME, None)?;
        let alive = Rc::new(Cell::new(true));
        let data = Box::new(Data {
            alive: alive.clone(),
            view: View::default(),
            layout: Layout::default(),
            scale: 1.0,
            hovered: None,
            pressed: None,
            long_fired: false,
            wheel_delta: 0,
            painted_revision: 0,
            anchor: None,
        });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as _);
        }
        MENU.with(|m| *m.borrow_mut() = popup(instance, MENU_CLASS, Some(hwnd)));
        Some(Self {
            hwnd,
            instance,
            alive,
            editor: RefCell::new(None),
        })
    }

    /// Windows shell input requires an owned popup. Reuse it across keystrokes;
    /// if the host destroyed it, recover before displaying the next candidate page.
    pub fn set_owner(&mut self, owner: HWND) -> Result<()> {
        if !self.alive.get() {
            *self = Self::new(self.instance).ok_or_else(|| Error::from_thread())?;
        }
        set_popup_owner(self.hwnd, owner)?;
        if let Some(editor) = self.editor.borrow().as_ref().filter(|e| e.alive.get()) {
            set_popup_owner(editor.hwnd, owner)?;
        }
        Ok(())
    }

    pub fn hide(&self) {
        if let Some(editor) = self.editor.borrow().as_ref() {
            editor.hide();
        }
        if !self.alive.get() {
            return;
        }
        menu::close();
        if let Some(data) = data_of(self.hwnd) {
            data.wheel_delta = 0;
            data.painted_revision = 0;
            data.anchor = None;
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), PREDICT_TIMER);
            let _ = KillTimer(Some(self.hwnd), ENHANCEMENT_TIMER);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        watch_outside_clicks(self.hwnd, false);
        // Keep our hidden timer/message window alive when the input host closes.
        if let Err(error) = set_popup_owner(self.hwnd, HWND::default()) {
            crate::log::write("取消候选窗关联失败", &error.to_string());
        }
        if let Some(editor) = self.editor.borrow().as_ref().filter(|e| e.alive.get()) {
            if let Err(error) = set_popup_owner(editor.hwnd, HWND::default()) {
                crate::log::write("取消整句编辑区关联失败", &error.to_string());
            }
        }
    }

    pub fn hwnd(&self) -> HWND {
        if self.alive.get() {
            self.hwnd
        } else {
            HWND::default()
        }
    }

    pub fn visible(&self) -> bool {
        self.alive.get() && unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn painted_revision(&self) -> u64 {
        if !self.visible() {
            return 0;
        }
        data_of(self.hwnd).map_or(0, |data| data.painted_revision)
    }

    /// 回到应用的消息循环以后叫一次 [`set_caret_handler`] 的处理函数；它说还要等，就定时再叫。
    pub fn request_caret_tick(&self) -> bool {
        self.alive.get()
            && unsafe { PostMessageW(Some(self.hwnd), WM_CARET_TICK, WPARAM(0), LPARAM(0)).is_ok() }
    }

    /// 空格编辑区由文本服务拥有，窗口只负责投递一次超时通知。
    pub fn set_prediction_space_timeout(&self, delay_ms: Option<u32>) {
        if !self.alive.get() {
            return;
        }
        unsafe {
            match delay_ms {
                Some(ms) => {
                    SetTimer(Some(self.hwnd), PREDICTION_SPACE_TIMER, ms, None);
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), PREDICTION_SPACE_TIMER);
                }
            }
        }
    }

    pub fn schedule_focus_notice(&self) {
        if self.alive.get() {
            unsafe {
                SetTimer(Some(self.hwnd), FOCUS_NOTICE_TIMER, 120, None);
            }
        }
    }

    /// 按应答更新；`caret` 是编辑区（或光标）在屏幕上的位置。
    /// 增强模型还在算当前输入时，每 100 毫秒问一次结果；结果到了或输入变了就停。
    pub fn poll_enhancement(&self, pending: bool) {
        unsafe {
            if pending {
                SetTimer(Some(self.hwnd), ENHANCEMENT_TIMER, 100, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), ENHANCEMENT_TIMER);
            }
        }
    }
}

/// 整句编辑区只读取普通候选窗的最终矩形；空间不足就收起，不反向挪动候选窗。
struct EditorWindow {
    hwnd: HWND,
    alive: Rc<Cell<bool>>,
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        if !self.alive.get() {
            return;
        }
        menu::destroy();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// 候选窗的菜单：不抢焦点（不然应用的编辑区会被结束），用鼠标捕获判断点在菜单外面。
mod menu;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_sits_on_the_chosen_side() {
        let caret = RECT {
            left: 100,
            top: 200,
            right: 102,
            bottom: 220,
        };
        assert_eq!(hint_origin("bottom-right", caret, 30, 30, 4), (106, 224));
        assert_eq!(hint_origin("top-left", caret, 30, 30, 4), (66, 166));
        assert_eq!(hint_origin("bottom", caret, 30, 30, 4), (85, 224));
        assert_eq!(
            hint_origin("bogus", caret, 30, 30, 4),
            (106, 224),
            "不认识的写法放右下"
        );
    }

    #[test]
    fn labels_default_to_numbers() {
        let view = View {
            candidates: vec!["a".into(), "b".into()],
            ..View::default()
        };
        assert_eq!(view.label(1), "2");
        assert_eq!(view.label(9), "0", "第 10 个候选标出实际选择键");
        let view = View {
            labels: vec!["".into(), "3".into()],
            ..view
        };
        assert_eq!(view.label(0), "");
        assert_eq!(view.label(1), "3");
    }
}

#[cfg(test)]
#[path = "ui_render_tests.rs"]
pub(crate) mod render_tests;
