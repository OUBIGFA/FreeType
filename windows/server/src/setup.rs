//! 安装器调用的注册命令。
//!
//! - `--register-tsf <bin 目录>`（需要管理员）：把 x64、x86 两份输入法 DLL 复制到
//!   `Program Files\FreeType`、`Program Files (x86)\FreeType`，注册 COM 类和 TSF 语言配置，
//!   在 `HKLM\SOFTWARE\FreeType` 记下服务进程的位置。放进 Program Files 是为了让商店应用、
//!   管理员进程也能加载输入法（用户目录里的 DLL 它们读不到或不信任）。
//! - `--unregister-tsf`（需要管理员）：注销并删掉上面的文件和注册表项。
//! - `--enable` / `--disable`：把 FreeType 加进 / 移出当前用户的输入法列表。
//!
//! DLL 正被应用加载时不能覆盖：先改名，重启后删除旧文件。

use std::path::{Path, PathBuf};
use std::process::Command;

use freetype_protocol::{LANGID_ZH_CN, PROFILE_GUID, TEXT_SERVICE_CLSID};
use windows::core::{s, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY,
    KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

const DLL_NAME: &str = "freetype_tsf.dll";
const ICON_NAME: &str = "freetype.ico";
pub const REGISTRY_KEY: &str = r"SOFTWARE\FreeType";

fn program_files(x86: bool) -> PathBuf {
    let var = if x86 { "ProgramFiles(x86)" } else { "ProgramW6432" };
    std::env::var_os(var)
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
        .join("FreeType")
}

fn installed_dll(x86: bool) -> PathBuf {
    program_files(x86).join(DLL_NAME)
}

/// 删掉文件；正被占用就改名，重启后删。
fn remove_file(path: &Path) -> Result<(), String> {
    if !path.exists() || std::fs::remove_file(path).is_ok() {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let old = path.with_file_name(format!("{name}.old-{stamp}"));
    std::fs::rename(path, &old).map_err(|e| format!("无法替换 {}：{e}", path.display()))?;
    unsafe {
        let _ = MoveFileExW(&HSTRING::from(old.as_os_str()), PCWSTR::null(), MOVEFILE_DELAY_UNTIL_REBOOT);
    }
    Ok(())
}

fn install_file(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
    }
    remove_file(target)?;
    std::fs::copy(source, target)
        .map(|_| ())
        .map_err(|e| format!("无法复制 {} 到 {}：{e}", source.display(), target.display()))
}

/// 在本进程里调 64 位 DLL 的 DllRegisterServer / DllUnregisterServer。
fn call_dll(path: &Path, export: windows::core::PCSTR) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| {
            let module = LoadLibraryW(&HSTRING::from(path.as_os_str()))
                .map_err(|e| format!("无法载入 {}：{e}", path.display()))?;
            let result = match GetProcAddress(module, export) {
                Some(f) => {
                    let f: extern "system" fn() -> HRESULT = std::mem::transmute(f);
                    f().ok().map_err(|e| format!("{} 调用失败：{e}", path.display()))
                }
                None => Err(format!("{} 缺少导出函数", path.display())),
            };
            let _ = FreeLibrary(module);
            result
        })();
        CoUninitialize();
        result
    }
}

/// 32 位 DLL 交给 SysWOW64 里的 regsvr32。
fn regsvr32_x86(path: &Path, unregister: bool) -> Result<(), String> {
    let windir = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let exe = PathBuf::from(windir).join("SysWOW64").join("regsvr32.exe");
    let mut command = Command::new(exe);
    command.arg("/s");
    if unregister {
        command.arg("/u");
    }
    let status = command
        .arg(path)
        .status()
        .map_err(|e| format!("无法运行 regsvr32：{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("regsvr32 注册 {} 失败（{status}）", path.display()))
    }
}

fn set_server_path(server: &Path) -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        let status = RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(REGISTRY_KEY),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | KEY_WOW64_64KEY,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            return Err(format!("无法写注册表（{}）", status.0));
        }
        let value: Vec<u8> = HSTRING::from(server.as_os_str())
            .iter()
            .chain(std::iter::once(&0u16))
            .flat_map(|w| w.to_le_bytes())
            .collect();
        let status = RegSetValueExW(key, &HSTRING::from("ServerPath"), None, REG_SZ, Some(&value));
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return Err(format!("无法写注册表（{}）", status.0));
        }
    }
    Ok(())
}

pub fn register_tsf(bin_dir: &Path) -> Result<(), String> {
    let x64 = installed_dll(false);
    let x86 = installed_dll(true);
    install_file(&bin_dir.join(DLL_NAME), &x64)?;
    install_file(&bin_dir.join("x86").join(DLL_NAME), &x86)?;
    // 系统输入法列表里显示的图标（DllRegisterServer 按 DLL 旁边的这个文件登记）
    for target in [&x64, &x86] {
        install_file(&bin_dir.join(ICON_NAME), &target.with_file_name(ICON_NAME))?;
    }
    call_dll(&x64, s!("DllRegisterServer"))?;
    regsvr32_x86(&x86, false)?;
    set_server_path(&bin_dir.join("FreeTypeServer.exe"))
}

pub fn unregister_tsf() -> Result<(), String> {
    let mut errors = Vec::new();
    let x64 = installed_dll(false);
    let x86 = installed_dll(true);
    if x64.exists() {
        if let Err(e) = call_dll(&x64, s!("DllUnregisterServer")) {
            errors.push(e);
        }
    }
    if x86.exists() {
        if let Err(e) = regsvr32_x86(&x86, true) {
            errors.push(e);
        }
    }
    for path in [&x64, &x86] {
        for file in [path.to_path_buf(), path.with_file_name(ICON_NAME)] {
            if let Err(e) = remove_file(&file) {
                errors.push(e);
            }
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
    let status = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(REGISTRY_KEY)) };
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        errors.push(format!("无法删除注册表项（{}）", status.0));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

/// InstallLayoutOrTip 用的字符串：`0804:{CLSID}{PROFILE}`。
pub fn tip_string() -> String {
    format!("{LANGID_ZH_CN:04x}:{{{TEXT_SERVICE_CLSID}}}{{{PROFILE_GUID}}}")
}

/// 加进 / 移出当前用户的输入法列表（input.dll 的 InstallLayoutOrTip）。
pub fn enable_for_user(enable: bool) -> Result<(), String> {
    const ILOT_UNINSTALL: u32 = 0x0000_0001;
    unsafe {
        let module = LoadLibraryW(&HSTRING::from("input.dll")).map_err(|e| format!("无法载入 input.dll：{e}"))?;
        let result = match GetProcAddress(module, s!("InstallLayoutOrTip")) {
            Some(f) => {
                let f: extern "system" fn(PCWSTR, u32) -> i32 = std::mem::transmute(f);
                let flags = if enable { 0 } else { ILOT_UNINSTALL };
                let tip = HSTRING::from(tip_string());
                if f(PCWSTR(tip.as_ptr()), flags) != 0 {
                    Ok(())
                } else {
                    Err("InstallLayoutOrTip 失败".to_string())
                }
            }
            None => Err("input.dll 缺少 InstallLayoutOrTip".to_string()),
        };
        let _ = FreeLibrary(module);
        result
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tip_string_layout() {
        assert_eq!(
            super::tip_string(),
            "0804:{6AEA9A65-33E4-4FFA-94E3-399C2ADC4CF1}{2C829DC6-FA03-477E-85D0-C64E1C64B209}"
        );
    }
}
