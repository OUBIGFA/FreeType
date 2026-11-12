//! FreeType 服务进程：常驻拼音引擎，经命名管道服务输入法 DLL 和设置程序。

pub mod dictfmt;
pub mod host;
pub mod input;
pub mod raw;
pub mod store;
pub mod tools;

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
    bin.parent().map(|root| root.join("data")).unwrap_or_else(|| bin.join("data"))
}

/// 追加一行日志到 `%LOCALAPPDATA%\FreeType\logs\server.log`（超过 1 MB 从头写）。
pub fn log(message: &str) {
    use std::io::Write;
    let dir = local_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("server.log");
    let append = std::fs::metadata(&path).map(|m| m.len() < 1 << 20).unwrap_or(true);
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
