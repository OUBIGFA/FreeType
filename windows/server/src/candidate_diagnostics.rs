//! Opt-in content diagnostics. Input only enqueues; all file operations run on a lower-priority worker.
use freetype_protocol::{CandidateDiagnostics, Response};
use serde_json::{json, Value};
use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender, SyncSender},
        Arc,
    },
    thread::JoinHandle,
};

const LIMIT: u64 = 16 * 1024 * 1024;
pub(crate) enum Action {
    Status,
    Clear,
    Export { path: PathBuf, evaluation: bool },
}
pub(crate) fn action(request: &freetype_protocol::Request) -> Option<Action> {
    use freetype_protocol::Request;
    match request {
        Request::GetCandidateDiagnostics => Some(Action::Status),
        Request::ClearCandidateDiagnostics => Some(Action::Clear),
        Request::ExportCandidateDiagnostics { path, evaluation } => Some(Action::Export {
            path: path.into(),
            evaluation: *evaluation,
        }),
        _ => None,
    }
}
enum Work {
    Events(Vec<Value>, String, u64),
    Action(Action, Sender<Response>),
}

pub(crate) struct Diagnostics {
    path: PathBuf,
    queue: OnceCell<SyncSender<Work>>,
    worker: OnceCell<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
}

impl Diagnostics {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            queue: OnceCell::new(),
            worker: OnceCell::new(),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }
    fn sender(&self) -> Result<&SyncSender<Work>, String> {
        if self.queue.get().is_none() {
            let (tx, rx) = mpsc::sync_channel(64);
            let path = self.path.clone();
            let dropped = self.dropped.clone();
            let worker = std::thread::Builder::new()
                .name("freetype-candidate-diagnostics".into())
                .spawn(move || {
                    crate::background_thread();
                    let mut error = String::new();
                    for work in rx {
                        match work {
                            Work::Events(events, app, session) => {
                                let count = events.len() as u64;
                                if let Err(reason) = append(&path, events, &app, session) {
                                    dropped.fetch_add(count, Ordering::Relaxed);
                                    error = reason;
                                }
                            }
                            Work::Action(action, reply) => {
                                let result = match action {
                                    Action::Status => Ok(()),
                                    Action::Clear => clear(&path).map(|()| {
                                        error.clear();
                                        dropped.store(0, Ordering::Relaxed);
                                    }),
                                    Action::Export {
                                        path: target,
                                        evaluation,
                                    } => export(&path, &target, evaluation),
                                };
                                let response = match result {
                                    Ok(()) => Response {
                                        candidate_diagnostics: Some(CandidateDiagnostics {
                                            bytes: bytes(&path) + bytes(&old_path(&path)),
                                            dropped: dropped.load(Ordering::Relaxed),
                                            error: error.clone(),
                                        }),
                                        ..Response::ok()
                                    },
                                    Err(reason) => Response::error(reason),
                                };
                                let _ = reply.send(response);
                            }
                        }
                    }
                })
                .map_err(|e| format!("无法启动候选诊断后台线程：{e}"))?;
            let _ = self.worker.set(worker);
            let _ = self.queue.set(tx);
        }
        Ok(self.queue.get().expect("queue initialized"))
    }
    pub fn record(&self, events: Vec<Value>, app: &str, session: u64) {
        if events.is_empty() {
            return;
        }
        let count = events.len() as u64;
        let sent = self.sender().ok().is_some_and(|queue| {
            queue
                .try_send(Work::Events(
                    events,
                    app.chars().take(200).collect(),
                    session,
                ))
                .is_ok()
        });
        if !sent {
            self.dropped.fetch_add(count, Ordering::Relaxed);
        }
    }
    pub fn request(&self, action: Action, reply: Sender<Response>) {
        match self.sender() {
            Ok(queue) => {
                if let Err(failed) = queue.try_send(Work::Action(action, reply)) {
                    let work = match failed {
                        mpsc::TrySendError::Full(work) | mpsc::TrySendError::Disconnected(work) => {
                            work
                        }
                    };
                    if let Work::Action(_, reply) = work {
                        let _ = reply.send(Response::error("候选诊断正在处理记录，请稍后重试"));
                    }
                }
            }
            Err(error) => {
                let _ = reply.send(Response::error(error));
            }
        }
    }
    /// Direct callers are component tests; the production dispatch sends the reply asynchronously.
    pub fn request_inline(&self, action: Action) -> Response {
        let (tx, rx) = mpsc::channel();
        self.request(action, tx);
        rx.recv_timeout(std::time::Duration::from_secs(20))
            .unwrap_or_else(|_| Response::error("候选诊断处理超时"))
    }
}
impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.queue.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn old_path(path: &Path) -> PathBuf {
    path.with_extension("jsonl.1")
}
fn bytes(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}
fn append(path: &Path, events: Vec<Value>, app: &str, session: u64) -> Result<(), String> {
    let mut data = Vec::new();
    for mut event in events {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        event["v"] = json!(1);
        event["t"] = json!(now);
        event["ver"] =
            json!(option_env!("FREETYPE_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")));
        event["app"] = json!(app);
        event["session"] = json!(session);
        serde_json::to_writer(&mut data, &event).map_err(|e| e.to_string())?;
        data.push(b'\n');
    }
    if data.len() as u64 > LIMIT {
        return Err("本次候选诊断记录过大，已跳过".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if bytes(path) > 0 && bytes(path) + data.len() as u64 > LIMIT {
        fs::rename(path, old_path(path)).map_err(|e| format!("轮换候选诊断记录失败：{e}"))?;
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(&data))
        .map_err(|e| format!("写入候选诊断失败：{e}"))
}
fn clear(path: &Path) -> Result<(), String> {
    for file in [old_path(path), path.to_path_buf()] {
        match fs::OpenOptions::new().write(true).truncate(true).open(file) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("清空候选诊断失败：{e}")),
        }
    }
    Ok(())
}
fn export(source: &Path, target: &Path, evaluation: bool) -> Result<(), String> {
    if !target.is_absolute() {
        return Err("请选择完整的导出文件路径".into());
    }
    let resolved = target
        .canonicalize()
        .or_else(|_| {
            target
                .parent()
                .ok_or_else(|| std::io::Error::other("没有父目录"))?
                .canonicalize()
                .map(|parent| parent.join(target.file_name().unwrap_or_default()))
        })
        .map_err(|e| format!("无法访问导出目录：{e}"))?;
    let mut content = String::new();
    for file in [old_path(source), source.to_path_buf()] {
        let normalized = file.canonicalize().unwrap_or_else(|_| file.clone());
        if normalized
            .to_string_lossy()
            .eq_ignore_ascii_case(&resolved.to_string_lossy())
        {
            return Err("导出文件不能覆盖正在使用的候选诊断记录".into());
        }
        match fs::read_to_string(&file) {
            Ok(text) => content.push_str(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("读取候选诊断失败：{e}")),
        }
    }
    if evaluation {
        content = evaluation_cases(&content)?;
    }
    crate::store::write_atomic(target, &content).map_err(|e| format!("导出候选诊断失败：{e}"))
}

/// Same commit/step contract as WowType devlog; the fourth TSV column records replay settings.
fn evaluation_cases(content: &str) -> Result<String, String> {
    let mut commits = Vec::<Value>::new();
    let mut last = HashMap::new();
    let mut undone = HashSet::new();
    for line in content.lines().filter(|s| !s.is_empty()) {
        let event: Value =
            serde_json::from_str(line).map_err(|e| format!("候选诊断记录格式损坏：{e}"))?;
        let session = event["session"].as_u64().unwrap_or(0);
        match event["e"].as_str() {
            Some("commit") => {
                last.insert(session, commits.len());
                commits.push(event);
            }
            Some("backspace") => {
                if let Some(index) = last.remove(&session) {
                    if commits[index]["text"] == event["text"] {
                        undone.insert(index);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = String::from("# 输入\t期望\t上文\t回放设置 JSON\n");
    let mut seen = HashSet::new();
    for (index, event) in commits.iter().enumerate() {
        if undone.contains(&index)
            || event["truncated"].as_bool() == Some(true)
            || event["cfg"]["traditional"].as_bool() == Some(true)
            || event["mode"] != "zh"
            || !event["steps"].as_array().is_some_and(|steps| {
                !steps.is_empty()
                    && steps.iter().all(|s| {
                        s["kind"] == "word"
                            && s["pick"].as_u64().unwrap_or(0) > 0
                            && s.get("char").is_none()
                    })
            })
        {
            continue;
        }
        let Some(input) = event["input"]
            .as_str()
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
        else {
            continue;
        };
        let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let context = event["context"].as_str().unwrap_or("");
        if [input, text, context]
            .iter()
            .any(|s| s.contains(['\t', '\r', '\n', '|']))
        {
            continue;
        }
        let mut config = event["cfg"].clone();
        config["scheme"] = event["scheme"].clone();
        let row = format!("{input}\t{text}\t{context}\t{config}\n");
        if seen.insert(row.clone()) {
            out.push_str(&row);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::var_os("FREETYPE_TEST_TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_temp/candidate-diagnostics")
            })
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn background_write_failure_is_visible_and_does_not_claim_recorded_data() {
        let dir = directory("write-error");
        let path = dir.join("blocked");
        fs::create_dir(&path).unwrap();
        let writer = Diagnostics::new(path);
        writer.record(vec![json!({"e":"commit","text":"测试"})], "test.exe", 1);
        let response = writer.request_inline(Action::Status);
        let status = response.candidate_diagnostics.unwrap();
        assert_eq!(status.dropped, 1);
        assert!(!status.error.is_empty());
    }
    #[test]
    fn rotating_record_keeps_one_previous_file_and_clear_empties_both() {
        let dir = directory("rotation");
        let path = dir.join("records.jsonl");
        fs::File::create(&path).unwrap().set_len(LIMIT).unwrap();
        append(
            &path,
            vec![json!({"e":"commit","text":"第一条"})],
            "test.exe",
            1,
        )
        .unwrap();
        assert_eq!(bytes(&old_path(&path)), LIMIT);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(LIMIT)
            .unwrap();
        append(
            &path,
            vec![json!({"e":"commit","text":"第二条"})],
            "test.exe",
            1,
        )
        .unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("第二条"));
        assert_eq!(bytes(&old_path(&path)), LIMIT);
        clear(&path).unwrap();
        assert_eq!(bytes(&path) + bytes(&old_path(&path)), 0);
    }
    #[test]
    fn evaluation_excludes_undone_raw_and_truncated_commits_per_session() {
        let commit = |session, input, text| {
            json!({"e":"commit","session":session,"input":input,"text":text,"mode":"zh","scheme":"pinyin",
            "context":"","cfg":{"traditional":false},"steps":[{"kind":"word","pick":1}]})
        };
        let mut events = vec![
            commit(1, "nihao", "你好"),
            commit(2, "shijie", "世界"),
            json!({"e":"backspace","session":1,"text":"你好"}),
        ];
        let mut truncated = commit(3, "cuowu", "错误");
        truncated["truncated"] = true.into();
        events.push(truncated);
        let text = events
            .into_iter()
            .map(|e| e.to_string() + "\n")
            .collect::<String>();
        let cases = evaluation_cases(&text).unwrap();
        assert!(cases.contains("shijie\t世界\t"));
        assert!(!cases.contains("nihao\t"));
        assert!(!cases.contains("cuowu\t"));
        assert!(evaluation_cases("broken json").is_err());
    }
}
