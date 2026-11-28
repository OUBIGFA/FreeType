//! 词库更新包。设置程序下载、校验后把更新包解到 `data-updates/<版本>/`；服务进程是
//! `state.json` 的唯一写入者：登记待生效版本，下次启动载入时整体切换。新版本载入失败时
//! 依次回退到当前版、上一版和随安装包的词库，不混用不同包的文件。模型、简繁映射等
//! 其余资源仍从安装目录读取。
use freetype_protocol::DictionaryUpdateStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 更新包能替换的文件；其余资源不受更新包影响。
pub const FILES: &[&str] = &["lexicon.ftd", "english.txt"];
pub const MANIFEST: &str = "PACKAGE.json";
const CHANNEL: &str = "freetype-data";
const PACKAGE_FORMAT: u32 = 1;
/// 本程序能读的主词库索引格式（`MappedLexicon` 的文件头）。
const LEXICON_FORMATS: &[&str] = &["FTD1", "FTD2"];
/// 待生效项为它时，下次启动换回随安装包的词库。
pub const BUNDLED: &str = "bundled";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub current: String,
    pub previous: String,
    pub pending: String,
    pub pending_automatic: bool,
    pub failed_version: String,
    pub failed_reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Package {
    format: u32,
    channel: String,
    version: String,
    lexicon_format: String,
    files: BTreeMap<String, FileRecord>,
}

#[derive(Deserialize)]
struct FileRecord {
    bytes: u64,
    sha256: String,
}

/// 版本形如 `2026.11.29.1`（日期加当天序号），可直接按数字排序，也不会含路径字符。
pub fn parse_version(text: &str) -> Option<(u32, u32, u32, u32)> {
    let parts: Vec<&str> = text.split(['.', '-']).collect();
    let [y, m, d, n] = parts.as_slice() else {
        return None;
    };
    let number = |s: &str, max: usize| {
        (!s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    Some((number(y, 4)?, number(m, 2)?, number(d, 2)?, number(n, 3)?))
}

fn valid_version(text: &str) -> bool {
    text.contains('.') && !text.contains('-') && parse_version(text).is_some()
}

pub fn root() -> PathBuf {
    crate::local_dir().join("data-updates")
}

pub fn read_state(root: &Path) -> State {
    match std::fs::read_to_string(root.join("state.json")) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            crate::log(&format!("词库更新记录无法读取，按未更新处理：{error}"));
            State::default()
        }),
        Err(_) => State::default(),
    }
}

fn write_state(root: &Path, state: &State) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    crate::store::write_atomic(&root.join("state.json"), &text).map_err(|e| e.to_string())
}

/// 核对更新包清单、文件范围和大小；`full` 时再逐个核对 SHA-256（启动切换时在载入线程做）。
pub fn check_package(dir: &Path, version: &str, full: bool) -> Result<(), String> {
    let text =
        std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| format!("更新包缺少清单：{e}"))?;
    let package: Package =
        serde_json::from_str(&text).map_err(|e| format!("更新包清单无法读取：{e}"))?;
    if package.format != PACKAGE_FORMAT || package.channel != CHANNEL {
        return Err("更新包格式不受这个版本的 FreeType 支持".into());
    }
    if package.version != version {
        return Err("更新包清单的版本与目录不一致".into());
    }
    if !LEXICON_FORMATS.contains(&package.lexicon_format.as_str()) {
        return Err(format!(
            "更新包的词库格式 {} 需要更新 FreeType",
            package.lexicon_format
        ));
    }
    let mut expected = FILES.to_vec();
    expected.sort_unstable();
    if !package.files.keys().map(String::as_str).eq(expected) {
        return Err("更新包的文件范围与本程序不一致".into());
    }
    for (name, record) in &package.files {
        let path = dir.join(name);
        let bytes = std::fs::metadata(&path)
            .map_err(|e| format!("更新包缺少 {name}：{e}"))?
            .len();
        if bytes != record.bytes {
            return Err(format!("更新包的 {name} 大小不正确"));
        }
        if full && file_sha256(&path)? != record.sha256 {
            return Err(format!("更新包的 {name} 校验失败"));
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    use std::io::Read;
    use windows::Win32::Security::Cryptography::{
        BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData, BCRYPT_HASH_HANDLE,
        BCRYPT_SHA256_ALG_HANDLE,
    };
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = BCRYPT_HASH_HANDLE::default();
    unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut hash, None, None, 0) }
        .ok()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            unsafe { BCryptHashData(hash, &buffer[..n], 0) }
                .ok()
                .map_err(|e| e.to_string())?;
        }
        let mut digest = [0u8; 32];
        unsafe { BCryptFinishHash(hash, &mut digest, 0) }
            .ok()
            .map_err(|e| e.to_string())?;
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    })();
    unsafe {
        let _ = BCryptDestroyHash(hash);
    }
    result
}

/// 本次启动实际使用的词库目录和版本（空串为随安装包的词库）。
static ACTIVE: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

/// 之后换主词库重新载入时沿用启动时选定的词库目录，不在输入过程中切换更新包。
pub fn active_dir(bundled: &Path) -> PathBuf {
    ACTIVE
        .lock()
        .unwrap()
        .as_ref()
        .map_or_else(|| bundled.to_path_buf(), |(dir, _)| dir.clone())
}

fn active_version() -> String {
    ACTIVE
        .lock()
        .unwrap()
        .as_ref()
        .map(|(_, version)| version.clone())
        .unwrap_or_default()
}

/// 启动时选包：依次尝试待生效版、当前版、上一版和随安装包的词库，第一份载入成功的生效。
/// 更新包失败而后面的词库成功，才说明是更新包的问题；全部失败时不改记录（例如学习记录读不了）。
pub fn activate<T>(
    root: &Path,
    bundled: &Path,
    mut load: impl FnMut(&Path) -> Result<T, String>,
) -> Result<T, String> {
    let mut state = read_state(root);
    let before = state.clone();
    let mut candidates: Vec<String> = Vec::new();
    for version in [&state.pending, &state.current, &state.previous] {
        if !version.is_empty() && !candidates.contains(version) {
            candidates.push(version.clone());
        }
    }
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut chosen = None;
    for version in candidates {
        if version == BUNDLED {
            break;
        }
        let dir = root.join(&version);
        // 待生效版首次启用时逐个核对摘要；之后只核对清单与大小，主词库仍由索引 CRC 把关。
        let checked = if valid_version(&version) {
            check_package(&dir, &version, version == state.pending)
        } else {
            Err("更新记录中的版本无效".into())
        };
        match checked.and_then(|()| load(&dir)) {
            Ok(value) => {
                chosen = Some((version, dir, value));
                break;
            }
            Err(reason) => {
                crate::log(&format!("词库更新包 {version} 未能载入：{reason}"));
                failed.push((version, reason));
            }
        }
    }
    let (version, dir, value) = match chosen {
        Some(found) => found,
        None => (String::new(), bundled.to_path_buf(), load(bundled)?),
    };
    let pending = std::mem::take(&mut state.pending);
    state.pending_automatic = false;
    if let Some((first, reason)) = failed.first() {
        state.failed_version = first.clone();
        state.failed_reason = reason.clone();
    } else if !pending.is_empty() {
        state.failed_version.clear();
        state.failed_reason.clear();
    }
    let old_current = state.current.clone();
    let requested = version == pending || (pending == BUNDLED && version.is_empty());
    if requested {
        // 用户或自动更新安排的切换：原来的当前版成为上一版，可以再换回来。
        state.current = version.clone();
        state.previous = old_current;
    } else if version != old_current {
        // 当前版载入失败，退到上一版或随安装包的词库。
        state.current = version.clone();
        if state.previous == version {
            state.previous.clear();
        }
    }
    let is_failed = |v: &String| failed.iter().any(|(f, _)| f == v);
    if is_failed(&state.current) {
        state.current.clear();
    }
    if is_failed(&state.previous) || state.previous == state.current {
        state.previous.clear();
    }
    if state != before {
        if let Err(error) = write_state(root, &state) {
            crate::log(&format!("词库更新记录写入失败：{error}"));
        }
    }
    *ACTIVE.lock().unwrap() = Some((dir, version));
    cleanup(root, &state);
    Ok(value)
}

/// 只保留当前版、上一版和待生效版；正在下载的 `.partial` 目录归设置程序管理。
fn cleanup(root: &Path, state: &State) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let kept = [&state.current, &state.previous, &state.pending].contains(&&name);
        if valid_version(&name) && !kept && entry.path().is_dir() {
            if let Err(error) = std::fs::remove_dir_all(entry.path()) {
                crate::log(&format!("旧词库更新包 {name} 未能清理：{error}"));
            }
        }
    }
}

pub fn status(root: &Path, bundled: &Path) -> DictionaryUpdateStatus {
    let state = read_state(root);
    DictionaryUpdateStatus {
        directory: root.display().to_string(),
        active: active_version(),
        bundled: std::fs::read_to_string(bundled.join("VERSION"))
            .map(|v| v.trim().to_string())
            .unwrap_or_default(),
        current: state.current,
        previous: state.previous,
        pending: state.pending,
        pending_automatic: state.pending_automatic,
        failed_version: state.failed_version,
        failed_reason: state.failed_reason,
    }
}

/// 设置程序解包并校验后登记；只读清单与文件大小，引擎线程上保持毫秒级。
pub fn stage(root: &Path, version: &str, automatic: bool) -> Result<(), String> {
    if !valid_version(version) {
        return Err("词库版本无效".into());
    }
    let mut state = read_state(root);
    if automatic && state.failed_version == version {
        return Err("这个版本上次未能生效，自动更新已跳过".into());
    }
    check_package(&root.join(version), version, false)?;
    if state.current == version {
        state.pending.clear();
        state.pending_automatic = false;
        write_state(root, &state)?;
        return Err("这个版本已在使用".into());
    }
    state.pending = version.to_string();
    state.pending_automatic = automatic;
    write_state(root, &state)
}

/// 下次启动换回上一份；没有上一份更新包时换回随安装包的词库。
pub fn restore_previous(root: &Path) -> Result<(), String> {
    let mut state = read_state(root);
    if state.current.is_empty() {
        return Err("正在使用随安装包的词库，没有可恢复的上一版".into());
    }
    state.pending = if state.previous.is_empty() {
        BUNDLED.to_string()
    } else {
        state.previous.clone()
    };
    state.pending_automatic = false;
    write_state(root, &state)
}

pub fn cancel_pending(root: &Path, automatic_only: bool) -> Result<(), String> {
    let mut state = read_state(root);
    if state.pending.is_empty() || (automatic_only && !state.pending_automatic) {
        return Ok(());
    }
    state.pending.clear();
    state.pending_automatic = false;
    write_state(root, &state)
}

/// 发布前的实际载入检查（`FreeTypeServer --check-dictionary <目录>`）：核对清单和全部摘要，
/// 用全新的个人数据目录载入引擎，并确认几组常用输入的首页候选。
pub fn check_built_package(dir: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| e.to_string())?;
    let package: Package = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    check_package(dir, &package.version, true)?;
    let store = std::env::temp_dir().join(format!("freetype-data-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store);
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let loaded = crate::host::load_engine_from(dir, dir, &store, "");
    let _ = std::fs::remove_dir_all(&store);
    let engine = loaded?.engine;
    for (raw, expected) in [
        ("nihao", "你好"),
        ("zhongguo", "中国"),
        ("shurufa", "输入法"),
        ("women", "我们"),
    ] {
        let page: Vec<String> = engine
            .input(raw)
            .into_iter()
            .take(5)
            .map(|c| c.text)
            .collect();
        if !page.iter().any(|text| text == expected) {
            return Err(format!("{raw} 的首页候选缺少「{expected}」：{page:?}"));
        }
    }
    Ok(package.version)
}

#[cfg(test)]
#[path = "dictionary_updates_tests.rs"]
mod tests;
