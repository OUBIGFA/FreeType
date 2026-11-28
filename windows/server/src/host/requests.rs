//! 管道请求分发：区分输入会话与设置程序请求。
use super::*;

impl Host {
    /// 处理一条请求；里面出了 panic 时记日志、丢掉这个连接的输入状态，不让服务进程退出。
    pub fn handle(&mut self, id: ClientId, request: Request) -> Response {
        let trace_was_enabled = self.config.candidate_diagnostics;
        let trace_mode = self.effective(self.mode_of(&self.app_of(id))) == Mode::Chinese;
        if let Some(client) = self.clients.get_mut(&id) {
            client.state.session.set_trace(
                self.config.candidate_diagnostics
                    && trace_mode
                    && client.state.tool.is_none()
                    && client.state.raw.is_none()
                    && client.state.lookup.is_none(),
            );
        }
        let may_edit = matches!(
            &request,
            Request::SetConfig { .. }
                | Request::SetQuickPhrases { .. }
                | Request::ImportQuickPhrases { .. }
                | Request::SetCommonPhrases { .. }
                | Request::ImportCommonPhrases { .. }
                | Request::ImportDictionary { .. }
                | Request::RemoveDictionary { .. }
                | Request::UpdateDictionary { .. }
                | Request::DeleteDictionaryEntries { .. }
                | Request::ImportSyncSettings { .. }
                | Request::ResetSyncRecords
                | Request::ImportLearning { .. }
                | Request::ImportSettings { .. }
                | Request::MergeSettings { .. }
                | Request::ForgetWords { .. }
                | Request::AddUserWord { .. }
                | Request::ClearLearned
                | Request::ToggleMode { .. }
                | Request::Command { .. }
                | Request::Reload
        );
        let record_edit = may_edit
            && !matches!(
                &request,
                Request::ResetSyncRecords
                    | Request::ToggleMode { .. }
                    | Request::Command {
                        command: Command::OpenSettings
                            | Command::Toolbar
                            | Command::EmojiPanel
                            | Command::EnglishCandidates
                            | Command::RadicalLookup
                    }
            );
        let user_restore = matches!(
            &request,
            Request::ImportSettings { .. }
                | Request::ImportSyncSettings { .. }
                | Request::ImportLearning { .. }
        );
        let scan = match &request {
            Request::PhysicalKey { key, scan, .. } if !key.test => Some(*scan),
            _ => None,
        };
        let preview = matches!(
            &request,
            Request::Key {
                key: Key { test: true, .. },
                ..
            } | Request::PhysicalKey {
                key: Key { test: true, .. },
                ..
            }
        );
        let actual = matches!(
            &request,
            Request::Key {
                key: Key { test: false, .. },
                ..
            } | Request::PhysicalKey {
                key: Key { test: false, .. },
                ..
            } | Request::Select { .. }
                | Request::CandidateAction {
                    action: CandidateAction::Select { .. },
                    ..
                }
        );
        if matches!(
            &request,
            Request::SensitiveFocus { active: true } | Request::Focus { active: false, .. }
        ) {
            self.stats.break_activity();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.handle_inner(id, request)
        }));
        match result {
            Ok(mut response) => {
                if !preview {
                    if trace_was_enabled && !self.config.candidate_diagnostics {
                        for client in self.clients.values_mut() {
                            client.state.session.set_trace(false);
                        }
                    } else if self.config.candidate_diagnostics {
                        if let Some(client) = self.clients.get_mut(&id) {
                            self.diagnostics.record(
                                client.state.session.take_trace(),
                                &client.state.app,
                                id,
                            );
                        }
                    }
                }
                if !preview {
                    if let (Some(frame), Some(client)) =
                        (&mut response.frame, self.clients.get_mut(&id))
                    {
                        if frame.candidates.is_empty() {
                            client.revision = 0;
                        } else if frame.handled || client.revision == 0 {
                            self.next_page_revision =
                                self.next_page_revision.wrapping_add(1).max(1);
                            client.revision = self.next_page_revision;
                        }
                        frame.page_revision = client.revision;
                    }
                }
                if let (Some(client), Some(frame)) = (self.clients.get(&id), &mut response.frame) {
                    frame.pair_enabled =
                        frame.mode == Mode::Chinese && client.state.pairs_enabled(&self.config);
                    frame.pair_absorb_closing =
                        frame.pair_enabled && self.config.auto_pair_absorb_closing;
                }
                let learned = self
                    .clients
                    .get_mut(&id)
                    .is_some_and(|client| client.state.session.take_learning_change());
                self.dirty |= learned;
                // 管理操作不在逐键热路径；已开启记录同步后登记增删，避免离线
                // 很久才上传时把旧编辑时间误当成本次上传时间。学习本身另有使用时间。
                // 合并同步在应用结果后自行登记，不经过这里。
                let response =
                    self.after_edit(response, may_edit || learned, record_edit, user_restore);
                // 自动同步等用户停手后再占用引擎线程，见 Status::input_idle_seconds。
                if actual {
                    self.last_input = Some(std::time::Instant::now());
                }
                if self.config.typing_stats
                    && actual
                    && self
                        .clients
                        .get(&id)
                        .is_some_and(|client| !client.state.session.learning_disabled())
                {
                    if let Some(frame) = response.frame.as_ref().filter(|frame| frame.handled) {
                        self.stats.record(
                            id,
                            scan,
                            frame.commit.as_deref(),
                            frame.selected_rank,
                            self.config.stats_retention,
                        );
                    }
                }
                response
            }
            Err(_) => {
                crate::log(&format!("处理请求时出错（连接 {id}），已重置这个输入框"));
                let engine = self.engine_ref();
                if let Some(client) = self.clients.get_mut(&id) {
                    let app = client.state.app.clone();
                    client.pending = None;
                    client.state = ClientState::new(engine, &self.config, app);
                }
                Response::error("服务进程内部出错，已重置这个输入框")
            }
        }
    }

    pub(super) fn handle_inner(&mut self, id: ClientId, request: Request) -> Response {
        if let Some(action) = crate::candidate_diagnostics::action(&request) {
            return self.diagnostics.request_inline(action);
        }
        match request {
            Request::GetCandidateDiagnostics
            | Request::ClearCandidateDiagnostics
            | Request::ExportCandidateDiagnostics { .. } => unreachable!("handled above"),
            Request::Hello { session, app } => {
                if session && !self.clients.contains_key(&id) {
                    self.connect(id, app.to_ascii_lowercase());
                }
                Response {
                    status: Some(self.status()),
                    ..Response::ok()
                }
            }
            Request::Key { key, before } => self.key(id, key, before),
            Request::PhysicalKey {
                key,
                before,
                shown_revision,
                ..
            } => {
                let selects = matches!(key.keysym,
                    keysym::SPACE_DOUBLE | 0x20 | 0x27 | 0x3b | 0x5b | 0x5d | 0x30..=0x39 | keysym::KP_0..=keysym::KP_9
                ) || (self
                    .clients
                    .get(&id)
                    .is_some_and(|client| client.state.session.calc_active())
                    && matches!(key.keysym, 0x41..=0x5a | 0x61..=0x7a | keysym::RETURN | keysym::KP_ENTER | 0x24 | 0xa5 | 0xffe5));
                if selects
                    && shown_revision.is_some_and(|revision| {
                        self.clients.get(&id).is_some_and(|client| {
                            client.revision != revision && (client.state.busy() || revision != 0)
                        })
                    })
                {
                    return self.with_client(id, |state, env| state.current(true, env));
                }
                self.key(id, key, before)
            }
            Request::CandidateAction { revision, action } => {
                self.candidate_action(id, revision, action)
            }
            Request::ListTypingStats => match self.stats.snapshot(self.config.stats_retention) {
                Ok(days) => Response {
                    typing_stats: Some(days),
                    ..Response::ok()
                },
                Err(error) => Response::error(error),
            },
            Request::ClearTypingStats => match self.stats.clear() {
                Ok(()) => Response::ok(),
                Err(error) => Response::error(error),
            },
            Request::SensitiveFocus { active } => {
                if active && self.focused == Some(id) {
                    self.focused = None;
                }
                let response = self.with_client(id, |state, env| {
                    state.set_sensitive(active);
                    state.current(false, env)
                });
                self.notify_bar();
                response
            }
            Request::ClosePair => self.with_client(id, |state, env| state.close_pair(env)),
            Request::UndoCommitLearning => {
                let changed = self.clients.get_mut(&id).is_some_and(|client| {
                    client.pending = None;
                    client.state.dismiss_prediction();
                    client.state.session.undo_commit_learning()
                });
                if changed {
                    self.dirty = true;
                    self.mark_sync_change();
                }
                Response::ok()
            }
            Request::Select { index } => {
                self.with_client(id, |state, env| state.select(index, env))
            }
            Request::Page { next } => self.with_client(id, |state, env| state.page(next, env)),
            Request::Forget { index } => self.forget(id, index),
            Request::Reset { commit } => {
                self.with_client(id, |state, env| state.reset(commit, env))
            }
            Request::Focus { active, before } => self.focus(id, active, before),
            Request::ToggleMode { shift } => self.toggle_mode(id, shift),
            Request::Command { command } => self.command(id, command),
            Request::CapsLock { on } => self.caps_lock(id, on),
            Request::Status => Response {
                status: Some(self.status()),
                ..Response::ok()
            },
            Request::ListApps => Response {
                apps: Some(
                    self.desktop
                        .as_ref()
                        .map(|desktop| desktop.open_apps())
                        .unwrap_or_default(),
                ),
                ..Response::ok()
            },
            Request::GetConfig => self.config_response(),
            Request::GetBrandIcon { known_revision } => Response {
                brand_icon: (known_revision != Some(self.brand_icon_revision)).then(|| BrandIcon {
                    revision: self.brand_icon_revision,
                    appearance: self.config.brand_icon.clone(),
                    custom_png: if self.config.brand_icon == "custom" {
                        self.config.custom_brand_icon.clone()
                    } else {
                        String::new()
                    },
                }),
                ..Response::ok()
            },
            Request::SetNeuralModel { path } => self.change_neural_model(path),
            Request::DictionaryUpdateStatus => self.dictionary_update_response(Ok(())),
            Request::StageDictionaryUpdate { version, automatic } => {
                let root = crate::dictionary_updates::root();
                let result = crate::dictionary_updates::stage(&root, &version, automatic);
                self.dictionary_update_response(result)
            }
            Request::RestorePreviousDictionary => {
                let result =
                    crate::dictionary_updates::restore_previous(&crate::dictionary_updates::root());
                self.dictionary_update_response(result)
            }
            Request::CancelPendingDictionary { automatic_only } => {
                let result = crate::dictionary_updates::cancel_pending(
                    &crate::dictionary_updates::root(),
                    automatic_only,
                );
                self.dictionary_update_response(result)
            }
            Request::EnhancementStatus => Response {
                enhancement: Some(self.enhancement.status()),
                ..Response::ok()
            },
            Request::SetEnhancement { enabled } => {
                let dir = self.store.dir().join("enhancement");
                if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| {
                    crate::store::write_atomic(
                        &dir.join("enabled.txt"),
                        if enabled { "1" } else { "0" },
                    )
                }) {
                    return Response::error(e.to_string());
                }
                self.enhancement.enable(enabled);
                if let Some(engine) = self.engine.as_deref() {
                    engine.set_enhancement(enabled.then(|| {
                        self.enhancement.clone() as Arc<dyn wowtype_core::enhancement::Backend>
                    }));
                }
                Response {
                    enhancement: Some(self.enhancement.status()),
                    ..Response::ok()
                }
            }
            Request::RefreshEnhancement { revision } => {
                if self
                    .clients
                    .get(&id)
                    .is_some_and(|c| c.revision == revision && c.state.session.composing())
                {
                    self.with_client(id, |state, env| {
                        let changed = state.session.refresh_enhancement();
                        state.current(changed, env)
                    })
                } else {
                    Response::ok()
                }
            }
            Request::SetConfig { config } => match self.save_config(config) {
                Ok(()) => self.config_response(),
                Err(e) => Response::error(e),
            },
            Request::ListQuickPhrases => Response {
                quick_phrases: Some(self.store.list_quick_phrases()),
                ..Response::ok()
            },
            Request::SetQuickPhrases { phrases } => {
                if let Err(error) = self.store.set_quick_phrases(&phrases) {
                    return Response::error(error);
                }
                self.apply_quick_phrases();
                Response::ok()
            }
            Request::ImportQuickPhrases { path, replace } => {
                let response =
                    Self::report(self.store.import_quick_phrases(Path::new(&path), replace));
                if response.ok {
                    self.apply_quick_phrases();
                }
                response
            }
            Request::ExportQuickPhrases { path } => {
                match self.store.export_quick_phrases(Path::new(&path)) {
                    Ok(count) => Response {
                        report: Some(ImportReport {
                            added: count,
                            ..ImportReport::default()
                        }),
                        ..Response::ok()
                    },
                    Err(error) => Response::error(error),
                }
            }
            Request::ListDictionaries => Response {
                dictionaries: Some(self.store.list_dictionaries()),
                ..Response::ok()
            },
            Request::ListCommonPhrases => match self.store.load_common_phrases() {
                Ok(phrases) => Response {
                    common_phrases: Some(phrases),
                    ..Response::ok()
                },
                Err(error) => Response::error(error),
            },
            Request::SetCommonPhrases { phrases } => {
                if let Err(error) = self.store.set_common_phrases(phrases) {
                    return Response::error(error);
                }
                self.load_phrases();
                Response::ok()
            }
            Request::ImportCommonPhrases { path, replace } => {
                let result = self.store.import_common_phrases(Path::new(&path), replace);
                if result.is_ok() {
                    self.load_phrases();
                }
                Self::report(result)
            }
            Request::ExportCommonPhrases { path } => Self::report(
                self.store
                    .export_common_phrases(Path::new(&path))
                    .map(|added| ImportReport {
                        added,
                        ..ImportReport::default()
                    }),
            ),
            Request::ImportDictionary { path, name } => {
                let reading = |words: &[String]| self.reading(words);
                let response = Self::report(self.store.import_dictionary(
                    Path::new(&path),
                    &name,
                    &reading,
                ));
                if response.ok {
                    self.refresh_dictionary_candidates();
                }
                response
            }
            Request::RemoveDictionary { name } => {
                if let Err(error) = self.store.remove_dictionary(&name) {
                    return Response::error(error);
                }
                self.refresh_dictionary_candidates();
                Response::ok()
            }
            Request::QueryDictionary {
                name,
                query,
                offset,
                limit,
            } => match self.store.query_dictionary(&name, &query, offset, limit) {
                Ok(page) => Response {
                    dictionary_page: Some(page),
                    ..Response::ok()
                },
                Err(error) => Response::error(error),
            },
            Request::UpdateDictionary {
                name,
                new_name,
                enabled,
                description,
            } => {
                if let Err(error) =
                    self.store
                        .update_dictionary(&name, &new_name, enabled, &description)
                {
                    return Response::error(error);
                }
                self.refresh_dictionary_candidates();
                Response {
                    dictionaries: Some(self.store.list_dictionaries()),
                    ..Response::ok()
                }
            }
            Request::DeleteDictionaryEntries { name, entries } => {
                let removed = match self.store.delete_dictionary_entries(&name, &entries) {
                    Ok(n) => n,
                    Err(error) => return Response::error(error),
                };
                self.refresh_dictionary_candidates();
                Self::report(Ok(ImportReport {
                    added: removed,
                    ..ImportReport::default()
                }))
            }
            Request::ExportUserDictionary { path } => {
                let Some(engine) = self.engine.as_deref() else {
                    return Response::error("词库还在载入，请稍后再试");
                };
                let entries: Vec<msime::DictEntry> = engine
                    .lexicon()
                    .words()
                    .into_iter()
                    .map(|(pinyin, word, freq)| msime::DictEntry {
                        word,
                        pinyin,
                        weight: freq.clamp(1.0, f64::from(u32::MAX)) as u32,
                    })
                    .collect();
                match crate::store::write_atomic(
                    Path::new(&path),
                    &msime::format_dictionary(&entries),
                ) {
                    Ok(()) => Response {
                        report: Some(ImportReport {
                            added: entries.len(),
                            ..ImportReport::default()
                        }),
                        ..Response::ok()
                    },
                    Err(e) => Response::error(format!("写入文件失败：{e}")),
                }
            }
            Request::ExportLearning { path } => self.export_learning(Path::new(&path)),
            Request::ImportLearning { path } => self.import_learning(Path::new(&path)),
            Request::ExportSettings { path, parts } => self.export_settings(
                Path::new(&path),
                parts.as_deref().unwrap_or(&BackupPart::ALL),
            ),
            request @ Request::ExportSyncSettings { .. } => self.run_inline(&request),
            // 先等后台登记做完，同步记录只在一处读写；经主循环时已排在后台任务之后，这里不再等待。
            Request::ImportSyncSettings {
                path,
                expected_path,
                ignore_records,
            } => {
                self.tasks.wait_idle();
                self.import_sync_settings(
                    Path::new(&path),
                    Path::new(&expected_path),
                    ignore_records,
                )
            }
            Request::ResetSyncRecords => {
                self.tasks.wait_idle();
                self.reset_sync_records()
            }
            Request::InspectSettingsBackup { path } => {
                Self::report(backup::read(Path::new(&path)).map(|data| data.report()))
            }
            Request::ImportSettings { path } => self.import_settings(Path::new(&path)),
            request @ (Request::PreviewSyncMerge { .. } | Request::MergeSyncSettings { .. }) => {
                self.run_inline(&request)
            }
            Request::MergeSettings { path } => self.merge_settings(Path::new(&path)),
            Request::ListUserWords => Response {
                user_words: Some(self.user_words()),
                ..Response::ok()
            },
            Request::ForgetWords { words } => {
                let Some(engine) = self.engine.as_deref() else {
                    return Response::error("词库还在载入，请稍后再试");
                };
                let removed = words.iter().filter(|w| engine.forget_word(w)).count();
                if let Err(error) = self.learned_changed() {
                    return Response::error(error);
                }
                Response {
                    report: Some(ImportReport {
                        added: removed,
                        ..ImportReport::default()
                    }),
                    user_words: Some(self.user_words()),
                    ..Response::ok()
                }
            }
            Request::AddUserWord { word, pinyin } => self.add_user_word(word, pinyin),
            Request::ClearLearned => {
                let Some(engine) = self.engine.as_deref() else {
                    return Response::error("词库还在载入，请稍后再试");
                };
                engine.clear_learned();
                if let Err(error) = self.learned_changed() {
                    return Response::error(error);
                }
                Response {
                    user_words: Some(Vec::new()),
                    ..Response::ok()
                }
            }
            Request::ClearClipboard => {
                wowtype_core::clipboard::clear();
                Response::ok()
            }
            Request::Shutdown => {
                if let Err(error) = self.stats.save(true) {
                    crate::log(&error);
                }
                for client in self.clients.values_mut() {
                    client.state.clear();
                    self.dirty |= client.state.session.take_learning_change();
                }
                if let Err(error) = self.saver.result(true) {
                    crate::log(&error);
                    self.dirty = true;
                }
                if self.dirty && self.engine.is_some() {
                    if let Err(error) = self.save_learning() {
                        return Response::error(error);
                    }
                }
                Response::ok()
            }
            Request::Reload => {
                let config = self.store.load_config();
                self.set_config(config);
                self.apply_user_data();
                self.lexicon_change = Some(self.config.lexicon.clone());
                Response {
                    status: Some(self.status()),
                    ..Response::ok()
                }
            }
        }
    }
}
