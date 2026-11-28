//! 候选窗的右键菜单：不抢焦点，用鼠标捕获判断点在菜单外面。
use super::*;

struct MenuData {
    entries: Vec<(String, Click)>,
    rows: Vec<RECT>,
    hovered: Option<usize>,
    texts: Vec<Text>,
    scale: f32,
    style: String,
    skin: Option<CandidateSkin>,
}

fn data(hwnd: HWND) -> Option<&'static mut MenuData> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MenuData).as_mut() }
}

fn hwnd() -> Option<HWND> {
    MENU.with(|m| *m.borrow())
}

pub fn show(
    _owner: HWND,
    at: POINT,
    entries: Vec<(String, Click)>,
    family: String,
    scale: f32,
    style: String,
    skin: Option<CandidateSkin>,
) {
    let Some(hwnd) = hwnd() else {
        return;
    };
    close();
    unsafe {
        let texts = Fonts::new(14.0 * scale, &family).and_then(|fonts| {
            entries
                .iter()
                .map(|(t, _)| fonts.main.text(t))
                .collect::<Result<Vec<_>>>()
        });
        let texts = match texts {
            Ok(texts) => texts,
            Err(error) => {
                crate::log::write("候选菜单排版失败", &error.to_string());
                return;
            }
        };
        let pad = (6.0 * scale) as i32;
        let row_h = texts.iter().map(|t| t.size.cy).max().unwrap_or(0) + (12.0 * scale) as i32;
        let width = texts.iter().map(|t| t.size.cx).max().unwrap_or(0) + (32.0 * scale) as i32;
        let rows: Vec<RECT> = (0..entries.len())
            .map(|i| rect(pad, pad + i as i32 * row_h, width, row_h))
            .collect();
        let (w, h) = (width + 2 * pad, entries.len() as i32 * row_h + 2 * pad);
        let old = SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            Box::into_raw(Box::new(MenuData {
                entries,
                rows,
                hovered: None,
                texts,
                scale,
                style,
                skin,
            })) as _,
        ) as *mut MenuData;
        if !old.is_null() {
            drop(Box::from_raw(old));
        }
        let work = work_area(at.x, at.y);
        let x = at.x.min(work.right - w).max(work.left);
        let y = if at.y + h > work.bottom {
            at.y - h
        } else {
            at.y
        };
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
        SetCapture(hwnd);
    }
}

pub fn close() {
    let Some(hwnd) = hwnd() else {
        return;
    };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return;
        }
        let _ = ShowWindow(hwnd, SW_HIDE);
        if GetCapture() == hwnd {
            let _ = ReleaseCapture();
        }
    }
}

pub fn destroy() {
    if let Some(hwnd) = MENU.with(|m| m.borrow_mut().take()) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn paint_menu(hwnd: HWND, d: &MenuData) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let mut wr = RECT::default();
        let _ = GetWindowRect(hwnd, &mut wr);
        let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
        let hdc = CreateCompatibleDC(Some(target));
        let bitmap = CreateCompatibleBitmap(target, w, h);
        let old_bitmap = SelectObject(hdc, bitmap.into());
        // 右键菜单跟随当前皮肤的背景、边框、文字和悬停底色
        let t = theme(&d.style, d.skin.as_ref());
        let result = Painter::new(hdc, &rect(0, 0, w, h), t.background).and_then(|painter| {
            painter.border(&rect(0, 0, w, h), t.border);
            for (i, text) in d.texts.iter().enumerate() {
                let mut r = d.rows[i];
                if d.hovered == Some(i) {
                    painter.fill_round(&r, t.hover, 4.0 * d.scale);
                }
                r.left += (12.0 * d.scale) as i32;
                let top = r.top + (r.bottom - r.top - text.size.cy).max(0) / 2;
                painter.text(r.left as f32, top as f32, text, t.text);
            }
            painter.finish()
        });
        match result {
            Ok(()) => {
                let _ = BitBlt(target, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY);
            }
            Err(error) => crate::log::write("候选菜单绘制失败", &error.to_string()),
        }
        SelectObject(hdc, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(hdc);
        let _ = EndPaint(hwnd, &ps);
    }
}

fn row_at(d: &MenuData, x: i32, y: i32) -> Option<usize> {
    d.rows
        .iter()
        .position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

pub unsafe extern "system" fn proc_(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCDESTROY => unsafe {
            // The host can destroy the owned menu without calling destroy().
            MENU.with(|menu| {
                if *menu.borrow() == Some(hwnd) {
                    *menu.borrow_mut() = None;
                }
            });
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut MenuData;
            if !ptr.is_null() {
                drop(Box::from_raw(ptr));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            if let Some(d) = data(hwnd) {
                paint_menu(hwnd, d);
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_SETCURSOR => {
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(Some(cursor));
                }
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(d) = data(hwnd) {
                let row = row_at(d, x, y);
                if row != d.hovered {
                    d.hovered = row;
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            let click = data(hwnd).and_then(|d| row_at(d, x, y).map(|i| d.entries[i].1));
            close();
            if let Some(click) = click {
                dispatch(click);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => LRESULT(0),
        WM_CAPTURECHANGED => {
            // 别的窗口拿走了鼠标（切了窗口）：收起
            if lparam.0 != hwnd.0 as isize {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
