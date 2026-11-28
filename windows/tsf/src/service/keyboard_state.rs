//! 键盘开关状态：敏感输入框、Ctrl+空格与系统中英文开关。
use super::*;

pub(super) fn set_sensitive(state: &Rc<RefCell<State>>, active: bool) {
    if active && state.borrow().caret_move.is_some() {
        pair::finish(state, false);
    }
    if active {
        finish_prediction_space(state, true, false);
    }
    if std::mem::replace(&mut state.borrow_mut().sensitive, active) == active {
        return;
    }
    if let Some(frame) = call(state, &Request::SensitiveFocus { active }) {
        note_frame(state, &frame);
    }
    if active {
        {
            let mut state = state.borrow_mut();
            state.key_preview = None;
            state.undo = None;
            state.space = None;
            state.prediction_escape = PredictionEscape::Free;
            state.pairs.clear();
        }
        with_window(state, |window| window.hide());
    }
}

pub(super) fn check_sensitive(state: &Rc<RefCell<State>>, context: Option<&ITfContext>) -> bool {
    if disabled(context) {
        set_sensitive(state, true);
        return true;
    }
    if let Some(context) = context {
        let client_id = state.borrow().client_id;
        let (flag, input) = (Rc::new(std::cell::Cell::new(None)), context.clone());
        let out = flag.clone();
        edit_with(context, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
            out.set(Some(sensitive_scope(&input, ec)));
            Ok(())
        });
        if let Some(active) = flag.get() {
            state.borrow_mut().sensitive_context = active.then(|| context.clone());
        }
    }
    // Like SampleIME/Weasel/msime, disabled compartments describe this key's
    // current context. Failure to obtain an optional read lock must not keep
    // an earlier non-editing surface disabled forever (RichEdit/CUAS).
    // A known password/PIN remains protected if its own read lock is unavailable.
    let active = state
        .borrow()
        .sensitive_context
        .as_ref()
        .is_some_and(|owner| context.is_some_and(|context| same_object(owner, context)));
    set_sensitive(state, active);
    active
}

/// 把系统的「键盘打开」设成打开（中英文由 FreeType 自己管）。
pub(super) fn set_keyboard_open(thread_mgr: &ITfThreadMgr, client_id: u32) {
    let Ok(mgr) = thread_mgr.cast::<ITfCompartmentMgr>() else {
        return;
    };
    if compartment_value(&mgr, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) == Some(1) {
        return;
    }
    unsafe {
        let Ok(compartment) = mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) else {
            return;
        };
        let mut value = VARIANT::default();
        (*value.Anonymous.Anonymous).vt = VT_I4;
        (*value.Anonymous.Anonymous).Anonymous.lVal = 1;
        if let Err(error) = compartment.SetValue(client_id, &value) {
            crate::log::write("恢复键盘打开状态失败", &error.to_string());
        }
    }
}

/// 系统把「键盘打开 / 关闭」改成关闭，且此刻正按着 Ctrl+空格：这是系统输入法开关键吃掉了按键。
/// 当成一次 Ctrl+空格 发给服务进程，由它按设置决定切不切换。
/// OnChange 中不能 SetValue（TSF 返回 E_UNEXPECTED）；排队到回调结束后再打开，
/// 否则下一次 Ctrl+空格 只会重新打开系统状态，表现为按两次才切换。
pub(super) fn ctrl_space_closed(state: &Rc<RefCell<State>>) {
    let ms = unsafe { GetMessageTime() } as u32;
    if !ctrl_space_handled(state, ms, false) {
        // 带上大写锁定的状态：服务进程按每个键里的这一位同步大写锁定
        let caps = keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) }) & modifier::CAPS;
        let key = Key {
            keysym: 0x20,
            modifiers: modifier::CTRL | caps,
            test: false,
        };
        let Some(frame) = call(
            state,
            &Request::PhysicalKey {
                key,
                before: None,
                scan: 0,
                shown_revision: None,
            },
        ) else {
            return;
        };
        if !frame.handled {
            return;
        }
        state.borrow_mut().ctrl_space_toggled = Some(ms);
        let context = state.borrow().context.clone();
        apply(state, context.as_ref(), frame, false);
    }
    if let Some(window) = &state.borrow().window {
        if let Err(error) = unsafe {
            PostMessageW(
                Some(window.hwnd()),
                ui::WM_RESTORE_KEYBOARD_OPEN,
                WPARAM(0),
                LPARAM(0),
            )
        } {
            crate::log::write("安排恢复键盘打开状态失败", &error.to_string());
        }
    }
}

/// 同一条按键消息的预询问、正式处理和系统通知只切换一次；长按重复也不再切换。
pub(super) fn ctrl_space_handled(state: &Rc<RefCell<State>>, ms: u32, repeat: bool) -> bool {
    let mut s = state.borrow_mut();
    if s.ctrl_space_toggled
        .is_some_and(|previous| previous == ms || repeat)
    {
        s.ctrl_space_toggled = Some(ms);
        true
    } else {
        false
    }
}

pub(super) fn open_close_source(thread_mgr: &ITfThreadMgr) -> Option<ITfSource> {
    let mgr = thread_mgr.cast::<ITfCompartmentMgr>().ok()?;
    unsafe {
        mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
            .ok()?
            .cast()
            .ok()
    }
}
