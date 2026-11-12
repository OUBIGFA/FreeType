//! 候选窗：每个线程一个不抢焦点的弹出窗口，用 GDI 双缓冲画。
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
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetDC, GetMonitorInfoW, GetStockObject, GetTextExtentPoint32W,
    InvalidateRect, MonitorFromPoint, Rectangle, ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor,
    TextOutW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL,
    FW_SEMIBOLD, HDC, HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST, NULL_BRUSH, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowLongPtrW, GetWindowRect, IsWindowVisible,
    KillTimer, LoadCursorW, RegisterClassExW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    CS_DROPSHADOW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW, IDC_HAND, MA_NOACTIVATE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("FreeTypeCandidateWindow");
const MENU_CLASS: PCWSTR = w!("FreeTypeCandidateMenu");
const HINT_TIMER: usize = 1;
const LONG_TIMER: usize = 2;
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
}

pub fn set_click_handler(handler: Option<Box<dyn Fn(Click)>>) {
    HANDLER.with(|h| *h.borrow_mut() = handler);
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
#[derive(Clone, Default)]
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
            format!("{}", i + 1)
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
    parts: Vec<(RECT, Part)>,
    /// 候选的底（点击区域）。
    items: Vec<RECT>,
    /// 翻页箭头（点击区域）。
    arrows: Vec<(RECT, bool)>,
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

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn font(size: f32, family: &str, weight: i32) -> HFONT {
    let family = if family.trim().is_empty() {
        freetype_protocol::DEFAULT_FONT
    } else {
        family
    };
    unsafe {
        CreateFontW(
            -(size.round() as i32),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            &HSTRING::from(family),
        )
    }
}

fn measure(hdc: HDC, text: &str) -> SIZE {
    let mut size = SIZE::default();
    unsafe {
        let _ = GetTextExtentPoint32W(hdc, &wide(text), &mut size);
    }
    size
}

struct Fonts {
    main: HFONT,
    small: HFONT,
    hint: HFONT,
}

impl Fonts {
    fn new(view: &View, scale: f32) -> Self {
        let size = view.font_size.max(12) as f32 * scale;
        Self {
            main: font(size, &view.font_family, FW_NORMAL.0 as i32),
            small: font(size * 0.78, &view.font_family, FW_NORMAL.0 as i32),
            hint: font(size, &view.font_family, FW_SEMIBOLD.0 as i32),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.main.into());
            let _ = DeleteObject(self.small.into());
            let _ = DeleteObject(self.hint.into());
        }
    }
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

fn layout(hdc: HDC, view: &View, fonts: &Fonts, scale: f32) -> Layout {
    let s = |v: f32| (v * scale).round() as i32;
    let pad = s(8.0);
    let mut out = Layout::default();
    unsafe {
        if let Some(hint) = &view.hint {
            SelectObject(hdc, fonts.hint.into());
            let size = measure(hdc, hint);
            let side = size.cx.max(size.cy);
            out.parts.push((rect(pad + (side - size.cx) / 2, pad, size.cx, size.cy), Part::Hint));
            out.width = side + 2 * pad;
            out.height = size.cy + 2 * pad;
            return out;
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
            SelectObject(hdc, fonts.small.into());
            let size = measure(hdc, &text);
            out.parts.push((rect(pad + s(4.0), y, size.cx, size.cy), part));
            width = width.max(size.cx + s(8.0));
            y += size.cy + s(4.0);
        }
        SelectObject(hdc, fonts.main.into());
        let line = measure(hdc, "中").cy;
        let item_pad = s(6.0);
        let h = line + s(6.0);
        // 每个候选的标号、正文大小
        let sizes: Vec<(SIZE, SIZE)> = view
            .candidates
            .iter()
            .enumerate()
            .map(|(i, text)| {
                SelectObject(hdc, fonts.small.into());
                let label = view.label(i);
                let label = if label.is_empty() { SIZE::default() } else { measure(hdc, &label) };
                SelectObject(hdc, fonts.main.into());
                (label, measure(hdc, text))
            })
            .collect();
        let item_width = |(label, body): &(SIZE, SIZE)| {
            let gap = if label.cx > 0 { s(4.0) } else { 0 };
            item_pad + label.cx + gap + body.cx + item_pad
        };
        let place = |out: &mut Layout, i: usize, x: i32, y: i32, w: i32| {
            let (label, body) = sizes[i];
            out.items.push(rect(x, y, w, h));
            let gap = if label.cx > 0 { s(4.0) } else { 0 };
            out.parts.push((rect(x + item_pad, y + (h - label.cy) / 2, label.cx, label.cy), Part::Label(i)));
            out.parts.push((
                rect(x + item_pad + label.cx + gap, y + (h - body.cy) / 2, body.cx, body.cy),
                Part::Candidate(i),
            ));
        };
        SelectObject(hdc, fonts.main.into());
        let arrow = measure(hdc, "‹");
        let arrow_w = arrow.cx + s(12.0);
        let paging = view.has_prev || view.has_next;
        let horizontal = view.panel == Panel::Normal && !view.vertical;
        if horizontal {
            let mut x = pad;
            for i in 0..sizes.len() {
                let w = item_width(&sizes[i]);
                place(&mut out, i, x, y, w);
                x += w + s(2.0);
            }
            width = width.max(x - pad);
            if paging {
                let ax = pad + width + s(4.0);
                out.arrows.push((rect(ax, y, arrow_w, h), false));
                out.arrows.push((rect(ax + arrow_w, y, arrow_w, h), true));
                width += s(4.0) + 2 * arrow_w;
            }
            if !sizes.is_empty() {
                y += h;
            }
        } else {
            let columns = if view.panel == Panel::Grid { view.columns.max(1) } else { 1 };
            let cell = sizes.iter().map(item_width).max().unwrap_or(0);
            let cell = if view.panel == Panel::Grid { cell.max(h) } else { cell.max(s(120.0)) };
            for i in 0..sizes.len() {
                let (row, col) = (i / columns, i % columns);
                place(&mut out, i, pad + col as i32 * (cell + s(2.0)), y + row as i32 * h, cell);
            }
            let rows = sizes.len().div_ceil(columns) as i32;
            let cols = columns.min(sizes.len().max(1)) as i32;
            width = width.max(cols * (cell + s(2.0)) - s(2.0));
            y += rows * h;
            if paging {
                let ax = pad + width - 2 * arrow_w;
                out.arrows.push((rect(ax, y, arrow_w, h), false));
                out.arrows.push((rect(ax + arrow_w, y, arrow_w, h), true));
                width = width.max(2 * arrow_w);
                y += h;
            }
        }
        out.width = width.max(s(40.0)) + 2 * pad;
        out.height = y + pad;
    }
    out
}

fn fill_round(hdc: HDC, r: &RECT, color: COLORREF, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID, 1, color);
        let ob = SelectObject(hdc, brush.into());
        let op = SelectObject(hdc, pen.into());
        let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
        SelectObject(hdc, ob);
        SelectObject(hdc, op);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
    }
}

fn paint(hwnd: HWND, data: &Data) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let (w, h) = (data.layout.width, data.layout.height);
        let hdc = CreateCompatibleDC(Some(target));
        let bitmap = CreateCompatibleBitmap(target, w, h);
        let old_bitmap = SelectObject(hdc, bitmap.into());
        let t = theme(&data.view.candidate_style);
        let fonts = Fonts::new(&data.view, data.scale);

        let background = CreateSolidBrush(t.background);
        FillRect(hdc, &rect(0, 0, w, h), background);
        let _ = DeleteObject(background.into());
        let border = CreatePen(PS_SOLID, 1, t.border);
        let old_pen = SelectObject(hdc, border.into());
        let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
        let _ = Rectangle(hdc, 0, 0, w, h);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(border.into());

        SetBkMode(hdc, TRANSPARENT);
        let radius = (6.0 * data.scale).round() as i32;
        match data.hovered {
            Some(Hover::Item(i)) if data.view.cursor != Some(i) => {
                if let Some(r) = data.layout.items.get(i) {
                    fill_round(hdc, r, t.hover, radius);
                }
            }
            Some(Hover::Arrow(next)) => {
                if let Some((r, _)) = data.layout.arrows.iter().find(|(_, n)| *n == next) {
                    fill_round(hdc, r, t.hover, radius);
                }
            }
            _ => {}
        }
        if let (Some(r), Some(accent)) = (data.view.cursor.and_then(|i| data.layout.items.get(i)), t.accent) {
            fill_round(hdc, r, accent, radius);
        }
        let selected = |i: usize| data.view.cursor == Some(i);
        let draw = |r: &RECT, text: &str, font: HFONT, color: COLORREF, center: bool| {
            SelectObject(hdc, font.into());
            SetTextColor(hdc, color);
            let size = measure(hdc, text);
            let x = if center { r.left + ((r.right - r.left) - size.cx).max(0) / 2 } else { r.left };
            let y = r.top + ((r.bottom - r.top) - size.cy).max(0) / 2;
            let _ = TextOutW(hdc, x, y, &wide(text));
        };
        for (r, part) in &data.layout.parts {
            match *part {
                Part::Hint => draw(r, &data.view.hint.clone().unwrap_or_default(), fonts.hint, t.emphasis, false),
                Part::Title => draw(r, data.view.title.as_deref().unwrap_or(""), fonts.small, t.muted, false),
                Part::Preedit => draw(r, &data.view.preedit, fonts.small, t.muted, false),
                Part::Label(i) => {
                    let label = data.view.label(i);
                    if !label.is_empty() {
                        draw(r, &label, fonts.small, if selected(i) { t.accent_text } else { t.muted }, false);
                    }
                }
                Part::Candidate(i) => draw(
                    r,
                    &data.view.candidates[i],
                    fonts.main,
                    if selected(i) { t.accent_text } else { t.text },
                    false,
                ),
            }
        }
        for (r, next) in &data.layout.arrows {
            let enabled = if *next { data.view.has_next } else { data.view.has_prev };
            draw(r, if *next { "›" } else { "‹" }, fonts.main, if enabled { t.text } else { t.border }, true);
        }

        let _ = BitBlt(target, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY);
        SelectObject(hdc, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(hdc);
        drop(fonts);
        let _ = EndPaint(hwnd, &ps);
    }
}

fn data_of(hwnd: HWND) -> Option<&'static mut Data> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Data;
        ptr.as_mut()
    }
}

fn hit(data: &Data, x: i32, y: i32) -> Option<Hover> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if let Some((_, next)) = data.layout.arrows.iter().find(|(r, _)| inside(r)) {
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

    fn show(&self, view: View, caret: RECT) {
        menu::close();
        unsafe {
            // 先挪到光标处，按那块显示器的 DPI 排版
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), caret.left, caret.bottom, 1, 1, SWP_NOACTIVATE);
            let dpi = GetDpiForWindow(self.hwnd).max(96);
            let scale = dpi as f32 / 96.0;
            let hdc = GetDC(Some(self.hwnd));
            let fonts = Fonts::new(&view, scale);
            let layout = layout(hdc, &view, &fonts, scale);
            drop(fonts);
            ReleaseDC(Some(self.hwnd), hdc);

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

            if let Some(data) = data_of(self.hwnd) {
                data.view = view;
                data.layout = layout;
                data.scale = scale;
                data.hovered = None;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                let _ = InvalidateRect(Some(self.hwnd), None, true);
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
        family: String,
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
            let hdc = GetDC(Some(hwnd));
            let f = font(14.0 * scale, &family, FW_NORMAL.0 as i32);
            let old = SelectObject(hdc, f.into());
            let pad = (6.0 * scale) as i32;
            let row_h = measure(hdc, "中").cy + (12.0 * scale) as i32;
            let width = entries.iter().map(|(t, _)| measure(hdc, t).cx).max().unwrap_or(0) + (32.0 * scale) as i32;
            SelectObject(hdc, old);
            let _ = DeleteObject(f.into());
            ReleaseDC(Some(hwnd), hdc);
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
                    family,
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
            let bg = CreateSolidBrush(t.background);
            FillRect(hdc, &rect(0, 0, w, h), bg);
            let _ = DeleteObject(bg.into());
            let pen = CreatePen(PS_SOLID, 1, t.border);
            let op = SelectObject(hdc, pen.into());
            let ob = SelectObject(hdc, GetStockObject(NULL_BRUSH));
            let _ = Rectangle(hdc, 0, 0, w, h);
            SelectObject(hdc, ob);
            SelectObject(hdc, op);
            let _ = DeleteObject(pen.into());
            SetBkMode(hdc, TRANSPARENT);
            let f = font(14.0 * d.scale, &d.family, FW_NORMAL.0 as i32);
            let old_font = SelectObject(hdc, f.into());
            for (i, (text, _)) in d.entries.iter().enumerate() {
                let r = d.rows[i];
                if d.hovered == Some(i) {
                    fill_round(hdc, &r, t.hover, (4.0 * d.scale) as i32);
                }
                SetTextColor(hdc, t.text);
                let size = measure(hdc, text);
                let _ = TextOutW(hdc, r.left + (12.0 * d.scale) as i32, r.top + (r.bottom - r.top - size.cy) / 2, &wide(text));
            }
            SelectObject(hdc, old_font);
            let _ = DeleteObject(f.into());
            let _ = BitBlt(target, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY);
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
        let view = View {
            labels: vec!["".into(), "3".into()],
            ..view
        };
        assert_eq!(view.label(0), "");
        assert_eq!(view.label(1), "3");
    }
}
