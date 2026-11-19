//! 在私有桌面操作真实系统语言栏 COM 对象；不注册、安装或切换用户当前输入法。

use super::*;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfLangBarItemButton, ITfThreadMgrEx,
    TF_TMAE_NOACTIVATETIP, TF_TMAE_NOACTIVATEKEYBOARDLAYOUT,
};
use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

const CUSTOM_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAJUlEQVR4nGNkSDnxn4ECwESJ5lEDIICJgULANGoAw2gYMFAeBgANKwJLBd4cmgAAAABJRU5ErkJggg==";
const CUSTOM_PNG_BYTES: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x10\x00\x00\x00\x10\x08\x06\x00\x00\x00\x1f\xf3\xff\x61\x00\x00\x00\x25\x49\x44\x41\x54\x78\x9c\x63\x64\x48\x39\xf1\x9f\x81\x02\xc0\x44\x89\xe6\x51\x03\x20\x80\x89\x81\x42\xc0\x34\x6a\x00\xc3\x68\x18\x30\x50\x1e\x06\x00\x0d\x2b\x02\x4b\x05\xde\x1c\x9a\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) { unsafe { CoUninitialize(); } }
}
struct ActiveManager(ITfThreadMgr);
impl Drop for ActiveManager {
    fn drop(&mut self) { unsafe { let _ = self.0.Deactivate(); } }
}

fn activate_manager(manager: &ITfThreadMgr) -> ActiveManager {
    // 只创建真实语言栏对象；与编辑区回归一样，不在测试线程激活用户已注册的输入法。
    let extended: ITfThreadMgrEx = manager.cast().unwrap();
    let mut client = 0;
    unsafe { extended.ActivateEx(&mut client, TF_TMAE_NOACTIVATETIP | TF_TMAE_NOACTIVATEKEYBOARDLAYOUT).unwrap(); }
    ActiveManager(manager.clone())
}

fn image(appearance: &str) -> BrandIcon {
    BrandIcon { revision: 1, appearance: appearance.into(), custom_png: CUSTOM_PNG.into() }
}

fn pixels(icon: HICON) -> (i32, i32, Vec<u32>) {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).unwrap();
        let mut bitmap = BITMAP::default();
        assert_ne!(GetObjectW(info.hbmColor.into(), std::mem::size_of::<BITMAP>() as i32,
            Some((&mut bitmap as *mut BITMAP).cast())), 0);
        let mut dib = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bitmap.bmWidth, biHeight: -bitmap.bmHeight, biPlanes: 1, biBitCount: 32,
                biCompression: BI_RGB.0, ..Default::default()
            }, ..Default::default()
        };
        let mut data = vec![0u32; (bitmap.bmWidth * bitmap.bmHeight) as usize];
        let dc = CreateCompatibleDC(None);
        let rows = GetDIBits(dc, info.hbmColor, 0, bitmap.bmHeight as u32,
            Some(data.as_mut_ptr().cast()), &mut dib, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DeleteObject(info.hbmMask.into());
        assert_eq!(rows, bitmap.bmHeight);
        (bitmap.bmWidth, bitmap.bmHeight, data)
    }
}

// 核对图像经管道 Base64 传输后仍与原 PNG 的系统缩放结果一致；Windows 会对边缘预乘取整。
fn expected_custom_pixels(size: i32) -> Vec<u32> {
    let icon = OwnedIcon(unsafe { CreateIconFromResourceEx(CUSTOM_PNG_BYTES, true, 0x30000, size, size, IMAGE_FLAGS(0)).unwrap() });
    pixels(icon.0).2
}

#[test]
fn brand_images_generate_native_icons_at_all_language_bar_sizes() {
    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    for size in [16, 20, 24, 32, 40, 48] {
        let default = native_icon(&image("default"), size).unwrap();
        let white = native_icon(&image("white"), size).unwrap();
        let custom = native_icon(&image("custom"), size).unwrap();
        let (width, height, default_pixels) = pixels(default.0);
        assert_eq!((width, height), (size, size));
        let (width, height, white_pixels) = pixels(white.0);
        assert_eq!((width, height), (size, size));
        assert_ne!(default_pixels, white_pixels, "默认和白色图标不能相同");
        let (width, height, custom_pixels) = pixels(custom.0);
        assert_eq!((width, height), (size, size));
        assert_eq!(custom_pixels, expected_custom_pixels(size));
    }
}

#[test]
fn real_system_language_bar_replaces_brand_pixels_and_restores_on_deactivation() {
    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap(); }
    let _apartment = ComApartment;
    for original_mode in [0, 1] {
        let manager: ITfThreadMgr = unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).unwrap() };
        let _active = activate_manager(&manager);
        {
            let items: ITfLangBarItemMgr = manager.cast().unwrap();
            let item = unsafe { items.GetItem(&GUID_TFCAT_TIP_KEYBOARD).unwrap() };
            let mode: ITfSystemDeviceTypeLangBarItem = item.cast().unwrap();
            unsafe { mode.SetIconMode(LANG_BAR_ITEM_ICON_MODE_FLAGS(original_mode)).unwrap(); }
            let button: ITfLangBarItemButton = item.cast().unwrap();
            let read = || {
                let icon = OwnedIcon(unsafe { button.GetIcon().unwrap() });
                (!icon.0.0.is_null()).then(|| pixels(icon.0))
            };
            let original = read();
            {
                let mut updater = BrandUpdater::new(&manager).unwrap();
                updater.set_image(&image("default"), 32).unwrap();
                let default = read().expect("系统必须返回刚设置的默认图像");
                updater.set_image(&image("white"), 32).unwrap();
                let white = read().expect("系统必须返回刚设置的白色图像");
                assert_ne!(default.2, white.2, "系统 COM 对象必须真的换成白色图像");
                updater.set_image(&image("custom"), 32).unwrap();
                let custom = read().expect("系统必须返回刚设置的自定义图像");
                assert_eq!(custom.2, expected_custom_pixels(32));
                let bad = BrandIcon { custom_png: "invalid image".into(), ..image("custom") };
                assert!(updater.set_image(&bad, 32).is_err());
                assert_eq!(read(), Some(custom), "损坏图像必须报告错误并保留之前的图标");
            }
            assert_eq!(read(), original, "停用后必须交还原图，不能影响其他输入法");
            assert_eq!(unsafe { mode.GetIconMode().unwrap() }, original_mode,
                "停用后必须恢复系统原有的默认或注册图标模式");
        }
    }
}

#[test]
fn brand_monitor_stops_its_window_when_the_language_button_detaches() {
    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap(); }
    let _apartment = ComApartment;
    let manager: ITfThreadMgr = unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).unwrap() };
    let _active = activate_manager(&manager);
    let monitor = BrandMonitor::new(&manager).unwrap();
    let hwnd = monitor.hwnd;
    assert!(unsafe { IsWindow(Some(hwnd)) }.as_bool());
    let mut class = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
    assert!(String::from_utf16_lossy(&class[..length]).starts_with(freetype_protocol::TSF_BRAND_MONITOR_CLASS));
    let class = HSTRING::from(String::from_utf16_lossy(&class[..length]));
    let count = || {
        let mut count = 0;
        let mut previous = None;
        while let Ok(window) = unsafe { FindWindowExW(Some(HWND_MESSAGE), previous, &class, windows::core::PCWSTR::null()) } {
            if window.0.is_null() { break; }
            count += 1;
            previous = Some(window);
        }
        count
    };
    assert_eq!(count(), 1);
    let button = crate::langbar::create(Rc::new(|_| {}));
    button.watch_brand(&manager).unwrap();
    assert_eq!(count(), 2);
    button.detach();
    assert_eq!(count(), 1, "按钮停用必须释放它持有的监视窗口");
    drop(monitor);
    assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool(), "停用后不能遗留回调窗口或定时器");
    assert_eq!(count(), 0);
    let mut info = WNDCLASSW::default();
    assert!(unsafe { GetClassInfoW(Some(crate::exports::instance()), &class, &mut info) }.is_err(),
        "最后一个监视窗口关闭后必须释放窗口类及桌面资源");
}
