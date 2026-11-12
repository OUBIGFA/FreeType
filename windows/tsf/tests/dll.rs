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

fn dll_path() -> std::path::PathBuf {
    // target\<profile>\deps\dll-xxxx.exe -> target\<profile>\freetype_tsf.dll
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().join("freetype_tsf.dll")
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
        // 同一个对象支持输入法需要的全部接口
        service.cast::<ITfTextInputProcessor>().unwrap();
        service.cast::<ITfKeyEventSink>().unwrap();
        service.cast::<ITfCompositionSink>().unwrap();
        assert_ne!(can_unload(), S_OK, "对象还活着时不能卸载");
        drop(service);
        drop(factory);
        assert_eq!(can_unload(), S_OK, "对象都释放后可以卸载");
        let _ = FreeLibrary(module);
    }
}
