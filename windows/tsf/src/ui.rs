//! 候选窗：每个线程一个不抢焦点的弹出窗口，用 DirectWrite 排版、Direct2D 双缓冲画。
//!
//! - 横排：顶上是标题（工具栏、表情面板、邮箱补全的说明）和编辑区拼音（小字），下面是
//!   「1 候选 2 候选 …」和翻页箭头；竖排、列表每个候选一行；网格（展开面板、表情）按列排。
//! - 鼠标停在候选、箭头上是手形指针并有浅色底；点候选选词，点箭头翻页。
//! - 长按候选（半秒）或右键候选：菜单里能删除这个词的记忆；右键任何地方都有「打开设置」。
//! - 切换中英文等提示只显示一小块，一会儿自动收起，位置按设置放在光标的某一边。
//!
//! 颜色跟随系统的浅色 / 深色模式。字体、字号来自设置。

use std::cell::RefCell;

use freetype_protocol::{Frame, Panel};
use windows::core::{w, HSTRING, PCWSTR, Result};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EndPaint,
    GetMonitorInfoW, InvalidateRect, MonitorFromPoint, MonitorFromWindow, SelectObject, UpdateWindow,
    HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

#[path = "ui_text.rs"]
mod text;
use text::{Fonts, Painter, Text};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowLongPtrW, GetWindowRect, IsWindowVisible,
    KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, CS_DROPSHADOW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW, IDC_HAND,
    MA_NOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_APP, WM_CAPTURECHANGED, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER,
    WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("FreeTypeCandidateWindow");
const MENU_CLASS: PCWSTR = w!("FreeTypeCandidateMenu");
const HINT_TIMER: usize = 1;
const LONG_TIMER: usize = 2;
/// 成对标点等着挪光标（等用户松开 Shift）时隔一会儿再看一次。
const CARET_TIMER: usize = 3;
const CARET_RETRY_MS: u32 = 15;
/// 发给自己的消息：回到应用的消息循环以后，叫文本服务去挪光标（见 [`set_caret_handler`]）。
const WM_CARET_TICK: u32 = WM_APP + 1;
const HINT_MS: u32 = 1000;
/// 按住多久算长按。
const LONG_PRESS_MS: u32 = 500;
/// 鼠标离开窗口（TrackMouseEvent）。
const WM_MOUSELEAVE: u32 = 0x02a3;

/// 候选窗上的点击（交给文本服务处理）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Select(usize),
    Page(bool),
    /// 删除这一页第几个候选的记忆。
    Forget(usize),
    Settings,
}

thread_local! {
    /// 点击交给文本服务处理（每个线程一个文本服务）。
    static HANDLER: RefCell<Option<Box<dyn Fn(Click)>>> = const { RefCell::new(None) };
    /// 这个线程的右键 / 长按菜单窗口。
    static MENU: RefCell<Option<HWND>> = const { RefCell::new(None) };
    /// 文本服务挪光标的处理函数：返回 true 表示还没挪成，过一会儿再叫一次。
    static CARET_HANDLER: RefCell<Option<Box<dyn Fn() -> bool>>> = const { RefCell::new(None) };
}

pub fn set_click_handler(handler: Option<Box<dyn Fn(Click)>>) {
    HANDLER.with(|h| *h.borrow_mut() = handler);
}

/// 候选窗是这个线程里输入法自己的窗口，文本服务借它「回到消息循环以后再做」和定时重试
/// （[`CandidateWindow::request_caret_tick`]）。
pub fn set_caret_handler(handler: Option<Box<dyn Fn() -> bool>>) {
    CARET_HANDLER.with(|h| *h.borrow_mut() = handler);
}

fn caret_tick(hwnd: HWND) {
    let again = CARET_HANDLER.with(|h| h.borrow().as_ref().is_some_and(|handler| handler()));
    unsafe {
        if again {
            SetTimer(Some(hwnd), CARET_TIMER, CARET_RETRY_MS, None);
        } else {
            let _ = KillTimer(Some(hwnd), CARET_TIMER);
        }
    }
}

fn dispatch(click: Click) {
    HANDLER.with(|h| {
        if let Some(handler) = h.borrow().as_ref() {
            handler(click);
        }
    });
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

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}

fn light_mode() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
    }
    value != 0
}

/// 按设置里的候选强调色（`Config::candidate_style`）和系统浅色 / 深色配色。
/// 设置页的预览（windows/app/pages.go 的 candidateStyles）用同样的颜色，改一边要改另一边。
fn theme(style: &str) -> Theme {
    let light = light_mode();
    let white = rgb(0xff, 0xff, 0xff);
    // (底色块, 块上的字, 提示字色)
    let (accent, accent_text, emphasis) = match (style, light) {
        // 以前的蓝色
        ("blue", true) => (Some(rgb(0x2b, 0x6c, 0xe6)), white, rgb(0x2b, 0x6c, 0xe6)),
        ("blue", false) => (Some(rgb(0x3d, 0x7f, 0xf0)), white, rgb(0x6c, 0x9e, 0xff)),
        // 微信输入法的绿；浅色背景上的提示字用深一档的绿，不然看不清
        ("green", true) => (Some(rgb(0x07, 0xc1, 0x60)), white, rgb(0x06, 0xa0, 0x50)),
        ("green", false) => (Some(rgb(0x07, 0xc1, 0x60)), white, rgb(0x07, 0xc1, 0x60)),
        // 搜狗那样：没有底色块，选中的字是蓝色
        ("sogou", true) => (None, rgb(0x1e, 0x6f, 0xff), rgb(0x1e, 0x6f, 0xff)),
        ("sogou", false) => (None, rgb(0x6c, 0x9e, 0xff), rgb(0x6c, 0x9e, 0xff)),
        // 默认：品牌粉色。粉色很浅，块上用深色字（和设置窗口一样）；提示字用深一档的粉
        (_, true) => (Some(rgb(0xff, 0xb3, 0xfa)), rgb(0x18, 0x18, 0x1b), rgb(0xc0, 0x3c, 0xb4)),
        (_, false) => (Some(rgb(0xff, 0xb3, 0xfa)), rgb(0x18, 0x18, 0x1b), rgb(0xff, 0xb3, 0xfa)),
    };
    // 鼠标停留的底用中性灰，几种强调色都配得上
    if light {
        Theme {
            background: white,
            border: rgb(0xd6, 0xd6, 0xd6),
            text: rgb(0x1f, 0x1f, 0x1f),
            muted: rgb(0x80, 0x80, 0x80),
            hover: rgb(0xf0, 0xf0, 0xf0),
            accent,
            accent_text,
            emphasis,
        }
    } else {
        Theme {
            background: rgb(0x2b, 0x2b, 0x2b),
            border: rgb(0x48, 0x48, 0x48),
            text: rgb(0xee, 0xee, 0xee),
            muted: rgb(0x9a, 0x9a, 0x9a),
            hover: rgb(0x3a, 0x3a, 0x3a),
            accent,
            accent_text,
            emphasis,
        }
    }
}

/// 要画的东西。
#[derive(Clone, Default, PartialEq)]
struct View {
    title: Option<String>,
    preedit: String,
    candidates: Vec<String>,
    labels: Vec<String>,
    cursor: Option<usize>,
    has_prev: bool,
    has_next: bool,
    hint: Option<String>,
    hint_position: String,
    font_size: u32,
    font_family: String,
    /// 候选强调色，见 [`theme`]。
    candidate_style: String,
    vertical: bool,
    panel: Panel,
    columns: usize,
    forgettable: bool,
}

impl View {
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
    Title,
    Preedit,
    Label(usize),
    Candidate(usize),
    Hint,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hover {
    Item(usize),
    Arrow(bool),
}

/// 排好的版：每一块的位置。
#[derive(Default)]
struct Layout {
    width: i32,
    height: i32,
    /// (画的位置, 是什么)
    parts: Vec<(RECT, Part, Text)>,
    /// 候选的底（点击区域）。
    items: Vec<RECT>,
    /// 翻页箭头（点击区域）。
    arrows: Vec<(RECT, bool, Text)>,
}

struct Data {
    view: View,
    layout: Layout,
    scale: f32,
    hovered: Option<Hover>,
    /// 按下鼠标时在第几个候选上（长按用）。
    pressed: Option<usize>,
    /// 长按已经弹出菜单了（松开时不再选词）。
    long_fired: bool,
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

fn layout(view: &View, fonts: &Fonts, scale: f32) -> Result<Layout> {
    let s = |v: f32| (v * scale).round() as i32;
    let pad = s(8.0);
    let mut out = Layout::default();
    {
        if let Some(hint) = &view.hint {
            let text = fonts.hint.text(hint)?;
            let size = text.size;
            let side = size.cx.max(size.cy);
            out.parts.push((rect(pad + (side - size.cx) / 2, pad, size.cx, size.cy), Part::Hint, text));
            out.width = side + 2 * pad;
            out.height = size.cy + 2 * pad;
            return Ok(out);
        }
        let mut y = pad;
        let mut width = 0;
        for (text, part) in [
            (view.title.clone().unwrap_or_default(), Part::Title),
            (view.preedit.clone(), Part::Preedit),
        ] {
            if text.is_empty() {
                continue;
            }
            let text = fonts.small.text(&text)?;
            let size = text.size;
            out.parts.push((rect(pad + s(4.0), y, size.cx, size.cy), part, text));
            width = width.max(size.cx + s(8.0));
            y += size.cy + s(4.0);
        }
        let line = fonts.main.text("中")?.size.cy;
        let item_pad = s(6.0);
        // 排版结果保留到绘制时使用；emoji 回退字体可能比中文行高，不能按“中”的高度裁切。
        let texts: Vec<(Option<Text>, Text)> = view
            .candidates
            .iter()
            .enumerate()
            .map(|(i, text)| -> Result<_> {
                let label = view.label(i);
                let label = if label.is_empty() { None } else { Some(fonts.small.text(&label)?) };
                Ok((label, fonts.main.text(text)?))
            })
            .collect::<Result<_>>()?;
        let h = texts.iter().fold(line, |h, (label, body)| h.max(body.size.cy).max(label.as_ref().map_or(0, |l| l.size.cy))) + s(6.0);
        let item_width = |(label, body): &(Option<Text>, Text)| {
            let label_w = label.as_ref().map_or(0, |l| l.size.cx + s(4.0));
            item_pad + label_w + body.size.cx + item_pad
        };
        let place = |out: &mut Layout, i: usize, x: i32, y: i32, w: i32| {
            let (label, body) = &texts[i];
            out.items.push(rect(x, y, w, h));
            let label_w = label.as_ref().map_or(0, |l| l.size.cx + s(4.0));
            if let Some(label) = label {
                out.parts.push((rect(x + item_pad, y + (h - label.size.cy) / 2, label.size.cx, label.size.cy), Part::Label(i), label.clone()));
            }
            out.parts.push((
                rect(x + item_pad + label_w, y + (h - body.size.cy) / 2, body.size.cx, body.size.cy),
                Part::Candidate(i), body.clone(),
            ));
        };
        let prev = fonts.main.text("‹")?;
        let next = fonts.main.text("›")?;
        let arrow_w = prev.size.cx.max(next.size.cx) + s(12.0);
        let paging = view.has_prev || view.has_next;
        let horizontal = view.panel == Panel::Normal && !view.vertical;
        if horizontal {
            let mut x = pad;
            for i in 0..texts.len() {
                let w = item_width(&texts[i]);
                place(&mut out, i, x, y, w);
                x += w + s(2.0);
            }
            width = width.max(x - pad);
            if paging {
                let ax = pad + width + s(4.0);
                out.arrows.push((rect(ax, y, arrow_w, h), false, prev.clone()));
                out.arrows.push((rect(ax + arrow_w, y, arrow_w, h), true, next.clone()));
                width += s(4.0) + 2 * arrow_w;
            }
            if !texts.is_empty() {
                y += h;
            }
        } else {
            let columns = if view.panel == Panel::Grid { view.columns.max(1) } else { 1 };
            let cell = texts.iter().map(item_width).max().unwrap_or(0);
            let cell = if view.panel == Panel::Grid { cell.max(h) } else { cell.max(s(120.0)) };
            for i in 0..texts.len() {
                let (row, col) = (i / columns, i % columns);
                place(&mut out, i, pad + col as i32 * (cell + s(2.0)), y + row as i32 * h, cell);
            }
            let rows = texts.len().div_ceil(columns) as i32;
            let cols = columns.min(texts.len().max(1)) as i32;
            width = width.max(cols * (cell + s(2.0)) - s(2.0));
            y += rows * h;
            if paging {
                let ax = pad + width - 2 * arrow_w;
                out.arrows.push((rect(ax, y, arrow_w, h), false, prev));
                out.arrows.push((rect(ax + arrow_w, y, arrow_w, h), true, next));
                width = width.max(2 * arrow_w);
                y += h;
            }
        }
        out.width = width.max(s(40.0)) + 2 * pad;
        out.height = y + pad;
    }
    Ok(out)
}

fn paint(hwnd: HWND, data: &Data) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let (w, h) = (data.layout.width, data.layout.height);
        let hdc = CreateCompatibleDC(Some(target));
        let bitmap = CreateCompatibleBitmap(target, w, h);
        let old_bitmap = SelectObject(hdc, bitmap.into());
        match draw(hdc, data) {
            Ok(()) => { let _ = BitBlt(target, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY); }
            Err(error) => crate::log::write("候选窗绘制失败", &error.to_string()),
        }
        SelectObject(hdc, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(hdc);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// 窗口和离屏回归使用同一条绘制路径，验证实际字体与表情，不用近似预览代替候选窗。
fn draw(hdc: HDC, data: &Data) -> Result<()> {
        let (w, h) = (data.layout.width, data.layout.height);
        let t = theme(&data.view.candidate_style);
        let painter = Painter::new(hdc, &rect(0, 0, w, h), t.background)?;
        painter.border(&rect(0, 0, w, h), t.border);
        let radius = 3.0 * data.scale;
        match data.hovered {
            Some(Hover::Item(i)) if data.view.cursor != Some(i) => {
                if let Some(r) = data.layout.items.get(i) {
                    painter.fill_round(r, t.hover, radius);
                }
            }
            Some(Hover::Arrow(next)) => {
                if let Some((r, _, _)) = data.layout.arrows.iter().find(|(_, n, _)| *n == next) {
                    painter.fill_round(r, t.hover, radius);
                }
            }
            _ => {}
        }
        if let (Some(r), Some(accent)) = (data.view.cursor.and_then(|i| data.layout.items.get(i)), t.accent) {
            painter.fill_round(r, accent, radius);
        }
        let selected = |i: usize| data.view.cursor == Some(i);
        for (r, part, text) in &data.layout.parts {
            let color = match *part {
                Part::Hint => t.emphasis,
                Part::Title | Part::Preedit => t.muted,
                Part::Label(i) => if selected(i) { t.accent_text } else { t.muted },
                Part::Candidate(i) => if selected(i) { t.accent_text } else { t.text },
            };
            painter.text(r, text, color, false);
        }
        for (r, next, text) in &data.layout.arrows {
            let enabled = if *next { data.view.has_next } else { data.view.has_prev };
            painter.text(r, text, if enabled { t.text } else { t.border }, true);
        }
        painter.finish()
}

fn data_of(hwnd: HWND) -> Option<&'static mut Data> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Data;
        ptr.as_mut()
    }
}

fn hit(data: &Data, x: i32, y: i32) -> Option<Hover> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if let Some((_, next, _)) = data.layout.arrows.iter().find(|(r, _, _)| inside(r)) {
        let enabled = if *next { data.view.has_next } else { data.view.has_prev };
        return enabled.then_some(Hover::Arrow(*next));
    }
    data.layout.items.iter().position(inside).map(Hover::Item)
}

fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32)
}

/// 右键、长按弹出的菜单：候选上的「删除记忆」和「打开设置」。
fn open_menu(owner: HWND, item: Option<usize>) {
    let Some(data) = data_of(owner) else {
        return;
    };
    let mut entries: Vec<(String, Click)> = Vec::new();
    if let Some(i) = item.filter(|_| data.view.forgettable) {
        if let Some(word) = data.view.candidates.get(i) {
            entries.push((format!("删除「{word}」的记忆"), Click::Forget(i)));
        }
    }
    entries.push(("打开 FreeType 设置".into(), Click::Settings));
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    menu::show(owner, pt, entries, data.view.font_family.clone(), data.scale);
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            if let Some(data) = data_of(hwnd) {
                paint(hwnd, data);
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_SETCURSOR => {
            let hovering = data_of(hwnd).is_some_and(|d| d.hovered.is_some());
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, if hovering { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(cursor));
                }
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(data) = data_of(hwnd) {
                let hovered = hit(data, x, y);
                if hovered != data.hovered {
                    data.hovered = hovered;
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                        if let Ok(cursor) = LoadCursorW(None, if hovered.is_some() { IDC_HAND } else { IDC_ARROW }) {
                            SetCursor(Some(cursor));
                        }
                    }
                }
            }
            unsafe {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(data) = data_of(hwnd) {
                if data.hovered.take().is_some() {
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            if let Some(data) = data_of(hwnd) {
                data.long_fired = false;
                data.pressed = match hit(data, x, y) {
                    Some(Hover::Item(i)) => Some(i),
                    _ => None,
                };
                unsafe {
                    SetCapture(hwnd);
                    if data.pressed.is_some() && data.view.forgettable {
                        SetTimer(Some(hwnd), LONG_TIMER, LONG_PRESS_MS, None);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            unsafe {
                let _ = KillTimer(Some(hwnd), LONG_TIMER);
                if GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
            }
            let click = data_of(hwnd).and_then(|data| {
                let pressed = data.pressed.take();
                if data.long_fired {
                    return None;
                }
                match hit(data, x, y) {
                    Some(Hover::Item(i)) if pressed == Some(i) => Some(Click::Select(i)),
                    Some(Hover::Arrow(next)) => Some(Click::Page(next)),
                    _ => None,
                }
            });
            if let Some(click) = click {
                dispatch(click);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            let item = data_of(hwnd).and_then(|data| match hit(data, x, y) {
                Some(Hover::Item(i)) => Some(i),
                _ => None,
            });
            open_menu(hwnd, item);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == LONG_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), LONG_TIMER);
                if GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
            }
            let item = data_of(hwnd).and_then(|data| {
                let item = data.pressed?;
                data.long_fired = true;
                Some(item)
            });
            if let Some(item) = item {
                open_menu(hwnd, Some(item));
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == CARET_TIMER => {
            caret_tick(hwnd);
            LRESULT(0)
        }
        WM_CARET_TICK => {
            caret_tick(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == HINT_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), HINT_TIMER);
            }
            if data_of(hwnd).is_some_and(|d| d.view.hint.is_some()) {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => unsafe {
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Data;
            if !ptr.is_null() {
                drop(Box::from_raw(ptr));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn register_classes(instance: HINSTANCE) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        for (name, proc_) in [
            (CLASS_NAME, window_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            (MENU_CLASS, menu::proc_),
        ] {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
                lpfnWndProc: Some(proc_),
                hInstance: instance,
                lpszClassName: name,
                ..Default::default()
            };
            RegisterClassExW(&class);
        }
    });
}

fn round_corners(hwnd: HWND) {
    let corner = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );
    }
}

fn popup(instance: HINSTANCE, class: PCWSTR) -> Option<HWND> {
    register_classes(instance);
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            &HSTRING::from("FreeType"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance),
            None,
        )
        .ok()?;
        round_corners(hwnd);
        Some(hwnd)
    }
}

/// 屏幕上能放窗口的区域（光标所在显示器的工作区）。
fn work_area(x: i32, y: i32) -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork
        } else {
            rect(0, 0, 1 << 15, 1 << 15)
        }
    }
}

pub struct CandidateWindow {
    hwnd: HWND,
}

impl CandidateWindow {
    pub fn new(instance: HINSTANCE) -> Option<Self> {
        let hwnd = popup(instance, CLASS_NAME)?;
        let data = Box::new(Data {
            view: View::default(),
            layout: Layout::default(),
            scale: 1.0,
            hovered: None,
            pressed: None,
            long_fired: false,
        });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as _);
        }
        MENU.with(|m| *m.borrow_mut() = popup(instance, MENU_CLASS));
        Some(Self { hwnd })
    }

    pub fn hide(&self) {
        menu::close();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    /// 回到应用的消息循环以后叫一次 [`set_caret_handler`] 的处理函数；它说还要等，就定时再叫。
    pub fn request_caret_tick(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CARET_TICK, WPARAM(0), LPARAM(0));
        }
    }

    /// 按应答更新；`caret` 是编辑区（或光标）在屏幕上的位置。
    pub fn update(&self, frame: &Frame, caret: Option<RECT>) {
        let view = View {
            title: frame.title.clone(),
            preedit: frame.preedit.clone(),
            candidates: frame.candidates.clone(),
            labels: frame.labels.clone(),
            cursor: frame.cursor,
            has_prev: frame.has_prev,
            has_next: frame.has_next,
            hint: frame.hint.clone(),
            hint_position: frame.hint_position.clone(),
            font_size: frame.font_size,
            font_family: frame.font_family.clone(),
            candidate_style: frame.candidate_style.clone(),
            vertical: frame.vertical,
            panel: frame.panel,
            columns: frame.columns,
            forgettable: frame.forgettable,
        };
        let visible = view.hint.is_some() || !view.candidates.is_empty() || view.title.is_some();
        if !visible {
            self.hide();
            return;
        }
        let Some(caret) = caret else {
            return;
        };
        self.show(view, caret);
    }

    /// 编辑区挪了位置（应用排好版以后）：窗口跟过去。
    pub fn reposition(&self, caret: RECT) {
        if !self.visible() {
            return;
        }
        if let Some(data) = data_of(self.hwnd) {
            let view = data.view.clone();
            self.show(view, caret);
        }
    }

    /// 排版、摆位置、画出来。
    ///
    /// 打字时每个键都会走到这里（浏览器类应用排好版后还会再来一次），所以已经显示着的窗口
    /// 只做必要的改动：不先挪走再挪回来，内容和位置都没变就什么都不做，变了就当场画完。
    /// （以前每次都先缩成 1×1 挪到光标处取 DPI，再放大、等消息循环空了才重画，每打一个字母闪一下。）
    fn show(&self, view: View, caret: RECT) {
        menu::close();
        unsafe {
            // 按光标所在显示器的 DPI 排版：窗口还没显示、或光标到了另一块显示器时，先把窗口挪过去
            let visible = self.visible();
            let at = POINT { x: caret.left, y: caret.bottom };
            if !visible
                || MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST) != MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST)
            {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), at.x, at.y, 1, 1, SWP_NOACTIVATE);
            }
            let dpi = GetDpiForWindow(self.hwnd).max(96);
            let scale = dpi as f32 / 96.0;
            let result = Fonts::new(view.font_size.max(12) as f32 * scale, &view.font_family)
                .and_then(|fonts| layout(&view, &fonts, scale));
            let layout = match result {
                Ok(layout) => layout,
                Err(error) => {
                    crate::log::write("候选窗排版失败", &error.to_string());
                    return;
                }
            };

            let gap = (4.0 * scale) as i32;
            let work = work_area(caret.left, caret.bottom);
            let (w, h) = (layout.width, layout.height);
            let hint = view.hint.is_some();
            let (mut x, mut y) = if hint {
                hint_origin(&view.hint_position, caret, w, h, gap)
            } else {
                (caret.left, caret.bottom + gap)
            };
            if !hint && y + h > work.bottom {
                y = caret.top - gap - h;
            }
            x = x.min(work.right - w).max(work.left);
            y = y.min(work.bottom - h).max(work.top);

            let mut current = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut current);
            let mut changed = false;
            if let Some(data) = data_of(self.hwnd) {
                changed = !visible || current != rect(x, y, w, h) || data.view != view;
                if changed {
                    data.view = view;
                    data.layout = layout;
                    data.scale = scale;
                    data.hovered = None;
                }
            }
            if changed {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                // 整个窗口都自己画（见 paint），不用先擦背景；马上画完，不让变大的部分先露出旧内容
                let _ = InvalidateRect(Some(self.hwnd), None, false);
                let _ = UpdateWindow(self.hwnd);
            }
            if hint {
                SetTimer(Some(self.hwnd), HINT_TIMER, HINT_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), HINT_TIMER);
            }
        }
    }
}

/// 提示窗的左上角：放在光标的哪一边。
fn hint_origin(position: &str, caret: RECT, w: i32, h: i32, gap: i32) -> (i32, i32) {
    let center = caret.left - w / 2;
    match position {
        "top-left" => (caret.left - w - gap, caret.top - h - gap),
        "top" => (center, caret.top - h - gap),
        "top-right" => (caret.right + gap, caret.top - h - gap),
        "bottom-left" => (caret.left - w - gap, caret.bottom + gap),
        "bottom" => (center, caret.bottom + gap),
        _ => (caret.right + gap, caret.bottom + gap),
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        menu::destroy();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// 候选窗的菜单：不抢焦点（不然应用的编辑区会被结束），用鼠标捕获判断点在菜单外面。
mod menu {
    use super::*;

    struct MenuData {
        entries: Vec<(String, Click)>,
        rows: Vec<RECT>,
        hovered: Option<usize>,
        texts: Vec<Text>,
        scale: f32,
    }

    fn data(hwnd: HWND) -> Option<&'static mut MenuData> {
        unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MenuData).as_mut() }
    }

    fn hwnd() -> Option<HWND> {
        MENU.with(|m| *m.borrow())
    }

    pub fn show(_owner: HWND, at: POINT, entries: Vec<(String, Click)>, family: String, scale: f32) {
        let Some(hwnd) = hwnd() else {
            return;
        };
        close();
        unsafe {
            let texts = Fonts::new(14.0 * scale, &family)
                .and_then(|fonts| entries.iter().map(|(t, _)| fonts.main.text(t)).collect::<Result<Vec<_>>>());
            let texts = match texts {
                Ok(texts) => texts,
                Err(error) => {
                    crate::log::write("候选菜单排版失败", &error.to_string());
                    return;
                }
            };
            let pad = (6.0 * scale) as i32;
            let row_h = texts.iter().map(|t| t.size.cy).max().unwrap_or(0) + (12.0 * scale) as i32;
            let width = texts.iter().map(|t| t.size.cx).max().unwrap_or(0) + (32.0 * scale) as i32;
            let rows: Vec<RECT> = (0..entries.len())
                .map(|i| rect(pad, pad + i as i32 * row_h, width, row_h))
                .collect();
            let (w, h) = (width + 2 * pad, entries.len() as i32 * row_h + 2 * pad);
            let old = SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                Box::into_raw(Box::new(MenuData {
                    entries,
                    rows,
                    hovered: None,
                    texts,
                    scale,
                })) as _,
            ) as *mut MenuData;
            if !old.is_null() {
                drop(Box::from_raw(old));
            }
            let work = work_area(at.x, at.y);
            let x = at.x.min(work.right - w).max(work.left);
            let y = if at.y + h > work.bottom { at.y - h } else { at.y };
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = InvalidateRect(Some(hwnd), None, true);
            SetCapture(hwnd);
        }
    }

    pub fn close() {
        let Some(hwnd) = hwnd() else {
            return;
        };
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                return;
            }
            let _ = ShowWindow(hwnd, SW_HIDE);
            if GetCapture() == hwnd {
                let _ = ReleaseCapture();
            }
        }
    }

    pub fn destroy() {
        if let Some(hwnd) = MENU.with(|m| m.borrow_mut().take()) {
            unsafe {
                let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut MenuData;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    fn paint_menu(hwnd: HWND, d: &MenuData) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let target = BeginPaint(hwnd, &mut ps);
            let mut wr = RECT::default();
            let _ = GetWindowRect(hwnd, &mut wr);
            let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
            let hdc = CreateCompatibleDC(Some(target));
            let bitmap = CreateCompatibleBitmap(target, w, h);
            let old_bitmap = SelectObject(hdc, bitmap.into());
            // 菜单只用到背景、边框、文字和停留底色，和强调色无关
            let t = theme("");
            let result = Painter::new(hdc, &rect(0, 0, w, h), t.background).and_then(|painter| {
                painter.border(&rect(0, 0, w, h), t.border);
                for (i, text) in d.texts.iter().enumerate() {
                    let mut r = d.rows[i];
                    if d.hovered == Some(i) {
                        painter.fill_round(&r, t.hover, 2.0 * d.scale);
                    }
                    r.left += (12.0 * d.scale) as i32;
                    painter.text(&r, text, t.text, false);
                }
                painter.finish()
            });
            match result {
                Ok(()) => { let _ = BitBlt(target, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY); }
                Err(error) => crate::log::write("候选菜单绘制失败", &error.to_string()),
            }
            SelectObject(hdc, old_bitmap);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(hdc);
            let _ = EndPaint(hwnd, &ps);
        }
    }

    fn row_at(d: &MenuData, x: i32, y: i32) -> Option<usize> {
        d.rows
            .iter()
            .position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
    }

    pub unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_PAINT => {
                if let Some(d) = data(hwnd) {
                    paint_menu(hwnd, d);
                    return LRESULT(0);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SETCURSOR => {
                unsafe {
                    if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                        SetCursor(Some(cursor));
                    }
                }
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_point(lparam);
                if let Some(d) = data(hwnd) {
                    let row = row_at(d, x, y);
                    if row != d.hovered {
                        d.hovered = row;
                        unsafe {
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                let (x, y) = lparam_point(lparam);
                let click = data(hwnd).and_then(|d| row_at(d, x, y).map(|i| d.entries[i].1));
                close();
                if let Some(click) = click {
                    dispatch(click);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => LRESULT(0),
            WM_CAPTURECHANGED => {
                // 别的窗口拿走了鼠标（切了窗口）：收起
                if lparam.0 != hwnd.0 as isize {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }
}

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
        assert_eq!(hint_origin("bogus", caret, 30, 30, 4), (106, 224), "不认识的写法放右下");
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
mod render_tests;
