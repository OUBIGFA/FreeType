//! 输入法 DLL 的日志：`%LOCALAPPDATA%\FreeType\logs\tsf.log`，一行一条，带程序名和进程号。
//!
//! DLL 在每个应用里都有一份，同一种事 30 秒内只记一次，免得卡住的连接每个键都写一行。
//! 沙箱里的应用（低完整性、应用容器）写不了这个目录，写不了就算了。

use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;

/// 同一种事多久记一次。
const QUIET_MS: u64 = 30_000;
/// 文件超过这么大就从头写。
const MAX_BYTES: u64 = 1 << 20;

static LAST: Mutex<Option<HashMap<&'static str, u64>>> = Mutex::new(None);

fn now_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

fn program() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// 当前程序名（小写），服务进程按它记各应用的中英文状态。
pub fn app_name() -> String {
    program().to_ascii_lowercase()
}

/// 记一条。`kind` 是事情的种类（限流按它算），`detail` 是具体内容。
pub fn write(kind: &'static str, detail: &str) {
    let now = now_ms();
    if let Ok(mut last) = LAST.lock() {
        let map = last.get_or_insert_with(HashMap::new);
        if map.get(kind).is_some_and(|&at| now.saturating_sub(at) < QUIET_MS) {
            return;
        }
        map.insert(kind, now);
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let dir = std::path::PathBuf::from(local).join("FreeType").join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("tsf.log");
    let append = std::fs::metadata(&path).map(|m| m.len() < MAX_BYTES).unwrap_or(true);
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
        let _ = writeln!(file, "[{secs}] {} ({}) {kind}：{detail}", program(), std::process::id());
    }
}
