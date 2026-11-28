//! 只替换系统语言栏的品牌图像，不改变中 / 英按钮、输入法注册或当前组字。
//! 图像经本用户的只读管道取得；支持低完整性、商店应用和管理员输入宿主。

use freetype_protocol::{BrandIcon, Request};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::{
    ERROR_CLASS_HAS_WINDOWS, E_FAIL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Security::Cryptography::{CryptStringToBinaryW, CRYPT_STRING_BASE64};
use windows::Win32::UI::TextServices::{
    ITfLangBarItemMgr, ITfSystemDeviceTypeLangBarItem, ITfSystemLangBarItem, ITfThreadMgr,
    GUID_TFCAT_TIP_KEYBOARD, LANG_BAR_ITEM_ICON_MODE_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::*;

struct OwnedIcon(HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

fn native_icon(brand: &BrandIcon, size: i32) -> Result<OwnedIcon> {
    let custom;
    let bytes: &[u8] = match brand.appearance.as_str() {
        "default" => include_bytes!("../../assets/freetype-logo.png"),
        "white" => include_bytes!("../../assets/freetype-white-logo.png"),
        "custom" => {
            let wide: Vec<u16> = brand.custom_png.encode_utf16().collect();
            let mut length = 0;
            unsafe {
                CryptStringToBinaryW(&wide, CRYPT_STRING_BASE64, None, &mut length, None, None)?;
            }
            custom = {
                let mut data = vec![0u8; length as usize];
                unsafe {
                    CryptStringToBinaryW(
                        &wide,
                        CRYPT_STRING_BASE64,
                        Some(data.as_mut_ptr()),
                        &mut length,
                        None,
                        None,
                    )?;
                }
                data.truncate(length as usize);
                data
            };
            &custom
        }
        _ => return Err(E_FAIL.into()),
    };
    // Vista 及后续 Windows 可直接从 PNG 图像资源生成带透明度的 HICON，并按系统 DPI 缩放。
    unsafe {
        CreateIconFromResourceEx(bytes, true, 0x0003_0000, size, size, IMAGE_FLAGS(0))
            .map(OwnedIcon)
    }
}

pub(crate) struct BrandUpdater {
    system: ITfSystemLangBarItem,
    mode: ITfSystemDeviceTypeLangBarItem,
    original_mode: u32,
    native: Option<OwnedIcon>,
    client: crate::client::Client,
    revision: Option<u64>,
    image: Option<BrandIcon>,
    size: i32,
    pub(crate) had_focus: bool,
}

impl BrandUpdater {
    pub(crate) fn new(manager: &ITfThreadMgr) -> Result<Self> {
        let items: ITfLangBarItemMgr = manager.cast()?;
        let item = unsafe { items.GetItem(&GUID_TFCAT_TIP_KEYBOARD)? };
        let system = item.cast()?;
        let mode: ITfSystemDeviceTypeLangBarItem = item.cast()?;
        let original_mode = unsafe { mode.GetIconMode()? };
        Ok(Self {
            system,
            mode,
            original_mode,
            native: None,
            client: crate::client::Client::for_brand_icon(),
            revision: None,
            image: None,
            size: 0,
            had_focus: false,
        })
    }

    /// `force` 时整幅图重取并重设；`reapply` 时按已知修订号查询（未变化时应答不带图像），
    /// 再用缓存的图像重设系统图标。
    pub(crate) fn refresh(&mut self, force: bool, reapply: bool) -> Result<()> {
        let response = self
            .client
            .call(&Request::GetBrandIcon {
                known_revision: if force { None } else { self.revision },
            })
            .ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        if force || reapply || response.brand_icon.is_some() || self.size != size {
            let image = response
                .brand_icon
                .as_ref()
                .or(self.image.as_ref())
                .cloned();
            if let Some(image) = image {
                self.set_image(&image, size)?;
                self.size = size;
            }
        }
        if let Some(image) = response.brand_icon {
            self.revision = Some(image.revision);
            self.image = Some(image);
        }
        Ok(())
    }

    fn set_image(&mut self, image: &BrandIcon, size: i32) -> Result<()> {
        let icon = native_icon(image, size)?;
        // SetIcon 保留传入句柄：在下一幅图像设置成功或清除覆盖之前必须让它保持有效。
        // 系统菜单发出 TF_LBI_ICON 通知；GetIcon 的调用方取得独立副本。
        unsafe {
            self.system.SetIcon(icon.0)?;
        }
        self.native = Some(icon);
        // USEPROFILEICON 会优先显示注册资源；改为运行时图像模式才能真正显示新图标。
        // 停用时恢复 original_mode，保留其他输入法的显示约定。
        unsafe {
            self.mode.SetIconMode(LANG_BAR_ITEM_ICON_MODE_FLAGS(0))?;
        }
        Ok(())
    }
}

impl Drop for BrandUpdater {
    fn drop(&mut self) {
        // 清除运行时覆盖并恢复系统原有的默认 / 配置图标模式，再释放当前 HICON。
        if let Err(error) = unsafe { self.system.SetIcon(HICON::default()) } {
            crate::log::write("恢复语言栏图标失败", &error.to_string());
        }
        if let Err(error) = unsafe {
            self.mode
                .SetIconMode(LANG_BAR_ITEM_ICON_MODE_FLAGS(self.original_mode))
        } {
            crate::log::write("恢复语言栏图标模式失败", &error.to_string());
        }
    }
}

struct MonitorState {
    manager: ITfThreadMgr,
    updater: Option<BrandUpdater>,
}

// 监视器由当前语言栏按钮持有，随实际输入组件激活、停用；旧外壳交接到新 DLL 也会接入。
pub(crate) struct BrandMonitor {
    hwnd: HWND,
    class: HSTRING,
    instance: HINSTANCE,
    state: Rc<RefCell<MonitorState>>,
}

fn changed_message() -> u32 {
    static MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe {
        RegisterWindowMessageW(&HSTRING::from(
            freetype_protocol::BRAND_ICON_CHANGED_MESSAGE,
        ))
    })
}

fn refresh_monitor(state: &Rc<RefCell<MonitorState>>, force: bool) -> Result<()> {
    let (manager, mut updater) = {
        let mut state = state.borrow_mut();
        (state.manager.clone(), state.updater.take())
    };
    let focused = unsafe { manager.IsThreadFocus() }.is_ok_and(|focus| focus.as_bool());
    if !force && !focused {
        if let Some(updater) = &mut updater {
            updater.had_focus = false;
        }
        state.borrow_mut().updater = updater;
        return Ok(());
    }
    // 焦点一直在本线程时不再每秒经管道询问（每个输入线程一次往返，最长等 200 ms，
    // 发生在应用的界面线程上）。图标变化由服务广播通知；漏掉的在下次得到焦点时补上。
    if !force && updater.as_ref().is_some_and(|updater| updater.had_focus) {
        state.borrow_mut().updater = updater;
        return Ok(());
    }
    let mut updater = match updater {
        Some(updater) => updater,
        None => BrandUpdater::new(&manager)?,
    };
    // 得到焦点时系统可能换回了注册的图标，要重设；图像本身只在修订号变化时重取。
    let result = updater.refresh(force, focused && !updater.had_focus);
    // 失败（服务未启动、超时）时下一秒再试，成功后才停止轮询。
    updater.had_focus = focused && result.is_ok();
    state.borrow_mut().updater = Some(updater);
    result
}

unsafe extern "system" fn monitor(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message != 0 && (message == changed_message() || message == WM_TIMER && wparam.0 == 1) {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Weak<RefCell<MonitorState>>;
            if let Some(state) = ptr.as_ref().and_then(Weak::upgrade) {
                if let Err(error) = refresh_monitor(&state, message == changed_message()) {
                    crate::log::write("语言栏图标更新失败", &error.to_string());
                }
            }
            return LRESULT(0);
        }
        if message == WM_NCDESTROY {
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Weak<RefCell<MonitorState>>;
            if !ptr.is_null() {
                drop(Box::from_raw(ptr));
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

impl BrandMonitor {
    pub(crate) fn new(manager: &ITfThreadMgr) -> Result<Self> {
        let state = Rc::new(RefCell::new(MonitorState {
            manager: manager.clone(),
            updater: None,
        }));
        // 不同 DLL 的窗口类不能共用指向 Rust 状态的回调；类名以本版本函数地址区分。
        let class = HSTRING::from(format!(
            "{}-{:x}",
            freetype_protocol::TSF_BRAND_MONITOR_CLASS,
            monitor as *const () as usize
        ));
        let instance = crate::exports::instance();
        let hwnd = unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(monitor),
                hInstance: instance,
                lpszClassName: windows::core::PCWSTR(class.as_ptr()),
                ..Default::default()
            });
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                &class,
                &HSTRING::new(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )?
        };
        let result = Self {
            hwnd,
            class,
            instance,
            state,
        };
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                Box::into_raw(Box::new(Rc::downgrade(&result.state))) as _,
            );
            if SetTimer(Some(hwnd), 1, 1000, None) == 0 {
                return Err(windows::core::Error::from_thread());
            }
            let message = changed_message();
            if message == 0 {
                return Err(windows::core::Error::from_thread());
            }
            // 刷新消息不携带图像或配置；管理员宿主可安全接收普通用户服务的只读通知。
            if let Err(error) = ChangeWindowMessageFilterEx(hwnd, message, MSGFLT_ALLOW, None) {
                crate::log::write("语言栏图标通知权限设置失败", &error.to_string());
            }
        }
        if let Err(error) = refresh_monitor(&result.state, false) {
            crate::log::write("语言栏图标更新失败", &error.to_string());
        }
        Ok(result)
    }
}

impl Drop for BrandMonitor {
    fn drop(&mut self) {
        if let Err(error) = unsafe { DestroyWindow(self.hwnd) } {
            crate::log::write("停止语言栏图标监视失败", &error.to_string());
        }
        let updater = self.state.borrow_mut().updater.take();
        drop(updater);
        // 窗口类也持有桌面资源；最后一个监视窗口关闭后释放，其他线程仍使用时保留。
        if let Err(error) = unsafe { UnregisterClassW(&self.class, Some(self.instance)) } {
            if error.code() != ERROR_CLASS_HAS_WINDOWS.to_hresult() {
                crate::log::write("释放语言栏图标监视窗口类失败", &error.to_string());
            }
        }
    }
}

#[cfg(test)]
#[path = "brand_icon_tests.rs"]
mod tests;
