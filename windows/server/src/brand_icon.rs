//! Windows 输入指示器从 RegisterProfile 指定的 PE 图标资源读取品牌图像。
//! 独立、按图像摘要命名的资源副本避开 Shell 路径缓存，不覆盖已加载的输入法代码。

use freetype_protocol::{DISPLAY_NAME, LANGID_ZH_CN, PROFILE_GUID, TEXT_SERVICE_CLSID};
use std::path::{Path, PathBuf};
use windows::core::{w, GUID, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, ERROR_FILE_NOT_FOUND, HMODULE, LPARAM, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, FindResourceW, LoadLibraryExW, LoadResource,
    LockResource, SizeofResource, UpdateResourceW, LOAD_LIBRARY_AS_DATAFILE,
    LOAD_LIBRARY_AS_IMAGE_RESOURCE,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::Shell::{
    IsUserAnAdmin, SHChangeNotify, ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS,
    SHCNE_ASSOCCHANGED, SHCNF_FLUSHNOWAIT, SHCNF_IDLIST, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, DestroyIcon, SendMessageTimeoutW, HWND_BROADCAST, IMAGE_FLAGS,
    SMTO_ABORTIFHUNG, SW_HIDE, WM_SETTINGCHANGE,
};

struct IconResources<'a> {
    group: Vec<u8>,
    images: Vec<&'a [u8]>,
}

fn resources(ico: &[u8]) -> Result<IconResources<'_>, String> {
    let invalid = || "品牌图标不是有效的 ICO 资源".to_string();
    if ico.len() < 6 || ico.len() > 2 << 20 || ico[..4] != [0, 0, 1, 0] {
        return Err(invalid());
    }
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let header = 6 + count * 16;
    if !(1..=32).contains(&count) || header > ico.len() {
        return Err(invalid());
    }
    let mut result = IconResources {
        group: ico[..6].to_vec(),
        images: Vec::with_capacity(count),
    };
    for index in 0..count {
        let entry = &ico[6 + index * 16..6 + (index + 1) * 16];
        let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        if offset < header
            || length == 0
            || offset.checked_add(length).is_none_or(|end| end > ico.len())
        {
            return Err(invalid());
        }
        let image = &ico[offset..offset + length];
        let width = if entry[0] == 0 {
            256
        } else {
            i32::from(entry[0])
        };
        let height = if entry[1] == 0 {
            256
        } else {
            i32::from(entry[1])
        };
        if width != height {
            return Err(invalid());
        }
        // 使用 Windows 的真实图像解码器校验，损坏图像不能进入系统登记路径。
        let icon = unsafe {
            CreateIconFromResourceEx(image, true, 0x30000, width, height, IMAGE_FLAGS(0))
        }
        .map_err(|error| format!("品牌图像无法读取：{error}"))?;
        unsafe {
            DestroyIcon(icon).map_err(|error| error.to_string())?;
        }
        result.group.extend_from_slice(&entry[..12]);
        result
            .group
            .extend_from_slice(&((index + 1) as u16).to_le_bytes());
        result.images.push(image);
    }
    Ok(result)
}

fn revision(ico: &[u8]) -> Result<String, String> {
    let mut hash = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, ico, &mut hash).ok() }
        .map_err(|error| error.to_string())?;
    Ok(hash.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn resource(module: HMODULE, kind: usize, id: usize) -> Result<Vec<u8>, String> {
    // 模块只以 DATAFILE 方式读取，绝不执行副本的 DLL 入口。
    unsafe {
        let item = FindResourceW(
            Some(module),
            PCWSTR(id as *const u16),
            PCWSTR(kind as *const u16),
        );
        if item.0.is_null() {
            return Err("系统品牌模块缺少图标资源".into());
        }
        let length = SizeofResource(Some(module), item) as usize;
        let data =
            LockResource(LoadResource(Some(module), item).map_err(|error| error.to_string())?);
        if data.is_null() || length == 0 {
            return Err("系统品牌图标资源为空".into());
        }
        Ok(std::slice::from_raw_parts(data.cast(), length).to_vec())
    }
}

fn module_matches(path: &Path, expected: &IconResources<'_>) -> Result<bool, String> {
    let module = unsafe {
        LoadLibraryExW(
            &HSTRING::from(path.as_os_str()),
            None,
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )
    }
    .map_err(|error| error.to_string())?;
    let result = (|| {
        if resource(module, 14, 1)? != expected.group {
            return Ok(false);
        }
        for (index, image) in expected.images.iter().enumerate() {
            if resource(module, 3, index + 1)? != *image {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    unsafe {
        FreeLibrary(module).map_err(|error| error.to_string())?;
    }
    result
}

fn stage_icon_module(template: &Path, ico: &[u8], directory: &Path) -> Result<PathBuf, String> {
    let expected = resources(ico)?;
    let path = directory.join(format!("{}.dll", revision(ico)?));
    if path.exists() {
        if !module_matches(&path, &expected)? {
            return Err("已保存的系统品牌资源与图像摘要不符".into());
        }
        return Ok(path);
    }
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!("{}-pending.dll", std::process::id()));
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&std::fs::read(template).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    drop(file);
    let update = unsafe { BeginUpdateResourceW(&HSTRING::from(temporary.as_os_str()), true) }
        .map_err(|error| error.to_string())?;
    let result = (|| {
        unsafe {
            UpdateResourceW(
                update,
                PCWSTR(14usize as *const u16),
                PCWSTR(1usize as *const u16),
                0,
                Some(expected.group.as_ptr().cast()),
                expected.group.len() as u32,
            )
            .map_err(|error| error.to_string())?;
            for (index, image) in expected.images.iter().enumerate() {
                UpdateResourceW(
                    update,
                    PCWSTR(3usize as *const u16),
                    PCWSTR((index + 1) as *const u16),
                    0,
                    Some(image.as_ptr().cast()),
                    image.len() as u32,
                )
                .map_err(|error| error.to_string())?;
            }
        }
        Ok::<_, String>(())
    })();
    let finish =
        unsafe { EndUpdateResourceW(update, result.is_err()) }.map_err(|error| error.to_string());
    result.and(finish)?;
    if !module_matches(&temporary, &expected)? {
        return Err("写入的系统品牌资源与图像不符".into());
    }
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn registry_icon() -> Result<Option<(PathBuf, u32)>, String> {
    let key = HSTRING::from(format!(
        r"SOFTWARE\Microsoft\CTF\TIP\{{{TEXT_SERVICE_CLSID}}}\LanguageProfile\0x{LANGID_ZH_CN:08x}\{{{PROFILE_GUID}}}"
    ));
    let mut text = vec![0u16; 32768];
    let mut length = (text.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            w!("IconFile"),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut length),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    status.ok().map_err(|error| error.to_string())?;
    let path = PathBuf::from(String::from_utf16_lossy(
        &text[..(length as usize / 2).saturating_sub(1)],
    ));
    let mut index = 0u32;
    let mut length = 4;
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            w!("IconIndex"),
            RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
            None,
            Some((&mut index as *mut u32).cast()),
            Some(&mut length),
        )
    }
    .ok()
    .map_err(|error| error.to_string())?;
    Ok(Some((path, index)))
}

/// 覆盖安装只保留本程序生成的受保护资源，避免重新注册 DLL 时恢复成默认图标。
pub(crate) fn installed_icon() -> Result<Option<PathBuf>, String> {
    let directory = crate::setup::program_files(false).join("brand");
    Ok(registry_icon()?.and_then(|(path, index)| {
        (index == 0
            && path.parent() == Some(directory.as_path())
            && path
                .file_stem()
                .is_some_and(|stem| crate::setup::valid_revision(&stem.to_string_lossy()))
            && path.is_file())
        .then_some(path)
    }))
}

fn register_icon(path: &Path) -> Result<(), String> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|error| error.to_string())?;
    }
    let result = (|| {
        let clsid = GUID::try_from(TEXT_SERVICE_CLSID).unwrap();
        let profile = GUID::try_from(PROFILE_GUID).unwrap();
        let profiles: ITfInputProcessorProfileMgr = unsafe {
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)
        }
        .map_err(|error| error.to_string())?;
        let description: Vec<u16> = DISPLAY_NAME.encode_utf16().collect();
        let icon: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().collect();
        // 使用官方注册 API；保持 CLSID、语言、配置和启用规则，只改变图标来源。
        unsafe {
            profiles.RegisterProfile(
                &clsid,
                LANGID_ZH_CN,
                &profile,
                &description,
                &icon,
                0,
                HKL::default(),
                0,
                true,
                0,
            )
        }
        .map_err(|error| error.to_string())
    })();
    unsafe {
        CoUninitialize();
    }
    result?;
    if registry_icon()? != Some((path.to_path_buf(), 0)) {
        return Err("系统未保存新的品牌图标来源".into());
    }
    // 路径变化使 Shell 重新取资源；同时刷新系统输入指示器和各应用的传统语言栏。
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            SHCNF_IDLIST | SHCNF_FLUSHNOWAIT,
            None,
            None,
        );
        if SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("intl").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            100,
            None,
        )
        .0 == 0
        {
            crate::log("系统图标已登记，部分程序未响应刷新通知");
        }
    }
    crate::desktop::notify_brand_icon_changed();
    Ok(())
}

/// 安装完成后的最终登记必须指向含 RT_GROUP_ICON 的模块，独立 ICO 不符合输入指示器约定。
pub(crate) fn register_installation_icon(
    template: &Path,
    ico: &Path,
    previous: Option<&Path>,
) -> Result<(), String> {
    let path = match previous {
        Some(path) => path.to_path_buf(),
        None => stage_icon_module(
            template,
            &std::fs::read(ico).map_err(|error| error.to_string())?,
            &crate::setup::program_files(false).join("brand"),
        )?,
    };
    register_icon(&path)
}

fn read_icon(path: &Path) -> Result<Vec<u8>, String> {
    if std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len()
        > 2 << 20
    {
        return Err("系统品牌图标不能超过 2 MB".into());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    resources(&bytes)?;
    Ok(bytes)
}

fn is_current(ico: &[u8]) -> Result<bool, String> {
    let path = crate::setup::program_files(false)
        .join("brand")
        .join(format!("{}.dll", revision(ico)?));
    if registry_icon()? != Some((path.clone(), 0)) || !path.is_file() {
        return Ok(false);
    }
    module_matches(&path, &resources(ico)?)
}

/// 此入口只由短暂提权的助手使用；读取 ICO，不加载或执行调用方提供的程序。
pub fn install(ico: &Path) -> Result<(), String> {
    if !crate::instance_suffix().is_empty() {
        return Err("测试实例不能修改系统图标登记".into());
    }
    if registry_icon()?.is_none() {
        return Err("FreeType 尚未完成输入法注册".into());
    }
    let bytes = read_icon(ico)?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let template = executable.with_file_name("freetype_tsf.dll");
    let path = stage_icon_module(
        &template,
        &bytes,
        &crate::setup::program_files(false).join("brand"),
    )?;
    register_icon(&path)?;
    if !is_current(&bytes)? {
        return Err("系统品牌资源核对失败".into());
    }
    Ok(())
}

/// 已登记的图像无需再次提权；其他情况由当前用户发起一次 UAC，并核对实际保存结果。
pub fn apply(ico: &Path) -> Result<(), String> {
    if !crate::instance_suffix().is_empty() {
        return Err("测试实例不能修改系统图标登记".into());
    }
    let bytes = read_icon(ico)?;
    if is_current(&bytes)? {
        return Ok(());
    }
    if unsafe { IsUserAnAdmin() }.as_bool() {
        return install(ico);
    }
    let executable = HSTRING::from(
        std::env::current_exe()
            .map_err(|error| error.to_string())?
            .as_os_str(),
    );
    let result_file = ico.with_extension("result.json");
    if result_file.exists() {
        return Err("图标更新结果文件已存在，请重试".into());
    }
    let arguments = HSTRING::from(format!(
        "--set-brand-icon-elevated \"{}\" \"{}\"",
        ico.display(),
        result_file.display()
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(executable.as_ptr()),
        lpParameters: PCWSTR(arguments.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|error| format!("管理员授权未完成：{error}"))?;
    }
    let result = (|| {
        if unsafe { WaitForSingleObject(info.hProcess, INFINITE) } != WAIT_OBJECT_0 {
            return Err("无法等待系统图标更新结果".to_string());
        }
        let mut code = 1;
        unsafe {
            GetExitCodeProcess(info.hProcess, &mut code).map_err(|error| error.to_string())?;
        }
        let response: freetype_protocol::Response = serde_json::from_slice(
            &std::fs::read(&result_file).map_err(|error| format!("未取得图标更新结果：{error}"))?,
        )
        .map_err(|error| error.to_string())?;
        if code != 0 || !response.ok {
            return Err(response
                .error
                .unwrap_or_else(|| "系统图标更新未完成".into()));
        }
        if !is_current(&bytes)? {
            return Err("系统图标来源未更新，请重新选择图标".into());
        }
        Ok(())
    })();
    unsafe {
        CloseHandle(info.hProcess).map_err(|error| error.to_string())?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::FreeLibrary;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::System::LibraryLoader::{
        LoadLibraryExW, LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, LoadImageW, HICON, ICONINFO, IMAGE_FLAGS, IMAGE_ICON,
    };

    const PNG: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x10\x00\x00\x00\x10\x08\x06\x00\x00\x00\x1f\xf3\xff\x61\x00\x00\x00\x25\x49\x44\x41\x54\x78\x9c\x63\x64\x48\x39\xf1\x9f\x81\x02\xc0\x44\x89\xe6\x51\x03\x20\x80\x89\x81\x42\xc0\x34\x6a\x00\xc3\x68\x18\x30\x50\x1e\x06\x00\x0d\x2b\x02\x4b\x05\xde\x1c\x9a\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

    fn ico() -> Vec<u8> {
        // 手工登记一幅纯色 16 px PNG，其他 DPI 由 Windows 的真实资源读取器缩放。
        let mut data = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
        data.extend_from_slice(&(PNG.len() as u32).to_le_bytes());
        data.extend_from_slice(&22u32.to_le_bytes());
        data.extend_from_slice(PNG);
        data
    }

    fn directory() -> PathBuf {
        let root = std::env::var_os("FREETYPE_TEST_TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_temp/brand-tests")
            });
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        root.join(format!("icon-{}-{stamp}", std::process::id()))
    }

    fn pixels(icon: HICON, size: i32) -> Vec<u32> {
        unsafe {
            let mut info = ICONINFO::default();
            GetIconInfo(icon, &mut info).unwrap();
            let mut bitmap = BITMAP::default();
            assert_ne!(
                GetObjectW(
                    info.hbmColor.into(),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some((&mut bitmap as *mut BITMAP).cast())
                ),
                0
            );
            assert_eq!((bitmap.bmWidth, bitmap.bmHeight), (size, size));
            let mut dib = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    biHeight: -size,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut data = vec![0u32; (size * size) as usize];
            let dc = CreateCompatibleDC(None);
            assert_eq!(
                GetDIBits(
                    dc,
                    info.hbmColor,
                    0,
                    size as u32,
                    Some(data.as_mut_ptr().cast()),
                    &mut dib,
                    DIB_RGB_COLORS
                ),
                size
            );
            let _ = DeleteDC(dc);
            let _ = DeleteObject(info.hbmColor.into());
            let _ = DeleteObject(info.hbmMask.into());
            data
        }
    }

    #[test]
    fn persisted_brand_module_reopens_as_real_windows_icons_at_every_indicator_size() {
        let template = std::env::current_exe().unwrap();
        let before = std::fs::read(&template).unwrap();
        let directory = directory();
        let path = stage_icon_module(&template, &ico(), &directory).unwrap();
        assert_eq!(
            std::fs::read(&template).unwrap(),
            before,
            "不能修改输入法的原始模块"
        );
        // 两次关闭并重新读取磁盘模块，覆盖注册后及下一次登录的资源读取路径。
        for _ in 0..2 {
            unsafe {
                let module = LoadLibraryExW(
                    &HSTRING::from(path.as_os_str()),
                    None,
                    LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
                )
                .unwrap();
                for size in [16, 20, 24, 32, 40, 48] {
                    let handle = LoadImageW(
                        Some(module.into()),
                        PCWSTR(1usize as *const u16),
                        IMAGE_ICON,
                        size,
                        size,
                        IMAGE_FLAGS(0),
                    )
                    .expect("系统必须能从登记模块读出品牌图像");
                    let icon = HICON(handle.0);
                    // 直接解码原 PNG 作独立参照，保留 Windows 缩放时的透明度和通道取整。
                    let expected =
                        CreateIconFromResourceEx(PNG, true, 0x30000, size, size, IMAGE_FLAGS(0))
                            .unwrap();
                    assert_eq!(pixels(icon, size), pixels(expected, size));
                    DestroyIcon(expected).unwrap();
                    DestroyIcon(icon).unwrap();
                }
                FreeLibrary(module).unwrap();
            }
        }
    }

    #[test]
    fn invalid_brand_images_do_not_create_a_system_resource() {
        let directory = directory();
        let template = std::env::current_exe().unwrap();
        for input in [b"not an ico".as_slice(), &ico()[..21], &ico()[..30]] {
            assert!(stage_icon_module(&template, input, &directory).is_err());
            assert!(!directory.exists(), "无效图像不能留下可登记的模块");
        }
    }

    #[test]
    fn bundled_brand_modules_keep_all_sizes_and_distinct_cache_paths() {
        let directory = directory();
        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let template = std::env::current_exe().unwrap();
        let mut previous = None;
        for name in ["freetype-logo.png", "freetype-white-logo.png"] {
            let icons = directory.join(name);
            let status = std::process::Command::new("python")
                .args(["-X", "utf8"])
                .arg(project.join("windows/tools/make_icons.py"))
                .arg(project.join("windows/assets").join(name))
                .arg(&icons)
                .status()
                .unwrap();
            assert!(status.success(), "构建实际内置图标失败");
            let ico = std::fs::read(icons.join("freetype.ico")).unwrap();
            let path = stage_icon_module(&template, &ico, &directory.join("modules")).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(
                stage_icon_module(&template, &ico, &directory.join("modules")).unwrap(),
                path
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                bytes,
                "缓存命中不能改写已保存的资源模块"
            );
            unsafe {
                let module = LoadLibraryExW(
                    &HSTRING::from(path.as_os_str()),
                    None,
                    LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
                )
                .unwrap();
                let group = resource(module, 14, 1).unwrap();
                assert_eq!(u16::from_le_bytes(group[4..6].try_into().unwrap()), 10);
                for (index, size) in [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(group[6 + index * 14], size as u8);
                    let icon = LoadImageW(
                        Some(module.into()),
                        PCWSTR(1usize as *const u16),
                        IMAGE_ICON,
                        size,
                        size,
                        IMAGE_FLAGS(0),
                    )
                    .unwrap();
                    let _ = pixels(HICON(icon.0), size);
                    DestroyIcon(HICON(icon.0)).unwrap();
                }
                FreeLibrary(module).unwrap();
                // RegisterProfile 使用图标索引 0，核对 Shell 的实际索引读取路径。
                let mut small = HICON::default();
                assert_eq!(
                    windows::Win32::UI::Shell::ExtractIconExW(
                        &HSTRING::from(path.as_os_str()),
                        0,
                        None,
                        Some(&mut small),
                        1
                    ),
                    1
                );
                let image = pixels(
                    small,
                    windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                        windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON,
                    ),
                );
                DestroyIcon(small).unwrap();
                if let Some((previous_path, previous_image)) = previous.take() {
                    assert_ne!(path, previous_path, "不同品牌图像必须有不同系统缓存路径");
                    assert_ne!(image, previous_image, "Shell 必须实际读出不同图像");
                }
                previous = Some((path, image));
            }
        }
    }
}
