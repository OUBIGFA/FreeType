//! 按服务进程的应答改写文档、显示候选窗，并安排成对标点的光标移动。
use super::*;

pub(super) fn replace_prediction_space(
    state: &Rc<RefCell<State>>,
    context: &ITfContext,
    ec: u32,
    text: &[u16],
) -> Result<ITfRange> {
    let pending = state.borrow().space.clone();
    let failed = || windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL);
    let pending = pending
        .filter(|pending| same_object(&pending.context, context))
        .ok_or_else(failed)?;
    let selection = selection_range(context, ec).ok_or_else(failed)?;
    unsafe {
        let range = pending.composition.GetRange()?;
        if !selection.IsEmpty(ec)?.as_bool()
            || selection.CompareStart(ec, &range, TF_ANCHOR_END)? != 0
        {
            return Err(failed());
        }
        range.SetText(ec, 0, text)?;
        range.Collapse(ec, TF_ANCHOR_END)?;
        take_prediction_space(state);
        pending.composition.EndComposition(ec)?;
        Ok(range)
    }
}

/// 订这个文档的排版变化（浏览器类应用打字时还没排好版，排好了再把候选窗挪过去）。
pub(super) fn watch_layout(state: &Rc<RefCell<State>>, context: &ITfContext) {
    let (current, sink) = {
        let s = state.borrow();
        (
            s.layout_cookie.as_ref().map(|(c, _)| c.clone()),
            s.layout_sink.clone(),
        )
    };
    if current.as_ref() == Some(context) {
        return;
    }
    unwatch_layout(state);
    let Some(sink) = sink else {
        return;
    };
    unsafe {
        if let Ok(source) = context.cast::<ITfSource>() {
            if let Ok(cookie) = source.AdviseSink(&ITfTextLayoutSink::IID, &sink) {
                state.borrow_mut().layout_cookie = Some((context.clone(), cookie));
            }
        }
    }
}

pub(super) fn unwatch_layout(state: &Rc<RefCell<State>>) {
    let old = state.borrow_mut().layout_cookie.take();
    if let Some((context, cookie)) = old {
        unsafe {
            if let Ok(source) = context.cast::<ITfSource>() {
                let _ = source.UnadviseSink(cookie);
            }
        }
    }
}

/// 在编辑会话里按应答改文档：先上屏，再更新编辑区。返回候选窗该放的位置。
pub(super) fn apply_document(
    state: &Rc<RefCell<State>>,
    context: &ITfContext,
    frame: &Frame,
    ec: u32,
) -> Result<()> {
    let (mut composition, sink) = {
        let s = state.borrow();
        (s.composition.clone(), s.sink.clone())
    };
    let wants_window =
        frame.hint.is_some() || !frame.candidates.is_empty() || frame.title.is_some();
    let result = (|| -> Result<Option<RECT>> {
        unsafe {
            if let Some(text) = &frame.commit {
                let text = wide(text);
                let range = if frame.replace_space {
                    replace_prediction_space(state, context, ec, &text)?
                } else if let Some(comp) = composition.take() {
                    let range = comp.GetRange()?;
                    range.SetText(ec, 0, &text)?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    comp.EndComposition(ec)?;
                    range
                } else {
                    let insert: ITfInsertAtSelection = context.cast()?;
                    let range = insert.InsertTextAtSelection(
                        ec,
                        INSERT_TEXT_AT_SELECTION_FLAGS(0),
                        &text,
                    )?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    range
                };
                // Chromium 系与 CUAS 的临时文档由宿主自己把光标放在上屏文字末尾。它们的
                // TSF 选区可能落后于网页里的真实光标（如 Codex 编辑已发送消息时，成对标点
                // 后模拟的 ← 没同步到 TSF）；再按旧位置设选区，会把光标拉到右半边标点外。
                // 联想上屏仍要靠这次选区定位下一次空格，照旧设置。
                if frame.replace_space || frame.predict || !transitory(context) {
                    set_selection(context, ec, &range)?;
                }
                if frame.predict {
                    // Chromium may replay the renderer's committed suffix after
                    // the edit lock ends. Follow its final insertion to the right;
                    // the next first-space key creates its own replacement range.
                    let anchor = range.Clone().and_then(|caret| {
                        caret.SetGravity(ec, TF_GRAVITY_FORWARD, TF_GRAVITY_FORWARD)?;
                        Ok(caret)
                    });
                    let mut s = state.borrow_mut();
                    s.prediction_caret = anchor.ok();
                }
                if frame.caret_back == 1 && frame.pair_enabled {
                    if let Some(pair) = frame
                        .commit
                        .as_ref()
                        .and_then(|text| pair::Pair::after_commit(context, &range, ec, text))
                    {
                        let mut s = state.borrow_mut();
                        if s.pairs.len() == 16 {
                            s.pairs.remove(0);
                        }
                        s.pairs.push(pair);
                    }
                }
                // Record a short, transient suffix only after TSF accepted the commit.
                if frame.undo_learning && frame.caret_back == 0 {
                    let watch = before_text(context, ec).and_then(|before| {
                        let (start, _) = before.grapheme_indices(true).next_back()?;
                        (start > 0).then(|| CommitWatch {
                            context: context.clone(),
                            shortened: before[..start].to_owned(),
                            before,
                            armed: false,
                        })
                    });
                    state.borrow_mut().undo = watch;
                }
            }
            if !frame.preedit.is_empty() {
                let comp = match composition.take() {
                    Some(comp) => comp,
                    None => {
                        let insert: ITfInsertAtSelection = context.cast()?;
                        let range = insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
                        let start: ITfContextComposition = context.cast()?;
                        start.StartComposition(ec, &range, sink.as_ref())?
                    }
                };
                let range = comp.GetRange()?;
                let preedit = frame.inline_preedit.as_deref().unwrap_or(&frame.preedit);
                range.SetText(ec, 0, &wide(preedit))?;
                let caret = range.Clone()?;
                caret.Collapse(ec, TF_ANCHOR_START)?;
                let mut moved = 0;
                let shift = if frame.inline_preedit.is_some() {
                    frame.inline_cursor
                } else {
                    frame.preedit_cursor
                } as i32;
                caret.ShiftEnd(ec, shift, &mut moved, std::ptr::null())?;
                caret.ShiftStart(ec, shift, &mut moved, std::ptr::null())?;
                set_selection(context, ec, &caret)?;
                let rect = text_rect(context, ec, &range);
                composition = Some(comp);
                return Ok(rect.or_else(|| selection_rect(context, ec)));
            }
            if let Some(comp) = composition.take() {
                // 编辑区清空了（Esc、退格删完）
                let range = comp.GetRange()?;
                range.SetText(ec, 0, &[])?;
                comp.EndComposition(ec)?;
            }
            if let Some(action) = frame.pair_edit {
                let pair = state.borrow_mut().pairs.pop();
                if let Some(pair) = pair {
                    pair.apply(context, ec, action)?;
                }
            }
            Ok(if wants_window || frame.caret_back != 0 {
                selection_rect(context, ec)
            } else {
                None
            })
        }
    })();
    let mut s = state.borrow_mut();
    s.composition = composition;
    match result {
        Ok(rect) => {
            if rect.is_some() {
                s.last_caret = rect;
            }
            s.caret = rect;
            Ok(())
        }
        Err(e) => {
            s.caret = None;
            Err(e)
        }
    }
}

/// 记下应答里的状态（中英文、要不要光标前的字），更新任务栏按钮。
pub(super) fn note_frame(state: &Rc<RefCell<State>>, frame: &Frame) {
    note_preview(state, frame);
    let mut s = state.borrow_mut();
    s.predicting = frame.predict && !frame.candidates.is_empty();
    s.predict_double_space = s.predicting && frame.predict_double_space;
    if !s.predicting {
        s.prediction_caret = None;
    }
    s.pair_enabled = frame.pair_enabled;
    s.pair_absorb_closing = frame.pair_absorb_closing;
    if !frame.pair_enabled {
        s.pairs.clear();
    }
    s.busy = !s.predicting
        && (!frame.preedit.is_empty() || !frame.candidates.is_empty() || frame.title.is_some());
}

pub(super) fn clear_prediction(state: &Rc<RefCell<State>>) -> bool {
    finish_prediction_space(state, true, false);
    let mut s = state.borrow_mut();
    if !std::mem::take(&mut s.predicting) {
        return false;
    }
    s.predict_double_space = false;
    s.prediction_caret = None;
    drop(s);
    with_window(state, |window| window.hide());
    true
}

/// 服务进程那边也要收起，不然看不见的联想词还会吃掉数字键和空格。
pub(super) fn dismiss_prediction(state: &Rc<RefCell<State>>) {
    if clear_prediction(state) {
        if let Some(frame) = call(state, &Request::Reset { commit: false }) {
            note_frame(state, &frame);
        }
    }
}

/// OnTestKeyDown 只询问是否接收按键，预判结果不能提前改变真实组字状态。
pub(super) fn note_preview(state: &Rc<RefCell<State>>, frame: &Frame) {
    let button = {
        let mut s = state.borrow_mut();
        s.mode = frame.mode;
        s.want_before = frame.want_before;
        s.button.clone()
    };
    if let Some(button) = button {
        button.set_mode(frame.mode);
    }
}

/// 候选窗的 Win32 调用（摆位置、显示、隐藏、改所属窗口）会同步发消息给宿主窗口，
/// 宿主（如记事本的 RichEdit）可能当场回调 TSF。调用期间不持有 State 借用：先取出
/// 窗口，用完放回；期间重入的回调看到「没有候选窗」，不会重复借用。
pub(super) fn with_window<R>(
    state: &Rc<RefCell<State>>,
    f: impl FnOnce(&mut CandidateWindow) -> R,
) -> Option<R> {
    let (mut window, generation) = {
        let Ok(mut s) = state.try_borrow_mut() else {
            crate::log::write("候选窗调用重入", "已跳过");
            return None;
        };
        (s.window.take()?, s.generation)
    };
    let result = f(&mut window);
    let stale = {
        let mut s = state.borrow_mut();
        if s.generation == generation && s.window.is_none() {
            s.window = Some(window);
            None
        } else {
            Some(window)
        }
    };
    // 期间停用或重建了候选窗：旧窗口在借用之外销毁。
    drop(stale);
    Some(result)
}

/// 候选窗按应答更新；应用给不出位置时用上一次的或系统记的插入符。
pub(super) fn show_window(state: &Rc<RefCell<State>>, frame: &Frame) {
    let wants = frame.hint.is_some() || !frame.candidates.is_empty() || frame.title.is_some();
    // GetWnd belongs to the host and can reenter TSF: release our state borrow
    // before calling it. Microsoft IME requirements specify GetFocus as fallback.
    // https://learn.microsoft.com/windows/apps/develop/input/input-method-editor-requirements#owned-window
    let context = state.borrow().context.clone();
    let owner = wants.then(|| unsafe {
        context
            .as_ref()
            .and_then(|context| context.GetActiveView().and_then(|view| view.GetWnd()).ok())
            .filter(|window| !window.is_invalid())
            .unwrap_or_else(|| GetFocus())
    });
    let caret = {
        let s = state.borrow();
        if wants {
            s.caret.or(s.last_caret).or_else(fallback_caret)
        } else {
            s.caret
        }
    };
    let shown = with_window(state, |window| {
        if let Some(owner) = owner {
            if let Err(error) = window.set_owner(owner) {
                crate::log::write("关联候选窗失败", &error.to_string());
            }
        }
        window.update(frame, caret);
    });
    let unseen = wants && frame.predict && (caret.is_none() || shown.is_none());
    if unseen {
        // 不知道光标在哪、联想词摆不出来：不留看不见的联想
        dismiss_prediction(state);
    }
}

/// 应答落到文档和候选窗上。
pub(super) fn apply(
    state: &Rc<RefCell<State>>,
    context: Option<&ITfContext>,
    frame: Frame,
    sync: bool,
) {
    let expired = {
        let service = state.borrow();
        service.caret_move.is_some_and(|motion| {
            service
                .window
                .as_ref()
                .is_none_or(|window| window.hwnd().is_invalid())
                || motion.step(now_ms(), true, false) == keys::CaretStep::Drop
        })
    };
    if expired {
        pair::finish(state, true);
    }
    let deferred = {
        let mut service = state.borrow_mut();
        service.caret_move.is_some() && context.is_some() && service.pair_delivery.defer(&frame)
    };
    if deferred {
        note_frame(state, &frame);
        show_window(state, &frame);
        return;
    }
    // 鼠标选词、切换模式等也先落实单次空格；只有双击选词替换这个编辑区。
    if !frame.replace_space {
        finish_prediction_space(state, true, sync);
    }
    note_frame(state, &frame);
    let Some(context) = context else {
        state.borrow_mut().caret = None;
        show_window(state, &frame);
        return;
    };
    if !frame.preedit.is_empty() {
        watch_layout(state, context);
    }
    let client_id = state.borrow().client_id;
    let shared = Rc::new(frame);
    if let Some(pair) = pair::prepare_commit(state, context, &shared, client_id) {
        pair::commit_prepared(state, pair, shared, client_id);
        return;
    }
    if sync {
        let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
        edit(context, client_id, true, move |ec| {
            let result = apply_document(&state2, &context2, &frame2, ec);
            if let Err(error) = &result {
                crate::log::write("更新文档失败", &error.to_string());
                if frame2.undo_learning {
                    let _ = call(&state2, &Request::UndoCommitLearning);
                }
            } else {
                caret_back(&state2, &frame2);
            }
            result
        });
        show_window(state, &shared);
    } else {
        // 异步：编辑会话里改完文档再摆候选窗
        let (state2, context2, frame2) = (state.clone(), context.clone(), shared.clone());
        let generation = state.borrow().generation;
        edit(context, client_id, false, move |ec| {
            if state2.borrow().generation != generation {
                return Ok(());
            }
            let result = apply_document(&state2, &context2, &frame2, ec);
            if result.is_ok() {
                caret_back(&state2, &frame2);
            }
            show_window(&state2, &frame2);
            result
        });
    }
}

/// 成对标点：上屏以后让应用收到几下 ←，光标停在两半中间（和搜狗一样；
/// 直接改选区的话，有的应用会在编辑结束时把光标挪回末尾）。
///
/// 这里只记下来、请候选窗回到消息循环以后叫 [`caret_tick`]，不当场发：见 [`keys::CaretMove`]。
pub(super) fn caret_back(state: &Rc<RefCell<State>>, frame: &Frame) {
    if frame.commit.is_none() || frame.caret_back == 0 || !frame.preedit.is_empty() {
        return;
    }
    let mut s = state.borrow_mut();
    if s.window
        .as_ref()
        .is_some_and(CandidateWindow::request_caret_tick)
    {
        s.caret_move = Some(keys::CaretMove::new(frame.caret_back, now_ms()));
        s.pair_delivery.sent = false;
        s.pair_delivery.released = false;
    } else {
        crate::log::write("发送辅助按键失败", "无法安排成对标点的光标移动");
    }
}

/// 等着的 [`keys::CaretMove`]：能挪就挪。返回 true 表示修饰键还按着，过一会儿再来。
///
/// 两处会叫它：候选窗的消息和定时器（[`ui::set_caret_handler`]），以及修饰键松开的时候
/// （[`key_released`]：用户一松开 Shift 就发，赶在下一个字前面）。
pub(super) fn caret_tick(state: &Rc<RefCell<State>>) -> bool {
    let manager = state.borrow().thread_mgr.clone();
    // 焦点不在这个线程了：← 会落到别的程序里，不能发
    let focused = manager
        .as_ref()
        .is_some_and(|mgr| unsafe { mgr.IsThreadFocus() }.is_ok_and(|focus| focus.as_bool()));
    let mut s = state.borrow_mut();
    let Some(pending) = s.caret_move else {
        return false;
    };
    // 此刻键盘上的状态：消息里记的状态这时已经过时了
    let held = [0x10, 0x11, 0x12, 0x5b, 0x5c]
        .into_iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk) } < 0);
    let steps = match pending.step(now_ms(), focused, held) {
        keys::CaretStep::Wait => return true,
        keys::CaretStep::Drop => {
            drop(s);
            pair::finish(state, focused);
            return false;
        }
        keys::CaretStep::Send(_) if s.pair_delivery.sent => {
            let released = s.pair_delivery.released;
            drop(s);
            let placement = pair::placement(state);
            if placement == pair::Placement::Inside
                || (released && placement != pair::Placement::AfterTerminal)
            {
                pair::finish(state, true);
                return false;
            }
            return true;
        }
        keys::CaretStep::Send(steps) => steps,
    };
    s.pair_delivery.sent = true;
    drop(s);
    // 只为成对标点挪光标；联想选词始终通过 TSF 编辑区完成，不发送辅助键。
    let key = |up: bool, last: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_LEFT,
                wScan: 0x4b,
                dwFlags: if up {
                    KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
                } else {
                    KEYEVENTF_EXTENDEDKEY
                },
                dwExtraInfo: if last {
                    keys::CARET_END_TAG
                } else {
                    keys::OWN_INPUT_TAG
                },
                ..Default::default()
            },
        },
    };
    let inputs: Vec<INPUT> = (0..steps)
        .flat_map(|index| [key(false, false), key(true, index + 1 == steps)])
        .collect();
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        crate::log::write("发送辅助按键失败", &format!("{sent}/{}", inputs.len()));
        pair::finish(state, true);
        return false;
    }
    true
}
