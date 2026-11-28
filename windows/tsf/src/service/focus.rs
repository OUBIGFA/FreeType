//! 输入焦点变化：恢复组字、焦点提示与文档切换。
use super::*;

/// 焦点通知说的就是正在组字的那个文档时，返回组字所在的上下文。
///
/// 应用会对同一个输入框再报一次焦点：资源管理器地址栏（XAML）点进去以后，要等第一个键
/// 打出来才报，这时编辑区和候选窗已经在了。这不是换输入框，见 [`on_focus`]。
pub(super) fn composing_context(
    composition: Option<&ITfComposition>,
    focus: Option<&ITfDocumentMgr>,
) -> Option<ITfContext> {
    let context = unsafe { composition?.GetRange().and_then(|range| range.GetContext()) }.ok()?;
    let owner = unsafe { context.GetDocumentMgr() }.ok()?;
    same_object(&owner, focus?).then_some(context)
}

/// 焦点还在（或回到了）正在组字的输入框：编辑区留着，候选窗接着显示。
///
/// 做法照水杉输入法 Windows 版的 `ITfThreadMgrEventSink::OnSetFocus`：候选窗属于哪个文档，
/// 焦点落在那个文档上就显示，落在别处才收起。不提示中英文状态（用户正在打字），
/// 也不清掉记着的光标位置（候选窗靠它摆）。
pub(super) fn resume_composition(state: &Rc<RefCell<State>>, context: ITfContext) {
    state.borrow_mut().context = Some(context);
    // 焦点中途丢过的话服务进程已经当这个输入框不在用了：重新报上，顺便拿回这一页候选
    let Some(mut frame) = call(
        state,
        &Request::Focus {
            active: true,
            before: None,
        },
    ) else {
        return;
    };
    frame.hint = None;
    note_frame(state, &frame);
    show_window(state, &frame);
}

/// 换了输入框：设回「键盘打开」，告诉服务进程，按设置提示中英文状态。
///
/// 焦点通知不一定是换了输入框：正在组字的文档自己又报一次焦点时，编辑区和候选窗原样保留
/// （[`resume_composition`]）。以前一律收起候选窗，第一个字母就像直接成了英文。
/// 焦点暂时落空（`focus` 是 None）时只收起候选窗，编辑区不动，回来以后同样接着显示。
pub(super) fn on_focus(state: &Rc<RefCell<State>>, focus: Option<&ITfDocumentMgr>) {
    state.borrow_mut().key_preview = None;
    let context = focus.and_then(|doc| unsafe { doc.GetTop().ok() });
    focus_notice::observe(state, context.as_ref());
    // A rich editor can reassert the same focus during the commit, before
    // the prediction has even painted. Don't clear the new page or its range.
    if context
        .as_ref()
        .is_some_and(|context| prediction_in(&state.borrow(), context))
    {
        return;
    }
    let composition = state.borrow().composition.clone();
    if let Some(context) = composing_context(composition.as_ref(), focus) {
        resume_composition(state, context);
        return;
    }
    dismiss_prediction(state);
    let previous = state.borrow().context.clone();
    let same_context = previous
        .as_ref()
        .zip(context.as_ref())
        .is_some_and(|(old, new)| same_object(old, new));
    let keep_move = same_context
        || previous
            .as_ref()
            .zip(context.as_ref())
            .is_some_and(|(old, new)| {
                let (suffix, client) = {
                    let service = state.borrow();
                    (
                        service
                            .caret_move
                            .and_then(|_| service.pairs.last().map(pair::Pair::suffix)),
                        service.client_id,
                    )
                };
                let Some(suffix) = suffix else {
                    return false;
                };
                if view_window(old).is_none_or(|window| Some(window) != view_window(new))
                    || disabled(Some(new))
                {
                    return false;
                }
                let verified = Rc::new(std::cell::Cell::new(false));
                let (result, target) = (verified.clone(), new.clone());
                edit_with(new, client, TF_ES_SYNC | TF_ES_READ, move |cookie| {
                    result.set(
                        !sensitive_scope(&target, cookie)
                            && before_text(&target, cookie)
                                .is_some_and(|text| text.ends_with(&suffix)),
                    );
                    Ok(())
                });
                verified.get()
            });
    if !keep_move && state.borrow().caret_move.is_some() {
        pair::finish(state, false);
    }
    {
        let mut s = state.borrow_mut();
        s.shift.cancel();
        s.prediction_escape = PredictionEscape::Free;
        if !same_context {
            s.pairs.clear();
        }
        s.undo = None;
        s.last_caret = None;
    }
    with_window(state, |window| window.hide());
    let Some(context) = context else {
        state.borrow_mut().context = None;
        if let Some(frame) = call(
            state,
            &Request::Focus {
                active: false,
                before: None,
            },
        ) {
            note_frame(state, &frame);
        }
        return;
    };
    let (thread_mgr, client_id) = {
        let mut s = state.borrow_mut();
        s.context = Some(context.clone());
        (s.thread_mgr.clone(), s.client_id)
    };
    if let Some(thread_mgr) = &thread_mgr {
        set_keyboard_open(thread_mgr, client_id);
    }
    if disabled(Some(&context)) {
        set_sensitive(state, true);
        return;
    }
    // 读光标前的字和光标位置要编辑会话；这里不能同步要，异步做完再告诉服务进程
    let (state2, context2) = (state.clone(), context.clone());
    let generation = state.borrow().generation;
    edit_with(
        &context,
        client_id,
        TF_ES_ASYNCDONTCARE | TF_ES_READ,
        move |ec| {
            if state2.borrow().generation != generation {
                return Ok(());
            }
            if !state2
                .borrow()
                .context
                .as_ref()
                .is_some_and(|current| same_object(current, &context2))
            {
                return Ok(());
            }
            let sensitive = sensitive_scope(&context2, ec);
            state2.borrow_mut().sensitive_context = sensitive.then(|| context2.clone());
            set_sensitive(&state2, sensitive);
            if sensitive {
                return Ok(());
            }
            // A queued focus read can arrive after the first commit in this field.
            if prediction_in(&state2.borrow(), &context2) {
                return Ok(());
            }
            let before = match surrounding(&context2) {
                Surrounding::Tsf(target) if same_object(&target, &context2) => {
                    before_text(&context2, ec)
                }
                // 父文档要另开读锁；这里先报不知道，下一个键会照常读到
                Surrounding::Tsf(_) => None,
                Surrounding::Imm32 => imm32_before(&context2),
            };
            // 大写锁定可能在别的窗口里开关过：先对齐，下面拿到的中英文状态才是对的
            let on = caps_on();
            state2.borrow_mut().caps = on;
            let _ = call(&state2, &Request::CapsLock { on });
            let Some(frame) = call(
                &state2,
                &Request::Focus {
                    active: true,
                    before,
                },
            ) else {
                return Ok(());
            };
            note_frame(&state2, &frame);
            focus_notice::capture(&state2, frame);
            Ok(())
        },
    );
}
