//! TSF 生命周期外壳：由稳定的 COM 对象持有可替换实现，不依赖宿主进程卸载旧 DLL。
//! 监视器只在所属输入线程的消息循环里交接；跨 DLL 仅传 COM 接口和 Windows ABI。

use crate::{service::TextService, DllRef};
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use windows::core::{implement, s, w, Interface, Ref, Result, BOOL, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    E_FAIL, E_POINTER, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GetProcAddress, LoadLibraryW,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
};
use windows::Win32::UI::TextServices::{
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl, ITfThreadMgr,
};
use windows::Win32::UI::WindowsAndMessaging::*;

type CreateService = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CanReload = unsafe extern "system" fn(*mut c_void) -> BOOL;
const CLASS: PCWSTR = w!("FreeTypeComponentMonitor");
const TIMER: usize = 1;

fn component_path(root: &Path, revision: &str) -> Option<PathBuf> {
    (revision.len() == 64 && revision.bytes().all(|b| b.is_ascii_hexdigit())).then(|| {
        root.join("versions")
            .join(revision)
            .join("freetype_tsf.dll")
    })
}

fn module_path(instance: HINSTANCE) -> PathBuf {
    let mut path = [0u16; 32768];
    let n = unsafe { GetModuleFileNameW(Some(instance.into()), &mut path) } as usize;
    PathBuf::from(String::from_utf16_lossy(&path[..n]))
}

fn installation_root(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(path);
    match parent
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "versions"))
    {
        Some(versions) => versions.parent().unwrap_or(parent).to_path_buf(),
        None => parent.to_path_buf(),
    }
}

struct Loaded {
    service: ITfTextInputProcessorEx,
    can_reload: CanReload,
    path: PathBuf,
}

impl Loaded {
    fn local(instance: HINSTANCE) -> Self {
        Self {
            service: TextService::new(instance).into(),
            can_reload: can_reload_service,
            path: module_path(instance),
        }
    }

    fn load(path: &Path) -> Result<Self> {
        // 窗口类和 Windows 回调会引用 DLL 代码；每个版本只映射一次并保留到宿主退出。
        // COM 对象仍正常释放，不能对第三方进程调用 FreeLibrary 强行回收代码。
        static MODULES: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, usize>>> =
            std::sync::Mutex::new(None);
        let module = {
            let mut map = MODULES
                .lock()
                .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
            let map = map.get_or_insert_with(Default::default);
            if let Some(handle) = map.get(path) {
                HMODULE(*handle as *mut _)
            } else {
                let module = unsafe { LoadLibraryW(&HSTRING::from(path.as_os_str()))? };
                map.insert(path.to_path_buf(), module.0 as usize);
                module
            }
        };
        unsafe {
            let create: CreateService = std::mem::transmute(
                GetProcAddress(module, s!("FreeTypeCreateServiceV1"))
                    .ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?,
            );
            let can_reload: CanReload = std::mem::transmute(
                GetProcAddress(module, s!("FreeTypeCanReloadServiceV1"))
                    .ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?,
            );
            let mut raw = std::ptr::null_mut();
            create(&mut raw).ok()?;
            if raw.is_null() {
                return Err(E_POINTER.into());
            }
            Ok(Self {
                service: ITfTextInputProcessorEx::from_raw(raw),
                can_reload,
                path: path.to_path_buf(),
            })
        }
    }
}

struct State {
    current: Option<Loaded>,
    active: Option<(ITfThreadMgr, u32, u32)>,
    root: PathBuf,
    window: Option<HWND>,
    /// 上次已处理完的 current.txt 的修改时间和大小；没变就不再打开文件。
    seen: Option<(u64, u32)>,
}

/// 只读目录项里的修改时间和大小，不打开文件（每个输入线程每秒一次，打开文件会触发杀毒扫描）。
fn file_stamp(path: &Path) -> Option<(u64, u32)> {
    use windows::Win32::Storage::FileSystem::{
        GetFileAttributesExW, GetFileExInfoStandard, WIN32_FILE_ATTRIBUTE_DATA,
    };
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe {
        GetFileAttributesExW(
            &HSTRING::from(path.as_os_str()),
            GetFileExInfoStandard,
            &mut data as *mut _ as *mut c_void,
        )
        .ok()?;
    }
    let time = u64::from(data.ftLastWriteTime.dwHighDateTime) << 32
        | u64::from(data.ftLastWriteTime.dwLowDateTime);
    Some((time, data.nFileSizeLow))
}

#[implement(ITfTextInputProcessorEx)]
pub struct ReloadableService {
    state: Rc<RefCell<State>>,
    instance: HINSTANCE,
    _dll: DllRef,
}

impl ReloadableService {
    pub fn new(instance: HINSTANCE) -> Self {
        let current = Loaded::local(instance);
        Self {
            state: Rc::new(RefCell::new(State {
                root: installation_root(&current.path),
                current: Some(current),
                active: None,
                window: None,
                seen: None,
            })),
            instance,
            _dll: DllRef::new(),
        }
    }
}

impl Drop for ReloadableService {
    fn drop(&mut self) {
        let mut s = self.state.borrow_mut();
        unsafe {
            if let Some(hwnd) = s.window.take() {
                let _ = DestroyWindow(hwnd);
            }
            if s.active.take().is_some() {
                if let Some(current) = &s.current {
                    if let Err(error) = current.service.Deactivate() {
                        crate::log::write("输入法组件停用失败", &error.to_string());
                    }
                }
            }
        }
    }
}

fn refresh(state: &Rc<RefCell<State>>) -> Result<()> {
    let marker = state.borrow().root.join("current.txt");
    let stamp = file_stamp(&marker);
    if stamp.is_some() && stamp == state.borrow().seen {
        return Ok(());
    }
    let (path, activation) = {
        let mut s = state.borrow_mut();
        let Some(current) = &s.current else {
            return Ok(());
        };
        let Ok(revision) = std::fs::read_to_string(&marker) else {
            return Ok(());
        };
        let Some(path) = component_path(&s.root, revision.trim()) else {
            return Err(E_FAIL.into());
        };
        if path == current.path {
            s.seen = stamp;
            return Ok(());
        }
        // 正在组字等不能交接时不记下时间，下一秒再试。
        if !unsafe { (current.can_reload)(current.service.as_raw()) }.as_bool() {
            return Ok(());
        }
        (path, s.active.clone())
    };
    // 先验证新组件可加载，再触碰旧组件；坏文件不能让当前输入法失效。
    let next = Loaded::load(&path)?;
    let current = state
        .borrow_mut()
        .current
        .take()
        .ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
    if let Some((manager, client, flags)) = activation {
        unsafe {
            if let Err(error) = current.service.Deactivate() {
                state.borrow_mut().current = Some(current);
                return Err(error);
            }
            if let Err(error) = next.service.ActivateEx(&manager, client, flags) {
                let _ = next.service.Deactivate();
                let restored = current.service.ActivateEx(&manager, client, flags);
                state.borrow_mut().current = Some(current);
                restored?;
                return Err(error);
            }
        }
    }
    {
        let mut s = state.borrow_mut();
        s.current = Some(next);
        s.seen = stamp;
    }
    crate::log::write("输入法组件已更新", &path.display().to_string());
    Ok(())
}

unsafe extern "system" fn monitor(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_TIMER && wparam.0 == TIMER {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Weak<RefCell<State>>;
            if let Some(state) = ptr.as_ref().and_then(Weak::upgrade) {
                if let Err(error) = refresh(&state) {
                    crate::log::write("输入法组件更新失败", &error.to_string());
                }
            }
            return LRESULT(0);
        }
        if message == WM_NCDESTROY {
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Weak<RefCell<State>>;
            if !ptr.is_null() {
                drop(Box::from_raw(ptr));
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

impl ITfTextInputProcessor_Impl for ReloadableService_Impl {
    fn Activate(&self, manager: Ref<ITfThreadMgr>, client: u32) -> Result<()> {
        self.ActivateEx(manager, client, 0)
    }
    fn Deactivate(&self) -> Result<()> {
        let (window, service) = {
            let mut s = self.state.borrow_mut();
            s.active = None;
            (
                s.window.take(),
                s.current.as_ref().map(|c| c.service.clone()),
            )
        };
        unsafe {
            if let Some(hwnd) = window {
                DestroyWindow(hwnd)?;
            }
            if let Some(service) = service {
                service.Deactivate()?;
            }
        }
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for ReloadableService_Impl {
    fn ActivateEx(&self, manager: Ref<ITfThreadMgr>, client: u32, flags: u32) -> Result<()> {
        self.Deactivate()?;
        if let Err(error) = refresh(&self.state) {
            crate::log::write("输入法组件更新失败", &error.to_string());
        }
        let manager = manager.ok()?.clone();
        let service = self
            .state
            .borrow()
            .current
            .as_ref()
            .unwrap()
            .service
            .clone();
        // 已注册的窗口类保留函数地址。固定外壳也需保留映射，不能让 COM 空闲卸载它。
        let mut pinned = HMODULE::default();
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                PCWSTR(monitor as *const u16),
                &mut pinned,
            )?;
        }
        unsafe {
            service.ActivateEx(&manager, client, flags)?;
        }
        self.state.borrow_mut().active = Some((manager, client, flags));
        let result = unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(monitor),
                hInstance: self.instance,
                lpszClassName: CLASS,
                ..Default::default()
            });
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(self.instance),
                None,
            )
        };
        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                self.Deactivate()?;
                return Err(error);
            }
        };
        self.state.borrow_mut().window = Some(hwnd);
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                // windows-rs 在 x86 将此 API 映射为 i32，x64 则为 isize。
                Box::into_raw(Box::new(Rc::downgrade(&self.state))) as _,
            );
            if SetTimer(Some(hwnd), TIMER, 1000, None) == 0 {
                self.Deactivate()?;
                return Err(windows::core::Error::from_thread());
            }
        }
        Ok(())
    }
}

/// V1 ABI 固定为 ITfTextInputProcessorEx 的所有权指针；不跨 DLL 暴露 Rust 结构体。
#[no_mangle]
unsafe extern "system" fn FreeTypeCreateServiceV1(output: *mut *mut c_void) -> HRESULT {
    if output.is_null() {
        return E_POINTER;
    }
    let service: ITfTextInputProcessorEx = TextService::new(crate::exports::instance()).into();
    unsafe {
        *output = service.into_raw();
    }
    HRESULT(0)
}

unsafe extern "system" fn can_reload_service(raw: *mut c_void) -> BOOL {
    if raw.is_null() {
        return false.into();
    }
    let service = std::mem::ManuallyDrop::new(unsafe { ITfTextInputProcessorEx::from_raw(raw) });
    service
        .cast_object_ref::<TextService>()
        .is_ok_and(|object| object.can_reload())
        .into()
}

#[no_mangle]
unsafe extern "system" fn FreeTypeCanReloadServiceV1(raw: *mut c_void) -> BOOL {
    unsafe { can_reload_service(raw) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_component_stays_inside_installation() {
        let root = std::path::Path::new(r"C:\Program Files\FreeType");
        let revision = "a".repeat(64);
        assert_eq!(
            component_path(root, &revision),
            Some(
                root.join("versions")
                    .join(&revision)
                    .join("freetype_tsf.dll")
            )
        );
        for invalid in [
            "",
            "../elsewhere",
            "C:\\other.dll",
            "short",
            &"g".repeat(64),
        ] {
            assert_eq!(component_path(root, invalid), None);
        }
    }
}
