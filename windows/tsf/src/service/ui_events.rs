//! 候选窗点击、任务栏按钮与右键菜单。
use super::*;

pub(super) fn on_click(state: &Weak<RefCell<State>>, click: Click) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let request = match click {
        Click::RefreshEnhancement(revision) => {
            let Some(frame) = call(&state, &Request::RefreshEnhancement { revision }) else {
                return;
            };
            // 多数轮询还没有新结果：服务进程原样返回这一页（未处理、页码不变）。
            // 这时不重写编辑区、不重画候选窗，免得应用每 100 毫秒重排一次文字。
            if !frame.handled && frame.page_revision == revision {
                if let Some(window) = state.borrow().window.as_ref() {
                    window.poll_enhancement(frame.enhancement_pending);
                }
                return;
            }
            let context = state.borrow().context.clone();
            return apply(&state, context.as_ref(), frame, false);
        }
        Click::LanguageMenu(x, y) => {
            let (context, generation) = {
                let mut s = state.borrow_mut();
                if s.language_menu != MenuState::Queued {
                    return;
                }
                s.language_menu = MenuState::Open;
                (s.context.clone(), s.generation)
            };
            // Like Weasel, anchor the native menu to the focused document. Its lifetime
            // is independent of the candidate and hint windows and their hide timers.
            let owner = unsafe {
                context
                    .and_then(|c| c.GetActiveView().and_then(|v| v.GetWnd()).ok())
                    .filter(|w| !w.is_invalid())
                    .unwrap_or_else(|| windows::Win32::UI::Input::KeyboardAndMouse::GetFocus())
            };
            let _dll = DllRef::new();
            let selected = button_menu(owner, x, y);
            let mut s = state.borrow_mut();
            if s.generation != generation {
                return;
            }
            s.language_menu = MenuState::Closed;
            let Some(request) = selected else {
                return;
            };
            request
        }
        Click::Candidate { revision, action } => Request::CandidateAction { revision, action },
        Click::PinChoices { .. } => return,
        Click::Dismiss => return dismiss_prediction(&state),
        Click::PredictionSpaceTimeout => return prediction_space_timeout(&state),
        Click::FocusNotice => return focus_notice::on_timer(&state),
        Click::RestoreKeyboardOpen => {
            let (manager, client_id) = {
                let s = state.borrow();
                (s.thread_mgr.clone(), s.client_id)
            };
            if let Some(manager) = manager {
                set_keyboard_open(&manager, client_id);
            }
            return;
        }
        Click::Settings => Request::Command {
            command: Command::OpenSettings,
        },
    };
    let Some(frame) = call(&state, &request) else {
        return;
    };
    if matches!(
        request,
        Request::Command {
            command: Command::OpenSettings
        }
    ) {
        return;
    }
    let context = state.borrow().context.clone();
    apply(&state, context.as_ref(), frame, false);
}

/// 任务栏按钮：左键切换中英文，右键菜单。
pub(super) fn on_button(state: &Weak<RefCell<State>>, click: ButtonClick) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let request = match click {
        ButtonClick::Toggle => Request::ToggleMode { shift: None },
        ButtonClick::Menu(x, y) => {
            // Return to the language bar immediately. Coalesce clicks even while
            // Windows dispatches messages inside TrackPopupMenu.
            let mut s = state.borrow_mut();
            if s.language_menu == MenuState::Closed {
                if let Some(window) = &s.window {
                    if unsafe {
                        PostMessageW(
                            Some(window.hwnd()),
                            ui::WM_LANGUAGE_MENU,
                            WPARAM(x as usize),
                            LPARAM(y as isize),
                        )
                    }
                    .is_ok()
                    {
                        s.language_menu = MenuState::Queued;
                    }
                }
            }
            return;
        }
    };
    let Some(frame) = call(&state, &request) else {
        return;
    };
    let context = state.borrow().context.clone();
    apply(&state, context.as_ref(), frame, false);
}

pub(super) fn button_menu(owner: HWND, x: i32, y: i32) -> Option<Request> {
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WS_EX_TOOLWINDOW, WS_POPUP,
    };
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let items: [(u32, &str); 7] = [
            (1, "切换中英文"),
            (2, "中文 / 英文标点"),
            (3, "简体 / 繁体"),
            (4, "表情与符号"),
            (5, "工具栏"),
            (7, "英文候选"),
            (8, "拆字查字"),
        ];
        for (id, text) in items {
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                id as usize,
                &windows::core::HSTRING::from(text),
            );
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 6, windows::core::w!("FreeType 设置"));
        // The shell can deliver the click without a focused edit control. A separate
        // owner in that case survives every hide/update of the candidate window.
        let temporary = if owner.is_invalid() {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                windows::core::w!("STATIC"),
                None,
                WS_POPUP,
                x,
                y,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .ok()
        } else {
            None
        };
        let owner = temporary.unwrap_or(owner);
        let cmd = if owner.is_invalid() {
            BOOL(0)
        } else {
            TrackPopupMenu(
                menu,
                TPM_NONOTIFY | TPM_RETURNCMD | TPM_RIGHTBUTTON,
                x,
                y,
                None,
                owner,
                None,
            )
        };
        let _ = DestroyMenu(menu);
        if let Some(hwnd) = temporary {
            let _ = DestroyWindow(hwnd);
        }
        let command = match cmd.0 {
            1 => return Some(Request::ToggleMode { shift: None }),
            2 => Command::TogglePunct,
            3 => Command::ToggleTraditional,
            4 => Command::EmojiPanel,
            5 => Command::Toolbar,
            6 => Command::OpenSettings,
            7 => Command::EnglishCandidates,
            8 => Command::RadicalLookup,
            _ => return None,
        };
        Some(Request::Command { command })
    }
}
