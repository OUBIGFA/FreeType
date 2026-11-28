//! TSF 回调接口：激活停用、按键、开关、线程焦点、组字终止与排版变化。
use super::*;

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        pair::finish(&self.state, false);
        finish_prediction_space(&self.state, true, false);
        let menu = std::mem::take(&mut self.state.borrow_mut().language_menu);
        if menu == MenuState::Open {
            unsafe {
                let _ = EndMenu();
            }
        }
        ui::set_click_handler(None);
        ui::set_caret_handler(None);
        self.state.borrow_mut().focus_notice = Default::default();
        unwatch_layout(&self.state);
        let mut s = self.state.borrow_mut();
        s.generation = s.generation.wrapping_add(1);
        s.caret_move = None;
        if let Some(thread_mgr) = s.thread_mgr.take() {
            unsafe {
                if let (Some(button), Ok(items)) =
                    (s.button.take(), thread_mgr.cast::<ITfLangBarItemMgr>())
                {
                    button.detach();
                    if let Some(item) = button.as_item() {
                        let _ = items.RemoveItem(&item);
                    }
                }
                if let Ok(keystrokes) = thread_mgr.cast::<ITfKeystrokeMgr>() {
                    let _ = keystrokes.UnadviseKeyEventSink(s.client_id);
                }
                if let (Some(cookie), Ok(source)) =
                    (s.thread_sink_cookie.take(), thread_mgr.cast::<ITfSource>())
                {
                    let _ = source.UnadviseSink(cookie);
                }
                if let (Some(cookie), Ok(source)) =
                    (s.thread_focus_cookie.take(), thread_mgr.cast::<ITfSource>())
                {
                    let _ = source.UnadviseSink(cookie);
                }
                if let (Some(cookie), Some(source)) =
                    (s.open_close_cookie.take(), open_close_source(&thread_mgr))
                {
                    let _ = source.UnadviseSink(cookie);
                }
            }
        }
        s.composition = None;
        s.context = None;
        s.sensitive_context = None;
        s.key_preview = None;
        s.ctrl_space_toggled = None;
        s.busy = false;
        s.predicting = false;
        s.prediction_caret = None;
        s.space = None;
        s.prediction_escape = PredictionEscape::Free;
        s.sink = None;
        s.layout_sink = None;
        s.window = None;
        s.client.disconnect();
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, ptim: Ref<ITfThreadMgr>, tid: u32, _flags: u32) -> Result<()> {
        let thread_mgr = ptim.ok()?.clone();
        let key_sink: ITfKeyEventSink = self.to_interface();
        let thread_sink: ITfThreadMgrEventSink = self.to_interface();
        let composition_sink: ITfCompositionSink = self.to_interface();
        let layout_sink: ITfTextLayoutSink = self.to_interface();
        unsafe {
            let keystrokes: ITfKeystrokeMgr = thread_mgr.cast()?;
            keystrokes.AdviseKeyEventSink(tid, &key_sink, true)?;
            let source: ITfSource = thread_mgr.cast()?;
            let cookie = source
                .AdviseSink(&ITfThreadMgrEventSink::IID, &thread_sink)
                .ok();
            self.state.borrow_mut().thread_sink_cookie = cookie;
            let focus_sink: windows::Win32::UI::TextServices::ITfThreadFocusSink =
                self.to_interface();
            match source.AdviseSink(
                &windows::Win32::UI::TextServices::ITfThreadFocusSink::IID,
                &focus_sink,
            ) {
                Ok(cookie) => self.state.borrow_mut().thread_focus_cookie = Some(cookie),
                Err(error) => crate::log::write("订阅线程焦点失败", &error.to_string()),
            }
            let open_close_sink: ITfCompartmentEventSink = self.to_interface();
            let cookie = open_close_source(&thread_mgr).and_then(|source| {
                source
                    .AdviseSink(&ITfCompartmentEventSink::IID, &open_close_sink)
                    .ok()
            });
            self.state.borrow_mut().open_close_cookie = cookie;
        }
        let weak = Rc::downgrade(&self.state);
        let button = langbar::create(Rc::new(move |click| on_button(&weak, click)));
        if let Err(error) = button.watch_brand(&thread_mgr) {
            crate::log::write("语言栏图标监视启动失败", &error.to_string());
        }
        unsafe {
            if let (Ok(items), Some(item)) =
                (thread_mgr.cast::<ITfLangBarItemMgr>(), button.as_item())
            {
                let _ = items.AddItem(&item);
            }
        }
        set_keyboard_open(&thread_mgr, tid);
        let focus = unsafe { thread_mgr.GetFocus().ok() };
        {
            let mut s = self.state.borrow_mut();
            s.generation = s.generation.wrapping_add(1);
            s.thread_mgr = Some(thread_mgr);
            s.client_id = tid;
            s.sink = Some(composition_sink);
            s.layout_sink = Some(layout_sink);
            s.window = CandidateWindow::new(self.instance);
            s.button = Some(button);
        }
        let weak = Rc::downgrade(&self.state);
        ui::set_click_handler(Some(Box::new(move |click| on_click(&weak, click))));
        let weak = Rc::downgrade(&self.state);
        ui::set_caret_handler(Some(Box::new(move || {
            weak.upgrade().is_some_and(|state| caret_tick(&state))
        })));
        if let Some(focus) = focus {
            on_focus(&self.state, Some(&focus));
        }
        Ok(())
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> Result<()> {
        if !foreground.as_bool() {
            pair::finish(&self.state, false);
            self.state.borrow_mut().pairs.clear();
            dismiss_prediction(&self.state);
            self.state.borrow_mut().key_preview = None;
            self.state.borrow_mut().prediction_escape = PredictionEscape::Free;
            self.state.borrow_mut().ctrl_space_toggled = None;
            with_window(&self.state, |window| window.hide());
        }
        Ok(())
    }

    fn OnTestKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        self.state.borrow_mut().key_preview = None;
        if own_input() {
            return Ok((!self.state.borrow().pair_delivery.sent).into());
        }
        let ms = unsafe { GetMessageTime() } as u32;
        let vk = wparam.0 as u16;
        note_key_down(&self.state, wparam, lparam);
        // 先取得正式按下回调，再同步落实预编辑空格；预询问不是应用已处理按键的屏障。
        if vk != 0x1b && self.state.borrow().space.is_some() {
            return Ok(true.into());
        }
        if vk == keys::VK_CAPITAL {
            caps_changed(&self.state, pic.as_ref());
        }
        if keys::is_modifier(vk) || check_sensitive(&self.state, pic.as_ref()) {
            return Ok(false.into());
        }
        if let Some(context) = pic.as_ref() {
            discard_prediction_from_other_context(&self.state, context);
        }
        watch_backspace(&self.state, pic.as_ref(), vk);
        let Some(key) = translate(wparam, lparam, true) else {
            dismiss_prediction(&self.state);
            // 组字时不认识的键也吃掉，免得漏给应用
            return Ok(composing(&self.state).into());
        };
        let repeat = lparam.0 & (1 << 30) != 0;
        // 同一消息已由按键回调或系统状态变化处理过；新按下不依赖上一回是否收到松开。
        if keys::is_ctrl_space(&key) && ctrl_space_handled(&self.state, ms, repeat) {
            return Ok(true.into());
        }
        if prediction_escape_down(&self.state, pic.as_ref(), key, repeat)
            || verified_pair_key(&self.state, pic.as_ref(), key).is_some()
        {
            return Ok(true.into());
        }
        let (key, prediction_space) = pic.as_ref().map_or((key, false), |context| {
            prediction_space_key(
                &self.state,
                context,
                key,
                repeat,
                shown_revision(&self.state),
                ms,
            )
        });
        if prediction_space && key.keysym == 0x20 && self.state.borrow().predict_double_space {
            return Ok(true.into());
        }
        // 在决定接管按键前读取上文。若等到 OnKeyDown 才读，CUAS 的空文档会让
        // 自动空格从“接管并上屏”变成“放行”，重命名等输入框会丢掉这一次按键。
        let before = pic
            .as_ref()
            .filter(|_| !prediction_space)
            .and_then(|context| key_before(&self.state, context));
        let revision = shown_revision(&self.state);
        let frame = call(
            &self.state,
            &Request::PhysicalKey {
                key,
                before: before.clone(),
                scan: 0,
                shown_revision: Some(revision),
            },
        );
        if let Some(frame) = &frame {
            // 放行的数字等会由宿主立即写入，无法进入延迟队列；不能让旧的左移键
            // 跟在这些文字后执行。接管的拼音仍保持等待左移完成的原有顺序。
            if !frame.handled && self.state.borrow().caret_move.is_some() {
                pair::finish(&self.state, true);
            }
            note_preview(&self.state, frame);
            if frame.handled {
                if let Some(context) = pic.as_ref() {
                    self.state.borrow_mut().key_preview = Some(KeyPreview {
                        key: Key { test: false, ..key },
                        context: context.clone(),
                        before,
                    });
                }
            }
            // A passed-through key won't get OnKeyDown. The server has
            // already put the prediction aside; don't leave the old popup.
            if !frame.handled && !frame.predict {
                clear_prediction(&self.state);
            }
        }
        Ok(frame.is_some_and(|f| f.handled).into())
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let preview = self.state.borrow_mut().key_preview.take();
        if own_input() {
            return Ok((!self.state.borrow().pair_delivery.sent).into());
        }
        let ms = unsafe { GetMessageTime() } as u32;
        let vk = wparam.0 as u16;
        note_key_down(&self.state, wparam, lparam);
        let repeat = lparam.0 & (1 << 30) != 0;
        let plain = plain_key(Key {
            modifiers: keys::modifiers(|vk| unsafe { GetKeyState(i32::from(vk)) }),
            ..Key::default()
        });
        if self.state.borrow().space.is_some() && (vk != 0x20 || repeat || !plain) {
            let erase = vk == 0x08 && plain;
            finish_prediction_space(&self.state, !erase, true);
            if erase {
                dismiss_prediction(&self.state);
                return Ok(true.into());
            }
        }
        if vk == keys::VK_CAPITAL {
            caps_changed(&self.state, pic.as_ref());
        }
        if keys::is_modifier(vk) || check_sensitive(&self.state, pic.as_ref()) {
            return Ok(false.into());
        }
        watch_backspace(&self.state, pic.as_ref(), vk);
        let context = pic.ok()?.clone();
        discard_prediction_from_other_context(&self.state, &context);
        self.state.borrow_mut().context = Some(context.clone());
        let Some(key) = translate(wparam, lparam, false) else {
            // 翻不成键码的键（功能键等）交给应用；显示着的联想词先收起
            dismiss_prediction(&self.state);
            return Ok(composing(&self.state).into());
        };
        if keys::is_ctrl_space(&key) && ctrl_space_handled(&self.state, ms, repeat) {
            return Ok(true.into());
        }
        if prediction_escape_down(&self.state, Some(&context), key, repeat) {
            return Ok(true.into());
        }
        if let Some((action, early)) = verified_pair_key(&self.state, Some(&context), key) {
            if let Some(mut frame) = call(&self.state, &Request::ClosePair) {
                if early {
                    // 左移键还没发：只吞掉右键，保留待发的左移键，光标照常回到两半之间。
                    // 不交给 Pair::apply，它会取走成对标点的位置，之后的跳过和退格删整对就失效。
                    self.state.borrow_mut().pair_delivery.absorbed = true;
                } else {
                    frame.pair_edit = Some(action);
                }
                apply(&self.state, Some(&context), frame, true);
                return Ok(true.into());
            }
        }
        let (key, prediction_space) = prediction_space_key(
            &self.state,
            &context,
            key,
            repeat,
            shown_revision(&self.state),
            ms,
        );
        if prediction_space && key.keysym == 0x20 && self.state.borrow().predict_double_space {
            return Ok(true.into());
        }
        let before = if prediction_space {
            None
        } else {
            preview
                .filter(|preview| preview.key == key && same_object(&preview.context, &context))
                .map(|preview| preview.before)
                .unwrap_or_else(|| key_before(&self.state, &context))
        };
        let scan = ((lparam.0 >> 16) & 0x1ff) as u16;
        let revision = shown_revision(&self.state);
        let frame = match call(
            &self.state,
            &Request::PhysicalKey {
                key,
                before,
                scan,
                shown_revision: Some(revision),
            },
        ) {
            Some(frame) => frame,
            None => {
                // 服务进程没了：结束编辑区，键交给应用
                finish_prediction_space(&self.state, true, true);
                if composing(&self.state) {
                    apply(&self.state, Some(&context), Frame::default(), true);
                }
                return Ok(false.into());
            }
        };
        let handled = frame.handled;
        // 部分宿主直接调用正式按键回调，仍需在把键交给宿主前取消旧移动。
        if !handled && self.state.borrow().caret_move.is_some() {
            pair::finish(&self.state, true);
        }
        if handled && keys::is_ctrl_space(&key) {
            self.state.borrow_mut().ctrl_space_toggled = Some(ms);
        }
        let visible = self
            .state
            .borrow()
            .window
            .as_ref()
            .is_some_and(|w| w.visible());
        if handled || frame.commit.is_some() || composing(&self.state) || visible {
            apply(&self.state, Some(&context), frame, true);
        } else {
            note_frame(&self.state, &frame);
        }
        Ok(handled.into())
    }

    fn OnTestKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        if prediction_escape_up(&self.state, wparam, true) {
            return Ok(true.into());
        }
        key_released(&self.state, pic.as_ref(), wparam, lparam);
        Ok(false.into())
    }

    fn OnKeyUp(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        if prediction_escape_up(&self.state, wparam, false) {
            return Ok(true.into());
        }
        key_released(&self.state, pic.as_ref(), wparam, lparam);
        Ok(false.into())
    }

    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(false.into())
    }
}

impl ITfCompartmentEventSink_Impl for TextService_Impl {
    fn OnChange(&self, rguid: *const GUID) -> Result<()> {
        if unsafe { rguid.as_ref() } != Some(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) {
            return Ok(());
        }
        // 应用或系统改这个状态的时机不受我们控制；正在改 State 时不处理，避免在宿主进程里 panic
        let Ok(thread_mgr) = self.state.try_borrow().map(|s| s.thread_mgr.clone()) else {
            return Ok(());
        };
        let closed = thread_mgr
            .and_then(|t| t.cast::<ITfCompartmentMgr>().ok())
            .is_some_and(|mgr| {
                compartment_value(&mgr, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) == Some(0)
            });
        let held = unsafe {
            let down = |vk: i32| GetAsyncKeyState(vk) < 0;
            down(0x11) && down(0x20) && !down(0x10) && !down(0x12) && !down(0x5b) && !down(0x5c)
        };
        if closed && held {
            ctrl_space_closed(&self.state);
        }
        Ok(())
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnSetFocus(&self, focus: Ref<ITfDocumentMgr>, _previous: Ref<ITfDocumentMgr>) -> Result<()> {
        on_focus(&self.state, focus.as_ref());
        Ok(())
    }

    fn OnPushContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    /// 应用结束了编辑区（点了别处、切了输入框）：编辑区的字留在文档里，会话清空。
    fn OnCompositionTerminated(&self, _ec: u32, composition: Ref<ITfComposition>) -> Result<()> {
        let Some(composition) = composition.as_ref() else {
            return Ok(());
        };
        let pending = self
            .state
            .borrow()
            .space
            .as_ref()
            .is_some_and(|space| same_object(&space.composition, composition));
        if pending {
            take_prediction_space(&self.state);
            clear_prediction(&self.state);
        } else if !self
            .state
            .borrow()
            .composition
            .as_ref()
            .is_some_and(|current| same_object(current, composition))
        {
            // 输入法主动结束的旧编辑区，不能清掉已开始的下一次输入。
            return Ok(());
        }
        self.state.borrow_mut().composition = None;
        with_window(&self.state, |window| window.hide());
        if let Some(frame) = call(&self.state, &Request::Reset { commit: false }) {
            note_frame(&self.state, &frame);
        }
        Ok(())
    }
}

impl ITfTextLayoutSink_Impl for TextService_Impl {
    /// 应用排好了版（浏览器类应用打字后一会儿才给得出位置）：候选窗挪到编辑区下面。
    fn OnLayoutChange(
        &self,
        pic: Ref<ITfContext>,
        _lcode: TfLayoutCode,
        _view: Ref<ITfContextView>,
    ) -> Result<()> {
        let Some(context) = pic.as_ref().cloned() else {
            return Ok(());
        };
        // RichEdit 等宿主会在我们改状态或摆候选窗时同步发来排版通知。这时跳过，
        // 编辑区下一次排版或下一个键会再摆一次；在宿主进程里 panic 会直接让应用崩溃。
        let Ok((composition, client_id, generation)) = self.state.try_borrow().map(|s| {
            (
                s.composition
                    .clone()
                    .or_else(|| s.space.as_ref().map(|space| space.composition.clone())),
                s.client_id,
                s.generation,
            )
        }) else {
            return Ok(());
        };
        let Some(composition) = composition else {
            return Ok(());
        };
        let state = self.state.clone();
        let context2 = context.clone();
        edit_with(
            &context,
            client_id,
            TF_ES_ASYNCDONTCARE | TF_ES_READ,
            move |ec| {
                if state
                    .try_borrow()
                    .map_or(true, |s| s.generation != generation)
                {
                    return Ok(());
                }
                let rect = unsafe { composition.GetRange() }
                    .ok()
                    .and_then(|range| text_rect(&context2, ec, &range));
                if let Some(rect) = rect {
                    {
                        let Ok(mut s) = state.try_borrow_mut() else {
                            return Ok(());
                        };
                        s.caret = Some(rect);
                        s.last_caret = Some(rect);
                    }
                    with_window(&state, |window| window.reposition(rect));
                }
                Ok(())
            },
        );
        Ok(())
    }
}
