//! 安装器调用的注册命令。
//!
//! - `--register-tsf <bin 目录>`（需要管理员）：把 x64、x86 两份输入法 DLL 复制到
//!   `Program Files\FreeType`、`Program Files (x86)\FreeType`，注册 COM 类和 TSF 语言配置，
//!   在 `HKLM\SOFTWARE\FreeType` 记下服务进程的位置。放进 Program Files 是为了让商店应用、
//!   管理员进程也能加载输入法（用户目录里的 DLL 它们读不到或不信任）。
//! - `--unregister-tsf`（需要管理员）：注销并删掉上面的文件和注册表项。
//! - `--enable` / `--disable`：把 FreeType 加进 / 移出当前用户的输入法列表。
//!
//! DLL 按内容摘要并行保存；注册成功后原子发布 current.txt，由运行中的 TSF 外壳安全交接。

use std::path::{Path, PathBuf};
use std::process::Command;

use freetype_protocol::{LANGID_ZH_CN, PROFILE_GUID, TEXT_SERVICE_CLSID};
use windows::core::{s, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY,
    KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

const DLL_NAME: &str = "freetype_tsf.dll";
const ICON_NAME: &str = "freetype.ico";
pub const REGISTRY_KEY: &str = r"SOFTWARE\FreeType";

/// 提权调用拿不到退出码；只在操作成功后创建本次私有目录中的标记，不覆盖已有文件。
pub fn record_setup_result(result: Result<(), String>, marker: Option<&Path>) -> Result<(), String> {
    result?;
    if let Some(marker) = marker {
        std::fs::OpenOptions::new().write(true).create_new(true).open(marker)
            .map_err(|error| format!("无法创建安装结果标记 {}：{error}", marker.display()))?;
    }
    Ok(())
}

pub(crate) fn program_files(x86: bool) -> PathBuf {
    let var = if x86 { "ProgramFiles(x86)" } else { "ProgramW6432" };
    std::env::var_os(var)
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
        .join("FreeType")
}

fn installed_dll(x86: bool) -> PathBuf {
    let root = program_files(x86);
    std::fs::read_to_string(root.join("current.txt")).ok()
        .filter(|revision| valid_revision(revision.trim()))
        .map(|revision| root.join("versions").join(revision.trim()).join(DLL_NAME))
        .unwrap_or_else(|| root.join(DLL_NAME))
}

pub(crate) fn valid_revision(revision: &str) -> bool {
    revision.len() == 64 && revision.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 固定测试版号也必须识别新代码；目录按实际 DLL SHA-256 命名，不覆盖被加载的版本。
fn stage_component(source: &Path, root: &Path) -> Result<(String, PathBuf), String> {
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    let mut hash = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, &bytes, &mut hash).ok() }.map_err(|e| e.to_string())?;
    let revision: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let target = root.join("versions").join(&revision).join(DLL_NAME);
    if target.exists() {
        if std::fs::read(&target).map_err(|e| e.to_string())? != bytes {
            return Err(format!("已保存的输入法组件摘要不符：{}", target.display()));
        }
    } else {
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        let temporary = target.with_extension(format!("pending-{}", std::process::id()));
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &target).map_err(|e| e.to_string())?;
    }
    Ok((revision, target))
}

fn publish_component(root: &Path, revision: &str) -> Result<(), String> {
    if !valid_revision(revision) || !root.join("versions").join(revision).join(DLL_NAME).is_file() {
        return Err("输入法组件未准备完成，不能发布".into());
    }
    let temporary = root.join(format!("current-{}.tmp", std::process::id()));
    std::fs::write(&temporary, revision).map_err(|e| e.to_string())?;
    unsafe { MoveFileExW(&HSTRING::from(temporary.as_os_str()), &HSTRING::from(root.join("current.txt").as_os_str()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }
        .map_err(|e| format!("无法发布输入法组件：{e}"))
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
    let previous_icon = crate::brand_icon::installed_icon()?;
    let root64 = program_files(false);
    let root86 = program_files(true);
    let (revision64, x64) = stage_component(&bin_dir.join(DLL_NAME), &root64)?;
    let (revision86, x86) = stage_component(&bin_dir.join("x86").join(DLL_NAME), &root86)?;
    // 系统输入法列表里显示的图标（DllRegisterServer 按 DLL 旁边的这个文件登记）
    for target in [&x64, &x86] {
        install_file(&bin_dir.join(ICON_NAME), &target.with_file_name(ICON_NAME))?;
    }
    call_dll(&x64, s!("DllRegisterServer"))?;
    regsvr32_x86(&x86, false)?;
    crate::brand_icon::register_installation_icon(&x64, &bin_dir.join(ICON_NAME), previous_icon.as_deref())?;
    set_server_path(&bin_dir.join("FreeTypeServer.exe"))?;
    // 两种架构全部可用后才通知现存组件切换；注册失败保留旧的发布指针。
    publish_component(&root64, &revision64)?;
    publish_component(&root86, &revision86)
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
    for root in [program_files(false), program_files(true)] {
        let mut components = vec![root.join(DLL_NAME)];
        if let Ok(entries) = std::fs::read_dir(root.join("versions")) {
            for entry in entries.flatten() {
                if valid_revision(&entry.file_name().to_string_lossy()) { components.push(entry.path().join(DLL_NAME)); }
            }
        }
        for path in components {
            for file in [path.to_path_buf(), path.with_file_name(ICON_NAME)] {
                if let Err(e) = remove_file(&file) {
                    errors.push(e);
                }
            }
            if let Some(dir) = path.parent().filter(|dir| *dir != root) { let _ = std::fs::remove_dir(dir); }
        }
        if let Err(e) = remove_file(&root.join("current.txt")) { errors.push(e); }
        if let Ok(entries) = std::fs::read_dir(root.join("brand")) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().split('.').next().is_some_and(valid_revision) {
                    if let Err(error) = remove_file(&entry.path()) { errors.push(error); }
                }
            }
            let _ = std::fs::remove_dir(root.join("brand"));
        }
        let _ = std::fs::remove_dir(root.join("versions"));
        let _ = std::fs::remove_dir(root);
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

/// 安装器已释放启动屏障。启动新服务并等词库可用，失败时安装器保留待更新记录。
pub fn start_and_check() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32};
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let expected = executable.canonicalize().map_err(|e| e.to_string())?;
    let name = freetype_protocol::pipe_name(&crate::pipe::current_user_sid().map_err(|e| e.to_string())?) + &crate::instance_suffix();
    Command::new(&executable).creation_flags(0x0800_0000 | 0x0000_0008).spawn().map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    while std::time::Instant::now() < deadline {
        if let Ok((response, pid)) = crate::pipe::request_with_server(&name, &freetype_protocol::Request::Status) {
            let path = unsafe {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(|e| e.to_string())?;
                let mut path = [0u16; 32768];
                let mut len = path.len() as u32;
                let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len);
                let _ = CloseHandle(process);
                result.map_err(|e| e.to_string())?;
                PathBuf::from(String::from_utf16_lossy(&path[..len as usize]))
            };
            if path.canonicalize().map_err(|e| e.to_string())? != expected { return Err("仍有其他安装目录的旧服务运行，更新尚未生效".into()); }
            if let Some(status) = response.status {
                if let Some(error) = status.error { return Err(error); }
                if response.ok && status.ready { return Ok(()); }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err("新版输入法服务未在 45 秒内就绪".into())
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
    fn component_publication_preserves_old_bytes_and_rejects_incomplete_versions() {
        let marker = marker_path();
        let root = marker.parent().unwrap();
        let source = root.join("source.dll");
        std::fs::write(&source, b"first component").unwrap();
        let (first, old) = super::stage_component(&source, root).unwrap();
        assert_eq!(first.len(), 64);
        super::publish_component(root, &first).unwrap();
        std::fs::write(&source, b"second component").unwrap();
        let (second, new) = super::stage_component(&source, root).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&old).unwrap(), b"first component");
        assert_eq!(std::fs::read(&new).unwrap(), b"second component");
        super::publish_component(root, &second).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("current.txt")).unwrap(), second);
        for invalid in ["../escape", &"a".repeat(64)] {
            assert!(super::publish_component(root, invalid).is_err());
            assert_eq!(std::fs::read_to_string(root.join("current.txt")).unwrap(), second);
        }
        std::fs::write(&new, b"corrupt").unwrap();
        assert!(super::stage_component(&source, root).is_err());
    }

    fn marker_path() -> std::path::PathBuf {
        let root = std::env::var_os("FREETYPE_TEST_TEMP")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_temp/setup-tests"));
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let directory = root.join(format!("marker-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("setup.ok")
    }

    #[test]
    fn setup_marker_is_created_only_for_success() {
        let marker = marker_path();
        assert_eq!(super::record_setup_result(Err("unregister failed".into()), Some(&marker)),
                   Err("unregister failed".into()));
        assert!(!marker.exists());
        assert_eq!(super::record_setup_result(Ok(()), Some(&marker)), Ok(()));
        assert!(marker.is_file());
    }

    #[test]
    fn setup_marker_does_not_overwrite_existing_files() {
        let marker = marker_path();
        std::fs::write(&marker, "existing").unwrap();
        assert!(super::record_setup_result(Ok(()), Some(&marker)).is_err());
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "existing");
    }

    #[test]
    fn setup_marker_creation_failure_is_not_success() {
        let marker = marker_path().join("missing/setup.ok");
        assert!(super::record_setup_result(Ok(()), Some(&marker)).is_err());
        assert!(!marker.exists());
        assert_eq!(super::record_setup_result(Ok(()), None), Ok(()));
    }

    #[test]
    fn tip_string_layout() {
        assert_eq!(
            super::tip_string(),
            "0804:{6AEA9A65-33E4-4FFA-94E3-399C2ADC4CF1}{2C829DC6-FA03-477E-85D0-C64E1C64B209}"
        );
    }
}
