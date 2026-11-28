//! TSF 文档访问：编辑会话、选区与光标矩形、光标前文字、过渡文档与敏感输入框判断。
use super::*;

/// 编辑会话：把一段代码交给 TSF，在它给的编辑 cookie 下执行。
#[implement(ITfEditSession)]
pub(super) struct EditSession {
    action: RefCell<Option<Box<dyn FnOnce(u32) -> Result<()>>>>,
    _dll: DllRef,
}

impl ITfEditSession_Impl for EditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        match self.action.borrow_mut().take() {
            Some(action) => action(ec),
            None => Ok(()),
        }
    }
}

pub(super) fn edit_with(
    context: &ITfContext,
    client_id: u32,
    flags: windows::Win32::UI::TextServices::TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    action: impl FnOnce(u32) -> Result<()> + 'static,
) {
    let _ = request_edit(context, client_id, flags, action);
}

pub(super) fn request_edit(
    context: &ITfContext,
    client_id: u32,
    flags: windows::Win32::UI::TextServices::TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    action: impl FnOnce(u32) -> Result<()> + 'static,
) -> Result<()> {
    let session: ITfEditSession = EditSession {
        action: RefCell::new(Some(Box::new(action))),
        _dll: DllRef::new(),
    }
    .into();
    unsafe { context.RequestEditSession(client_id, &session, flags)?.ok() }
}

pub(super) fn edit(
    context: &ITfContext,
    client_id: u32,
    sync: bool,
    action: impl FnOnce(u32) -> Result<()> + 'static,
) {
    let flags = if sync {
        TF_ES_SYNC | TF_ES_READWRITE
    } else {
        TF_ES_ASYNCDONTCARE | TF_ES_READWRITE
    };
    edit_with(context, client_id, flags, action);
}

pub(super) fn set_selection(context: &ITfContext, ec: u32, range: &ITfRange) -> Result<()> {
    let selection = TF_SELECTION {
        range: ManuallyDrop::new(Some(range.clone())),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_NONE,
            fInterimChar: false.into(),
        },
    };
    let result = unsafe { context.SetSelection(ec, &[selection.clone()]) };
    drop(ManuallyDrop::into_inner(selection.range));
    result
}

pub(super) fn text_rect(context: &ITfContext, ec: u32, range: &ITfRange) -> Option<RECT> {
    unsafe {
        let view = context.GetActiveView().ok()?;
        let mut rect = RECT::default();
        let mut clipped = BOOL::default();
        view.GetTextExt(ec, range, &mut rect, &mut clipped).ok()?;
        (rect.right > rect.left || rect.bottom > rect.top || rect.left != 0).then_some(rect)
    }
}

pub(super) fn selection_range(context: &ITfContext, ec: u32) -> Option<ITfRange> {
    unsafe {
        let mut selection = [TF_SELECTION::default()];
        let mut fetched = 0u32;
        context
            .GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)
            .ok()?;
        let range = ManuallyDrop::into_inner(std::mem::take(&mut selection[0].range))?;
        (fetched > 0).then_some(range)
    }
}

pub(super) fn selection_rect(context: &ITfContext, ec: u32) -> Option<RECT> {
    selection_range(context, ec).and_then(|range| text_rect(context, ec, &range))
}

/// 光标前最多 [`BEFORE_CHARS`] 个字（只读）。
pub(super) fn before_text(context: &ITfContext, ec: u32) -> Option<String> {
    unsafe {
        let range = selection_range(context, ec)?;
        range.Collapse(ec, TF_ANCHOR_START).ok()?;
        let mut moved = 0;
        range
            .ShiftStart(ec, -BEFORE_CHARS, &mut moved, std::ptr::null())
            .ok()?;
        let mut buf = [0u16; BEFORE_CHARS as usize * 2];
        let mut len = 0u32;
        range.GetText(ec, 0, &mut buf, &mut len).ok()?;
        Some(String::from_utf16_lossy(
            &buf[..(len as usize).min(buf.len())],
        ))
    }
}

/// CUAS 文档管理器上未公开的标记：值的最低位为 1 表示它替 IMM32 应用模拟 TSF 文档。
/// 来源：Mozc `src/win32/tip/tip_transitory_extension.cc` 的 `kTsfEmulatedDocumentMgrGuid`。
pub(super) const GUID_COMPARTMENT_EMULATED_DOCUMENTMGR: GUID =
    GUID::from_u128(0xa94c5fd2_c471_4031_9546_709c17300cb9);

/// 光标前文字从哪里读。照搬 Mozc 的 `TipTransitoryExtension::AsFullContext` 与
/// `TipSurroundingText::Get`（`src/win32/tip/`）的判断：
///
/// - 非临时文档（Word、WPF、Firefox 等）：直接读这份 TSF 文档；
/// - 临时文档带完整的父文档（Edit、RichEdit 的 Transitory Extensions）：读父文档；
///   父文档也是临时的，或拿不到父文档的顶层上下文：按 IMM32 应用处理；
/// - 临时文档带 CUAS 模拟标记（IMM32 应用，如用 mygo 写的 FreeMD）：TSF 文档只装编辑区、
///   上屏后即清空，只能用 IMR_DOCUMENTFEED 向应用窗口要；
/// - 其余临时文档（Chromium 系）：TSF 文档里有正文，直接读。
///
/// 真机核对（Windows 11）：CUAS 文档的父文档隔间也是 VT_UNKNOWN，但它不是
/// ITfDocumentMgr，必须按 Mozc 的做法转换接口后再判断，不能只看是否非空。
pub(super) enum Surrounding {
    Tsf(ITfContext),
    Imm32,
}

pub(super) fn transitory(context: &ITfContext) -> bool {
    unsafe { context.GetStatus() }.is_ok_and(|status| status.dwStaticFlags & TF_SS_TRANSITORY != 0)
}

pub(super) fn view_window(context: &ITfContext) -> Option<HWND> {
    unsafe { context.GetActiveView().and_then(|view| view.GetWnd()).ok() }
        .filter(|window| !window.is_invalid())
}

pub(super) fn surrounding(context: &ITfContext) -> Surrounding {
    unsafe {
        let Ok(status) = context.GetStatus() else {
            return Surrounding::Imm32;
        };
        if status.dwStaticFlags & TF_SS_TRANSITORY == 0 {
            return Surrounding::Tsf(context.clone());
        }
        let Ok(doc) = context.GetDocumentMgr() else {
            return Surrounding::Imm32;
        };
        let Ok(mgr) = doc.cast::<ITfCompartmentMgr>() else {
            return Surrounding::Tsf(context.clone());
        };
        if let Some(parent) = transitory_parent(&mgr).filter(|parent| !same_object(parent, &doc)) {
            return match parent.GetTop() {
                Ok(top)
                    if top
                        .GetStatus()
                        .is_ok_and(|s| s.dwStaticFlags & TF_SS_TRANSITORY == 0) =>
                {
                    Surrounding::Tsf(top)
                }
                _ => Surrounding::Imm32,
            };
        }
        if compartment_value(&mgr, &GUID_COMPARTMENT_EMULATED_DOCUMENTMGR)
            .is_some_and(|v| v & 1 != 0)
        {
            Surrounding::Imm32
        } else {
            Surrounding::Tsf(context.clone())
        }
    }
}

/// Transitory Extensions 的父文档；隔间里的对象不是文档管理器时视为没有。
pub(super) fn transitory_parent(mgr: &ITfCompartmentMgr) -> Option<ITfDocumentMgr> {
    unsafe {
        let mut value = mgr
            .GetCompartment(&GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT)
            .ok()?
            .GetValue()
            .ok()?;
        let inner = &value.Anonymous.Anonymous;
        let parent = if inner.vt == VT_UNKNOWN {
            inner
                .Anonymous
                .punkVal
                .as_ref()
                .and_then(|unknown| unknown.cast::<ITfDocumentMgr>().ok())
        } else {
            None
        };
        let _ = VariantClear(&mut value);
        parent
    }
}

/// IMM32 应用的光标前文字：向文档所在窗口要；拿不到窗口或应用不答都算不知道。
pub(super) fn imm32_before(context: &ITfContext) -> Option<String> {
    unsafe { context.GetActiveView().and_then(|view| view.GetWnd()) }
        .ok()
        .filter(|window| !window.is_invalid())
        .and_then(document_feed)
}

/// 向 IMM32 应用要光标前最多 [`BEFORE_CHARS`] 个字（IMR_DOCUMENTFEED）。应用不答时返回 None：
/// 那是不知道上文，不是空文档，服务进程会沿用自己记下的上屏文字。
pub(super) fn document_feed(window: HWND) -> Option<String> {
    let request = |data: LPARAM| unsafe {
        SendMessageW(
            window,
            WM_IME_REQUEST,
            Some(WPARAM(IMR_DOCUMENTFEED as usize)),
            Some(data),
        )
        .0
    };
    let head = std::mem::size_of::<RECONVERTSTRING>();
    let size = usize::try_from(request(LPARAM(0)))
        .ok()
        .filter(|&size| size >= head)?;
    // RECONVERTSTRING 只含 DWORD，按 u32 分配即可对齐
    let mut buffer = vec![0u32; size.div_ceil(4)];
    let header = buffer.as_mut_ptr().cast::<RECONVERTSTRING>();
    unsafe {
        (*header).dwSize = size as u32;
    }
    if request(LPARAM(header as isize)) == 0 {
        return None;
    }
    let info = unsafe { *header };
    // 长度按 UTF-16 计，偏移按字节计；应用填的值越界就当没答
    let (start, len, caret) = (
        info.dwStrOffset as usize,
        info.dwStrLen as usize,
        info.dwCompStrOffset as usize / 2,
    );
    if start % 2 != 0 || start.checked_add(len * 2)? > size || caret > len {
        return None;
    }
    let units = unsafe {
        std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(start).cast::<u16>(), caret)
    };
    let before: Vec<char> = char::decode_utf16(units.iter().copied())
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    Some(
        before[before.len().saturating_sub(BEFORE_CHARS as usize)..]
            .iter()
            .collect(),
    )
}

/// 应用给不出光标位置时：系统记的插入符，再不行用输入框窗口的左下角。
pub(super) fn fallback_caret() -> Option<RECT> {
    unsafe {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(GetCurrentThreadId(), &mut info).ok()?;
        if !info.hwndCaret.is_invalid() {
            let r = info.rcCaret;
            let mut pts = [
                POINT {
                    x: r.left,
                    y: r.top,
                },
                POINT {
                    x: r.right,
                    y: r.bottom,
                },
            ];
            MapWindowPoints(Some(info.hwndCaret), None, &mut pts);
            return Some(RECT {
                left: pts[0].x,
                top: pts[0].y,
                right: pts[1].x,
                bottom: pts[1].y,
            });
        }
        let focus = if info.hwndFocus.is_invalid() {
            info.hwndActive
        } else {
            info.hwndFocus
        };
        if focus.is_invalid() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(focus, &mut r).ok()?;
        Some(RECT {
            left: r.left + 8,
            top: r.bottom - 24,
            right: r.left + 9,
            bottom: r.bottom - 4,
        })
    }
}

pub(super) fn compartment_value(mgr: &ITfCompartmentMgr, guid: &GUID) -> Option<i32> {
    unsafe {
        let compartment = mgr.GetCompartment(guid).ok()?;
        let value = compartment.GetValue().ok()?;
        let inner = &value.Anonymous.Anonymous;
        (inner.vt == VT_I4).then(|| inner.Anonymous.lVal)
    }
}

/// 应用说这里不用输入法（密码框、禁用输入法的窗口）。
pub(super) fn disabled(context: Option<&ITfContext>) -> bool {
    let Some(mgr) = context.and_then(|c| c.cast::<ITfCompartmentMgr>().ok()) else {
        return false;
    };
    compartment_value(&mgr, &GUID_COMPARTMENT_KEYBOARD_DISABLED).is_some_and(|v| v != 0)
        || compartment_value(&mgr, &GUID_COMPARTMENT_EMPTYCONTEXT).is_some_and(|v| v != 0)
}

/// 输入范围由宿主声明；读取密码、数字密码及各类 PIN，只读取范围，不读取文字。
pub(super) fn sensitive_scope(context: &ITfContext, ec: u32) -> bool {
    unsafe {
        let Some(selection) = selection_range(context, ec) else {
            return false;
        };
        // InputScope belongs to the host's ITextStoreACP/ITfContextOwner;
        // Microsoft specifies GetAppProperty, not the service-owned GetProperty.
        let Ok(property) = context.GetAppProperty(&GUID_PROP_INPUTSCOPE) else {
            return false;
        };
        let Ok(mut value) = property.GetValue(ec, &selection) else {
            return false;
        };
        let inner = &*value.Anonymous.Anonymous;
        let scopes = if inner.vt == VT_UNKNOWN {
            inner
                .Anonymous
                .punkVal
                .as_ref()
                .and_then(|value| value.cast::<ITfInputScope>().ok())
        } else {
            None
        };
        let _ = VariantClear(&mut value);
        let Some(scopes) = scopes else {
            return false;
        };
        let mut data = std::ptr::null_mut();
        let mut count = 0;
        if scopes.GetInputScopes(&mut data, &mut count).is_err() || data.is_null() {
            return false;
        }
        let sensitive = std::slice::from_raw_parts(data, count as usize)
            .iter()
            .any(|scope| {
                matches!(
                    *scope,
                    IS_PASSWORD
                        | IS_NUMERIC_PASSWORD
                        | IS_NUMERIC_PIN
                        | IS_ALPHANUMERIC_PIN
                        | IS_ALPHANUMERIC_PIN_SET
                )
            });
        CoTaskMemFree(Some(data.cast()));
        sensitive
    }
}
