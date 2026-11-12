//! 任务栏上的输入模式按钮（TSF 语言栏的 `GUID_LBI_INPUTMODE`）：显示「中」或「英」，
//! 左键切换，右键弹出菜单（中英文标点、简繁、表情、工具栏、设置）。
//!
//! 每个文本服务（每个线程）一个按钮，激活时加到语言栏，停用时拿掉。

use std::cell::RefCell;
use std::rc::Rc;

use freetype_protocol::Mode;
use windows::core::{implement, w, Ref, Result, BOOL, BSTR, GUID, IUnknown, Interface};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, GetTextExtentPoint32W,
    SelectObject, SetBkMode, SetTextColor, TextOutW, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, FF_DONTCARE, FW_NORMAL,
    OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::TextServices::{
    ITfLangBarItem, ITfLangBarItemButton, ITfLangBarItemButton_Impl, ITfLangBarItemSink, ITfLangBarItem_Impl,
    ITfMenu, ITfSource, ITfSource_Impl, TfLBIClick, GUID_LBI_INPUTMODE, TF_LANGBARITEMINFO, TF_LBI_CLK_LEFT,
    TF_LBI_ICON, TF_LBI_STYLE_BTN_BUTTON, TF_LBI_STYLE_SHOWNINTRAY, TF_LBI_TEXT, TF_LBI_TOOLTIP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};

use crate::DllRef;

/// 按钮被点了。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonClick {
    Toggle,
    Menu(i32, i32),
}

/// 只有这一个 sink 的 cookie。
const SINK_COOKIE: u32 = 1;

struct State {
    mode: Mode,
    sink: Option<ITfLangBarItemSink>,
    on_click: Option<Rc<dyn Fn(ButtonClick)>>,
}

#[implement(ITfLangBarItemButton, ITfSource)]
pub struct ModeButton {
    state: Rc<RefCell<State>>,
    _dll: DllRef,
}

/// 文本服务拿着的一头：改状态、通知语言栏重画。
#[derive(Clone)]
pub struct ButtonHandle {
    state: Rc<RefCell<State>>,
    pub item: ITfLangBarItemButton,
}

pub fn create(on_click: Rc<dyn Fn(ButtonClick)>) -> ButtonHandle {
    let state = Rc::new(RefCell::new(State {
        mode: Mode::Chinese,
        sink: None,
        on_click: Some(on_click),
    }));
    let item: ITfLangBarItemButton = ModeButton {
        state: state.clone(),
        _dll: DllRef::new(),
    }
    .into();
    ButtonHandle { state, item }
}

impl ButtonHandle {
    pub fn set_mode(&self, mode: Mode) {
        let sink = {
            let mut s = self.state.borrow_mut();
            if s.mode == mode {
                return;
            }
            s.mode = mode;
            s.sink.clone()
        };
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnUpdate(TF_LBI_ICON | TF_LBI_TEXT | TF_LBI_TOOLTIP);
            }
        }
    }

    pub fn as_item(&self) -> Option<ITfLangBarItem> {
        self.item.cast().ok()
    }

    /// 停用时断开回调，免得按钮还被系统拿着时调到已经停用的文本服务。
    pub fn detach(&self) {
        let mut s = self.state.borrow_mut();
        s.on_click = None;
        s.sink = None;
    }
}

fn text_of(mode: Mode) -> &'static str {
    if mode == Mode::Chinese {
        "中"
    } else {
        "英"
    }
}

fn copy_into(dst: &mut [u16], text: &str) {
    for (slot, c) in dst.iter_mut().zip(text.encode_utf16().chain(std::iter::once(0))) {
        *slot = c;
    }
}

/// 任务栏是浅色的（文字画成深色）。
fn light_taskbar() -> bool {
    let mut value = 0u32;
    let mut size = 4u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
    }
    value != 0
}

/// 画一个写着一个字的图标（透明底）。
fn text_icon(text: &str) -> Result<HICON> {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dc = CreateCompatibleDC(None);
        let color = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, color.into());
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        pixels.fill(0);
        // 先在黑底上画白字，再把亮度当透明度
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(0x00ff_ffff));
        let font = CreateFontW(
            -(size * 15 / 16),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Microsoft YaHei UI"),
        );
        let old_font = SelectObject(dc, font.into());
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut extent = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &wide, &mut extent);
        let _ = TextOutW(dc, (size - extent.cx) / 2, (size - extent.cy) / 2, &wide);
        SelectObject(dc, old_font);
        let _ = DeleteObject(font.into());
        SelectObject(dc, old);
        let (r, g, b) = if light_taskbar() { (0x1f, 0x1f, 0x1f) } else { (0xff, 0xff, 0xff) };
        for p in pixels.iter_mut() {
            let a = (*p & 0xff).max((*p >> 8) & 0xff).max((*p >> 16) & 0xff);
            // 预乘透明度
            *p = (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255);
        }
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        });
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
        let _ = DeleteDC(dc);
        icon
    }
}

impl ITfLangBarItem_Impl for ModeButton_Impl {
    fn GetInfo(&self, pinfo: *mut TF_LANGBARITEMINFO) -> Result<()> {
        if pinfo.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let mut info = TF_LANGBARITEMINFO {
            clsidService: crate::register::clsid(),
            guidItem: GUID_LBI_INPUTMODE,
            dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
            ulSort: 0,
            ..Default::default()
        };
        copy_into(&mut info.szDescription, "FreeType 中 / 英");
        unsafe { *pinfo = info };
        Ok(())
    }

    fn GetStatus(&self) -> Result<u32> {
        Ok(0)
    }

    fn Show(&self, _fshow: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        let mode = self.state.borrow().mode;
        Ok(BSTR::from(format!("FreeType：{}（单按 Shift 或点这里切换）", text_of(mode))))
    }
}

impl ITfLangBarItemButton_Impl for ModeButton_Impl {
    fn OnClick(&self, click: TfLBIClick, pt: &POINT, _prcarea: *const RECT) -> Result<()> {
        let handler = self.state.borrow().on_click.clone();
        if let Some(handler) = handler {
            if click == TF_LBI_CLK_LEFT {
                handler(ButtonClick::Toggle);
            } else {
                handler(ButtonClick::Menu(pt.x, pt.y));
            }
        }
        Ok(())
    }

    fn InitMenu(&self, _pmenu: Ref<ITfMenu>) -> Result<()> {
        Ok(())
    }

    fn OnMenuSelect(&self, _wid: u32) -> Result<()> {
        Ok(())
    }

    fn GetIcon(&self) -> Result<HICON> {
        text_icon(text_of(self.state.borrow().mode))
    }

    fn GetText(&self) -> Result<BSTR> {
        Ok(BSTR::from(text_of(self.state.borrow().mode)))
    }
}

impl ITfSource_Impl for ModeButton_Impl {
    fn AdviseSink(&self, riid: *const GUID, punk: Ref<IUnknown>) -> Result<u32> {
        if riid.is_null() || unsafe { *riid } != ITfLangBarItemSink::IID {
            return Err(windows::core::HRESULT(0x8004_0202_u32 as i32).into());
        }
        let sink: ITfLangBarItemSink = punk.ok()?.cast()?;
        self.state.borrow_mut().sink = Some(sink);
        Ok(SINK_COOKIE)
    }

    fn UnadviseSink(&self, dwcookie: u32) -> Result<()> {
        if dwcookie != SINK_COOKIE {
            return Err(windows::core::HRESULT(0x8004_0200_u32 as i32).into());
        }
        self.state.borrow_mut().sink = None;
        Ok(())
    }
}

