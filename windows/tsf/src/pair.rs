//! 成对标点的文档锚点。只有当前选区确实位于这次插入的右标点前才移动或删除。
use super::{selection_range, set_selection, same_object, ITfContext, ITfRange, Result, TF_ANCHOR_END, TF_ANCHOR_START};
use freetype_protocol::{keysym, modifier, Key, PairEdit};
use windows::Win32::UI::TextServices::{TF_GRAVITY_BACKWARD, TF_GRAVITY_FORWARD};

pub(super) struct Pair {
    pub context: ITfContext,
    left: ITfRange,
    right: ITfRange,
    opening: char,
    closing: char,
}

impl Pair {
    pub fn after_commit(context: &ITfContext, end: &ITfRange, ec: u32, text: &str) -> Option<Self> {
        let mut chars = text.chars().rev();
        let closing = chars.next()?;
        let opening = chars.next()?;
        unsafe {
            let right = end.Clone().ok()?;
            let mut moved = 0;
            right.ShiftStart(ec, -1, &mut moved, std::ptr::null()).ok()?;
            if moved != -1 { return None; }
            let left = right.Clone().ok()?;
            left.Collapse(ec, TF_ANCHOR_START).ok()?;
            left.ShiftStart(ec, -1, &mut moved, std::ptr::null()).ok()?;
            if moved != -1 { return None; }
            // Appending text at the boundary should leave the left range before the new text,
            // and push the right range after it.
            left.SetGravity(ec, TF_GRAVITY_BACKWARD, TF_GRAVITY_BACKWARD).ok()?;
            right.SetGravity(ec, TF_GRAVITY_FORWARD, TF_GRAVITY_FORWARD).ok()?;
            let pair = Self { context: context.clone(), left, right, opening, closing };
            pair.intact(ec).then_some(pair)
        }
    }

    pub fn for_key(&self, key: Key, composing: bool) -> Option<PairEdit> {
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 { return None; }
        if !composing && key.keysym == keysym::BACKSPACE && key.modifiers & modifier::SHIFT == 0 {
            return Some(PairEdit::Erase);
        }
        let typed = char::from_u32(key.keysym)?;
        closes_with(self.opening, self.closing, typed).then_some(PairEdit::Skip)
    }

    fn intact(&self, ec: u32) -> bool {
        range_has(&self.left, ec, self.opening) && range_has(&self.right, ec, self.closing)
    }

    pub fn permits(&self, context: &ITfContext, ec: u32, caret: &ITfRange, action: PairEdit) -> bool {
        same_object(&self.context, context) && self.intact(ec) && unsafe {
            caret.IsEmpty(ec).is_ok_and(|empty| empty.as_bool())
                && caret.CompareStart(ec, &self.right, TF_ANCHOR_START) == Ok(0)
                && (action == PairEdit::Skip || self.left.CompareEnd(ec, &self.right, TF_ANCHOR_START) == Ok(0))
        }
    }

    pub fn apply(&self, context: &ITfContext, ec: u32, action: PairEdit) -> Result<bool> {
        let Some(caret) = selection_range(context, ec) else { return Ok(false); };
        if !self.permits(context, ec, &caret, action) { return Ok(false); }
        unsafe {
            let at = self.right.Clone()?;
            if action == PairEdit::Erase {
                let mut moved = 0;
                at.ShiftStart(ec, -1, &mut moved, std::ptr::null())?;
                if moved != -1 { return Ok(false); }
                at.SetText(ec, 0, &[])?;
            }
            at.Collapse(ec, TF_ANCHOR_END)?;
            set_selection(context, ec, &at)?;
        }
        Ok(true)
    }
}

fn range_has(range: &ITfRange, ec: u32, expected: char) -> bool {
    let mut text = [0u16; 2];
    let mut read = 0;
    (unsafe { range.GetText(ec, 0, &mut text, &mut read).is_ok() })
        && text[..(read as usize).min(text.len())].iter().copied().eq(expected.encode_utf16(&mut [0; 2]).iter().copied())
}

fn closes_with(open: char, close: char, key: char) -> bool {
    key == close || matches!((open, close, key),
        ('（', '）', ')') | ('【', '】', ']') | ('《', '》', '>') | ('「', '」', '}')
        | ('“', '”', '"') | ('「', '」', '"') | ('‘', '’', '\''))
}

#[test]
fn closing_key_depends_on_pair() {
    assert!(closes_with('（', '）', ')'));
    assert!(closes_with('「', '」', '"'));
    assert!(!closes_with('【', '】', ')'));
}
