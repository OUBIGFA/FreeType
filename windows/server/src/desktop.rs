//! 服务进程的桌面部分，在单独一个线程里：
//!
//! - **悬浮状态栏**（照水杉输入法的悬浮工具栏）：中 / 英、中文 / 英文标点、简 / 繁三个按钮和设置按钮；
//!   品牌入口在最前，末端握把能拖动；位置记在 `%APPDATA%\FreeType\statusbar.txt`。
//!   有输入框在用 FreeType 时显示。不抢焦点，点按钮不会让应用的输入框失去焦点。
//! - **剪贴板历史**：设置里打开时，系统剪贴板一有文字就交给核心记下（只在内存里）。
//!   密码管理器标了「不进剪贴板历史」的不记。
//! - **前台程序**：前台窗口换了就把它所属的程序名发给引擎线程（指定软件的预设输入状态用）。
//!   靠系统的前台窗口事件，不轮询。
//!
//! 状态由引擎线程经 [`DesktopHandle`] 推过来；按钮点了、前台换了经 `events` 发回引擎线程。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use freetype_protocol::{AppWindow, Mode};
use windows::core::{w, BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, COLORREF, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
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
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, EnumWindows,
    FindWindowExW, GetClassNameW, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassExW, RegisterWindowMessageW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu,
    TranslateMessage, CS_DROPSHADOW, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, GW_OWNER, HTCAPTION, HTCLIENT,
    HWND_MESSAGE, HWND_TOPMOST, IDC_ARROW, IDC_HAND, IDC_SIZEALL, MA_NOACTIVATE, MF_STRING, MSG, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_CLIPBOARDUPDATE, WM_EXITSIZEMOVE, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::host::{BarState, Desktop};

/// 桌面线程发回引擎线程的事：状态栏上点了什么，前台换到了哪个程序。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarEvent {
    ToggleMode,
    TogglePunct,
    ToggleTraditional,
    EmojiPanel,
    OpenSettings,
    HideBar,
    /// 前台窗口换到了这个程序的窗口（小写文件名，见 [`window_app`]）。
    Foreground(String),
}

const BAR_CLASS: PCWSTR = w!("FreeTypeStatusBar");
const CLIP_CLASS: PCWSTR = w!("FreeTypeClipboardListener");
/// 引擎线程推了新状态。
const WM_STATE: u32 = WM_APP + 1;
const HIDE_TIMER: usize = 1;
/// 鼠标离开窗口（TrackMouseEvent）。
const WM_MOUSELEAVE: u32 = 0x02a3;
/// 握把宽、按钮宽高（96 DPI 下）。
const GRIP: f32 = 14.0;
const BUTTON: f32 = 34.0;
const HEIGHT: f32 = 38.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Mode,
    Punct,
    Traditional,
    Emoji,
    Settings,
}

fn visible_parts(mask: u8) -> impl Iterator<Item = Part> {
    // 组件位掩码保持兼容，品牌入口始终排在第一个可见按钮。
    [(Part::Settings, 8), (Part::Mode, 0), (Part::Punct, 1), (Part::Traditional, 2), (Part::Emoji, 4)]
        .into_iter().filter_map(move |(part, bit)| (bit == 0 || mask & bit != 0).then_some(part))
}

fn bar_state() -> BarState {
    shared().and_then(|shared| shared.state.lock().ok().map(|state| state.clone())).unwrap_or_default()
}

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

    fn brand_icon_changed(&self) {
        notify_brand_icon_changed();
    }

    fn foreground_app(&self) -> Option<String> {
        window_app(unsafe { GetForegroundWindow() })
    }

    fn open_apps(&self) -> Vec<AppWindow> {
        open_apps()
    }
}

/// 保存设置及更新系统资源后共用同一通知；不切换输入法，不改变前台焦点。
pub(crate) fn notify_brand_icon_changed() {
    // message-only 窗口不接收 HWND_BROADCAST，需要逐个投递。
    let message = unsafe { RegisterWindowMessageW(&HSTRING::from(freetype_protocol::BRAND_ICON_CHANGED_MESSAGE)) };
    if message == 0 {
        crate::log("无法注册语言栏图标刷新消息");
        return;
    }
    let mut previous = None;
    while let Ok(hwnd) = unsafe { FindWindowExW(Some(HWND_MESSAGE), previous, PCWSTR::null(), PCWSTR::null()) } {
        if hwnd.0.is_null() { break; }
        previous = Some(hwnd);
        let mut name = [0u16; 128];
        let length = unsafe { GetClassNameW(hwnd, &mut name) } as usize;
        if !String::from_utf16_lossy(&name[..length]).starts_with(freetype_protocol::TSF_BRAND_MONITOR_CLASS) { continue; }
        if let Err(error) = unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) } {
            crate::log(&format!("语言栏图标刷新通知失败：{error}"));
        }
    }
}

/// 打开设置程序（安装目录里的 FreeType.exe，本程序在它下面的 bin）。
pub fn open_emoji_panel() {
    // The bar does not activate itself, so Windows opens its picker over the active app.
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY};
    let sequence = [(0x5b, false), (0xbe, false), (0xbe, true), (0x5b, true)].map(|(vk, up)| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
    });
    unsafe { SendInput(&sequence, std::mem::size_of::<INPUT>() as i32); }
}

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
        // 前台窗口一换就收到一次（由下面的消息循环送达），不用定时器去查。
        // 钩子跟着这个线程，进程退出时系统收回。
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
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

// —— 前台程序 ——

/// 任务栏和通知区域的窗口类。点托盘图标（包括 FreeType 自己的）会先让任务栏到前台；
/// 它们不是用户打字的地方，不算离开了原来的程序。
const SHELL_CLASSES: [&str; 4] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
];

unsafe extern "system" fn foreground_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if let Some(app) = window_app(hwnd) {
        send(BarEvent::Foreground(app));
    }
}

/// 进程的可执行文件完整路径。打不开的进程（系统保护的）返回 None。
fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        result.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// 窗口属于哪个程序：小写文件名（code.exe），和输入法 DLL 报的程序名、规则里的写法一致。
///
/// None：不算换了程序 —— 没有窗口、任务栏和通知区域，或者是 FreeType 自己的窗口
/// （状态栏、设置窗口、托盘菜单；候选窗在各应用自己的进程里，本来就不会被当成别的程序）。
/// 认不出的程序返回空串：算离开了原来的程序，但对不上任何规则。
pub fn window_app(hwnd: HWND) -> Option<String> {
    if hwnd.is_invalid() {
        return None;
    }
    unsafe {
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class).max(0) as usize;
        if SHELL_CLASSES.contains(&String::from_utf16_lossy(&class[..n]).as_str()) {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return Some(String::new());
        }
        if pid == GetCurrentProcessId() {
            return None;
        }
        let Some(path) = process_path(pid) else {
            return Some(String::new());
        };
        let path = path.to_lowercase();
        static SETTINGS: OnceLock<String> = OnceLock::new();
        if &path == SETTINGS.get_or_init(|| settings_exe().to_string_lossy().to_lowercase()) {
            return None;
        }
        Some(path.rsplit(['\\', '/']).next().unwrap_or_default().to_string())
    }
}

/// 现在开着窗口的程序：任务栏上看得到的那些窗口（看得见、有标题、不是工具窗口和别的窗口的附属窗口、
/// 没被系统藏起来），一个程序只列一次，按程序名排序。
pub fn open_apps() -> Vec<AppWindow> {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let apps = &mut *(lparam.0 as *mut Vec<AppWindow>);
            let mut title = [0u16; 256];
            let n = GetWindowTextW(hwnd, &mut title).max(0) as usize;
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
            let listed = IsWindowVisible(hwnd).as_bool()
                && n > 0
                && cloaked == 0
                && GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 == 0
                && GetWindow(hwnd, GW_OWNER).is_err();
            if listed {
                if let Some(app) = window_app(hwnd).filter(|app| !app.is_empty()) {
                    if apps.iter().all(|a| a.app != app) {
                        apps.push(AppWindow { app, title: String::from_utf16_lossy(&title[..n]) });
                    }
                }
            }
            true.into()
        }
    }
    let mut apps: Vec<AppWindow> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut apps as *mut Vec<AppWindow> as isize));
    }
    apps.sort_by(|a, b| a.app.cmp(&b.app));
    apps
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

fn scale_of(hwnd: HWND, state: &BarState) -> f32 {
    (unsafe { GetDpiForWindow(hwnd).max(96) } as f32 / 96.0) * state.scale.clamp(75, 150) as f32 / 100.0
}

fn size_of_bar(scale: f32, parts: u8) -> (i32, i32) {
    (
        ((GRIP + BUTTON * visible_parts(parts).count() as f32 + 8.0) * scale).round() as i32,
        (HEIGHT * scale).round() as i32,
    )
}

fn part_rect(i: usize, scale: f32) -> RECT {
    let x = ((4.0 + BUTTON * i as f32) * scale).round() as i32;
    let pad = (4.0 * scale).round() as i32;
    RECT {
        left: x,
        top: pad,
        right: x + ((BUTTON - 2.0) * scale).round() as i32,
        bottom: (HEIGHT * scale).round() as i32 - pad,
    }
}

fn part_at(hwnd: HWND, x: i32, y: i32) -> Option<Part> {
    let state = bar_state();
    let scale = scale_of(hwnd, &state);
    visible_parts(state.parts).enumerate().find_map(|(i, part)| {
        let r = part_rect(i, scale);
        (x >= r.left && x < r.right && y >= r.top && y < r.bottom).then_some(part)
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
        Part::Emoji => "☺",
        Part::Settings => "",
    }
}

fn tip(part: Part) -> &'static str {
    match part {
        Part::Mode => "中 / 英",
        Part::Punct => "中文 / 英文标点",
        Part::Traditional => "简体 / 繁体",
        Part::Emoji => "表情与符号",
        Part::Settings => "打开 FreeType 设置",
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

struct BarIcon {
    appearance: String,
    custom: Arc<str>,
    size: i32,
    icon: windows::Win32::UI::WindowsAndMessaging::HICON,
}
impl Drop for BarIcon {
    fn drop(&mut self) { unsafe { let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(self.icon); } }
}
thread_local! { static BRAND: std::cell::RefCell<Option<BarIcon>> = const { std::cell::RefCell::new(None) }; }

/// 先按最终物理像素进行面积采样，再生成同尺寸图标，避免系统直接缩小大 PNG 时产生锯齿。
fn scaled_brand_icon(bytes: &[u8], size: i32) -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
    struct Apartment;
    impl Drop for Apartment { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = Apartment;
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)?;
        let frame = decoder.GetFrame(0)?;
        // 先预乘透明度再缩放，透明边缘不会混入原图中不可见的颜色。
        let source = factory.CreateFormatConverter()?;
        source.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&source, size as u32, size as u32, WICBitmapInterpolationModeFant)?;
        // 缩放用预乘像素，交给 HICON 前转回独立透明度；否则原生绘制会再次
        // 乘 alpha，让半透明的粉色或白色轮廓变成灰黑边。
        let output = factory.CreateFormatConverter()?;
        output.Initialize(&scaler, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size, biHeight: -size, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let result = (|| {
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
            output.CopyPixels(std::ptr::null(), (size * 4) as u32, pixels)?;
            let mask_bits = vec![0u8; ((size as usize + 15) / 16) * 2 * size as usize];
            let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));
            if mask.0.is_null() { return Err(windows::core::Error::from_thread()); }
            let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: bitmap, ..Default::default() });
            let _ = DeleteObject(mask.into());
            icon
        })();
        let _ = DeleteObject(bitmap.into());
        result
    }
}

// 图像解码只在桌面线程的品牌或 DPI 变化时执行，不进入按键线程。
fn draw_brand(hdc: windows::Win32::Graphics::Gdi::HDC, area: RECT, state: &BarState, size: i32) {
    use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL};
    BRAND.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|c| c.appearance != state.brand_icon || c.custom != state.custom_brand_icon || c.size != size) {
            let custom;
            let bytes: &[u8] = match state.brand_icon.as_str() {
                "white" => include_bytes!("../../assets/freetype-white-logo.png"),
                "custom" => {
                    use windows::Win32::Security::Cryptography::{CryptStringToBinaryW, CRYPT_STRING_BASE64};
                    let encoded = wide(&state.custom_brand_icon);
                    let mut length = 0;
                    let decoded = (|| -> windows::core::Result<Vec<u8>> { unsafe {
                        CryptStringToBinaryW(&encoded, CRYPT_STRING_BASE64, None, &mut length, None, None)?;
                        let mut bytes = vec![0; length as usize];
                        CryptStringToBinaryW(&encoded, CRYPT_STRING_BASE64, Some(bytes.as_mut_ptr()), &mut length, None, None)?;
                        bytes.truncate(length as usize);
                        Ok(bytes)
                    } })();
                    match decoded { Ok(bytes) => { custom = bytes; &custom }, Err(error) => {
                        crate::log(&format!("状态栏品牌图标读取失败：{error}")); return;
                    } }
                }
                _ => include_bytes!("../../assets/freetype-logo.png"),
            };
            match scaled_brand_icon(bytes, size) {
                Ok(icon) => *cache = Some(BarIcon { appearance: state.brand_icon.clone(), custom: state.custom_brand_icon.clone(), size, icon }),
                Err(error) => { crate::log(&format!("状态栏品牌图标创建失败：{error}")); return; }
            }
        }
        if let Some(icon) = cache.as_ref() { unsafe {
            let _ = DrawIconEx(hdc, (area.left + area.right - size) / 2, (area.top + area.bottom - size) / 2,
                icon.icon, size, size, 0, None, DI_NORMAL);
        } }
    });
}

fn paint(hwnd: HWND, print_dc: Option<windows::Win32::Graphics::Gdi::HDC>) {
    let Some(shared) = shared() else {
        return;
    };
    let state = shared.state.lock().map(|s| s.clone()).unwrap_or_default();
    let hovered = shared.hovered.lock().ok().and_then(|h| *h);
    let scale = scale_of(hwnd, &state);
    let (w, h) = size_of_bar(scale, state.parts);
    let light = light_mode();
    let (background, border, text, muted, hover, accent) = if light {
        (rgb(0xfa, 0xf8, 0xf9), rgb(0xd5, 0xcc, 0xd0), rgb(0x35, 0x2e, 0x32), rgb(0x87, 0x7b, 0x82), rgb(0xee, 0xe3, 0xe8), rgb(0xa4, 0x45, 0x70))
    } else {
        (rgb(0x28, 0x25, 0x29), rgb(0x53, 0x49, 0x50), rgb(0xed, 0xe7, 0xeb), rgb(0xa6, 0x98, 0xa1), rgb(0x42, 0x35, 0x3e), rgb(0xe4, 0xa1, 0xbf))
    };
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = print_dc.unwrap_or_else(|| BeginPaint(hwnd, &mut ps));
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

        // 握把放末端，品牌成为最前面的入口；拖动命中区独立于按钮。
        let dot = CreateSolidBrush(muted);
        let size = (2.0 * scale).round().max(1.0) as i32;
        for row in 0..3 {
            for col in 0..2 {
                let x = w - (GRIP * scale) as i32 + col * size * 2;
                let y = h / 2 - size * 3 + row * size * 2 + size / 2;
                FillRect(hdc, &RECT { left: x, top: y, right: x + size, bottom: y + size }, dot);
            }
        }
        let _ = DeleteObject(dot.into());

        SetBkMode(hdc, TRANSPARENT);
        let font = CreateFontW(
            -((state.icon_size.clamp(12, 23) as f32 * scale).round() as i32),
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
        for (i, part) in visible_parts(state.parts).enumerate() {
            let r = part_rect(i, scale);
            if hovered == Some(part) {
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
            if part == Part::Settings {
                draw_brand(hdc, r, &state, ((state.icon_size.clamp(12, 23) + 5) as f32 * scale).round() as i32);
                continue;
            }
            let label = wide(label(part, &state));
            let mut size = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &label, &mut size);
            let color = if part == Part::Mode { accent } else { text };
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
        if print_dc.is_none() { let _ = EndPaint(hwnd, &ps); }
    }
}

/// 读记下的位置；没有就放在主显示器右下角、任务栏上面。
fn initial_position(hwnd: HWND, state: &BarState) -> (i32, i32) {
    let scale = scale_of(hwnd, state);
    let (w, h) = size_of_bar(scale, state.parts);
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
        let scale = scale_of(hwnd, &state);
        let (w, h) = size_of_bar(scale, state.parts);
        let _ = windows::Win32::UI::WindowsAndMessaging::SetClassLongW(hwnd,
            windows::Win32::UI::WindowsAndMessaging::GCL_STYLE, if state.shadow { CS_DROPSHADOW.0 as i32 } else { 0 });
        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let first = rect.right - rect.left <= 1;
        if first {
            let (x, y) = initial_position(hwnd, &state);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        } else {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), rect.left, rect.top, w, h, SWP_NOACTIVATE);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(Some(hwnd), None, false);
        if state.autohide {
            SetTimer(Some(hwnd), HIDE_TIMER, state.hide_seconds.clamp(1, 60) * 1000, None);
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
            paint(hwnd, None);
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_PRINTCLIENT => {
            paint(hwnd, Some(windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _)));
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // 握把拖动（系统负责移动），其余是按钮
            let mut rect = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut rect);
            }
            let local = x - rect.left;
            if local >= rect.right - rect.left - (GRIP * scale_of(hwnd, &bar_state())) as i32 {
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
                if let Ok(state) = shared.state.lock() {
                    if state.autohide {
                        unsafe { SetTimer(Some(hwnd), HIDE_TIMER, state.hide_seconds.clamp(1, 60) * 1000, None); }
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
                Some(Part::Emoji) => send(BarEvent::EmojiPanel),
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

#[cfg(test)]
#[path = "desktop_tests.rs"]
mod tests;
