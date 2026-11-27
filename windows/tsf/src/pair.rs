//! 成对标点的文档锚点。只有当前选区确实位于这次插入的右标点前才移动或删除。
use super::{selection_range, set_selection, same_object, ITfContext, ITfRange, Result, TF_ANCHOR_END, TF_ANCHOR_START};
use freetype_protocol::{keysym, modifier, Key, PairEdit};
use windows::Win32::UI::TextServices::{TF_GRAVITY_BACKWARD, TF_GRAVITY_FORWARD};

#[derive(Clone)]
pub(super) struct Pair {
    pub context: ITfContext,
    left: ITfRange,
    right: ITfRange,
    opening: char,
    closing: char,
}

impl Pair {
    pub fn suffix(&self) -> String { [self.opening, self.closing].into_iter().collect() }

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

#[derive(Default)]
pub(super) struct Delivery {
    pub sent: bool,
    frames: Vec<freetype_protocol::Frame>,
}

impl Delivery {
    pub fn defer(&mut self, frame: &freetype_protocol::Frame) -> bool {
        if frame.commit.is_none() && frame.preedit.is_empty() && self.frames.is_empty() { return false; }
        if self.frames.last().is_some_and(|previous| previous.commit.is_none() && !previous.preedit.is_empty()) {
            self.frames.pop();
        }
        self.frames.push(frame.clone());
        true
    }
}

pub(super) fn arrived(state: &std::rc::Rc<std::cell::RefCell<super::State>>) -> bool {
    let (target, pair, client) = {
        let service = state.borrow();
        (service.context.clone(), service.pairs.last().cloned(), service.client_id)
    };
    let (Some(target), Some(pair)) = (target, pair) else { return false; };
    if !same_object(&target, &pair.context) { return false; }
    let result = std::rc::Rc::new(std::cell::Cell::new(false));
    let (checked, context) = (result.clone(), target.clone());
    super::edit_with(&target, client, super::TF_ES_SYNC | super::TF_ES_READ, move |cookie| {
        if let Some(caret) = selection_range(&context, cookie) { checked.set(pair.permits(&context, cookie, &caret, PairEdit::Skip)); }
        Ok(())
    });
    result.get()
}

pub(super) fn finish(state: &std::rc::Rc<std::cell::RefCell<super::State>>, same_field: bool) {
    let (delivery, context, client, anchor) = {
        let mut service = state.borrow_mut();
        service.caret_move = None;
        let delivery = std::mem::take(&mut service.pair_delivery);
        if delivery.frames.is_empty() { return; }
        (delivery, service.context.clone(), service.client_id, service.pairs.last().cloned())
    };
    let Some(context) = context else { return; };
    if same_field {
        for frame in delivery.frames { super::apply(state, Some(&context), frame, true); }
    } else {
        let _ = super::call(state, &freetype_protocol::Request::Reset { commit: false });
        let committed: String = delivery.frames.into_iter().filter_map(|frame| frame.commit).collect();
        if committed.is_empty() { return; }
        let Some(anchor) = anchor.filter(|anchor| same_object(&context, &anchor.context)) else {
            crate::log::write("更新文档失败", "原输入框的成对标点位置已失效");
            return;
        };
        let result = super::request_edit(&context, client, super::TF_ES_ASYNCDONTCARE | super::TF_ES_READWRITE, move |cookie| unsafe {
            if !anchor.intact(cookie) { return Err(windows::Win32::Foundation::E_FAIL.into()); }
            let insertion = anchor.right.Clone()?;
            insertion.Collapse(cookie, TF_ANCHOR_START)?;
            insertion.SetText(cookie, 0, &super::wide(&committed))
        });
        if let Err(error) = result { crate::log::write("更新文档失败", &error.to_string()); }
    }
}

#[test]
fn closing_key_depends_on_pair() {
    assert!(closes_with('（', '）', ')'));
    assert!(closes_with('「', '」', '"'));
    assert!(!closes_with('【', '】', ')'));
}
