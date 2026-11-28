//! 上屏联想的单次与双击空格、Esc 收起和换输入框时的清理。
use super::*;

pub(super) fn prediction_in(s: &State, context: &ITfContext) -> bool {
    s.predicting
        && s.context
            .as_ref()
            .is_some_and(|owner| same_object(owner, context))
}

pub(super) fn double_space_wait() -> u32 {
    unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }.clamp(200, 700)
}

/// 从第一次按下起拥有空格的编辑区，CUAS 和原生 TSF 使用完全相同的事务。
pub(super) fn begin_prediction_space(
    state: &Rc<RefCell<State>>,
    context: &ITfContext,
    revision: u64,
    ms: u32,
) -> bool {
    if state
        .borrow()
        .space
        .as_ref()
        .is_some_and(|pending| pending.pressed_ms == ms)
    {
        return true;
    }
    finish_prediction_space(state, true, true);
    let (client_id, sink) = {
        let s = state.borrow();
        (s.client_id, s.sink.clone())
    };
    let done = Rc::new(std::cell::Cell::new(false));
    let (done2, state2, doc) = (done.clone(), state.clone(), context.clone());
    let requested = request_edit(
        context,
        client_id,
        TF_ES_SYNC | TF_ES_READWRITE,
        move |ec| {
            let result = (|| -> Result<()> {
                unsafe {
                    let insert: ITfInsertAtSelection = doc.cast()?;
                    let range = insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
                    let owner: ITfContextComposition = doc.cast()?;
                    let composition = owner.StartComposition(ec, &range, sink.as_ref())?;
                    let result = (|| -> Result<ITfRange> {
                        let range = composition.GetRange()?;
                        range.SetText(ec, 0, &[u16::from(b' ')])?;
                        let caret = range.Clone()?;
                        caret.Collapse(ec, TF_ANCHOR_END)?;
                        set_selection(&doc, ec, &caret)?;
                        Ok(caret)
                    })();
                    let caret = match result {
                        Ok(caret) => caret,
                        Err(error) => {
                            let _ = range.SetText(ec, 0, &[]);
                            let _ = composition.EndComposition(ec);
                            return Err(error);
                        }
                    };
                    let wait = double_space_wait();
                    // GetTextExt 由应用实现，调用它时不持有可重入文本服务的状态借用。
                    let rect = text_rect(&doc, ec, &caret);
                    let at = {
                        let mut s = state2.borrow_mut();
                        s.space = Some(PredictionSpace {
                            context: doc.clone(),
                            composition,
                            revision,
                            pressed_ms: ms,
                            deadline_ms: now_ms() + u64::from(wait),
                        });
                        s.undo = None;
                        s.caret = rect.or(s.caret);
                        s.caret
                    };
                    with_window(&state2, |window| {
                        window.set_prediction_space_timeout(Some(wait));
                        if let Some(at) = at {
                            window.reposition(at);
                        }
                    });
                    done2.set(true);
                    Ok(())
                }
            })();
            if let Err(error) = &result {
                crate::log::write("保留联想空格失败", &error.to_string());
            }
            result
        },
    );
    if let Err(error) = requested {
        crate::log::write("保留联想空格失败", &error.to_string());
    }
    done.get()
}

pub(super) fn take_prediction_space(state: &Rc<RefCell<State>>) -> Option<PredictionSpace> {
    let mut s = state.borrow_mut();
    let pending = s.space.take()?;
    if let Some(window) = &s.window {
        window.set_prediction_space_timeout(None);
    }
    Some(pending)
}

/// 只结束这个空格拥有的编辑区。异步焦点清理也不会把旧文字写到新的输入框。
pub(super) fn finish_prediction_space(state: &Rc<RefCell<State>>, commit: bool, sync: bool) {
    let Some(pending) = take_prediction_space(state) else {
        return;
    };
    let (client_id, state2) = (state.borrow().client_id, state.clone());
    let context = pending.context.clone();
    let flags = if sync {
        TF_ES_SYNC | TF_ES_READWRITE
    } else {
        TF_ES_ASYNCDONTCARE | TF_ES_READWRITE
    };
    let requested = request_edit(&context.clone(), client_id, flags, move |ec| {
        let result = (|| -> Result<()> {
            unsafe {
                let range = pending.composition.GetRange()?;
                if !commit {
                    range.SetText(ec, 0, &[])?;
                }
                let caret = range.Clone()?;
                caret.Collapse(ec, TF_ANCHOR_END)?;
                caret.SetGravity(ec, TF_GRAVITY_FORWARD, TF_GRAVITY_FORWARD)?;
                pending.composition.EndComposition(ec)?;
                if !commit {
                    set_selection(&context, ec, &caret)?;
                }
                let mut s = state2.borrow_mut();
                if prediction_in(&s, &context) {
                    s.prediction_caret = Some(caret);
                }
                Ok(())
            }
        })();
        if let Err(error) = &result {
            crate::log::write("结束联想空格失败", &error.to_string());
        }
        result
    });
    if let Err(error) = requested {
        crate::log::write("结束联想空格失败", &error.to_string());
    }
}

pub(super) fn prediction_space_timeout(state: &Rc<RefCell<State>>) {
    let remaining = state
        .borrow()
        .space
        .as_ref()
        .map(|pending| pending.deadline_ms.saturating_sub(now_ms()));
    match remaining {
        Some(0) => finish_prediction_space(state, true, false),
        Some(ms) => {
            if let Some(window) = &state.borrow().window {
                window.set_prediction_space_timeout(Some(ms as u32));
            }
        }
        None => {}
    }
}

/// Both TSF callbacks use the same candidate page and caret. A fresh text
/// snapshot can lag behind the commit in a rich editor, so do not send it as
/// `before` for this space. A verified move still invalidates the prediction.
pub(super) fn prediction_space_key(
    state: &Rc<RefCell<State>>,
    context: &ITfContext,
    key: Key,
    repeat: bool,
    revision: u64,
    ms: u32,
) -> (Key, bool) {
    if key.keysym != 0x20 || !plain_key(key) || repeat || revision == 0 {
        return (key, false);
    }
    let (client_id, anchor, pending, double) = {
        let s = state.borrow();
        if !prediction_in(&s, context) {
            return (key, false);
        }
        (
            s.client_id,
            s.prediction_caret.clone(),
            s.space.clone(),
            s.predict_double_space,
        )
    };
    let pending = pending.filter(|p| p.revision == revision && same_object(&p.context, context));
    let second = double
        && pending.as_ref().is_some_and(|pending| {
            let elapsed = ms.wrapping_sub(pending.pressed_ms);
            elapsed > 0 && elapsed <= double_space_wait()
        });
    let result = Rc::new(std::cell::Cell::new(false));
    let (result2, doc) = (result.clone(), context.clone());
    edit_with(context, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
        let Some(caret) = selection_range(&doc, ec) else {
            return Ok(());
        };
        unsafe {
            if !caret.IsEmpty(ec)?.as_bool() {
                result2.set(true);
                return Ok(());
            }
            if let Some(pending) = pending {
                let range = pending.composition.GetRange()?;
                result2.set(caret.CompareStart(ec, &range, TF_ANCHOR_END)? != 0);
            } else if let Some(anchor) = anchor {
                result2.set(caret.CompareStart(ec, &anchor, TF_ANCHOR_END)? != 0);
            }
        }
        Ok(())
    });
    if result.get() {
        dismiss_prediction(state);
        return (key, false);
    }
    if second {
        return (
            Key {
                keysym: keysym::SPACE_DOUBLE,
                ..key
            },
            true,
        );
    }
    if double && !key.test && !begin_prediction_space(state, context, revision, ms) {
        dismiss_prediction(state);
        return (key, false);
    }
    (key, true)
}

/// A visible prediction owns plain Esc until release, including repeats
/// after the window closes. A fresh press can still go to the application.
pub(super) fn prediction_escape_down(
    state: &Rc<RefCell<State>>,
    context: Option<&ITfContext>,
    key: Key,
    repeat: bool,
) -> bool {
    if key.keysym != keysym::ESCAPE {
        return false;
    }
    let (eat, close) = {
        let mut s = state.borrow_mut();
        if repeat && s.prediction_escape == PredictionEscape::Held {
            return true;
        }
        let visible = context.is_some_and(|context| prediction_in(&s, context))
            && s.window.as_ref().is_some_and(|window| window.visible());
        let claimed = visible && plain_key(key);
        s.prediction_escape = match (claimed, key.test) {
            (false, _) => PredictionEscape::Free,
            (true, true) => PredictionEscape::Preview,
            (true, false) => PredictionEscape::Held,
        };
        (claimed, claimed && !key.test)
    };
    if close {
        dismiss_prediction(state);
    }
    eat
}

pub(super) fn prediction_escape_up(state: &Rc<RefCell<State>>, wparam: WPARAM, test: bool) -> bool {
    if own_input() || wparam.0 as u16 != 0x1b {
        return false;
    }
    let claimed = {
        let mut s = state.borrow_mut();
        let claimed = s.prediction_escape;
        if claimed == PredictionEscape::Free {
            return false;
        }
        s.prediction_escape = if test {
            PredictionEscape::Held
        } else {
            PredictionEscape::Free
        };
        claimed
    };
    // Do not forward an unmatched release. If the host omitted KeyDown
    // after a handled test, this still closes the window without editing text.
    if claimed == PredictionEscape::Preview {
        dismiss_prediction(state);
    }
    true
}

pub(super) fn discard_prediction_from_other_context(
    state: &Rc<RefCell<State>>,
    context: &ITfContext,
) {
    let stale = {
        let s = state.borrow();
        s.predicting && !prediction_in(&s, context)
    };
    if stale {
        dismiss_prediction(state);
    }
}
