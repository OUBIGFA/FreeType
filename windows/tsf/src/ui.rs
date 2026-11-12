//! 候选窗：每个线程一个不抢焦点的弹出窗口，用 GDI 双缓冲画。
//!
//! 横排：上面一行是编辑区拼音（小字），下面是「1 候选 2 候选 …」和翻页箭头；竖排每个候选一行。
//! 切换中英文时只显示一个「中」或「英」，一会儿自动收起。颜色跟随系统的浅色 / 深色模式。

use std::cell::RefCell;

use freetype_protocol::Frame;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetDC, GetMonitorInfoW, GetTextExtentPoint32W,
    MonitorFromPoint, ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor, TextOutW, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, HDC, HFONT,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, KillTimer, RegisterClassExW,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_DROPSHADOW, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_NCDESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("FreeTypeCandidateWindow");
const HINT_TIMER: usize = 1;
const HINT_MS: u32 = 900;

/// 候选窗上的点击。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Select(usize),
    Page(bool),
}

thread_local! {
    /// 点击交给文本服务处理（每个线程一个文本服务）。
    static HANDLER: RefCell<Option<Box<dyn Fn(Click)>>> = const { RefCell::new(None) };
}

pub fn set_click_handler(handler: Option<Box<dyn Fn(Click)>>) {
    HANDLER.with(|h| *h.borrow_mut() = handler);
}

#[derive(Clone, Copy)]
struct Theme {
    background: COLORREF,
    border: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    accent: COLORREF,
    accent_text: COLORREF,
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

fn theme() -> Theme {
    if light_mode() {
        Theme {
            background: rgb(0xff, 0xff, 0xff),
            border: rgb(0xd6, 0xd6, 0xd6),
            text: rgb(0x1f, 0x1f, 0x1f),
            muted: rgb(0x80, 0x80, 0x80),
            accent: rgb(0x2b, 0x6c, 0xe6),
            accent_text: rgb(0xff, 0xff, 0xff),
        }
    } else {
        Theme {
            background: rgb(0x2b, 0x2b, 0x2b),
            border: rgb(0x48, 0x48, 0x48),
            text: rgb(0xee, 0xee, 0xee),
            muted: rgb(0x9a, 0x9a, 0x9a),
            accent: rgb(0x3d, 0x7f, 0xf0),
            accent_text: rgb(0xff, 0xff, 0xff),
        }
    }
}

/// 要画的东西。
#[derive(Clone, Default)]
struct View {
    preedit: String,
    candidates: Vec<String>,
    cursor: Option<usize>,
    has_prev: bool,
    has_next: bool,
    hint: Option<String>,
    font_size: u32,
    vertical: bool,
}

#[derive(Clone, Copy)]
enum Part {
    Preedit,
    Label(usize),
    Candidate(usize),
    Arrow(bool),
    Hint,
}

/// 排好的版：每一块的位置。
#[derive(Default)]
struct Layout {
    width: i32,
    height: i32,
    /// (画的位置, 是什么, 点击区域)
    parts: Vec<(RECT, Part)>,
    /// 候选的高亮底（点击区域）。
    items: Vec<RECT>,
}

struct Data {
    view: View,
    layout: Layout,
    scale: f32,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn font(size: f32) -> HFONT {
    unsafe {
        CreateFontW(
            -(size.round() as i32),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Microsoft YaHei UI"),
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
}

impl Fonts {
    fn new(view: &View, scale: f32) -> Self {
        let size = view.font_size.max(12) as f32 * scale;
        Self {
            main: font(size),
            small: font(size * 0.78),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.main.into());
            let _ = DeleteObject(self.small.into());
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
            SelectObject(hdc, fonts.main.into());
            let size = measure(hdc, hint);
            let side = size.cx.max(size.cy);
            out.parts.push((rect(pad + (side - size.cx) / 2, pad, size.cx, size.cy), Part::Hint));
            out.width = side + 2 * pad;
            out.height = size.cy + 2 * pad;
            return out;
        }
        let mut y = pad;
        let mut width = 0;
        if !view.preedit.is_empty() {
            SelectObject(hdc, fonts.small.into());
            let size = measure(hdc, &view.preedit);
            out.parts.push((rect(pad + s(4.0), y, size.cx, size.cy), Part::Preedit));
            width = width.max(size.cx + s(8.0));
            y += size.cy + s(4.0);
        }
        SelectObject(hdc, fonts.main.into());
        let line = measure(hdc, "中").cy;
        let item_pad = s(6.0);
        let mut x = pad;
        for (i, text) in view.candidates.iter().enumerate() {
            SelectObject(hdc, fonts.small.into());
            let label = measure(hdc, &format!("{}", i + 1));
            SelectObject(hdc, fonts.main.into());
            let body = measure(hdc, text);
            let w = item_pad + label.cx + s(4.0) + body.cx + item_pad;
            let h = line + s(6.0);
            let (ix, iy) = if view.vertical { (pad, y) } else { (x, y) };
            out.items.push(rect(ix, iy, w, h));
            out.parts.push((
                rect(ix + item_pad, iy + (h - label.cy) / 2, label.cx, label.cy),
                Part::Label(i),
            ));
            out.parts.push((
                rect(ix + item_pad + label.cx + s(4.0), iy + (h - body.cy) / 2, body.cx, body.cy),
                Part::Candidate(i),
            ));
            if view.vertical {
                width = width.max(w);
                y += h;
            } else {
                x += w + s(2.0);
            }
        }
        if !view.vertical {
            width = width.max(x - pad);
        }
        if view.has_prev || view.has_next {
            SelectObject(hdc, fonts.main.into());
            let arrow = measure(hdc, "‹");
            let w = arrow.cx + s(10.0);
            let h = line + s(6.0);
            let (ax, ay) = if view.vertical { (pad, y) } else { (pad + width + s(4.0), y) };
            out.parts.push((rect(ax, ay, w, h), Part::Arrow(false)));
            out.parts.push((rect(ax + w, ay, w, h), Part::Arrow(true)));
            if view.vertical {
                width = width.max(2 * w);
                y += h;
            } else {
                width += s(4.0) + 2 * w;
            }
        }
        if !view.vertical && !view.candidates.is_empty() {
            y += line + s(6.0);
        }
        out.width = width.max(s(40.0)) + 2 * pad;
        out.height = y + pad;
    }
    out
}

fn paint(hwnd: HWND, data: &Data) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let (w, h) = (data.layout.width, data.layout.height);
        let hdc = CreateCompatibleDC(Some(target));
        let bitmap = CreateCompatibleBitmap(target, w, h);
        let old_bitmap = SelectObject(hdc, bitmap.into());
        let t = theme();
        let fonts = Fonts::new(&data.view, data.scale);

        let background = CreateSolidBrush(t.background);
        FillRect(hdc, &rect(0, 0, w, h), background);
        let _ = DeleteObject(background.into());
        let border = CreatePen(PS_SOLID, 1, t.border);
        let old_pen = SelectObject(hdc, border.into());
        let hollow = windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::NULL_BRUSH);
        let old_brush = SelectObject(hdc, hollow);
        let _ = windows::Win32::Graphics::Gdi::Rectangle(hdc, 0, 0, w, h);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(border.into());

        SetBkMode(hdc, TRANSPARENT);
        let radius = (6.0 * data.scale).round() as i32;
        if let Some(i) = data.view.cursor {
            if let Some(r) = data.layout.items.get(i) {
                let brush = CreateSolidBrush(t.accent);
                let pen = CreatePen(PS_SOLID, 1, t.accent);
                let ob = SelectObject(hdc, brush.into());
                let op = SelectObject(hdc, pen.into());
                let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
                SelectObject(hdc, ob);
                SelectObject(hdc, op);
                let _ = DeleteObject(brush.into());
                let _ = DeleteObject(pen.into());
            }
        }
        for (r, part) in &data.layout.parts {
            let selected = |i: usize| data.view.cursor == Some(i);
            let (text, font, color): (String, HFONT, COLORREF) = match *part {
                Part::Hint => (data.view.hint.clone().unwrap_or_default(), fonts.main, t.text),
                Part::Preedit => (data.view.preedit.clone(), fonts.small, t.muted),
                Part::Label(i) => (
                    format!("{}", i + 1),
                    fonts.small,
                    if selected(i) { t.accent_text } else { t.muted },
                ),
                Part::Candidate(i) => (
                    data.view.candidates[i].clone(),
                    fonts.main,
                    if selected(i) { t.accent_text } else { t.text },
                ),
                Part::Arrow(next) => {
                    let enabled = if next { data.view.has_next } else { data.view.has_prev };
                    let arrow = if next { "›" } else { "‹" };
                    (arrow.to_string(), fonts.main, if enabled { t.text } else { t.border })
                }
            };
            SelectObject(hdc, font.into());
            SetTextColor(hdc, color);
            let size = measure(hdc, &text);
            let x = r.left + ((r.right - r.left) - size.cx).max(0) / 2 * i32::from(matches!(part, Part::Arrow(_)));
            let y = r.top + ((r.bottom - r.top) - size.cy).max(0) / 2;
            let _ = TextOutW(hdc, x, y, &wide(&text));
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

fn hit(data: &Data, x: i32, y: i32) -> Option<Click> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    for (r, part) in &data.layout.parts {
        if let Part::Arrow(next) = part {
            if inside(r) {
                return Some(Click::Page(*next));
            }
        }
    }
    data.layout.items.iter().position(inside).map(Click::Select)
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
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            if let Some(click) = data_of(hwnd).and_then(|data| hit(data, x, y)) {
                HANDLER.with(|h| {
                    if let Some(handler) = h.borrow().as_ref() {
                        handler(click);
                    }
                });
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
        WM_NCDESTROY => {
            unsafe {
                let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Data;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn register_class(instance: HINSTANCE) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&class);
    });
}

pub struct CandidateWindow {
    hwnd: HWND,
}

impl CandidateWindow {
    pub fn new(instance: HINSTANCE) -> Option<Self> {
        register_class(instance);
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                CLASS_NAME,
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
            let data = Box::new(Data {
                view: View::default(),
                layout: Layout::default(),
                scale: 1.0,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as _);
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const _,
                std::mem::size_of_val(&corner) as u32,
            );
            Some(Self { hwnd })
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// 按应答更新；`caret` 是编辑区（或光标）在屏幕上的位置。
    pub fn update(&self, frame: &Frame, caret: Option<RECT>) {
        let view = View {
            preedit: frame.preedit.clone(),
            candidates: frame.candidates.clone(),
            cursor: frame.cursor,
            has_prev: frame.has_prev,
            has_next: frame.has_next,
            hint: frame.hint.clone(),
            font_size: frame.font_size,
            vertical: frame.vertical,
        };
        let visible = view.hint.is_some() || !view.candidates.is_empty() || !view.preedit.is_empty();
        let Some(caret) = caret.filter(|_| visible) else {
            if !visible {
                self.hide();
            }
            return;
        };
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
            let monitor = MonitorFromPoint(
                POINT {
                    x: caret.left,
                    y: caret.bottom,
                },
                MONITOR_DEFAULTTONEAREST,
            );
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                info.rcWork
            } else {
                rect(0, 0, 1 << 15, 1 << 15)
            };
            let mut x = caret.left;
            let mut y = caret.bottom + gap;
            if y + layout.height > work.bottom {
                y = caret.top - gap - layout.height;
            }
            x = x.min(work.right - layout.width).max(work.left);
            y = y.max(work.top);

            let hint = view.hint.is_some();
            if let Some(data) = data_of(self.hwnd) {
                data.view = view;
                data.layout = layout;
                data.scale = scale;
                let (w, h) = (data.layout.width, data.layout.height);
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    w,
                    h,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.hwnd), None, true);
            }
            if hint {
                SetTimer(Some(self.hwnd), HINT_TIMER, HINT_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), HINT_TIMER);
            }
        }
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
