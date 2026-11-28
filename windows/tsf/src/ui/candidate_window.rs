//! 候选窗的显示、定位和整句编辑区的上下摆放。
use super::*;

impl CandidateWindow {
    pub fn update(&self, frame: &Frame, caret: Option<RECT>) {
        if !self.alive.get() {
            return;
        }
        // 宿主暂未返回光标位置时，也必须立即撤掉已关闭的辅助区。
        if frame.sentence.is_none() || frame.hint.is_some() || frame.predict {
            self.update_editor(None);
        }
        self.poll_enhancement(frame.enhancement_pending);
        let view = View {
            sentence: None,
            editor_above: frame.sentence_editor_above,
            title: frame.title.clone(),
            preedit: if frame.hide_candidate_preedit {
                String::new()
            } else {
                frame.preedit.clone()
            },
            candidates: frame.candidates.clone(),
            annotations: frame.annotations.clone(),
            badges: frame.badges.clone(),
            labels: frame.labels.clone(),
            cursor: frame.cursor,
            has_prev: frame.has_prev,
            has_next: frame.has_next,
            candidate_wheel: frame.candidate_wheel,
            revision: frame.page_revision,
            pin_slots: frame.pin_slots.clone(),
            pin_limit: frame.pin_limit,
            hint: frame.hint.clone(),
            hint_position: frame.hint_position.clone(),
            font_size: frame.font_size,
            font_family: frame.font_family.clone(),
            font_fallbacks: frame.font_fallbacks.clone(),
            preedit_size: frame.preedit_size,
            lock_position: frame.lock_candidate_position,
            wrap: frame.wrap_candidates,
            wrap_width: 0,
            available_width: 0,
            candidate_style: frame.candidate_style.clone(),
            candidate_skin: frame.candidate_skin.clone(),
            vertical: frame.vertical,
            panel: frame.panel,
            columns: frame.columns,
            forgettable: frame.forgettable,
            prediction: frame.predict,
        };
        let visible = view.hint.is_some() || !view.candidates.is_empty() || view.title.is_some();
        if !visible {
            self.hide();
            return;
        }
        // 左右键选联想只重画高亮。宿主此时返回的插入符可能仍在变动，不能用它
        // 重新定位；保留同一页已测量的文字、点击区域和窗口大小，直到真正上屏。
        if frame.commit.is_none() && self.visible() {
            if let Some(data) =
                data_of(self.hwnd).filter(|data| data.view.same_prediction_page(&view))
            {
                if data.view.revision != view.revision {
                    data.pressed = None;
                    data.painted_revision = 0;
                }
                data.view.cursor = view.cursor;
                data.view.revision = view.revision;
                unsafe {
                    let _ = InvalidateRect(Some(self.hwnd), None, false);
                    let _ = UpdateWindow(self.hwnd);
                    SetTimer(Some(self.hwnd), PREDICT_TIMER, PREDICT_MS, None);
                }
                watch_outside_clicks(self.hwnd, true);
                self.update_editor(None);
                return;
            }
        }
        let Some(caret) = caret else {
            return;
        };
        self.show(view, caret);
        self.update_editor(frame.sentence.as_ref());
        unsafe {
            // 每来一批新的联想词重新计时；换成普通候选就不计了
            if frame.predict {
                SetTimer(Some(self.hwnd), PREDICT_TIMER, PREDICT_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), PREDICT_TIMER);
            }
        }
        watch_outside_clicks(self.hwnd, frame.predict);
    }

    /// 编辑区挪了位置（应用排好版以后）：窗口跟过去。
    pub fn reposition(&self, caret: RECT) {
        if !self.visible() {
            return;
        }
        if let Some(data) = data_of(self.hwnd) {
            // 联想有独立的正文锚点；真正移动光标由文本服务收起这一页。
            if data.view.prediction {
                return;
            }
            let view = data.view.clone();
            self.show(view, caret);
            if let Some(editor) = self.editor.borrow().as_ref() {
                if editor.alive.get() {
                    if let Some(data) = data_of(editor.hwnd).filter(|d| d.view.sentence.is_some()) {
                        editor.show(data.view.clone(), self.hwnd);
                    }
                }
            }
        }
    }

    /// 排版、摆位置、画出来。
    ///
    /// 打字时每个键都会走到这里（浏览器类应用排好版后还会再来一次），所以已经显示着的窗口
    /// 只做必要的改动：不先挪走再挪回来，内容和位置都没变就什么都不做，变了就当场画完。
    /// （以前每次都先缩成 1×1 挪到光标处取 DPI，再放大、等消息循环空了才重画，每打一个字母闪一下。）
    pub(super) fn show(&self, mut view: View, mut caret: RECT) {
        if let Some(data) = data_of(self.hwnd) {
            if view.lock_position && view.hint.is_none() {
                caret = *data.anchor.get_or_insert(caret);
            } else {
                data.anchor = None;
            }
        }
        menu::close();
        unsafe {
            // 按光标所在显示器的 DPI 排版：窗口还没显示、或光标到了另一块显示器时，先把窗口挪过去
            let visible = self.visible();
            let at = POINT {
                x: caret.left,
                y: caret.bottom,
            };
            if !visible
                || MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST)
                    != MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST)
            {
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    at.x,
                    at.y,
                    1,
                    1,
                    SWP_NOACTIVATE,
                );
            }
            let dpi = GetDpiForWindow(self.hwnd).max(96);
            let scale = dpi as f32 / 96.0;
            let work = work_area(caret.left, caret.bottom);
            view.available_width = work.right - work.left;
            if view.wrap {
                view.wrap_width = work.right - work.left;
            }
            let result = Fonts::styled(
                view.font_size.max(12) as f32 * scale,
                &view.font_family,
                view.preedit_size as f32 * scale,
                &view.font_fallbacks,
                0.0,
            )
            .and_then(|fonts| layout(&view, &fonts, scale));
            let layout = match result {
                Ok(layout) => layout,
                Err(error) => {
                    crate::log::write("候选窗排版失败", &error.to_string());
                    return;
                }
            };

            let gap = (4.0 * scale) as i32;
            let (w, h) = (layout.width, layout.height);
            let hint = view.hint.is_some();
            let (mut x, mut y) = if hint {
                hint_origin(&view.hint_position, caret, w, h, gap)
            } else {
                (caret.left, caret.bottom + gap)
            };
            if !hint && y + h > work.bottom {
                y = caret.top - gap - h;
            }
            x = x.min(work.right - w).max(work.left);
            y = y.min(work.bottom - h).max(work.top);

            let mut current = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut current);
            let mut changed = false;
            if let Some(data) = data_of(self.hwnd) {
                changed = !visible || current != rect(x, y, w, h) || data.view != view;
                if changed {
                    if data.view.revision != view.revision {
                        data.pressed = None;
                        data.painted_revision = 0;
                    }
                    data.view = view;
                    data.layout = layout;
                    data.scale = scale;
                    data.hovered = None;
                }
            }
            if changed {
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    w,
                    h,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                // 整个窗口都自己画（见 paint），不用先擦背景；马上画完，不让变大的部分先露出旧内容
                let _ = InvalidateRect(Some(self.hwnd), None, false);
                let _ = UpdateWindow(self.hwnd);
            }
            if hint {
                SetTimer(Some(self.hwnd), HINT_TIMER, HINT_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), HINT_TIMER);
            }
        }
    }

    pub(super) fn update_editor(&self, sentence: Option<&freetype_protocol::SentenceView>) {
        let mut editor = self.editor.borrow_mut();
        let Some(sentence) = sentence.filter(|s| !s.blocks.is_empty() && self.visible()) else {
            if let Some(editor) = editor.as_ref() {
                editor.hide();
            }
            return;
        };
        let Some(candidate) =
            data_of(self.hwnd).filter(|d| d.view.hint.is_none() && !d.view.prediction)
        else {
            if let Some(editor) = editor.as_ref() {
                editor.hide();
            }
            return;
        };
        if editor.as_ref().is_none_or(|e| !e.alive.get()) {
            *editor = EditorWindow::new(self.instance, self.hwnd);
        }
        let Some(editor) = editor.as_ref() else {
            return;
        };
        let focused = sentence.cursor.is_some();
        let view = View {
            sentence: Some(sentence.clone()),
            editor_above: candidate.view.editor_above,
            title: Some(
                if focused {
                    "数字选局部备选；[ ] 调整切分；End 返回整句"
                } else {
                    "点词块修改"
                }
                .into(),
            ),
            candidates: if focused {
                sentence.alternatives.clone()
            } else {
                Vec::new()
            },
            has_prev: focused && sentence.has_prev,
            has_next: focused && sentence.has_next,
            // 辅助区域字号略小于主候选，提示行另外限制字号。
            font_size: (candidate.view.font_size * 9 / 10).max(12),
            font_family: candidate.view.font_family.clone(),
            font_fallbacks: candidate.view.font_fallbacks.clone(),
            candidate_style: candidate.view.candidate_style.clone(),
            candidate_skin: candidate.view.candidate_skin.clone(),
            candidate_wheel: candidate.view.candidate_wheel,
            revision: candidate.view.revision,
            wrap: true,
            ..View::default()
        };
        editor.show(view, self.hwnd);
    }
}
