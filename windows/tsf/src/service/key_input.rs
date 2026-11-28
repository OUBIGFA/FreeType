//! 按键预处理：辅助键识别、光标前文字、退格撤销、成对标点跳过、键码翻译与大写锁定。
use super::*;

/// 正在处理的按键是不是 FreeType 为成对标点移动光标发出的按键。
pub(super) fn own_input() -> bool {
    matches!(
        unsafe { GetMessageExtraInfo() }.0 as usize,
        keys::OWN_INPUT_TAG | keys::CARET_END_TAG
    )
}

/// 只读一下光标前的字（同步）。
pub(super) fn read_before(state: &Rc<RefCell<State>>, context: &ITfContext) -> Option<String> {
    let target = match surrounding(context) {
        Surrounding::Imm32 => return imm32_before(context),
        Surrounding::Tsf(target) => target,
    };
    let client_id = state.borrow().client_id;
    let out: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let (out2, context2) = (out.clone(), target.clone());
    edit_with(&target, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
        *out2.borrow_mut() = before_text(&context2, ec);
        Ok(())
    });
    let text = out.borrow_mut().take();
    text
}

pub(super) fn key_before(state: &Rc<RefCell<State>>, context: &ITfContext) -> Option<String> {
    let wants = !composing(state) && {
        let s = state.borrow();
        s.mode == Mode::Chinese || s.want_before
    };
    if wants {
        read_before(state, context)
    } else {
        None
    }
}

pub(super) fn watch_backspace(state: &Rc<RefCell<State>>, context: Option<&ITfContext>, vk: u16) {
    if vk != 0x08
        || moment().other_modifiers
        || unsafe { GetAsyncKeyState(0x10) } < 0
        || composing(state)
    {
        state.borrow_mut().undo = None;
        return;
    }
    let Some(context) = context else {
        return;
    };
    let before = {
        let state = state.borrow();
        let Some(watch) = state
            .undo
            .as_ref()
            .filter(|watch| same_object(&watch.context, context))
        else {
            return;
        };
        watch.before.clone()
    };
    let valid = read_before(state, context).as_deref() == Some(before.as_str());
    let mut state = state.borrow_mut();
    if valid {
        if let Some(watch) = state.undo.as_mut() {
            watch.armed = true;
        }
    } else {
        state.undo = None;
    }
}

pub(super) fn confirm_backspace(state: &Rc<RefCell<State>>, context: Option<&ITfContext>) {
    let Some(context) = context.filter(|context| !disabled(Some(context))) else {
        return;
    };
    let expected = {
        let state = state.borrow();
        let Some(watch) = state
            .undo
            .as_ref()
            .filter(|watch| watch.armed && same_object(&watch.context, context))
        else {
            return;
        };
        watch.shortened.clone()
    };
    if read_before(state, context).is_some_and(|before| before.ends_with(&expected)) {
        state.borrow_mut().undo = None;
        let _ = call(state, &Request::UndoCommitLearning);
    }
}

pub(super) fn verified_pair_key(
    state: &Rc<RefCell<State>>,
    context: Option<&ITfContext>,
    key: Key,
) -> Option<(PairEdit, bool)> {
    let context = context?;
    let (action, composition, client_id, arrow_pending) = {
        let s = state.borrow();
        if !s.pair_enabled || (s.busy && s.composition.is_none()) {
            return None;
        }
        let action = s.pairs.last()?.for_key(key, s.composition.is_some())?;
        // 左右两半都要按 Shift（（）、《》）时，用户常一直按着 Shift 连打两半；
        // 补全的左移键要等 Shift 松开才发，右半标点会先到。
        let arrow_pending = s.pair_absorb_closing
            && action == PairEdit::Skip
            && s.composition.is_none()
            && s.caret_move.is_some()
            && s.pair_delivery.untouched();
        (action, s.composition.clone(), s.client_id, arrow_pending)
    };
    let accepted = Rc::new(std::cell::Cell::new(None));
    let (out, state, doc) = (accepted.clone(), state.clone(), context.clone());
    edit_with(context, client_id, TF_ES_SYNC | TF_ES_READ, move |ec| {
        let caret = if let Some(comp) = composition {
            unsafe {
                comp.GetRange()
                    .and_then(|range| {
                        range.Collapse(ec, TF_ANCHOR_END)?;
                        Ok(range)
                    })
                    .ok()
            }
        } else {
            selection_range(&doc, ec)
        };
        let state = state.borrow();
        if let (Some(caret), Some(last)) = (caret, state.pairs.last()) {
            if last.permits(&doc, ec, &caret, action) {
                out.set(Some(false));
            } else if arrow_pending && last.caret_after_closing(&doc, ec, &caret) {
                // 光标已经在右标点后面，正是跳过后的位置：吞掉这个右键，
                // 由 OnKeyDown 撤销待发的左移键，不再插入第二个右标点（Issue #13）。
                out.set(Some(true));
            }
        }
        Ok(())
    });
    accepted.get().map(|cancel_arrow| (action, cancel_arrow))
}

pub(super) fn plain_key(key: Key) -> bool {
    key.modifiers & (modifier::SHIFT | modifier::CTRL | modifier::ALT | modifier::SUPER) == 0
}

/// 虚拟键翻成键码；翻不出来（非 ASCII 字符、功能键）返回 None。
pub(super) fn translate(wparam: WPARAM, lparam: LPARAM, test: bool) -> Option<Key> {
    let vk = wparam.0 as u16;
    let modifiers = keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) })
        | if lparam.0 & (1 << 30) != 0 {
            modifier::REPEAT
        } else {
            0
        };
    if let Some(keysym) = keys::named_keysym(vk) {
        return Some(Key {
            keysym,
            modifiers,
            test,
        });
    }
    unsafe {
        let mut state = [0u8; 256];
        GetKeyboardState(&mut state).ok()?;
        // Ctrl、Alt 按着时也要拿到字母本身（Ctrl+C 是 c，不是控制字符）
        for vk in [0x11, 0x12, 0xa2, 0xa3, 0xa4, 0xa5] {
            state[vk] = 0;
        }
        let scan = ((lparam.0 >> 16) & 0xff) as u32;
        let mut buf = [0u16; 8];
        // 4：不改动键盘状态（不打断死键）
        let n = ToUnicode(u32::from(vk), scan, Some(&state), &mut buf, 4);
        if n != 1 {
            return None;
        }
        let keysym = char::from_u32(u32::from(buf[0])).and_then(keys::char_keysym)?;
        Some(Key {
            keysym,
            modifiers,
            test,
        })
    }
}

/// 大写锁定现在开着没有（按处理到这条按键消息时的状态）。
pub(super) fn caps_on() -> bool {
    unsafe { GetKeyState(i32::from(keys::VK_CAPITAL)) & 1 != 0 }
}

/// Caps Lock 按下、松开时：大写锁定变了就告诉服务进程，按应答上屏打到一半的字母、提示新状态。
///
/// 按下和松开都看一遍：浏览器内核的程序先看一眼消息再取走，按下那一刻状态可能还没更新；
/// 同一次变化只报一遍（`State::caps` 记着上次报的）。应用禁用输入法的地方（密码框）不提示，
/// 服务进程从之后每个按键带的修饰键状态里照样能跟上。
pub(super) fn caps_changed(state: &Rc<RefCell<State>>, context: Option<&ITfContext>) {
    let on = caps_on();
    if std::mem::replace(&mut state.borrow_mut().caps, on) == on || disabled(context) {
        return;
    }
    if let Some(frame) = call(state, &Request::CapsLock { on }) {
        // 和单按 Shift 一样不强求同步：系统只是先问一声的时候不一定给同步的编辑会话
        apply(state, context, frame, false);
    }
}

/// 有键按下（OnTestKeyDown、OnKeyDown 共用）：记给单按 Shift 的判定。
/// 有的应用不先问就直接报按下，所以两处都记；重复记同一下没有影响。
///
/// 导航、删除或快捷键可能换了插入点；普通字母的预询问不表示光标已移动。
pub(super) fn note_key_down(state: &Rc<RefCell<State>>, wparam: WPARAM, lparam: LPARAM) {
    let vk = wparam.0 as u16;
    let repeat = lparam.0 & (1 << 30) != 0;
    let mut s = state.borrow_mut();
    let modifiers = moment();
    s.shift.key_down(vk, modifiers, repeat);
    if !keys::is_modifier(vk) {
        s.focus_notice.cancel();
    }
    if matches!(vk, 0x08 | 0x09 | 0x0d | 0x1b | 0x21..=0x28 | 0x2d | 0x2e)
        || (!keys::is_modifier(vk) && modifiers.other_modifiers)
    {
        let pending = s.caret_move.is_some();
        drop(s);
        if pending {
            pair::finish(state, true);
        }
    }
}

/// 有键松开（OnTestKeyUp、OnKeyUp 共用）：是单按 Shift 就切换中英文。
///
/// 系统对同一次松开可能问几遍：浏览器内核的程序（Electron、Chrome）空闲时先看一眼消息再取走，
/// 两次都走到 OnTestKeyUp。所以第一次报到就切换，[`ShiftTap`] 保证同一次松开不切第二遍。
/// （以前 OnTestKeyUp 只记下结果、等 OnKeyUp 去切换；第二遍 OnTestKeyUp 把记下的结果清掉，
/// 并回答「不要这个键」，OnKeyUp 不再被调用，这类程序里就切换不了。）
///
/// 不吃这个键：应用照常收到 Shift 松开，不会以为 Shift 还按着。
///
/// 成对标点还等着挪光标的话，修饰键一松开就去挪（见 [`caret_tick`]）。
pub(super) fn key_released(
    state: &Rc<RefCell<State>>,
    context: Option<&ITfContext>,
    wparam: WPARAM,
    lparam: LPARAM,
) {
    let vk = wparam.0 as u16;
    if own_input() {
        if vk == VK_LEFT.0
            && unsafe { GetMessageExtraInfo() }.0 as usize == keys::CARET_END_TAG
            && state.borrow().pair_delivery.sent
        {
            state.borrow_mut().pair_delivery.released = true;
            if pair::placement(state) != pair::Placement::AfterTerminal {
                pair::finish(state, true);
            }
        }
        return;
    }
    state.borrow_mut().key_preview = None;
    if vk == 0x20 {
        state.borrow_mut().ctrl_space_toggled = None;
    }
    if vk == 0x08 {
        confirm_backspace(state, context);
    }
    if keys::is_modifier(vk) && state.borrow().caret_move.is_some() {
        caret_tick(state);
    }
    if vk == keys::VK_CAPITAL {
        caps_changed(state, context);
        return;
    }
    let shift = state.borrow_mut().shift.key_up(vk, moment());
    if !shift || check_sensitive(state, context) {
        return;
    }
    state.borrow_mut().undo = None;
    let scan = ((lparam.0 >> 16) & 0xff) as u32;
    let request = Request::ToggleMode {
        shift: Some(keys::shift_side(vk, scan).name().to_string()),
    };
    if let Some(frame) = call(state, &request) {
        // 不强求同步：系统只是先问一声（OnTestKeyUp）的时候不一定给同步的编辑会话，
        // 这样写它能同步就同步、不能就排到后面，打到一半的字母照样能上屏
        apply(state, context, frame, false);
    }
}
