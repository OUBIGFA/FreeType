//! 整句编辑区窗口：层级低于候选窗，空间不足时收起。
use super::*;

impl EditorWindow {
    pub(super) fn new(instance: HINSTANCE, owner: HWND) -> Option<Self> {
        // 与候选窗共享宿主，而不是成为候选窗的 owned popup；后者会被系统强制压在主窗上面。
        let host = HWND(unsafe { GetWindowLongPtrW(owner, GWLP_HWNDPARENT) } as *mut _);
        let hwnd = popup(instance, EDITOR_CLASS, Some(host))?;
        let alive = Rc::new(Cell::new(true));
        let data = Box::new(Data {
            alive: alive.clone(),
            view: View::default(),
            layout: Layout::default(),
            scale: 1.0,
            hovered: None,
            pressed: None,
            long_fired: false,
            wheel_delta: 0,
            painted_revision: 0,
            anchor: None,
        });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as _);
        }
        Some(Self { hwnd, alive })
    }

    pub(super) fn hide(&self) {
        if !self.alive.get() {
            return;
        }
        if let Some(data) = data_of(self.hwnd) {
            data.painted_revision = 0;
            data.pressed = None;
            data.hovered = None;
            data.wheel_delta = 0;
            data.view.sentence = None;
        }
        unsafe {
            if GetCapture() == self.hwnd {
                let _ = ReleaseCapture();
            }
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub(super) fn show(&self, mut view: View, candidate: HWND) {
        if !self.alive.get() {
            return;
        }
        unsafe {
            let mut anchor = RECT::default();
            if GetWindowRect(candidate, &mut anchor).is_err() {
                self.hide();
                return;
            }
            let scale = GetDpiForWindow(candidate).max(96) as f32 / 96.0;
            let work = work_area(anchor.left, anchor.top);
            // 编辑区独立换行，局部备选变长也不能传回普通候选布局。
            view.available_width = (work.right - work.left).min((640.0 * scale) as i32);
            view.wrap_width = view.available_width;
            let result = Fonts::styled(
                view.font_size.max(12) as f32 * scale,
                &view.font_family,
                0.0,
                &view.font_fallbacks,
                (view.font_size as f32 * 0.72).clamp(11.0, 14.0) * scale,
            )
            .and_then(|fonts| layout(&view, &fonts, scale));
            let layout = match result {
                Ok(layout) => layout,
                Err(error) => {
                    crate::log::write("整句编辑区排版失败", &error.to_string());
                    self.hide();
                    return;
                }
            };
            let gap = (6.0 * scale).round() as i32;
            let y = if view.editor_above {
                anchor.top - gap - layout.height
            } else {
                anchor.bottom + gap
            };
            if y < work.top
                || y + layout.height > work.bottom
                || layout.width > work.right - work.left
            {
                self.hide();
                if let Some(data) = data_of(self.hwnd) {
                    data.view = view.clone();
                }
                // 空间不足时退出词块焦点，避免不可见的局部备选截获数字键。
                if view.sentence.as_ref().is_some_and(|s| s.cursor.is_some()) {
                    let _ = PostMessageW(
                        Some(candidate),
                        WM_EDITOR_UNAVAILABLE,
                        WPARAM(view.revision as usize),
                        LPARAM(0),
                    );
                }
                return;
            }
            let x = anchor.left.min(work.right - layout.width).max(work.left);
            let target = rect(x, y, layout.width, layout.height);
            let mut current = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut current);
            let Some(data) = data_of(self.hwnd) else {
                return;
            };
            if IsWindowVisible(self.hwnd).as_bool() && current == target && data.view == view {
                return;
            }
            data.pressed = None;
            data.painted_revision = 0;
            data.hovered = None;
            data.view = view;
            data.layout = layout;
            data.scale = scale;
            // 插在候选窗之后：同为置顶窗口，主候选仍保持更高的层级。
            let _ = SetWindowPos(
                self.hwnd,
                Some(candidate),
                x,
                y,
                target.right - x,
                target.bottom - y,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            let _ = UpdateWindow(self.hwnd);
        }
    }
}

impl Drop for EditorWindow {
    fn drop(&mut self) {
        if self.alive.get() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// 提示窗的左上角：放在光标的哪一边。
pub(super) fn hint_origin(position: &str, caret: RECT, w: i32, h: i32, gap: i32) -> (i32, i32) {
    let center = caret.left - w / 2;
    match position {
        "top-left" => (caret.left - w - gap, caret.top - h - gap),
        "top" => (center, caret.top - h - gap),
        "top-right" => (caret.right + gap, caret.top - h - gap),
        "bottom-left" => (caret.left - w - gap, caret.bottom + gap),
        "bottom" => (center, caret.bottom + gap),
        _ => (caret.right + gap, caret.bottom + gap),
    }
}
