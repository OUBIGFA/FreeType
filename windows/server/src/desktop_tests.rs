//! 在私有桌面创建真实状态栏，验证品牌入口顺序、命中区域与原生绘制。
use super::*;
use windows::Win32::System::StationsAndDesktops::*;
use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, SendMessageW};
use windows::Win32::Graphics::Gdi::*;

#[test]
fn small_brand_icons_filter_fine_detail_instead_of_aliasing() {
    // 128 px 黑白逐像素棋盘缩小时应成为稳定灰色，不能随机丢掉黑线或白线。
    // 通过生产的自定义图标入口和 DrawIconEx 验证真实输出，不复制缩放实现。
    let state = BarState { brand_icon: "custom".into(), custom_brand_icon: concat!(
        "iVBORw0KGgoAAAANSUhEUgAAAIAAAACACAYAAADDPmHLAAACYElEQVR4nO2WwQkAQRDC7L/pHNfEjmD241ckZCdJ+J+ZzR0qSphc7RDHzzaEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytYMGyDiEFSVMNIAQoAGEgNcQeANk/BuqKGFytcMHGZlpUpVAp/EAAAAASUVORK5CYII="
    ).into(), ..Default::default() };
    unsafe {
        for size in [15, 20, 25, 30, 40] {
            let memory = CreateCompatibleDC(None);
            let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size, biHeight: -size, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(memory, bitmap.into());
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (size*size*4) as usize);
            pixels.fill(0);
            draw_brand(memory, RECT { left: 0, top: 0, right: size, bottom: size }, &state, size);
            GdiFlush().unwrap();
            let failures = pixels.chunks_exact(4).filter(|p| !(110..=145).contains(&p[0]) || p[0] != p[1] || p[1] != p[2]).count();
            let cached = BRAND.with(|cache| cache.borrow().as_ref().unwrap().icon);
            draw_brand(memory, RECT { left: 0, top: 0, right: size, bottom: size }, &state, size);
            assert_eq!(cached, BRAND.with(|cache| cache.borrow().as_ref().unwrap().icon), "普通重画必须复用已缩放图标");
            SelectObject(memory, old); DeleteObject(bitmap.into()).unwrap(); DeleteDC(memory).unwrap();
            BRAND.with(|cache| *cache.borrow_mut() = None);
            assert_eq!(failures, 0, "{size}px 图标应对原图细节做面积采样，不能出现黑白锯齿");
        }
    }
}

#[test]
fn transparent_brand_edges_do_not_acquire_a_dark_halo() {
    unsafe {
        for appearance in ["default","white"] {
            let size=21;
            let memory=CreateCompatibleDC(None);
            let info=BITMAPINFO { bmiHeader:BITMAPINFOHEADER { biSize:std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth:size,biHeight:-size,biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default() },..Default::default() };
            let mut bits=std::ptr::null_mut();
            let bitmap=CreateDIBSection(Some(memory),&info,DIB_RGB_COLORS,&mut bits,None,0).unwrap();
            let old=SelectObject(memory,bitmap.into());
            let pixels=std::slice::from_raw_parts_mut(bits as *mut u8,(size*size*4) as usize);
            pixels.fill(255);
            draw_brand(memory,RECT { left:0,top:0,right:size,bottom:size },&BarState { brand_icon:appearance.into(),..Default::default() },size);
            GdiFlush().unwrap();
            // 两个原图的顶部轮廓红色分量均为 255，覆盖白底后也必须保持 255。
            // 若 alpha 重复相乘，半透明轮廓会变成明显的灰黑边。
            let red:Vec<_>=pixels[..size as usize*4].chunks_exact(4).map(|p|p[2]).collect();
            SelectObject(memory,old);DeleteObject(bitmap.into()).unwrap();DeleteDC(memory).unwrap();
            BRAND.with(|c|*c.borrow_mut()=None);
            assert!(red.iter().all(|r|*r>=253),"{appearance} 半透明边缘不能发黑：{red:?}");
        }
    }
}

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
        for (appearance, scale) in [("default", 75), ("default", 100), ("default", 125), ("default", 150), ("white", 100), ("white", 150)] {
            shared.state.lock().unwrap().scale = scale;
            shared.state.lock().unwrap().brand_icon = appearance.into();
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
                let name = if appearance == "default" { format!("statusbar-{scale}.ppm") } else { format!("statusbar-{appearance}-{scale}.ppm") };
                std::fs::write(PathBuf::from(directory).join(name), ppm).unwrap();
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
