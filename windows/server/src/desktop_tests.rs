//! 在私有桌面创建真实状态栏，验证品牌入口顺序、命中区域与原生绘制。
use super::*;
use windows::Win32::System::StationsAndDesktops::*;
use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, SendMessageW};
use windows::Win32::Graphics::Gdi::*;

#[test]
fn brand_leads_status_buttons_and_drag_grip_stays_separate() {
    unsafe {
        let previous = GetThreadDesktop(windows::Win32::System::Threading::GetCurrentThreadId()).unwrap();
        let name = HSTRING::from(format!("FreeType-status-test-{}", std::process::id()));
        let desktop = CreateDesktopW(&name, PCWSTR::null(), None, DESKTOP_CONTROL_FLAGS(0),
            DESKTOP_CREATEWINDOW.0 | DESKTOP_READOBJECTS.0 | DESKTOP_WRITEOBJECTS.0, None).unwrap();
        SetThreadDesktop(desktop).unwrap();
        let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap();
        RegisterClassExW(&WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(bar_proc), hInstance: instance.into(), lpszClassName: BAR_CLASS, ..Default::default() });
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared { state: Mutex::new(BarState { visible: true, scale: 100, icon_size: 16, parts: 15,
            chinese_punct: true, brand_icon: "default".into(), ..Default::default() }), hovered: Mutex::new(None),
            events: tx, clipboard_on: Arc::new(AtomicBool::new(false)), position_file: PathBuf::new() });
        *SHARED.lock().unwrap() = Some(shared.clone());
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, BAR_CLASS, w!("FreeType test"), WS_POPUP,
            100, 100, 1, 1, None, None, Some(instance.into()), None).unwrap();
        for scale in [75, 100, 150] {
            shared.state.lock().unwrap().scale = scale;
            apply_state(hwnd);
            let state = shared.state.lock().unwrap().clone();
            let factor = scale_of(hwnd, &state);
            assert_eq!(visible_parts(15).collect::<Vec<_>>(), vec![Part::Settings, Part::Mode, Part::Punct, Part::Traditional, Part::Emoji]);
            for (i, event) in [BarEvent::OpenSettings, BarEvent::ToggleMode, BarEvent::TogglePunct, BarEvent::ToggleTraditional, BarEvent::EmojiPanel].into_iter().enumerate() {
                let r = part_rect(i, factor);
                let point = LPARAM((((r.top + r.bottom)/2) << 16 | ((r.left + r.right)/2)) as isize);
                SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(point));
                assert_eq!(rx.try_recv().unwrap(), event);
            }
            let (width, height) = size_of_bar(factor, state.parts);
            assert!(part_at(hwnd, width - 5, height/2).is_none());
            let mut r = RECT::default(); GetWindowRect(hwnd, &mut r).unwrap();
            let point = LPARAM(((r.top + height/2) << 16 | (r.right - 5)) as isize);
            assert_eq!(SendMessageW(hwnd, WM_NCHITTEST, Some(WPARAM(0)), Some(point)).0, HTCAPTION as isize);
            InvalidateRect(Some(hwnd), None, false).unwrap(); UpdateWindow(hwnd).unwrap();
            if let Some(directory) = std::env::var_os("FREETYPE_RENDER_DIR") {
                std::fs::create_dir_all(&directory).unwrap();
                let dc = GetDC(Some(hwnd)); let memory = CreateCompatibleDC(Some(dc));
                let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
                let mut bits = std::ptr::null_mut();
                let bitmap = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
                let old = SelectObject(memory, bitmap.into());
                SendMessageW(hwnd, windows::Win32::UI::WindowsAndMessaging::WM_PRINTCLIENT,
                    Some(WPARAM(memory.0 as usize)), Some(LPARAM(0)));
                GdiFlush().unwrap();
                let pixels = std::slice::from_raw_parts(bits as *const u8, (width*height*4) as usize);
                let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
                for p in pixels.chunks_exact(4) { ppm.extend_from_slice(&[p[2], p[1], p[0]]); }
                std::fs::write(PathBuf::from(directory).join(format!("statusbar-{scale}.ppm")), ppm).unwrap();
                SelectObject(memory, old); DeleteObject(bitmap.into()).unwrap(); DeleteDC(memory).unwrap(); ReleaseDC(Some(hwnd), dc);
            }
        }
        DestroyWindow(hwnd).unwrap();
        BRAND.with(|c| *c.borrow_mut() = None);
        *SHARED.lock().unwrap() = None;
        windows::Win32::UI::WindowsAndMessaging::UnregisterClassW(BAR_CLASS, Some(instance.into())).unwrap();
        SetThreadDesktop(previous).unwrap(); CloseDesktop(desktop).unwrap();
    }
}
