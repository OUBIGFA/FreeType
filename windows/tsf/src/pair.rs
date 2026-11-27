//! 成对标点的文档锚点。只有当前选区确实位于这次插入的右标点前才移动或删除。
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{selection_range, set_selection, same_object, ITfContext, ITfRange, Result, State, TF_ANCHOR_END, TF_ANCHOR_START};
use freetype_protocol::{keysym, modifier, Frame, Key, PairEdit};
use windows::Win32::UI::TextServices::{TF_GRAVITY_BACKWARD, TF_GRAVITY_FORWARD};

#[derive(Clone)]
pub(super) struct Pair {
    pub context: ITfContext,
    left: ITfRange,
    right: ITfRange,
    opening: char,
    closing: char,
    terminal: bool,
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
            let terminal = super::transitory(context) && at_document_end(context, end, ec);
            let pair = Self { context: context.clone(), left, right, opening, closing, terminal };
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

    fn insert_before(&self, ec: u32, text: &str) -> Result<()> {
        if !self.intact(ec) { return Err(windows::Win32::Foundation::E_FAIL.into()); }
        unsafe {
            let insertion = self.right.Clone()?;
            insertion.Collapse(ec, TF_ANCHOR_START)?;
            insertion.SetText(ec, 0, &super::wide(text))
        }
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

fn at_document_end(context: &ITfContext, range: &ITfRange, cookie: u32) -> bool {
    (|| -> Result<bool> { unsafe {
        let end = context.GetEnd(cookie)?;
        if range.CompareEnd(cookie, &end, TF_ANCHOR_END)? == 0 { return Ok(true); }
        let tail = range.Clone()?;
        tail.Collapse(cookie, TF_ANCHOR_END)?;
        let mut moved = 0;
        tail.ShiftEnd(cookie, 2, &mut moved, std::ptr::null())?;
        if tail.CompareEnd(cookie, &end, TF_ANCHOR_END)? != 0 { return Ok(false); }
        let mut buffer = [0u16; 3];
        let mut read = 0;
        tail.GetText(cookie, 0, &mut buffer, &mut read)?;
        Ok(matches!(&buffer[..read as usize], [10] | [13] | [13, 10] | [10, 10]))
    } })().unwrap_or(false)
}

pub(super) fn prepare_commit(state: &Rc<RefCell<State>>, context: &ITfContext, frame: &Frame, client: u32) -> Option<Pair> {
    if frame.commit.as_ref().is_none_or(String::is_empty) || frame.replace_space || frame.pair_edit.is_some()
        || frame.caret_back != 0 || !frame.preedit.is_empty() { return None; }
    let (pair, composition, generation) = {
        let service = state.borrow();
        let pair = service.pairs.last().filter(|pair| pair.terminal)?.clone();
        (pair, service.composition.clone()?, service.generation)
    };
    if !same_object(&pair.context, context) { return None; }
    let prepared = Rc::new(Cell::new(false));
    let (done, shared, original) = (prepared.clone(), state.clone(), pair.clone());
    let result = super::request_edit(context, client, super::TF_ES_SYNC | super::TF_ES_READWRITE, move |cookie| unsafe {
        let range = composition.GetRange()?;
        if range.IsEmpty(cookie)?.as_bool() { return Ok(()); }
        let caret = range.Clone()?;
        caret.Collapse(cookie, TF_ANCHOR_END)?;
        if !original.permits(&original.context, cookie, &caret, PairEdit::Skip) || !at_document_end(&original.context, &original.right, cookie) { return Ok(()); }
        range.SetText(cookie, 0, &[])?;
        range.Collapse(cookie, TF_ANCHOR_START)?;
        set_selection(&original.context, cookie, &range)?;
        {
            let mut service = shared.borrow_mut();
            if service.generation != generation || !service.composition.as_ref().is_some_and(|live| same_object(live, &composition)) { return Err(windows::Win32::Foundation::E_ABORT.into()); }
            service.composition = None;
        }
        if let Err(error) = composition.EndComposition(cookie) {
            let mut service = shared.borrow_mut();
            if service.generation == generation && service.composition.is_none() && service.context.as_ref().is_some_and(|live| same_object(live, &original.context)) { service.composition = Some(composition); }
            return Err(error);
        }
        done.set(true);
        Ok(())
    });
    if let Err(error) = result { crate::log::write("分段结束预编辑失败", &error.to_string()); }
    prepared.get().then_some(pair)
}

pub(super) fn commit_prepared(state: &Rc<RefCell<State>>, pair: Pair, frame: Rc<Frame>, client: u32) {
    let entered = Rc::new(Cell::new(false));
    let submit = |flags| {
        let (ran, shared, target, update) = (entered.clone(), state.clone(), pair.clone(), frame.clone());
        super::request_edit(&pair.context, client, flags, move |cookie| {
            ran.set(true);
            let (current, composing) = { let service = shared.borrow(); (service.context.clone(), service.composition.is_some()) };
            let in_place = !composing && current.as_ref().is_some_and(|live| same_object(live, &target.context))
                && selection_range(&target.context, cookie).is_some_and(|caret| target.permits(&target.context, cookie, &caret, PairEdit::Skip));
            let result = if in_place {
                super::apply_document(&shared, &target.context, &update, cookie)
            } else { target.insert_before(cookie, update.commit.as_deref().expect("prepared commit")) };
            if let Err(error) = &result {
                crate::log::write("更新文档失败", &error.to_string());
                if update.undo_learning { let _ = super::call(&shared, &freetype_protocol::Request::UndoCommitLearning); }
            } else if in_place { super::show_window(&shared, &update); }
            result
        })
    };
    if submit(super::TF_ES_SYNC | super::TF_ES_READWRITE).is_err() && !entered.get() {
        if let Err(error) = submit(super::TF_ES_ASYNCDONTCARE | super::TF_ES_READWRITE) {
            if !entered.get() {
                crate::log::write("更新文档失败", &error.to_string());
                if frame.undo_learning { let _ = super::call(state, &freetype_protocol::Request::UndoCommitLearning); }
            }
        }
    }
}

fn closes_with(open: char, close: char, key: char) -> bool {
    key == close || matches!((open, close, key),
        ('（', '）', ')') | ('【', '】', ']') | ('《', '》', '>') | ('「', '」', '}')
        | ('“', '”', '"') | ('「', '」', '"') | ('‘', '’', '\''))
}

#[derive(Default)]
pub(super) struct Delivery {
    pub sent: bool,
    pub released: bool,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Placement { Inside, AfterTerminal, Unknown }

pub(super) fn placement(state: &Rc<RefCell<State>>) -> Placement {
    let (target, pair, client) = {
        let service = state.borrow();
        (service.context.clone(), service.pairs.last().cloned(), service.client_id)
    };
    let (Some(target), Some(pair)) = (target, pair) else { return Placement::Unknown; };
    if !same_object(&target, &pair.context) { return Placement::Unknown; }
    let result = Rc::new(Cell::new(Placement::Unknown));
    let (checked, context) = (result.clone(), target.clone());
    super::edit_with(&target, client, super::TF_ES_SYNC | super::TF_ES_READ, move |cookie| {
        if let Some(caret) = selection_range(&context, cookie) {
            if pair.intact(cookie) && unsafe { caret.IsEmpty(cookie)?.as_bool() } {
                if unsafe { caret.CompareStart(cookie, &pair.right, TF_ANCHOR_START)? } == 0 { checked.set(Placement::Inside); }
                else if pair.terminal && unsafe { caret.CompareStart(cookie, &pair.right, TF_ANCHOR_END)? } == 0 { checked.set(Placement::AfterTerminal); }
            }
        }
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
        let result = super::request_edit(&context, client, super::TF_ES_ASYNCDONTCARE | super::TF_ES_READWRITE,
            move |cookie| anchor.insert_before(cookie, &committed));
        if let Err(error) = result { crate::log::write("更新文档失败", &error.to_string()); }
    }
}

#[test]
fn closing_key_depends_on_pair() {
    assert!(closes_with('（', '）', ')'));
    assert!(closes_with('「', '」', '"'));
    assert!(!closes_with('【', '】', ')'));
}
