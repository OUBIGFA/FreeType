//! DllRegisterServer / DllUnregisterServer：COM 类、TSF 语言配置和类别。
//!
//! COM 类写在 `HKLM\SOFTWARE\Classes\CLSID`（32 位 DLL 由 regsvr32 写进 WOW6432Node）。
//! 图标用 DLL 旁边的 `freetype.ico`（注册程序一起复制过去）。

use windows::core::{Result, GUID, HSTRING};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HINSTANCE, MAX_PATH};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr, ITfInputProcessorProfileMgr,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT, GUID_TFCAT_TIP_KEYBOARD,
};

use freetype_protocol::{DISPLAY_NAME, LANGID_ZH_CN, PROFILE_GUID, TEXT_SERVICE_CLSID};

pub fn clsid() -> GUID {
    GUID::try_from(TEXT_SERVICE_CLSID).expect("CLSID 常量格式正确")
}

fn profile() -> GUID {
    GUID::try_from(PROFILE_GUID).expect("配置 GUID 常量格式正确")
}

const CATEGORIES: [GUID; 3] = [
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
];

fn module_path(instance: HINSTANCE) -> String {
    let mut buf = [0u16; MAX_PATH as usize * 4];
    let len = unsafe { GetModuleFileNameW(Some(instance.into()), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

fn clsid_key() -> String {
    format!(r"SOFTWARE\Classes\CLSID\{{{TEXT_SERVICE_CLSID}}}")
}

fn set_string(key: HKEY, name: Option<&str>, value: &str) -> Result<()> {
    let bytes: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let name = name.map(HSTRING::from);
    let status = unsafe {
        match &name {
            Some(n) => RegSetValueExW(key, n, None, REG_SZ, Some(&bytes)),
            None => RegSetValueExW(key, None, None, REG_SZ, Some(&bytes)),
        }
    };
    status.ok()
}

fn create_key(path: &str) -> Result<HKEY> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()?;
    }
    Ok(key)
}

fn register_com(dll: &str) -> Result<()> {
    let key = create_key(&clsid_key())?;
    let result = set_string(key, None, DISPLAY_NAME);
    unsafe {
        let _ = RegCloseKey(key);
    }
    result?;
    let key = create_key(&format!(r"{}\InprocServer32", clsid_key()))?;
    let result = set_string(key, None, dll).and_then(|_| set_string(key, Some("ThreadingModel"), "Apartment"));
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    let dll = module_path(instance);
    register_com(&dll)?;
    let icon = std::path::Path::new(&dll)
        .with_file_name("freetype.ico")
        .display()
        .to_string();
    let clsid = clsid();
    let profile = profile();
    unsafe {
        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        let name: Vec<u16> = DISPLAY_NAME.encode_utf16().collect();
        let icon: Vec<u16> = icon.encode_utf16().collect();
        profiles.RegisterProfile(&clsid, LANGID_ZH_CN, &profile, &name, &icon, 0, HKL::default(), 0, true, 0)?;
        let categories: ITfCategoryMgr = CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for category in CATEGORIES {
            categories.RegisterCategory(&clsid, &category, &clsid)?;
        }
    }
    Ok(())
}

pub fn unregister() -> Result<()> {
    let clsid = clsid();
    let profile = profile();
    unsafe {
        if let Ok(categories) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            for category in CATEGORIES {
                let _ = categories.UnregisterCategory(&clsid, &category, &clsid);
            }
        }
        if let Ok(profiles) = CoCreateInstance::<_, ITfInputProcessorProfileMgr>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            let _ = profiles.UnregisterProfile(&clsid, LANGID_ZH_CN, &profile, 0);
        }
        let status = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(clsid_key()));
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            return status.ok();
        }
    }
    Ok(())
}
