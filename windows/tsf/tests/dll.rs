//! 不注册，直接载入编出来的 freetype_tsf.dll：类工厂能建出文本服务，用完可以卸载。
#![cfg(windows)]

use std::ffi::c_void;

use windows::core::{Interface, GUID, HRESULT, HSTRING};
use windows::Win32::Foundation::{FreeLibrary, CLASS_E_CLASSNOTAVAILABLE, S_OK};
use windows::Win32::System::Com::{CoInitializeEx, IClassFactory, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::TextServices::{ITfCompositionSink, ITfKeyEventSink, ITfTextInputProcessor, ITfTextInputProcessorEx};

type GetClassObject = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type CanUnload = unsafe extern "system" fn() -> HRESULT;

#[path = "support/thread_manager.rs"]
mod thread_manager;

fn dll_path() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().join("freetype_tsf.dll")
}

#[test]
fn class_factory_creates_text_service() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let module = LoadLibraryW(&HSTRING::from(dll_path().as_os_str())).expect("载入 DLL");
        let get: GetClassObject = std::mem::transmute(GetProcAddress(module, windows::core::s!("DllGetClassObject")).unwrap());
        let can_unload: CanUnload = std::mem::transmute(GetProcAddress(module, windows::core::s!("DllCanUnloadNow")).unwrap());
        for name in ["DllRegisterServer", "DllUnregisterServer"] {
            let name = std::ffi::CString::new(name).unwrap();
            assert!(GetProcAddress(module, windows::core::PCSTR(name.as_ptr() as *const u8)).is_some());
        }

        let clsid = GUID::try_from(freetype_protocol::TEXT_SERVICE_CLSID).unwrap();
        let mut object = std::ptr::null_mut();
        assert_eq!(get(&GUID::zeroed(), &IClassFactory::IID, &mut object), CLASS_E_CLASSNOTAVAILABLE);
        assert_eq!(get(&clsid, &IClassFactory::IID, &mut object), S_OK);
        let factory = IClassFactory::from_raw(object);
        let service: ITfTextInputProcessorEx = factory.CreateInstance(None).expect("建文本服务");
        // TSF 外壳管理生命周期，实际实现自行订阅按键和组字接口。
        service.cast::<ITfTextInputProcessor>().unwrap();
        let create: unsafe extern "system" fn(*mut *mut c_void) -> HRESULT = std::mem::transmute(
            GetProcAddress(module, windows::core::s!("FreeTypeCreateServiceV1")).unwrap());
        let mut implementation = std::ptr::null_mut();
        create(&mut implementation).ok().unwrap();
        let implementation = ITfTextInputProcessorEx::from_raw(implementation);
        implementation.cast::<ITfKeyEventSink>().unwrap();
        implementation.cast::<ITfCompositionSink>().unwrap();
        drop(implementation);
        assert_ne!(can_unload(), S_OK, "对象还活着时不能卸载");
        drop(service);
        drop(factory);
        assert_eq!(can_unload(), S_OK, "对象都释放后可以卸载");
        let _ = FreeLibrary(module);
    }
}

#[test]
fn active_service_hands_over_to_published_dll_without_restarting_host() {
    // COM 和输入法窗口均只存在于私有桌面的专用线程，不注册或激活用户的输入法。
    std::thread::spawn(|| unsafe {
        use windows::Win32::System::StationsAndDesktops::*;
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::TextServices::ITfThreadMgr;
        use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, DispatchMessageW, MSG, PM_REMOVE};
        let tid = GetCurrentThreadId();
        let previous = GetThreadDesktop(tid).unwrap();
        let desktop = CreateDesktopW(&HSTRING::from(format!("FreeType-reload-{}-{tid}", std::process::id())),
            windows::core::PCWSTR::null(), None, DESKTOP_CONTROL_FLAGS(0),
            DESKTOP_CREATEWINDOW.0 | DESKTOP_READOBJECTS.0 | DESKTOP_WRITEOBJECTS.0, None).unwrap();
        SetThreadDesktop(desktop).unwrap();
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        let root = std::path::PathBuf::from(std::env::var_os("FREETYPE_TEST_TEMP").unwrap_or_else(||
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_temp/reload-tests").into_os_string()))
            .join(format!("reload-{}-{tid}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut modules = Vec::new();
        for generation in ["a", "b", "c"] {
            let directory = root.join("versions").join(generation.repeat(64));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("freetype_tsf.dll");
            std::fs::copy(dll_path(), &path).unwrap();
            modules.push(LoadLibraryW(&HSTRING::from(path.as_os_str())).unwrap());
        }
        let unloaded = |index: usize| {
            let check: CanUnload = std::mem::transmute(GetProcAddress(modules[index], windows::core::s!("DllCanUnloadNow")).unwrap());
            check() == S_OK
        };
        let get: GetClassObject = std::mem::transmute(GetProcAddress(modules[0], windows::core::s!("DllGetClassObject")).unwrap());
        let mut raw = std::ptr::null_mut();
        get(&GUID::try_from(freetype_protocol::TEXT_SERVICE_CLSID).unwrap(), &IClassFactory::IID, &mut raw).ok().unwrap();
        let factory = IClassFactory::from_raw(raw);
        let service: ITfTextInputProcessorEx = factory.CreateInstance(None).unwrap();
        let reject_next = std::rc::Rc::new(std::cell::Cell::new(false));
        let manager: ITfThreadMgr = thread_manager::Manager::new(reject_next.clone()).into();
        let client = manager.Activate().unwrap();
        service.ActivateEx(&manager, client, 0).unwrap();
        let pump = |ready: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
            while !ready() && std::time::Instant::now() < deadline {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() { DispatchMessageW(&message); }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(ready(), "运行中的输入法必须交接到已发布的新 DLL");
        };
        std::fs::write(root.join("current.txt"), "b".repeat(64)).unwrap();
        pump(&|| !unloaded(1));
        reject_next.set(true);
        std::fs::write(root.join("current.txt"), "c".repeat(64)).unwrap();
        pump(&|| !reject_next.get());
        assert!(!unloaded(1) && unloaded(2), "新版激活失败时恢复旧版订阅");
        pump(&|| unloaded(1) && !unloaded(2));
        // 残缺的新版本不能停用仍可工作的版本。
        std::fs::write(root.join("current.txt"), "d".repeat(64)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1200);
        pump(&|| std::time::Instant::now() >= deadline);
        assert!(!unloaded(2));
        service.Deactivate().unwrap();
        std::fs::write(root.join("current.txt"), "b".repeat(64)).unwrap();
        service.ActivateEx(&manager, client, 0).unwrap();
        assert!(!unloaded(1) && unloaded(2), "重新启用沿用最新发布组件，不重复注册订阅");
        service.Deactivate().unwrap();
        drop(service);
        drop(factory);
        assert!(unloaded(1) && unloaded(2), "停用并释放外壳后必须解除新版的订阅与引用");
        manager.Deactivate().unwrap();
        drop(manager);
        windows::Win32::System::Com::CoUninitialize();
        SetThreadDesktop(previous).unwrap();
        CloseDesktop(desktop).unwrap();
        // 已注册窗口类的 DLL 保留映射到进程退出；测试也不提前卸载代码。
    }).join().unwrap();
}
