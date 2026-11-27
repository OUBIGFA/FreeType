use super::*;
use windows::Win32::UI::TextServices::{ITfThreadFocusSink_Impl, TF_SD_READONLY};

#[derive(Default)]
pub(super) struct Notice {
    field: Option<HWND>,
    thread_lost: bool,
    system_panel: bool,
    pending: bool,
    captured: Option<Frame>,
    epoch: u64,
    measuring: bool,
}

impl Notice {
    pub fn cancel(&mut self) {
        if self.pending || self.measuring {
            self.epoch = self.epoch.wrapping_add(1);
            self.pending = false;
            self.measuring = false;
            self.captured = None;
        }
    }
}

fn field(context: Option<&ITfContext>) -> Option<HWND> {
    let context = context?;
    if disabled(Some(context)) || unsafe { context.GetStatus() }.is_ok_and(|status| status.dwDynamicFlags & TF_SD_READONLY != 0) {
        return None;
    }
    Some(view_window(context).unwrap_or_else(|| unsafe { GetFocus() }))
}

fn system_input_panel() -> bool {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false; };
        let mut path = [0u16; 1024];
        let mut size = path.len() as u32;
        let found = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut size).is_ok();
        let _ = CloseHandle(process);
        found && String::from_utf16_lossy(&path[..size as usize]).rsplit('\\').next().is_some_and(|name| name.eq_ignore_ascii_case("TextInputHost.exe"))
    }
}

fn schedule(state: &Rc<RefCell<State>>) {
    let mut service = state.borrow_mut();
    service.focus_notice.cancel();
    service.focus_notice.pending = true;
    service.focus_notice.captured = None;
    if let Some(window) = &service.window { window.schedule_focus_notice(); }
}

pub(super) fn observe(state: &Rc<RefCell<State>>, context: Option<&ITfContext>) {
    let field = field(context);
    let changed = {
        let mut service = state.borrow_mut();
        let notice = &mut service.focus_notice;
        if notice.system_panel { return; }
        let changed = notice.field != field;
        notice.field = field;
        if field.is_none() { notice.cancel(); }
        changed
    };
    if changed && field.is_some() { schedule(state); }
}

pub(super) fn capture(state: &Rc<RefCell<State>>, frame: Frame) {
    let mut service = state.borrow_mut();
    if service.focus_notice.pending { service.focus_notice.captured = Some(frame); }
}

impl ITfThreadFocusSink_Impl for TextService_Impl {
    fn OnSetThreadFocus(&self) -> Result<()> {
        let returned = {
            let mut service = self.state.borrow_mut();
            let notice = &mut service.focus_notice;
            let panel = std::mem::take(&mut notice.system_panel);
            std::mem::take(&mut notice.thread_lost) && !panel
        };
        if returned { schedule(&self.state); }
        Ok(())
    }

    fn OnKillThreadFocus(&self) -> Result<()> {
        pair::finish(&self.state, false);
        let panel = system_input_panel();
        {
            let mut service = self.state.borrow_mut();
            service.focus_notice.thread_lost = true;
            service.focus_notice.system_panel |= panel;
            service.focus_notice.cancel();
            service.pairs.clear();
        }
        with_window(&self.state, |window| window.hide());
        Ok(())
    }
}

pub(super) fn on_timer(state: &Rc<RefCell<State>>) {
    if !state.borrow().focus_notice.pending { return; }
    let manager = state.borrow().thread_mgr.clone();
    let doc = manager.and_then(|manager| unsafe {
        manager.IsThreadFocus().ok()?.as_bool().then_some(())?;
        manager.GetFocus().ok()
    });
    let active = doc.as_ref().and_then(|doc| unsafe { doc.GetTop().ok() });
    let current = state.borrow().context.clone();
    if active.as_ref().is_some_and(|target| !current.as_ref().is_some_and(|current| same_object(current, target))) {
        on_focus(state, doc.as_ref());
    }
    present(state, active.as_ref());
}

fn insertion_rect(context: &ITfContext, cookie: u32) -> Option<RECT> {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{GUI_INMENUMODE, GUI_POPUPMENUMODE, GUI_SYSTEMMENUMODE};
    unsafe {
        // Tray/context menus can retain the previous editor and its last caret.
        // Check inside the edit session too: it may run after the menu opens.
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(GetCurrentThreadId(), &mut info).ok()?;
        if (info.flags & (GUI_INMENUMODE | GUI_POPUPMENUMODE | GUI_SYSTEMMENUMODE)).0 != 0 { return None; }
        let mut selection = [TF_SELECTION::default()];
        let mut count = 0;
        context.GetSelection(cookie, TF_DEFAULT_SELECTION, &mut selection, &mut count).ok()?;
        let active = selection[0].style.ase;
        let range = ManuallyDrop::into_inner(std::mem::take(&mut selection[0].range))?;
        if count != 1 { return None; }
        range.Collapse(cookie, if active == windows::Win32::UI::TextServices::TF_AE_START { TF_ANCHOR_START } else { TF_ANCHOR_END }).ok()?;
        let mut rect = RECT::default();
        let mut clipped = BOOL::from(true);
        context.GetActiveView().ok()?.GetTextExt(cookie, &range, &mut rect, &mut clipped).ok()?;
        (!clipped.as_bool() && rect.bottom > rect.top && rect.right >= rect.left).then_some(rect)
    }
}

pub(super) fn present(state: &Rc<RefCell<State>>, active: Option<&ITfContext>) {
    let (captured, current, client, epoch) = {
        let mut service = state.borrow_mut();
        if !std::mem::take(&mut service.focus_notice.pending) { return; }
        let captured = service.focus_notice.captured.take();
        if service.focus_notice.thread_lost || service.sensitive || service.busy || service.predicting
            || service.space.is_some() || service.composition.is_some() { return; }
        (captured, service.context.clone(), service.client_id, service.focus_notice.epoch)
    };
    let Some(context) = active.filter(|target| current.as_ref().is_some_and(|old| same_object(old, *target))) else { return; };
    if field(Some(context)).is_none() { return; }
    let captured = captured.filter(|frame| frame.hint.is_some());
    state.borrow_mut().focus_notice.measuring = true;
    let (shared, target) = (state.clone(), context.clone());
    edit_with(context, client, TF_ES_ASYNCDONTCARE | TF_ES_READ, move |cookie| {
        {
            let service = shared.borrow();
            if service.focus_notice.epoch != epoch || !service.focus_notice.measuring
                || service.focus_notice.thread_lost || service.sensitive || service.busy || service.predicting
                || !service.context.as_ref().is_some_and(|context| same_object(context, &target)) { return Ok(()); }
        }
        if sensitive_scope(&target, cookie) { return Ok(()); }
        let Some(caret) = insertion_rect(&target, cookie) else { return Ok(()); };
        let owner = view_window(&target).unwrap_or_else(|| unsafe { GetFocus() });
        let frame = captured.or_else(|| {
            let _ = call(&shared, &Request::CapsLock { on: caps_on() });
            call(&shared, &Request::Focus { active: true, before: None })
        });
        if let Some(mut frame) = frame.filter(|frame| frame.hint.is_some()) {
            frame.candidates.clear();
            frame.title = None;
            {
                let mut service = shared.borrow_mut();
                if service.focus_notice.epoch != epoch || !std::mem::take(&mut service.focus_notice.measuring) { return Ok(()); }
                service.caret = Some(caret);
            }
            with_window(&shared, |window| {
                if let Err(error) = window.set_owner(owner) { crate::log::write("关联候选窗失败", &error.to_string()); return; }
                window.update(&frame, Some(caret));
            });
        }
        Ok(())
    });
}
