//! FreeType 的 TSF 文本服务（输入法 DLL）。
//!
//! DLL 只负责「按键进、文字出」：输入规则、词库、学习都在服务进程（FreeTypeServer.exe）里，
//! 每个应用进程只多一条管道连接和一个候选窗。

pub mod keys;

#[cfg(windows)]
mod client;
#[cfg(windows)]
mod langbar;
#[cfg(windows)]
mod log;
#[cfg(windows)]
mod register;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
mod exports {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows::core::{implement, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT};
    use windows::Win32::Foundation::{
        CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, HINSTANCE, S_FALSE, S_OK,
    };
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
    use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows::Win32::UI::TextServices::ITfTextInputProcessorEx;

    use crate::service::TextService;

    static INSTANCE: AtomicIsize = AtomicIsize::new(0);
    /// 活着的 COM 对象和 LockServer 次数；为 0 时 DLL 可以卸载。
    static REFS: AtomicIsize = AtomicIsize::new(0);

    pub(crate) fn instance() -> HINSTANCE {
        HINSTANCE(INSTANCE.load(Ordering::Relaxed) as *mut c_void)
    }

    /// 放进每个 COM 对象：活着时 DLL 不卸载。
    pub(crate) struct DllRef;

    impl DllRef {
        pub(crate) fn new() -> Self {
            REFS.fetch_add(1, Ordering::SeqCst);
            Self
        }
    }

    impl Drop for DllRef {
        fn drop(&mut self) {
            REFS.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[implement(IClassFactory)]
    struct ClassFactory {
        _dll: DllRef,
    }

    impl IClassFactory_Impl for ClassFactory_Impl {
        fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
            if object.is_null() || iid.is_null() {
                return Err(E_POINTER.into());
            }
            unsafe { *object = std::ptr::null_mut() };
            if outer.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let service: ITfTextInputProcessorEx = TextService::new(instance()).into();
            unsafe { service.query(iid, object).ok() }
        }

        fn LockServer(&self, lock: BOOL) -> Result<()> {
            if lock.as_bool() {
                REFS.fetch_add(1, Ordering::SeqCst);
            } else {
                REFS.fetch_sub(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    #[no_mangle]
    extern "system" fn DllMain(instance: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            INSTANCE.store(instance.0 as isize, Ordering::Relaxed);
            unsafe {
                let _ = DisableThreadLibraryCalls(instance.into());
            }
        }
        true.into()
    }

    #[no_mangle]
    unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, object: *mut *mut c_void) -> HRESULT {
        if clsid.is_null() || iid.is_null() || object.is_null() {
            return E_POINTER;
        }
        unsafe {
            *object = std::ptr::null_mut();
            if *clsid != crate::register::clsid() {
                return CLASS_E_CLASSNOTAVAILABLE;
            }
            let factory: IClassFactory = ClassFactory { _dll: DllRef::new() }.into();
            factory.query(iid, object)
        }
    }

    #[no_mangle]
    extern "system" fn DllCanUnloadNow() -> HRESULT {
        if REFS.load(Ordering::SeqCst) == 0 {
            S_OK
        } else {
            S_FALSE
        }
    }

    #[no_mangle]
    extern "system" fn DllRegisterServer() -> HRESULT {
        match crate::register::register(instance()) {
            Ok(()) => S_OK,
            Err(e) => {
                let _ = crate::register::unregister();
                e.code()
            }
        }
    }

    #[no_mangle]
    extern "system" fn DllUnregisterServer() -> HRESULT {
        match crate::register::unregister() {
            Ok(()) => S_OK,
            Err(e) => e.code(),
        }
    }
}

#[cfg(windows)]
pub(crate) use exports::DllRef;
