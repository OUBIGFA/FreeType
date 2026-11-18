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
    fn CreateContext(&self, _: u32, _: u32, _: Ref<IUnknown>, _: OutRef<ITfContext>, _: *mut u32) -> Result<()> { unsupported() }
    fn Push(&self, _: Ref<ITfContext>) -> Result<()> { unsupported() }
    fn Pop(&self, _: u32) -> Result<()> { unsupported() }
    fn GetTop(&self) -> Result<ITfContext> { unsupported() }
    fn GetBase(&self) -> Result<ITfContext> { unsupported() }
    fn EnumContexts(&self) -> Result<IEnumTfContexts> { unsupported() }
}

#[implement(ITfContext)]
struct Context {
    document: ITfDocumentMgr,
}

impl ITfContext_Impl for Context_Impl {
    fn RequestEditSession(&self, _: u32, _: Ref<ITfEditSession>, _: TF_CONTEXT_EDIT_CONTEXT_FLAGS) -> Result<HRESULT> { unsupported() }
    fn InWriteSession(&self, _: u32) -> Result<BOOL> { unsupported() }
    fn GetSelection(&self, _: u32, _: u32, _: u32, _: *mut TF_SELECTION, _: *mut u32) -> Result<()> { unsupported() }
    fn SetSelection(&self, _: u32, _: u32, _: *const TF_SELECTION) -> Result<()> { unsupported() }
    fn GetStart(&self, _: u32) -> Result<ITfRange> { unsupported() }
    fn GetEnd(&self, _: u32) -> Result<ITfRange> { unsupported() }
    fn GetActiveView(&self) -> Result<ITfContextView> { unsupported() }
    fn EnumViews(&self) -> Result<IEnumTfContextViews> { unsupported() }
    fn GetStatus(&self) -> Result<TS_STATUS> { unsupported() }
    fn GetProperty(&self, _: *const GUID) -> Result<ITfProperty> { unsupported() }
    fn GetAppProperty(&self, _: *const GUID) -> Result<ITfReadOnlyProperty> { unsupported() }
    fn TrackProperties(&self, _: *const *const GUID, _: u32, _: *const *const GUID, _: u32) -> Result<ITfReadOnlyProperty> { unsupported() }
    fn EnumProperties(&self) -> Result<IEnumTfProperties> { unsupported() }
    fn GetDocumentMgr(&self) -> Result<ITfDocumentMgr> { Ok(self.document.clone()) }
    fn CreateRangeBackup(&self, _: u32, _: Ref<ITfRange>) -> Result<ITfRangeBackup> { unsupported() }
}

#[implement(ITfRange)]
struct Range {
    context: ITfContext,
}

impl ITfRange_Impl for Range_Impl {
    fn GetText(&self, _: u32, _: u32, _: PWSTR, _: u32, _: *mut u32) -> Result<()> { unsupported() }
    fn SetText(&self, _: u32, _: u32, _: &PCWSTR, _: i32) -> Result<()> { unsupported() }
    fn GetFormattedText(&self, _: u32) -> Result<IDataObject> { unsupported() }
    fn GetEmbedded(&self, _: u32, _: *const GUID, _: *const GUID) -> Result<IUnknown> { unsupported() }
    fn InsertEmbedded(&self, _: u32, _: u32, _: Ref<IDataObject>) -> Result<()> { unsupported() }
    fn ShiftStart(&self, _: u32, _: i32, _: *mut i32, _: *const TF_HALTCOND) -> Result<()> { unsupported() }
    fn ShiftEnd(&self, _: u32, _: i32, _: *mut i32, _: *const TF_HALTCOND) -> Result<()> { unsupported() }
    fn ShiftStartToRange(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<()> { unsupported() }
    fn ShiftEndToRange(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<()> { unsupported() }
    fn ShiftStartRegion(&self, _: u32, _: TfShiftDir) -> Result<BOOL> { unsupported() }
    fn ShiftEndRegion(&self, _: u32, _: TfShiftDir) -> Result<BOOL> { unsupported() }
    fn IsEmpty(&self, _: u32) -> Result<BOOL> { unsupported() }
    fn Collapse(&self, _: u32, _: TfAnchor) -> Result<()> { unsupported() }
    fn IsEqualStart(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<BOOL> { unsupported() }
    fn IsEqualEnd(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<BOOL> { unsupported() }
    fn CompareStart(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<i32> { unsupported() }
    fn CompareEnd(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> Result<i32> { unsupported() }
    fn AdjustForInsert(&self, _: u32, _: u32) -> Result<BOOL> { unsupported() }
    fn GetGravity(&self, _: *mut TfGravity, _: *mut TfGravity) -> Result<()> { unsupported() }
    fn SetGravity(&self, _: u32, _: TfGravity, _: TfGravity) -> Result<()> { unsupported() }
    fn Clone(&self) -> Result<ITfRange> { unsupported() }
    fn GetContext(&self) -> Result<ITfContext> { Ok(self.context.clone()) }
}

#[implement(ITfComposition)]
struct Composition {
    range: ITfRange,
}

impl ITfComposition_Impl for Composition_Impl {
    fn GetRange(&self) -> Result<ITfRange> { Ok(self.range.clone()) }
    fn ShiftStart(&self, _: u32, _: Ref<ITfRange>) -> Result<()> { unsupported() }
    fn ShiftEnd(&self, _: u32, _: Ref<ITfRange>) -> Result<()> { unsupported() }
    fn EndComposition(&self, _: u32) -> Result<()> { unsupported() }
}

/// 在一个新文档里组字：返回文档、组字所在的上下文和编辑区。
fn composing() -> (ITfDocumentMgr, ITfContext, ITfComposition) {
    let document: ITfDocumentMgr = Document.into();
    let context: ITfContext = Context { document: document.clone() }.into();
    let range: ITfRange = Range { context: context.clone() }.into();
    (document, context, Composition { range }.into())
}

#[test]
fn focus_on_the_composing_document_keeps_the_composition() {
    // 资源管理器地址栏：第一个键打出来以后，同一个输入框才报焦点
    let (document, context, composition) = composing();
    let kept = composing_context(Some(&composition), Some(&document));
    assert_eq!(kept, Some(context), "焦点还在组字的文档上，要接着用组字所在的上下文");
}

#[test]
fn focus_elsewhere_is_a_real_change() {
    let (_, _, composition) = composing();
    let other: ITfDocumentMgr = Document.into();
    assert_eq!(composing_context(Some(&composition), Some(&other)), None, "换到了别的输入框");
    assert_eq!(composing_context(Some(&composition), None), None, "没有输入框了");
}

#[test]
fn focus_without_composition_is_a_real_change() {
    let document: ITfDocumentMgr = Document.into();
    assert_eq!(composing_context(None, Some(&document)), None);
}
