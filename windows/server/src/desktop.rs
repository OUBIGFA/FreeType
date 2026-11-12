//! 服务进程的桌面部分，在单独一个线程里：
//!
//! - **悬浮状态栏**（照水杉输入法的悬浮工具栏）：中 / 英、中文 / 英文标点、简 / 繁三个按钮和设置按钮；
//!   左边的握把能拖动，位置记在 `%APPDATA%\FreeType\statusbar.txt`；右键菜单能打开设置、隐藏状态栏。
//!   有输入框在用 FreeType 时显示。不抢焦点，点按钮不会让应用的输入框失去焦点。
//! - **剪贴板历史**：设置里打开时，系统剪贴板一有文字就交给核心记下（只在内存里）。
//!   密码管理器标了「不进剪贴板历史」的不记。
//!
//! 状态由引擎线程经 [`DesktopHandle`] 推过来；按钮点了经 `events` 发回引擎线程。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use freetype_protocol::Mode;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetMonitorInfoW, GetTextExtentPoint32W, InvalidateRect,
    MonitorFromPoint, RoundRect, SelectObject, SetBkMode, SetTextColor, TextOutW, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, GetCursorPos,
    GetMessageW, GetWindowRect, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetCursor,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage, CS_DROPSHADOW,
    HTCAPTION, HTCLIENT, HWND_MESSAGE, HWND_TOPMOST, IDC_ARROW, IDC_HAND, IDC_SIZEALL, MA_NOACTIVATE, MF_STRING,
    MSG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CLIPBOARDUPDATE, WM_EXITSIZEMOVE, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::host::{BarState, Desktop};

/// 状态栏上点了什么（发回引擎线程）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarEvent {
    ToggleMode,
    TogglePunct,
    ToggleTraditional,
    OpenSettings,
    HideBar,
}

const BAR_CLASS: PCWSTR = w!("FreeTypeStatusBar");
const CLIP_CLASS: PCWSTR = w!("FreeTypeClipboardListener");
/// 引擎线程推了新状态。
const WM_STATE: u32 = WM_APP + 1;
const HIDE_TIMER: usize = 1;
/// 鼠标离开窗口（TrackMouseEvent）。
const WM_MOUSELEAVE: u32 = 0x02a3;
/// 自动隐藏：多久不变就收起。
const HIDE_MS: u32 = 4000;

/// 握把宽、按钮宽高（96 DPI 下）。
const GRIP: f32 = 12.0;
const BUTTON: f32 = 30.0;
const HEIGHT: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Mode,
    Punct,
    Traditional,
    Settings,
}

const PARTS: [Part; 4] = [Part::Mode, Part::Punct, Part::Traditional, Part::Settings];

struct Shared {
    state: Mutex<BarState>,
    hovered: Mutex<Option<Part>>,
    events: Sender<BarEvent>,
    clipboard_on: Arc<AtomicBool>,
    position_file: PathBuf,
}

static SHARED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);
static BAR: AtomicIsize = AtomicIsize::new(0);

fn shared() -> Option<Arc<Shared>> {
    SHARED.lock().ok().and_then(|s| s.clone())
}

fn bar_hwnd() -> Option<HWND> {
    let raw = BAR.load(Ordering::Acquire);
    (raw != 0).then_some(HWND(raw as *mut _))
}

/// 引擎线程拿着的一头。
pub struct DesktopHandle {
    shared: Arc<Shared>,
    settings_exe: PathBuf,
}

impl Desktop for DesktopHandle {
    fn update(&self, state: &BarState) {
        if let Ok(mut s) = self.shared.state.lock() {
            *s = state.clone();
        }
        if let Some(hwnd) = bar_hwnd() {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_STATE, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn open_settings(&self) {
        open_settings(&self.settings_exe);
    }
}

/// 打开设置程序（安装目录里的 FreeType.exe，本程序在它下面的 bin）。
pub fn open_settings(exe: &std::path::Path) {
    if exe.exists() {
        let _ = std::process::Command::new(exe).spawn();
    } else {
        crate::log(&format!("找不到设置程序 {}", exe.display()));
    }
}

pub fn settings_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let bin = exe.parent().map(PathBuf::from).unwrap_or_default();
    bin.parent().unwrap_or(&bin).join("FreeType.exe")
}

/// 起桌面线程。`clipboard_on` 由引擎线程按设置更新。
pub fn start(events: Sender<BarEvent>, clipboard_on: Arc<AtomicBool>) -> DesktopHandle {
    let shared = Arc::new(Shared {
        state: Mutex::new(BarState::default()),
        hovered: Mutex::new(None),
        events,
        clipboard_on,
        position_file: crate::data_dir().join("statusbar.txt"),
    });
    if let Ok(mut slot) = SHARED.lock() {
        *slot = Some(shared.clone());
    }
    std::thread::Builder::new()
        .name("desktop".into())
        .spawn(run)
        .ok();
    DesktopHandle {
        shared,
        settings_exe: settings_exe(),
    }
}

fn run() {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let bar_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(bar_proc),
            hInstance: instance.into(),
            lpszClassName: BAR_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&bar_class);
        let clip_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(clip_proc),
            hInstance: instance.into(),
            lpszClassName: CLIP_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&clip_class);
        if let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            BAR_CLASS,
            &HSTRING::from("FreeType"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        ) {
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const _,
                std::mem::size_of_val(&corner) as u32,
            );
            BAR.store(hwnd.0 as isize, Ordering::Release);
            let _ = PostMessageW(Some(hwnd), WM_STATE, WPARAM(0), LPARAM(0));
        }
        if let Ok(hwnd) = CreateWindowExW(
            Default::default(),
            CLIP_CLASS,
            &HSTRING::from("FreeType clipboard"),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        ) {
            let _ = AddClipboardFormatListener(hwnd);
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// —— 剪贴板 ——

unsafe extern "system" fn clip_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        if shared().is_some_and(|s| s.clipboard_on.load(Ordering::Relaxed)) {
            if let Some(text) = unsafe { read_clipboard(hwnd) } {
                wowtype_core::clipboard::remember(&text);
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 读剪贴板里的文字。密码管理器标了「不进剪贴板历史」「别让监视程序处理」的不读。
unsafe fn read_clipboard(hwnd: HWND) -> Option<String> {
    unsafe {
        let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let no_history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
            return None;
        }
        if IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).is_err() {
            return None;
        }
        // 别的程序正开着剪贴板时稍等再试
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(Some(hwnd)).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened {
            return None;
        }
        let mut text = None;
        let allowed = no_history == 0 || !history_forbidden(no_history);
        if allowed {
            if let Ok(handle) = GetClipboardData(u32::from(CF_UNICODETEXT.0)) {
                let global = HGLOBAL(handle.0);
                let ptr = GlobalLock(global) as *const u16;
                if !ptr.is_null() {
                    let max = GlobalSize(global) / 2;
                    let slice = std::slice::from_raw_parts(ptr, max);
                    let len = slice.iter().position(|&c| c == 0).unwrap_or(max);
                    text = Some(String::from_utf16_lossy(&slice[..len]));
                    let _ = GlobalUnlock(global);
                }
            }
        }
        let _ = CloseClipboard();
        text
    }
}

/// 剪贴板里有 CanIncludeInClipboardHistory 且值为 0。
unsafe fn history_forbidden(format: u32) -> bool {
    unsafe {
        if IsClipboardFormatAvailable(format).is_err() {
            return false;
        }
        let Ok(handle) = GetClipboardData(format) else {
            return false;
        };
        let global = HGLOBAL(handle.0);
        let ptr = GlobalLock(global) as *const u32;
        if ptr.is_null() {
            return false;
        }
        let value = *ptr;
        let _ = GlobalUnlock(global);
        value == 0
    }
}

// —— 状态栏 ——

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

fn scale_of(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd).max(96) as f32 / 96.0 }
}

fn size_of_bar(scale: f32) -> (i32, i32) {
    (
        ((GRIP + BUTTON * PARTS.len() as f32 + 6.0) * scale).round() as i32,
        (HEIGHT * scale).round() as i32,
    )
}

fn part_rect(i: usize, scale: f32) -> RECT {
    let x = ((GRIP + BUTTON * i as f32) * scale).round() as i32;
    let pad = (3.0 * scale).round() as i32;
    RECT {
        left: x,
        top: pad,
        right: x + (BUTTON * scale).round() as i32,
        bottom: (HEIGHT * scale).round() as i32 - pad,
    }
}

fn part_at(hwnd: HWND, x: i32, y: i32) -> Option<Part> {
    let scale = scale_of(hwnd);
    PARTS.iter().enumerate().find_map(|(i, part)| {
        let r = part_rect(i, scale);
        (x >= r.left && x < r.right && y >= r.top && y < r.bottom).then_some(*part)
    })
}

fn label(part: Part, state: &BarState) -> &'static str {
    match part {
        Part::Mode => {
            if state.mode == Mode::Chinese {
                "中"
            } else {
                "英"
            }
        }
        Part::Punct => {
            if state.chinese_punct {
                "，"
            } else {
                ","
            }
        }
        Part::Traditional => {
            if state.traditional {
                "繁"
            } else {
                "简"
            }
        }
        Part::Settings => "⚙",
    }
}

fn tip(part: Part) -> &'static str {
    match part {
        Part::Mode => "中 / 英",
        Part::Punct => "中文 / 英文标点",
        Part::Traditional => "简体 / 繁体",
        Part::Settings => "设置",
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn paint(hwnd: HWND) {
    let Some(shared) = shared() else {
        return;
    };
    let state = shared.state.lock().map(|s| s.clone()).unwrap_or_default();
    let hovered = shared.hovered.lock().ok().and_then(|h| *h);
    let scale = scale_of(hwnd);
    let (w, h) = size_of_bar(scale);
    let light = light_mode();
    let (background, border, text, muted, hover, accent) = if light {
        (rgb(0xfb, 0xfb, 0xfb), rgb(0xd9, 0xd9, 0xd9), rgb(0x1f, 0x1f, 0x1f), rgb(0xa0, 0xa0, 0xa0), rgb(0xea, 0xea, 0xea), rgb(0x2b, 0x6c, 0xe6))
    } else {
        (rgb(0x2b, 0x2b, 0x2b), rgb(0x45, 0x45, 0x45), rgb(0xee, 0xee, 0xee), rgb(0x80, 0x80, 0x80), rgb(0x3a, 0x3a, 0x3a), rgb(0x6c, 0x9e, 0xff))
    };
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let hdc = CreateCompatibleDC(Some(target));
        let bitmap = CreateCompatibleBitmap(target, w, h);
        let old_bitmap = SelectObject(hdc, bitmap.into());
        let brush = CreateSolidBrush(background);
        FillRect(hdc, &RECT { left: 0, top: 0, right: w, bottom: h }, brush);
        let _ = DeleteObject(brush.into());
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_pen = SelectObject(hdc, pen.into());
        let hollow = windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::NULL_BRUSH);
        let old_brush = SelectObject(hdc, hollow);
        let _ = windows::Win32::Graphics::Gdi::Rectangle(hdc, 0, 0, w, h);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen.into());

        // 握把：两列小点
        let dot = CreateSolidBrush(muted);
        let size = (2.0 * scale).round().max(1.0) as i32;
        for row in 0..3 {
            for col in 0..2 {
                let x = (4.0 * scale) as i32 + col * size * 2;
                let y = h / 2 - size * 3 + row * size * 2 + size / 2;
                FillRect(hdc, &RECT { left: x, top: y, right: x + size, bottom: y + size }, dot);
            }
        }
        let _ = DeleteObject(dot.into());

        SetBkMode(hdc, TRANSPARENT);
        let font = CreateFontW(
            -((15.0 * scale).round() as i32),
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
        );
        let old_font = SelectObject(hdc, font.into());
        let radius = (6.0 * scale).round() as i32;
        for (i, part) in PARTS.iter().enumerate() {
            let r = part_rect(i, scale);
            if hovered == Some(*part) {
                let b = CreateSolidBrush(hover);
                let p = CreatePen(PS_SOLID, 1, hover);
                let ob = SelectObject(hdc, b.into());
                let op = SelectObject(hdc, p.into());
                let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
                SelectObject(hdc, ob);
                SelectObject(hdc, op);
                let _ = DeleteObject(b.into());
                let _ = DeleteObject(p.into());
            }
            let label = wide(label(*part, &state));
            let mut size = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &label, &mut size);
            let color = if *part == Part::Mode { accent } else { text };
            SetTextColor(hdc, color);
            let _ = TextOutW(
                hdc,
                r.left + (r.right - r.left - size.cx) / 2,
                r.top + (r.bottom - r.top - size.cy) / 2,
                &label,
            );
        }
        SelectObject(hdc, old_font);
        let _ = DeleteObject(font.into());
        let _ = BitBlt(target, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY);
        SelectObject(hdc, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(hdc);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// 读记下的位置；没有就放在主显示器右下角、任务栏上面。
fn initial_position(hwnd: HWND) -> (i32, i32) {
    let scale = scale_of(hwnd);
    let (w, h) = size_of_bar(scale);
    let saved = shared()
        .and_then(|s| std::fs::read_to_string(&s.position_file).ok())
        .and_then(|text| {
            let (x, y) = text.trim().split_once(',')?;
            Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
        });
    unsafe {
        let point = saved.map(|(x, y)| POINT { x, y }).unwrap_or_default();
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork
        } else {
            RECT { left: 0, top: 0, right: 1280, bottom: 720 }
        };
        let (x, y) = saved.unwrap_or((work.right - w - (24.0 * scale) as i32, work.bottom - h - (16.0 * scale) as i32));
        // 显示器拔掉了、分辨率变了：拉回屏幕里
        (x.clamp(work.left, work.right - w), y.clamp(work.top, work.bottom - h))
    }
}

fn save_position(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
    }
    if let Some(shared) = shared() {
        let _ = std::fs::create_dir_all(shared.position_file.parent().unwrap_or(std::path::Path::new(".")));
        let _ = std::fs::write(&shared.position_file, format!("{},{}\n", rect.left, rect.top));
    }
}

fn apply_state(hwnd: HWND) {
    let Some(shared) = shared() else {
        return;
    };
    let state = shared.state.lock().map(|s| s.clone()).unwrap_or_default();
    unsafe {
        if !state.visible {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return;
        }
        let scale = scale_of(hwnd);
        let (w, h) = size_of_bar(scale);
        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let first = rect.right - rect.left <= 1;
        if first {
            let (x, y) = initial_position(hwnd);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        } else {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), rect.left, rect.top, w, h, SWP_NOACTIVATE);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(Some(hwnd), None, false);
        if state.autohide {
            SetTimer(Some(hwnd), HIDE_TIMER, HIDE_MS, None);
        } else {
            let _ = KillTimer(Some(hwnd), HIDE_TIMER);
        }
    }
}

fn send(event: BarEvent) {
    if let Some(shared) = shared() {
        let _ = shared.events.send(event);
    }
}

fn context_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let _ = AppendMenuW(menu, MF_STRING, 1, w!("打开设置"));
        let _ = AppendMenuW(menu, MF_STRING, 2, w!("隐藏状态栏"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // 菜单要求窗口在前台，否则点别处不会收起
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = DestroyMenu(menu);
        match cmd.0 {
            1 => send(BarEvent::OpenSettings),
            2 => send(BarEvent::HideBar),
            _ => {}
        }
    }
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_STATE => {
            apply_state(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // 握把拖动（系统负责移动），其余是按钮
            let mut rect = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut rect);
            }
            let local = x - rect.left;
            if local < (GRIP * scale_of(hwnd)) as i32 {
                LRESULT(HTCAPTION as isize)
            } else {
                LRESULT(HTCLIENT as isize)
            }
        }
        WM_SETCURSOR => {
            let hit = (lparam.0 & 0xffff) as u32;
            unsafe {
                let hovered = shared().and_then(|s| s.hovered.lock().ok().and_then(|h| *h));
                let cursor = if hit == HTCAPTION {
                    IDC_SIZEALL
                } else if hovered.is_some() {
                    IDC_HAND
                } else {
                    IDC_ARROW
                };
                if let Ok(c) = LoadCursorW(None, cursor) {
                    SetCursor(Some(c));
                }
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let part = part_at(hwnd, x, y);
            if let Some(shared) = shared() {
                let mut hovered = shared.hovered.lock().unwrap_or_else(|e| e.into_inner());
                if *hovered != part {
                    *hovered = part;
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
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
                let _ = KillTimer(Some(hwnd), HIDE_TIMER);
            }
            if let Some(part) = part {
                set_tooltip(hwnd, tip(part));
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(shared) = shared() {
                if let Ok(mut hovered) = shared.hovered.lock() {
                    *hovered = None;
                }
                if shared.state.lock().is_ok_and(|s| s.autohide) {
                    unsafe {
                        SetTimer(Some(hwnd), HIDE_TIMER, HIDE_MS, None);
                    }
                }
            }
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            match part_at(hwnd, x, y) {
                Some(Part::Mode) => send(BarEvent::ToggleMode),
                Some(Part::Punct) => send(BarEvent::TogglePunct),
                Some(Part::Traditional) => send(BarEvent::ToggleTraditional),
                Some(Part::Settings) => send(BarEvent::OpenSettings),
                None => {}
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            context_menu(hwnd);
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            save_position(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == HIDE_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), HIDE_TIMER);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 鼠标停在按钮上时的说明（用窗口标题给读屏软件；不另开提示窗）。
fn set_tooltip(hwnd: HWND, text: &str) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(hwnd, &HSTRING::from(format!("FreeType {text}")));
    }
}
