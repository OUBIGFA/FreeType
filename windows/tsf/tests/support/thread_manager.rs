//! 只替代系统注册边界：保留真实 COM 引用和唯一按键订阅，DLL、窗口与交接均为生产实现。
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::core::{implement, Ref, Result, IUnknown, GUID, BOOL, BSTR, PCWSTR};
use windows::Win32::Foundation::{E_NOTIMPL, E_FAIL, HWND, WPARAM, LPARAM};
use windows::Win32::UI::TextServices::*;

#[implement(ITfThreadMgr, ITfKeystrokeMgr, ITfSource)]
#[derive(Default)]
pub struct Manager {
    key: RefCell<Option<ITfKeyEventSink>>,
    source: RefCell<Option<IUnknown>>,
    reject_next: Rc<Cell<bool>>,
}
impl Manager {
    pub fn new(reject_next: Rc<Cell<bool>>) -> Self { Self { reject_next, ..Default::default() } }
}
impl ITfThreadMgr_Impl for Manager_Impl {
    fn Activate(&self) -> Result<u32> { Ok(1) }
    fn Deactivate(&self) -> Result<()> { Ok(()) }
    fn CreateDocumentMgr(&self) -> Result<ITfDocumentMgr> { Err(E_NOTIMPL.into()) }
    fn EnumDocumentMgrs(&self) -> Result<IEnumTfDocumentMgrs> { Err(E_NOTIMPL.into()) }
    fn GetFocus(&self) -> Result<ITfDocumentMgr> { Err(E_FAIL.into()) }
    fn SetFocus(&self, _: Ref<ITfDocumentMgr>) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn AssociateFocus(&self, _: HWND, _: Ref<ITfDocumentMgr>) -> Result<ITfDocumentMgr> { Err(E_NOTIMPL.into()) }
    fn IsThreadFocus(&self) -> Result<BOOL> { Ok(false.into()) }
    fn GetFunctionProvider(&self, _: *const GUID) -> Result<ITfFunctionProvider> { Err(E_NOTIMPL.into()) }
    fn EnumFunctionProviders(&self) -> Result<IEnumTfFunctionProviders> { Err(E_NOTIMPL.into()) }
    fn GetGlobalCompartment(&self) -> Result<ITfCompartmentMgr> { Err(E_NOTIMPL.into()) }
}
impl ITfSource_Impl for Manager_Impl {
    fn AdviseSink(&self, _: *const GUID, sink: Ref<IUnknown>) -> Result<u32> {
        if self.source.borrow().is_some() { return Err(E_FAIL.into()); }
        *self.source.borrow_mut() = sink.cloned(); Ok(1)
    }
    fn UnadviseSink(&self, _: u32) -> Result<()> { self.source.borrow_mut().take(); Ok(()) }
}
impl ITfKeystrokeMgr_Impl for Manager_Impl {
    fn AdviseKeyEventSink(&self, _: u32, sink: Ref<ITfKeyEventSink>, _: BOOL) -> Result<()> {
        if self.reject_next.replace(false) { return Err(E_FAIL.into()); }
        if self.key.borrow().is_some() { return Err(E_FAIL.into()); }
        *self.key.borrow_mut() = sink.cloned(); Ok(())
    }
    fn UnadviseKeyEventSink(&self, _: u32) -> Result<()> { self.key.borrow_mut().take(); Ok(()) }
    fn GetForeground(&self) -> Result<GUID> { Err(E_NOTIMPL.into()) }
    fn TestKeyDown(&self, _: WPARAM, _: LPARAM) -> Result<BOOL> { Err(E_NOTIMPL.into()) }
    fn TestKeyUp(&self, _: WPARAM, _: LPARAM) -> Result<BOOL> { Err(E_NOTIMPL.into()) }
    fn KeyDown(&self, _: WPARAM, _: LPARAM) -> Result<BOOL> { Err(E_NOTIMPL.into()) }
    fn KeyUp(&self, _: WPARAM, _: LPARAM) -> Result<BOOL> { Err(E_NOTIMPL.into()) }
    fn GetPreservedKey(&self, _: Ref<ITfContext>, _: *const TF_PRESERVEDKEY) -> Result<GUID> { Err(E_NOTIMPL.into()) }
    fn IsPreservedKey(&self, _: *const GUID, _: *const TF_PRESERVEDKEY) -> Result<BOOL> { Err(E_NOTIMPL.into()) }
    fn PreserveKey(&self, _: u32, _: *const GUID, _: *const TF_PRESERVEDKEY, _: &PCWSTR, _: u32) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn UnpreserveKey(&self, _: *const GUID, _: *const TF_PRESERVEDKEY) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn SetPreservedKeyDescription(&self, _: *const GUID, _: &PCWSTR, _: u32) -> Result<()> { Err(E_NOTIMPL.into()) }
    fn GetPreservedKeyDescription(&self, _: *const GUID) -> Result<BSTR> { Err(E_NOTIMPL.into()) }
    fn SimulatePreservedKey(&self, _: Ref<ITfContext>, _: *const GUID) -> Result<BOOL> { Err(E_NOTIMPL.into()) }
}
