//! 数据同步：快照、后台合并任务和同步记录。
use super::*;

/// 后台任务线程：给导出快照附上同步记录，写出同步文件。
pub(super) fn write_sync_snapshot(
    store: &Store,
    text: &str,
    stamp: u64,
    path: &Path,
) -> Result<(), String> {
    let mut snapshot: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    crate::sync_merge::attach(store, &mut snapshot, stamp, false)?;
    crate::store::write_atomic(path, &snapshot.to_string()).map_err(|e| e.to_string())
}

/// 后台任务线程：按记录比较本机与远端快照，得到合并后的备份和差异预览。
/// `expected` 是上次导出的本机快照，内容不同说明下载期间本机已修改。
pub(super) fn prepare_merge(
    store: &Store,
    text: &str,
    stamp: u64,
    path: &Path,
    expected: Option<&Path>,
) -> Result<(backup::PreparedBackup, freetype_protocol::SyncMergePreview), String> {
    let mut local: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if let Some(expected) = expected {
        let expected: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(expected).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if crate::sync_merge::without_records(local.clone())
            != crate::sync_merge::without_records(expected)
        {
            return Err("本机数据在预览后已修改，请重新预览".into());
        }
    }
    crate::sync_merge::attach(store, &mut local, stamp, false)?;
    let remote = backup::read(path)?;
    let remote = serde_json::to_value(remote.data).map_err(|e| e.to_string())?;
    let (merged, preview) = crate::sync_merge::merge(&local, &remote)?;
    Ok((backup::from_text(&merged.to_string())?, preview))
}

impl Host {
    pub(super) fn mark_sync_change(&mut self) {
        self.sync_modified_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }

    /// 引擎线程只取同步要用的快照（毫秒级）；比较、合并、写文件交给后台任务线程。
    pub(super) fn sync_snapshot(&self) -> Result<(String, ImportReport, u64, Store), String> {
        let engine = self.engine.as_deref().ok_or("词库还在载入，请稍后再试")?;
        let (text, report) = backup::snapshot(
            &self.store,
            &self.config,
            self.mode,
            &self.recent_emoji,
            engine,
        )?;
        let stamp = backup::modified_unix_ms(&self.store, &self.config)?.max(self.sync_modified_ms);
        Ok((
            text,
            report,
            stamp,
            Store::new(self.store.dir().to_path_buf()),
        ))
    }

    /// 同步导出、预览、合并的后台工作；其他请求返回 None，照常在引擎线程处理。
    pub(super) fn sync_job(&self, request: &Request) -> Option<Result<Job, Response>> {
        let job: Result<Job, String> = match request {
            Request::ExportSyncSettings { path } => {
                self.sync_snapshot().map(|(text, report, stamp, store)| {
                    let path = PathBuf::from(path);
                    Box::new(move || {
                        Step::Reply(match write_sync_snapshot(&store, &text, stamp, &path) {
                            Ok(()) => Response {
                                report: Some(report),
                                modified_unix_ms: Some(stamp),
                                sync_format: 1,
                                ..Response::ok()
                            },
                            Err(error) => Response::error(error),
                        })
                    }) as Job
                })
            }
            Request::PreviewSyncMerge { path } => {
                self.sync_snapshot().map(|(text, _, stamp, store)| {
                    let path = PathBuf::from(path);
                    Box::new(move || {
                        Step::Reply(match prepare_merge(&store, &text, stamp, &path, None) {
                            Ok((_, preview)) => Response {
                                sync_merge: Some(preview),
                                ..Response::ok()
                            },
                            Err(error) => Response::error(error),
                        })
                    }) as Job
                })
            }
            Request::MergeSyncSettings { .. }
                if self.clients.values().any(|client| client.state.busy()) =>
            {
                Err("正在输入，请结束输入后再同步".into())
            }
            Request::MergeSyncSettings {
                path,
                expected_path,
            } => self.sync_snapshot().map(|(text, _, stamp, store)| {
                let (path, expected) = (PathBuf::from(path), PathBuf::from(expected_path));
                Box::new(move || {
                    match prepare_merge(&store, &text, stamp, &path, Some(&expected)) {
                        Ok((prepared, preview)) => Step::Apply(Box::new(move |host: &mut Host| {
                            let response = host.apply_merge(&text, prepared, preview);
                            host.after_edit(response, true, true, false)
                        })),
                        Err(error) => Step::Reply(Response::error(error)),
                    }
                }) as Job
            }),
            _ => return None,
        };
        Some(job.map_err(Response::error))
    }

    /// 后台算好的合并结果回到引擎线程应用。比较期间本机数据变过就放弃，由同步重新比较，
    /// 不会用旧快照算出的结果覆盖刚做的修改。
    pub(super) fn apply_merge(
        &mut self,
        compared: &str,
        prepared: backup::PreparedBackup,
        preview: freetype_protocol::SyncMergePreview,
    ) -> Response {
        if self.clients.values().any(|client| client.state.busy()) {
            return Response::error("正在输入，请结束输入后再同步");
        }
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        let current = match backup::snapshot(
            &self.store,
            &self.config,
            self.mode,
            &self.recent_emoji,
            engine,
        ) {
            Ok((text, _)) => text,
            Err(error) => return Response::error(error),
        };
        if current != compared {
            let parse = |text: &str| {
                serde_json::from_str::<serde_json::Value>(text)
                    .map(crate::sync_merge::without_records)
                    .ok()
            };
            if parse(&current) != parse(compared) {
                return Response::error("本机数据在比较期间已修改，将重新比较版本");
            }
        }
        // 合并和替换共用现有事务写入、恢复前备份及引擎接续边界。
        let mut result = self.import_prepared(prepared);
        if result.ok {
            result.sync_merge = Some(preview);
        }
        result
    }

    /// 主循环入口：同步的重计算交给后台任务线程，按键不在它后面排队。
    pub fn dispatch(
        &mut self,
        id: ClientId,
        request: Request,
        reply: std::sync::mpsc::Sender<Response>,
    ) {
        if let Some(action) = crate::candidate_diagnostics::action(&request) {
            if matches!(request, Request::ClearCandidateDiagnostics) {
                // 清空的边界也清掉尚未完成的内存记录，不能在稍后选词时把旧正文写回来。
                for client in self.clients.values_mut() {
                    client.state.session.set_trace(false);
                    client.pending = None;
                }
            }
            self.diagnostics.request(action, reply);
            return;
        }
        let job =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.sync_job(&request)))
                .unwrap_or_else(|_| Some(Err(Response::error("服务进程内部出错"))));
        match job {
            Some(Ok(job)) => self.tasks.submit(job, Some(reply)),
            Some(Err(response)) => {
                let _ = reply.send(response);
            }
            // 手动恢复同步记录排在后台任务之后、回到引擎线程执行，不在引擎线程上等后台做完。
            None if matches!(
                request,
                Request::ImportSyncSettings { .. } | Request::ResetSyncRecords
            ) =>
            {
                self.tasks.submit(
                    Box::new(move || {
                        Step::Apply(Box::new(move |host: &mut Host| host.handle(id, request)))
                    }),
                    Some(reply),
                );
            }
            None => {
                let _ = reply.send(self.handle(id, request));
            }
        }
    }

    /// 主循环收后台任务交回的 [`crate::tasks::Apply`]，在引擎线程执行。
    pub fn set_apply_channel(&mut self, apply: std::sync::mpsc::Sender<crate::tasks::Apply>) {
        self.tasks.set_apply_channel(apply);
    }

    /// 没有主循环时（测试直接调用 [`Host::handle`]）在当前线程依次执行；先等已排队的登记做完，保持顺序。
    pub(super) fn run_inline(&mut self, request: &Request) -> Response {
        self.tasks.wait_idle();
        match self.sync_job(request) {
            Some(Ok(job)) => match job() {
                Step::Reply(response) => response,
                Step::Apply(apply) => apply(self),
                Step::Done => Response::ok(),
            },
            Some(Err(response)) => response,
            None => Response::error("不是同步请求"),
        }
    }

    /// 修改成功后：记下修改时间；开启了同步记录时登记这次变化。
    /// 登记要比较整份快照和同步记录（数十到上百毫秒），放到后台任务线程，失败只写日志，
    /// 下次导出仍会按当时的数据补登。
    pub(super) fn after_edit(
        &mut self,
        response: Response,
        mark: bool,
        record: bool,
        user_restore: bool,
    ) -> Response {
        if !response.ok {
            return response;
        }
        if mark {
            self.mark_sync_change();
        }
        if record && (user_restore || self.store.dir().join("sync-records.json").is_file()) {
            let snapshot = self
                .engine
                .as_deref()
                .ok_or_else(|| "词库尚未就绪".to_string())
                .and_then(|engine| {
                    backup::snapshot(
                        &self.store,
                        &self.config,
                        self.mode,
                        &self.recent_emoji,
                        engine,
                    )
                });
            match snapshot {
                Ok((text, _)) => {
                    let (store, stamp) = (
                        Store::new(self.store.dir().to_path_buf()),
                        self.sync_modified_ms,
                    );
                    self.tasks.submit(
                        Box::new(move || {
                            let recorded = serde_json::from_str::<serde_json::Value>(&text)
                                .map_err(|e| e.to_string())
                                .and_then(|mut snapshot| {
                                    crate::sync_merge::attach(
                                        &store,
                                        &mut snapshot,
                                        stamp,
                                        user_restore,
                                    )
                                });
                            if let Err(error) = recorded {
                                crate::log(&format!("操作已保存，但同步变更记录失败：{error}"));
                            }
                            Step::Done
                        }),
                        None,
                    );
                }
                Err(error) => crate::log(&format!("操作已保存，但同步变更记录失败：{error}")),
            }
        }
        response
    }

    pub(super) fn reset_sync_records(&mut self) -> Response {
        if self.clients.values().any(|client| client.state.busy()) {
            return Response::error("正在输入，请结束输入后再重置同步记录");
        }
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        let (text, _) = match backup::snapshot(
            &self.store,
            &self.config,
            self.mode,
            &self.recent_emoji,
            engine,
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => return Response::error(error),
        };
        let mut snapshot: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => return Response::error(error.to_string()),
        };
        let stamp = match backup::modified_unix_ms(&self.store, &self.config) {
            Ok(mtime) => wowtype_core::now_ms().max(mtime).max(self.sync_modified_ms),
            Err(error) => return Response::error(error),
        };
        if let Err(error) = crate::sync_merge::reset(&self.store, &mut snapshot, stamp) {
            return Response::error(error);
        }
        Response::ok()
    }

    pub(super) fn import_sync_settings(
        &mut self,
        path: &Path,
        expected_path: &Path,
        ignore_records: bool,
    ) -> Response {
        if self.clients.values().any(|client| client.state.busy()) {
            return Response::error("正在输入，自动同步将在稍后重试");
        }
        let Some(engine) = self.engine.as_deref() else {
            return Response::error("词库还在载入，请稍后再试");
        };
        let current = match backup::snapshot(
            &self.store,
            &self.config,
            self.mode,
            &self.recent_emoji,
            engine,
        ) {
            Ok((text, _)) => text,
            Err(error) => return Response::error(error),
        };
        let expected = match std::fs::read_to_string(expected_path) {
            Ok(text) => text,
            Err(error) => return Response::error(format!("读取同步比较快照失败：{error}")),
        };
        let actual: serde_json::Value = match serde_json::from_str(&current) {
            Ok(value) => value,
            Err(error) => return Response::error(error.to_string()),
        };
        let expected: serde_json::Value = match serde_json::from_str(&expected) {
            Ok(value) => value,
            Err(error) => return Response::error(error.to_string()),
        };
        if crate::sync_merge::without_records(actual)
            != crate::sync_merge::without_records(expected)
        {
            return Response::error("本机数据在下载期间已修改，将重新比较版本");
        }
        // 检查和恢复在同一宿主请求中完成，期间不处理其他输入或设置修改。
        if ignore_records {
            let prepared = match backup::read_ignoring_records(path) {
                Ok(prepared) => prepared,
                Err(error) => return Response::error(error),
            };
            let response = self.import_prepared(prepared);
            if response.ok {
                let _ = std::fs::remove_file(self.store.dir().join("sync-records.json"));
            }
            return response;
        }
        self.import_settings(path)
    }
}
