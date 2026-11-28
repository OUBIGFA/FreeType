//! FreeType 服务进程：常驻拼音引擎，经命名管道服务输入法 DLL 和设置程序。

mod backup;
mod candidate_diagnostics;
pub mod dictfmt;
mod dictionary_catalog;
#[cfg(windows)]
pub mod dictionary_updates;
#[cfg(windows)]
mod enhancement;
pub mod host;
pub mod input;
mod lookup_mode;
pub mod raw;
mod saver;
mod stats;
pub mod store;
mod sync_merge;
pub mod tasks;
pub mod tools;

#[cfg(windows)]
pub mod brand_icon;
#[cfg(windows)]
pub mod desktop;
#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
pub mod setup;

use std::path::PathBuf;

/// 测试用的实例名：设了 `FREETYPE_INSTANCE` 时管道名和单实例锁都带上它，不和真在用的服务进程冲突。
pub fn instance_suffix() -> String {
    std::env::var("FREETYPE_INSTANCE")
        .ok()
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .map(|s| format!("-{s}"))
        .unwrap_or_default()
}

/// 用户数据目录：`%APPDATA%\FreeType`，`FREETYPE_DATA_DIR` 可以换掉（测试、开发）。
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FREETYPE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("FreeType")
}

/// 本机缓存目录：`%LOCALAPPDATA%\FreeType`（词库快照、日志）。设了 `FREETYPE_DATA_DIR`
/// （测试、开发）时放在它下面的 `local`，不写进正在使用的输入法的日志。
pub fn local_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FREETYPE_DATA_DIR") {
        return PathBuf::from(dir).join("local");
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("FreeType")
}

/// 词库等随安装包的数据：`<安装目录>\data`（本程序在 `<安装目录>\bin`），
/// `FREETYPE_RESOURCE_DIR` 可以换掉。
pub fn resource_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FREETYPE_RESOURCE_DIR") {
        return PathBuf::from(dir);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let bin = exe.parent().map(PathBuf::from).unwrap_or_default();
    bin.parent()
        .map(|root| root.join("data"))
        .unwrap_or_else(|| bin.join("data"))
}

/// 输入路径优先：服务进程高于普通应用一级，并退出 Windows 的节能降速。
///
/// 默认由系统判断 QoS 时，没有前台窗口的服务进程常被放到能效核或降频运行；
/// 实测同一份词库逐键中位数 14–23 ms、最慢 117 ms，退出后为 8–9 ms、最慢 24 ms。
/// 后台工作线程用 [`background_thread`] 降回普通应用以下，不和按键争处理器。
#[cfg(windows)]
pub fn prioritize_input() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
        ABOVE_NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    };
    // ControlMask 含 EXECUTION_SPEED、StateMask 不含：明确要求高 QoS，不交给系统猜测。
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    unsafe {
        if let Err(error) = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS) {
            log(&format!("提高服务进程优先级失败：{error}"));
        }
        if let Err(error) = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            std::mem::size_of_val(&state) as u32,
        ) {
            log(&format!("关闭服务进程节能降速失败：{error}"));
        }
    }
}

/// 后台线程（写盘、同步计算）：优先级回到普通应用水平，并允许系统节能调度。
/// 只影响调用它的线程；输入路径的线程保持 [`prioritize_input`] 的设定。
pub fn background_thread() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadInformation, SetThreadPriority, ThreadPowerThrottling,
            THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            THREAD_POWER_THROTTLING_STATE, THREAD_PRIORITY_LOWEST,
        };
        let state = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        };
        // 进程是高于普通一级，LOWEST（-2）正好落回普通应用的基础优先级。失败只影响调度，不影响结果。
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        let _ = SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            (&state as *const THREAD_POWER_THROTTLING_STATE).cast(),
            std::mem::size_of_val(&state) as u32,
        );
    }
}

/// 追加一行日志到 `%LOCALAPPDATA%\FreeType\logs\server.log`（超过 1 MB 从头写）。
pub fn log(message: &str) {
    use std::io::Write;
    let dir = local_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("server.log");
    let append = std::fs::metadata(&path)
        .map(|m| m.len() < 1 << 20)
        .unwrap_or(true);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&path);
    if let Ok(mut file) = file {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{secs}] {message}");
    }
}
