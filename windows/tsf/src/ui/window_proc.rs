//! 候选窗的窗口过程、外部点击监视、定时器和窗口创建。
use super::*;

/// 联想显示时盯住本线程的鼠标按下，跟联想计时器同开同关。
///
/// 联想没有组字，应用不会因为点击结束它，只能自己看。只装线程钩子：不进别的进程，
/// 也看不到别的程序里的点击；切到别的程序由候选窗收到的 `WM_ACTIVATEAPP` 处理。
pub(super) fn watch_outside_clicks(hwnd: HWND, on: bool) {
    OUTSIDE.with(|cell| match (cell.get(), on) {
        (None, true) => {
            let hook = unsafe {
                SetWindowsHookExW(
                    WH_MOUSE,
                    Some(outside_click_proc),
                    None,
                    GetCurrentThreadId(),
                )
            };
            match hook {
                Ok(hook) => cell.set(Some((hook, hwnd))),
                // 受限的应用（如沙盒进程）可能不许装钩子：联想仍靠计时器收起，日志只记一次
                Err(error) => {
                    static LOGGED: std::sync::Once = std::sync::Once::new();
                    LOGGED.call_once(|| {
                        crate::log::write("联想点击监视安装失败", &error.to_string())
                    });
                }
            }
        }
        (Some((hook, _)), false) => {
            unsafe {
                let _ = UnhookWindowsHookEx(hook);
            }
            cell.set(None);
        }
        _ => {}
    });
}

/// 按下落在候选窗和它的右键菜单以外就通知候选窗。钩子里不直接收起：
/// 收起要找服务进程，放回消息循环再做，点击照常交给应用（光标照常挪过去）。
pub(super) unsafe extern "system" fn outside_click_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let down = matches!(
        wparam.0 as u32,
        WM_LBUTTONDOWN
            | WM_RBUTTONDOWN
            | WM_MBUTTONDOWN
            | WM_XBUTTONDOWN
            | WM_NCLBUTTONDOWN
            | WM_NCRBUTTONDOWN
            | WM_NCMBUTTONDOWN
            | WM_NCXBUTTONDOWN
    );
    if code == HC_ACTION as i32 && down {
        let target = unsafe { (*(lparam.0 as *const MOUSEHOOKSTRUCT)).hwnd };
        if let Some((_, owner)) = OUTSIDE.with(Cell::get) {
            if target != owner && MENU.with(|m| *m.borrow()) != Some(target) {
                unsafe {
                    let _ = PostMessageW(Some(owner), WM_OUTSIDE_CLICK, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 点到外面或切到别的程序：联想还显示着就收起（同 [`Click::Dismiss`]）。
pub(super) fn dismiss_on_leave(hwnd: HWND) {
    if OUTSIDE.with(Cell::get).is_none() {
        return;
    }
    watch_outside_clicks(hwnd, false);
    unsafe {
        let _ = KillTimer(Some(hwnd), PREDICT_TIMER);
    }
    dispatch(Click::Dismiss);
}

pub(super) fn caret_tick(hwnd: HWND) {
    let again = CARET_HANDLER.with(|h| h.borrow().as_ref().is_some_and(|handler| handler()));
    unsafe {
        if again {
            SetTimer(Some(hwnd), CARET_TIMER, CARET_RETRY_MS, None);
        } else {
            let _ = KillTimer(Some(hwnd), CARET_TIMER);
        }
    }
}

pub(super) fn dispatch(click: Click) {
    if let Click::PinChoices {
        owner,
        index,
        revision,
    } = click
    {
        open_pin_choices(owner, index, revision);
        return;
    }
    // A native menu pumps messages. TSF can detach the service during a click.
    if let Some(handler) = HANDLER.with(|h| h.borrow().clone()) {
        handler(click);
    }
}

/// 右键、长按弹出的菜单：候选上的「删除记忆」和「打开设置」。
pub(super) fn open_menu(owner: HWND, item: Option<usize>) {
    let Some(data) = data_of(owner) else {
        return;
    };
    let mut entries: Vec<(String, Click)> = Vec::new();
    if let Some(i) = item.filter(|_| data.view.forgettable) {
        if data.view.pin_slots.get(i).is_some_and(Option::is_some) {
            entries.push((
                "置顶".into(),
                data.click(CandidateAction::Pin { index: i, place: 1 }),
            ));
            entries.push((
                "固定排位".into(),
                Click::PinChoices {
                    owner,
                    index: i,
                    revision: data.painted_revision,
                },
            ));
        }
        if let Some(word) = data.view.candidates.get(i) {
            entries.push((
                format!("删除 {word} 的学习记录"),
                data.click(CandidateAction::Forget { index: i }),
            ));
        }
    }
    entries.push(("打开 FreeType 设置".into(), Click::Settings));
    show_candidate_menu(owner, data, entries);
}

pub(super) fn open_pin_choices(owner: HWND, index: usize, revision: u64) {
    let Some(data) =
        data_of(owner).filter(|data| revision != 0 && data.painted_revision == revision)
    else {
        return;
    };
    let Some(Some(current)) = data.view.pin_slots.get(index) else {
        return;
    };
    let mut entries: Vec<_> = (1..=data.view.pin_limit)
        .map(|place| {
            let label = format!(
                "{}固定到第 {place} 位",
                if *current == place { "✓ " } else { "" }
            );
            (label, data.click(CandidateAction::Pin { index, place }))
        })
        .collect();
    if *current != 0 {
        entries.push((
            "取消固定".into(),
            data.click(CandidateAction::Pin { index, place: 0 }),
        ));
    }
    show_candidate_menu(owner, data, entries);
}

pub(super) fn show_candidate_menu(owner: HWND, data: &Data, entries: Vec<(String, Click)>) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    menu::show(
        owner,
        pt,
        entries,
        data.view.font_family.clone(),
        data.scale,
        data.view.candidate_style.clone(),
        data.view.candidate_skin.clone(),
    );
}

/// 将高分辨率滚轮的零碎移动折合成系统的一格（120）。
pub(super) fn wheel_steps(remainder: &mut i32, delta: i32) -> i32 {
    let total = *remainder + delta;
    *remainder = total % 120;
    total / 120
}

#[test]
pub(super) fn wheel_pages_accumulate_and_cancel() {
    let mut rest = 0;
    assert_eq!(wheel_steps(&mut rest, -40), 0);
    assert_eq!(wheel_steps(&mut rest, 30), 0);
    assert_eq!(wheel_steps(&mut rest, -120), -1);
    assert_eq!(rest, -10);
    assert_eq!(wheel_steps(&mut rest, 250), 2);
    assert_eq!(rest, 0);
}

pub(super) unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_RESTORE_KEYBOARD_OPEN => {
            dispatch(Click::RestoreKeyboardOpen);
            return LRESULT(0);
        }
        WM_LANGUAGE_MENU => {
            dispatch(Click::LanguageMenu(wparam.0 as i32, lparam.0 as i32));
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            if let Some(data) = data_of(hwnd) {
                paint(hwnd, data);
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_SETCURSOR => {
            let hovering = data_of(hwnd).is_some_and(|d| d.hovered.is_some());
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, if hovering { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(cursor));
                }
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(data) = data_of(hwnd) {
                let hovered = hit(data, x, y);
                if hovered != data.hovered {
                    data.hovered = hovered;
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                        if let Ok(cursor) = LoadCursorW(
                            None,
                            if hovered.is_some() {
                                IDC_HAND
                            } else {
                                IDC_ARROW
                            },
                        ) {
                            SetCursor(Some(cursor));
                        }
                    }
                }
            }
            unsafe {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as i16 as i32;
            let steps = {
                let Some(data) = data_of(hwnd) else {
                    return LRESULT(0);
                };
                if !data.view.candidate_wheel || !(data.view.has_prev || data.view.has_next) {
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                }
                wheel_steps(&mut data.wheel_delta, delta)
            };
            // dispatch 会更新窗口数据，不能带着上面的借用调用。
            for _ in 0..steps.unsigned_abs() {
                if !data_of(hwnd).is_some_and(|data| {
                    if steps < 0 {
                        data.view.has_next
                    } else {
                        data.view.has_prev
                    }
                }) {
                    break;
                }
                if let Some(data) = data_of(hwnd) {
                    dispatch(
                        data.click(
                            if data
                                .view
                                .sentence
                                .as_ref()
                                .is_some_and(|v| v.cursor.is_some())
                            {
                                CandidateAction::SentencePage { next: steps < 0 }
                            } else {
                                CandidateAction::Page { next: steps < 0 }
                            },
                        ),
                    );
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(data) = data_of(hwnd) {
                data.wheel_delta = 0;
                if data.hovered.take().is_some() {
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            if let Some(data) = data_of(hwnd) {
                data.long_fired = false;
                data.pressed = hit(data, x, y);
                unsafe {
                    SetCapture(hwnd);
                    if matches!(data.pressed, Some(Hover::Item(_))) && data.view.forgettable {
                        SetTimer(Some(hwnd), LONG_TIMER, LONG_PRESS_MS, None);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            unsafe {
                let _ = KillTimer(Some(hwnd), LONG_TIMER);
                if GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
            }
            let click = data_of(hwnd).and_then(|data| {
                let pressed = data.pressed.take();
                if data.long_fired {
                    return None;
                }
                match hit(data, x, y) {
                    Some(Hover::Item(i)) if pressed == Some(Hover::Item(i)) => Some(
                        data.click(
                            if data
                                .view
                                .sentence
                                .as_ref()
                                .is_some_and(|v| v.cursor.is_some())
                            {
                                CandidateAction::SentenceChoose { index: i }
                            } else {
                                CandidateAction::Select { index: i }
                            },
                        ),
                    ),
                    Some(Hover::Sentence(at)) if pressed == Some(Hover::Sentence(at)) => {
                        Some(data.click(CandidateAction::SentenceFocus { at }))
                    }
                    Some(Hover::Letter(at)) if pressed == Some(Hover::Letter(at)) => {
                        Some(data.click(CandidateAction::SentenceCaret { at }))
                    }
                    Some(Hover::Arrow(next)) => Some(
                        data.click(
                            if data
                                .view
                                .sentence
                                .as_ref()
                                .is_some_and(|v| v.cursor.is_some())
                            {
                                CandidateAction::SentencePage { next }
                            } else {
                                CandidateAction::Page { next }
                            },
                        ),
                    ),
                    _ => None,
                }
            });
            if let Some(click) = click {
                dispatch(click);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            let item = data_of(hwnd).and_then(|data| match hit(data, x, y) {
                Some(Hover::Item(i)) => Some(i),
                _ => None,
            });
            open_menu(hwnd, item);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == LONG_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), LONG_TIMER);
                if GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
            }
            let item = data_of(hwnd).and_then(|data| {
                let Some(Hover::Item(item)) = data.pressed else {
                    return None;
                };
                data.long_fired = true;
                Some(item)
            });
            if let Some(item) = item {
                open_menu(hwnd, Some(item));
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == CARET_TIMER => {
            caret_tick(hwnd);
            LRESULT(0)
        }
        WM_CARET_TICK => {
            caret_tick(hwnd);
            LRESULT(0)
        }
        WM_EDITOR_UNAVAILABLE => {
            if let Some(data) = data_of(hwnd)
                .filter(|d| d.painted_revision != 0 && d.painted_revision == wparam.0 as u64)
            {
                dispatch(data.click(CandidateAction::SentenceLeave));
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == PREDICT_TIMER => {
            // 鼠标正停在联想词上：用户可能要点，下一轮再看
            if data_of(hwnd).is_none_or(|d| d.hovered.is_none()) {
                unsafe {
                    let _ = KillTimer(Some(hwnd), PREDICT_TIMER);
                }
                watch_outside_clicks(hwnd, false);
                dispatch(Click::Dismiss);
            }
            LRESULT(0)
        }
        WM_OUTSIDE_CLICK => {
            dismiss_on_leave(hwnd);
            LRESULT(0)
        }
        WM_ACTIVATEAPP if wparam.0 == 0 => {
            dismiss_on_leave(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == PREDICTION_SPACE_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), PREDICTION_SPACE_TIMER);
            }
            dispatch(Click::PredictionSpaceTimeout);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == FOCUS_NOTICE_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), FOCUS_NOTICE_TIMER);
            }
            dispatch(Click::FocusNotice);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == ENHANCEMENT_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), ENHANCEMENT_TIMER);
            }
            if let Some(data) = data_of(hwnd) {
                dispatch(Click::RefreshEnhancement(data.painted_revision));
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == HINT_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), HINT_TIMER);
            }
            if data_of(hwnd).is_some_and(|d| d.view.hint.is_some()) {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => unsafe {
            if OUTSIDE
                .with(Cell::get)
                .is_some_and(|(_, owner)| owner == hwnd)
            {
                watch_outside_clicks(hwnd, false);
            }
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Data;
            if !ptr.is_null() {
                (*ptr).alive.set(false);
                drop(Box::from_raw(ptr));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub(super) fn register_classes(instance: HINSTANCE) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        for (name, proc_) in [
            (
                CLASS_NAME,
                window_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
            ),
            (MENU_CLASS, menu::proc_),
            (EDITOR_CLASS, window_proc),
        ] {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                // 辅助编辑区不投影，避免阴影覆盖最重要的候选窗。
                style: CS_HREDRAW
                    | CS_VREDRAW
                    | if name == EDITOR_CLASS {
                        Default::default()
                    } else {
                        CS_DROPSHADOW
                    },
                lpfnWndProc: Some(proc_),
                hInstance: instance,
                lpszClassName: name,
                ..Default::default()
            };
            RegisterClassExW(&class);
        }
    });
}

pub(super) fn round_corners(hwnd: HWND) {
    let corner = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );
    }
}

pub(super) fn popup(instance: HINSTANCE, class: PCWSTR, owner: Option<HWND>) -> Option<HWND> {
    register_classes(instance);
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            &HSTRING::from("FreeType"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            owner,
            None,
            Some(instance),
            None,
        )
        .ok()?;
        round_corners(hwnd);
        Some(hwnd)
    }
}

pub(super) fn set_popup_owner(hwnd: HWND, owner: HWND) -> Result<()> {
    unsafe {
        if GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) as isize != owner.0 as isize {
            // Zero can also mean success when the previous owner was null.
            SetLastError(WIN32_ERROR(0));
            if SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as _) == 0 {
                let error = GetLastError();
                if error.0 != 0 {
                    return Err(Error::from_hresult(error.to_hresult()));
                }
            }
        }
    }
    Ok(())
}

/// 屏幕上能放窗口的区域（光标所在显示器的工作区）。
pub(super) fn work_area(x: i32, y: i32) -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork
        } else {
            rect(0, 0, 1 << 15, 1 << 15)
        }
    }
}
