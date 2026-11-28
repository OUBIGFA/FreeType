//! 焦点通知和组字的关系（[`composing_context`]）。
//!
//! 只替代应用那一侧的 TSF 对象：文档、上下文、范围、编辑区都是真的 COM 对象，
//! 判断走生产代码。用不到的方法一律 `E_NOTIMPL`。

use windows::core::{implement, IUnknown, OutRef, Ref, Result, BOOL, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::E_NOTIMPL;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::UI::TextServices::*;

use super::composing_context;

fn unsupported<T>() -> Result<T> {
    Err(E_NOTIMPL.into())
}

#[implement(ITfDocumentMgr)]
struct Document;

impl ITfDocumentMgr_Impl for Document_Impl {
    fn CreateContext(
        &self,
        _: u32,
        _: u32,
        _: Ref<IUnknown>,
        _: OutRef<ITfContext>,
        _: *mut u32,
    ) -> Result<()> {
        unsupported()
    }
    fn Push(&self, _: Ref<ITfContext>) -> Result<()> {
        unsupported()
    }
    fn Pop(&self, _: u32) -> Result<()> {
        unsupported()
    }
    fn GetTop(&self) -> Result<ITfContext> {
        unsupported()
    }
    fn GetBase(&self) -> Result<ITfContext> {
        unsupported()
    }
    fn EnumContexts(&self) -> Result<IEnumTfContexts> {
        unsupported()
    }
}

#[implement(ITfContext)]
struct Context {
    document: ITfDocumentMgr,
}

impl ITfContext_Impl for Context_Impl {
    fn RequestEditSession(
        &self,
        _: u32,
        _: Ref<ITfEditSession>,
        _: TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    ) -> Result<HRESULT> {
        unsupported()
    }
    fn InWriteSession(&self, _: u32) -> Result<BOOL> {
        unsupported()
    }
    fn GetSelection(
        &self,
        _: u32,
        _: u32,
        _: u32,
        _: *mut TF_SELECTION,
        _: *mut u32,
    ) -> Result<()> {
        unsupported()
    }
    fn SetSelection(&self, _: u32, _: u32, _: *const TF_SELECTION) -> Result<()> {
        unsupported()
    }
    fn GetStart(&self, _: u32) -> Result<ITfRange> {
        unsupported()
    }
    fn GetEnd(&self, _: u32) -> Result<ITfRange> {
        unsupported()
    }
    fn GetActiveView(&self) -> Result<ITfContextView> {
        unsupported()
    }
    fn EnumViews(&self) -> Result<IEnumTfContextViews> {
        unsupported()
    }
    fn GetStatus(&self) -> Result<TS_STATUS> {
        unsupported()
    }
    fn GetProperty(&self, _: *const GUID) -> Result<ITfProperty> {
        unsupported()
    }
    fn GetAppProperty(&self, _: *const GUID) -> Result<ITfReadOnlyProperty> {
        unsupported()
    }
    fn TrackProperties(
        &self,
        _: *const *const GUID,
        _: u32,
        _: *const *const GUID,
        _: u32,
    ) -> Result<ITfReadOnlyProperty> {
        unsupported()
    }
    fn EnumProperties(&self) -> Result<IEnumTfProperties> {
        unsupported()
    }
    fn GetDocumentMgr(&self) -> Result<ITfDocumentMgr> {
        Ok(self.document.clone())
    }
    fn CreateRangeBackup(&self, _: u32, _: Ref<ITfRange>) -> Result<ITfRangeBackup> {
        unsupported()
    }
}

#[implement(ITfRange)]
struct Range {
    context: ITfContext,
}

impl ITfRange_Impl for Range_Impl {
    fn GetText(&self, _: u32, _: u32, _: PWSTR, _: u32, _: *mut u32) -> Result<()> {
        unsupported()
    }
    fn SetText(&self, _: u32, _: u32, _: &PCWSTR, _: i32) -> Result<()> {
        unsupported()
    }
    fn GetFormattedText(&self, _: u32) -> Result<IDataObject> {
        unsupported()
    }
    fn GetEmbedded(&self, _: u32, _: *const GUID, _: *const GUID) -> Result<IUnknown> {
        unsupported()
    }
    fn InsertEmbedded(&self, _: u32, _: u32, _: Ref<IDataObject>) -> Result<()> {
        unsupported()
    }
    fn ShiftStart(&self, _: u32, _: i32, _: *mut i32, _: *const TF_HALTCOND) -> Result<()> {
        unsupported()
    }
    fn ShiftEnd(&self, _: u32, _: i32, _: *mut i32, _: *const TF_HALTCOND) -> Result<()> {
        unsupported()
    }
    fn ShiftStartToRange(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<()> {
        unsupported()
    }
    fn ShiftEndToRange(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<()> {
        unsupported()
    }
    fn ShiftStartRegion(&self, _: u32, _: TfShiftDir) -> Result<BOOL> {
        unsupported()
    }
    fn ShiftEndRegion(&self, _: u32, _: TfShiftDir) -> Result<BOOL> {
        unsupported()
    }
    fn IsEmpty(&self, _: u32) -> Result<BOOL> {
        unsupported()
    }
    fn Collapse(&self, _: u32, _: TfAnchor) -> Result<()> {
        unsupported()
    }
    fn IsEqualStart(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<BOOL> {
        unsupported()
    }
    fn IsEqualEnd(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<BOOL> {
        unsupported()
    }
    fn CompareStart(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<i32> {
        unsupported()
    }
    fn CompareEnd(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<i32> {
        unsupported()
    }
    fn AdjustForInsert(&self, _: u32, _: u32) -> Result<BOOL> {
        unsupported()
    }
    fn GetGravity(&self, _: *mut TfGravity, _: *mut TfGravity) -> Result<()> {
        unsupported()
    }
    fn SetGravity(&self, _: u32, _: TfGravity, _: TfGravity) -> Result<()> {
        unsupported()
    }
    fn Clone(&self) -> Result<ITfRange> {
        unsupported()
    }
    fn GetContext(&self) -> Result<ITfContext> {
        Ok(self.context.clone())
    }
}

#[implement(ITfComposition)]
struct Composition {
    range: ITfRange,
}

impl ITfComposition_Impl for Composition_Impl {
    fn GetRange(&self) -> Result<ITfRange> {
        Ok(self.range.clone())
    }
    fn ShiftStart(&self, _: u32, _: Ref<ITfRange>) -> Result<()> {
        unsupported()
    }
    fn ShiftEnd(&self, _: u32, _: Ref<ITfRange>) -> Result<()> {
        unsupported()
    }
    fn EndComposition(&self, _: u32) -> Result<()> {
        unsupported()
    }
}

/// 在一个新文档里组字：返回文档、组字所在的上下文和编辑区。
fn composing() -> (ITfDocumentMgr, ITfContext, ITfComposition) {
    let document: ITfDocumentMgr = Document.into();
    let context: ITfContext = Context {
        document: document.clone(),
    }
    .into();
    let range: ITfRange = Range {
        context: context.clone(),
    }
    .into();
    (document, context, Composition { range }.into())
}

#[test]
fn focus_on_the_composing_document_keeps_the_composition() {
    // 资源管理器地址栏：第一个键打出来以后，同一个输入框才报焦点
    let (document, context, composition) = composing();
    let kept = composing_context(Some(&composition), Some(&document));
    assert_eq!(
        kept,
        Some(context),
        "焦点还在组字的文档上，要接着用组字所在的上下文"
    );
}

#[test]
fn repeated_language_button_clicks_queue_one_menu_outside_the_tsf_callback() {
    use super::{langbar, on_button, ui, CandidateWindow, Rc, TextService, HINSTANCE, POINT};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::TextServices::TF_LBI_CLK_RIGHT;
    use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE};
    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    let service = TextService::new(HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0));
    service.state.borrow_mut().window = CandidateWindow::new(service.instance);
    let weak = Rc::downgrade(&service.state);
    let button = langbar::create(Rc::new(move |click| on_button(&weak, click)));
    let point = POINT { x: 13, y: 24 };
    for _ in 0..6 {
        unsafe {
            button
                .item
                .OnClick(TF_LBI_CLK_RIGHT, point, std::ptr::null())
                .unwrap();
        }
    }
    let hwnd = service.state.borrow().window.as_ref().unwrap().hwnd();
    let mut msg = MSG::default();
    let next = |msg: &mut MSG| unsafe {
        PeekMessageW(
            msg,
            Some(hwnd),
            ui::WM_LANGUAGE_MENU,
            ui::WM_LANGUAGE_MENU,
            PM_REMOVE,
        )
        .as_bool()
    };
    assert!(next(&mut msg));
    assert_eq!((msg.wParam.0 as i32, msg.lParam.0 as i32), (13, 24));
    assert!(
        !next(&mut msg),
        "repeated clicks must not nest or backlog menus"
    );
}

#[test]
fn language_menu_outlives_hint_timeout_and_discards_nested_clicks() {
    use super::{
        langbar, on_button, on_click, ui, CandidateWindow, Click, Frame, MenuState, Rc,
        TextService, HINSTANCE, POINT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, EndMenu, GetGUIThreadInfo, KillTimer, PeekMessageW,
        SetTimer, GUITHREADINFO, GUI_INMENUMODE, MSG, PM_REMOVE, WS_POPUP, WS_VISIBLE,
    };

    thread_local! {
        static INSPECT: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
    }
    unsafe extern "system" fn inspect(
        _: windows::Win32::Foundation::HWND,
        _: u32,
        timer: usize,
        _: u32,
    ) {
        let _ = KillTimer(None, timer);
        INSPECT.with(|slot| {
            if let Some(inspect) = slot.borrow_mut().take() {
                inspect();
            }
        });
        let _ = EndMenu();
    }

    let _desktop = crate::ui::render_tests::PrivateDesktop::new();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let service = TextService::new(instance);
    service.state.borrow_mut().window = CandidateWindow::new(instance);
    let owner = unsafe {
        CreateWindowExW(
            Default::default(),
            windows::core::w!("STATIC"),
            None,
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            100,
            100,
            None,
            None,
            Some(instance),
            None,
        )
        .unwrap()
    };
    struct OwnedWindow(windows::Win32::Foundation::HWND);
    impl Drop for OwnedWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    let _owner = OwnedWindow(owner);
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(owner);
        let _ = SetFocus(Some(owner));
    }
    let weak = Rc::downgrade(&service.state);
    let button = langbar::create(Rc::new(move |click| on_button(&weak, click)));
    for attempt in 0..6 {
        if attempt == 5 {
            unsafe {
                let _ = SetFocus(None);
            }
        }
        let candidate = service.state.borrow().window.as_ref().unwrap().hwnd();
        service.state.borrow().window.as_ref().unwrap().update(
            &Frame {
                hint: Some("中".into()),
                font_size: 18,
                font_family: "Microsoft YaHei".into(),
                ..Frame::default()
            },
            Some(windows::Win32::Foundation::RECT {
                left: 100,
                top: 100,
                right: 102,
                bottom: 120,
            }),
        );
        unsafe {
            button
                .item
                .OnClick(TF_LBI_CLK_RIGHT, POINT { x: 120, y: 140 }, std::ptr::null())
                .unwrap();
        }
        let mut message = MSG::default();
        assert!(unsafe {
            PeekMessageW(
                &mut message,
                Some(candidate),
                ui::WM_LANGUAGE_MENU,
                ui::WM_LANGUAGE_MENU,
                PM_REMOVE,
            )
            .as_bool()
        });
        let state = service.state.clone();
        let button = button.clone();
        let seen = Rc::new(std::cell::Cell::new(false));
        let observed = seen.clone();
        let details = Rc::new(std::cell::RefCell::new(String::new()));
        let captured_details = details.clone();
        INSPECT.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let mut info = GUITHREADINFO {
                    cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                    ..Default::default()
                };
                let menu_open = unsafe {
                    GetGUIThreadInfo(
                        windows::Win32::System::Threading::GetCurrentThreadId(),
                        &mut info,
                    )
                }
                .is_ok()
                    && info.flags.contains(GUI_INMENUMODE);
                *captured_details.borrow_mut() = format!(
                    "flags={} menu_state={} hint_visible={} menu_owner={:?}",
                    info.flags.0,
                    state.borrow().language_menu as u8,
                    state.borrow().window.as_ref().unwrap().visible(),
                    info.hwndMenuOwner
                );
                observed.set(
                    menu_open
                        && state.borrow().language_menu == MenuState::Open
                        && !state.borrow().window.as_ref().unwrap().visible(),
                );
                for _ in 0..6 {
                    unsafe {
                        button
                            .item
                            .OnClick(TF_LBI_CLK_RIGHT, POINT { x: 120, y: 140 }, std::ptr::null())
                            .unwrap();
                    }
                }
            }))
        });
        let timer = unsafe { SetTimer(None, 0, 1300, Some(inspect)) };
        assert_ne!(timer, 0);
        on_click(
            &Rc::downgrade(&service.state),
            Click::LanguageMenu(120, 140),
        );
        unsafe {
            let _ = KillTimer(None, timer);
        }
        INSPECT.with(|slot| *slot.borrow_mut() = None);
        assert!(
            seen.get(),
            "menu {attempt} must remain open after the hint disappears: {}",
            details.borrow()
        );
        assert_eq!(service.state.borrow().language_menu, MenuState::Closed);
        assert!(
            !unsafe {
                PeekMessageW(
                    &mut message,
                    Some(candidate),
                    ui::WM_LANGUAGE_MENU,
                    ui::WM_LANGUAGE_MENU,
                    PM_REMOVE,
                )
                .as_bool()
            },
            "nested clicks must leave no menu backlog"
        );
    }
    unsafe {
        let _ = SetFocus(None);
    }
}

#[test]
fn focus_elsewhere_is_a_real_change() {
    let (_, _, composition) = composing();
    let other: ITfDocumentMgr = Document.into();
    assert_eq!(
        composing_context(Some(&composition), Some(&other)),
        None,
        "换到了别的输入框"
    );
    assert_eq!(
        composing_context(Some(&composition), None),
        None,
        "没有输入框了"
    );
}

#[test]
fn focus_without_composition_is_a_real_change() {
    let document: ITfDocumentMgr = Document.into();
    assert_eq!(composing_context(None, Some(&document)), None);
}
