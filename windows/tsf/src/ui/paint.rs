//! 绘制候选窗与命中测试。
use super::*;

pub(super) fn paint(hwnd: HWND, data: &mut Data) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let (w, h) = (data.layout.width, data.layout.height);
        let hdc = CreateCompatibleDC(Some(target));
        let bitmap = CreateCompatibleBitmap(target, w, h);
        let old_bitmap = SelectObject(hdc, bitmap.into());
        match draw(hdc, data) {
            Ok(()) if BitBlt(target, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY).is_ok() => {
                data.painted_revision = data.view.revision;
            }
            Ok(()) => data.painted_revision = 0,
            Err(error) => {
                data.painted_revision = 0;
                crate::log::write("候选窗绘制失败", &error.to_string());
            }
        }
        SelectObject(hdc, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(hdc);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// 窗口和离屏回归使用同一条绘制路径，验证实际字体与表情，不用近似预览代替候选窗。
pub(super) fn draw(hdc: HDC, data: &Data) -> Result<()> {
    let (w, h) = (data.layout.width, data.layout.height);
    let t = theme(
        &data.view.candidate_style,
        data.view.candidate_skin.as_ref(),
    );
    let painter = Painter::new(hdc, &rect(0, 0, w, h), t.background)?;
    painter.border(&rect(0, 0, w, h), t.border);
    let radius = 4.0 * data.scale;
    match data.hovered {
        Some(Hover::Item(i)) if data.view.cursor != Some(i) => {
            if let Some(r) = data.layout.items.get(i) {
                painter.fill_round(r, t.hover, radius);
            }
        }
        Some(Hover::Arrow(next)) => {
            if let Some((r, _, _)) = data.layout.arrows.iter().find(|(_, n, _)| *n == next) {
                painter.fill_round(r, t.hover, radius);
            }
        }
        _ => {}
    }
    if let (Some(r), Some(accent)) = (
        data.view.cursor.and_then(|i| data.layout.items.get(i)),
        t.accent,
    ) {
        painter.fill_round(r, accent, radius);
    }
    let selected = |i: usize| data.view.cursor == Some(i);
    for piece in &data.layout.parts {
        let color = match piece.part {
            Part::Hint => t.emphasis,
            Part::Letter => t.muted,
            Part::Sentence(i) => match data.view.sentence.as_ref().and_then(|v| v.blocks.get(i)) {
                Some(b) if b.focused => t.emphasis,
                Some(b) if b.confirmed => rgb(0x27, 0x87, 0x64),
                Some(b) if b.corrected => rgb(0x80, 0x5b, 0xa8),
                Some(b) if b.uncertain => rgb(0xa4, 0x69, 0x28),
                _ => t.text,
            },
            Part::Title | Part::Preedit => t.muted,
            Part::Label(i) => {
                if selected(i) {
                    t.accent_text
                } else {
                    t.muted
                }
            }
            Part::Candidate(i) => {
                if selected(i) {
                    t.accent_text
                } else {
                    t.text
                }
            }
        };
        painter.text(piece.at.0, piece.at.1, &piece.text, color);
    }
    for (r, next, text) in &data.layout.arrows {
        let enabled = if *next {
            data.view.has_next
        } else {
            data.view.has_prev
        };
        // 箭头是一个符号：墨迹放在它的格子正中
        let (x, y) = text.centered_in(r);
        painter.text(x, y, text, if enabled { t.text } else { t.border });
    }
    painter.finish()
}

pub(super) fn data_of(hwnd: HWND) -> Option<&'static mut Data> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Data;
        ptr.as_mut()
    }
}

pub(super) fn hit(data: &Data, x: i32, y: i32) -> Option<Hover> {
    let contains = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if let Some((_, at)) = data.layout.sentence_hits.iter().find(|(r, _)| contains(r)) {
        return Some(Hover::Sentence(*at));
    }
    if let Some((_, at)) = data.layout.letter_hits.iter().find(|(r, _)| contains(r)) {
        return Some(Hover::Letter(*at));
    }
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if let Some((_, next, _)) = data.layout.arrows.iter().find(|(r, _, _)| inside(r)) {
        let enabled = if *next {
            data.view.has_next
        } else {
            data.view.has_prev
        };
        return enabled.then_some(Hover::Arrow(*next));
    }
    data.layout.items.iter().position(inside).map(Hover::Item)
}

pub(super) fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xffff) as i16 as i32,
        ((lparam.0 >> 16) & 0xffff) as i16 as i32,
    )
}
