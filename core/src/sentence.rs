//! Sentence editing uses the existing decoder and original key ranges. A user's
//! explicit choice is a constraint, not a frequency hint or an immediate commit.
use crate::{CandidateWord, Engine};
use std::collections::BTreeMap;

/// Canonical readings make learned boundaries portable across input schemes.
pub type Boundaries = BTreeMap<String, (Vec<(String, String)>, u64)>;
pub(crate) fn valid_boundaries(rows: &Boundaries) -> bool {
    rows.len() <= 2000
        && rows.iter().all(|(key, (parts, _))| {
            !key.is_empty()
                && key.len() <= 1536
                && parts.len() >= 2
                && parts.len() <= 256
                && parts.iter().all(|(text, reading)| {
                    !text.trim().is_empty()
                        && !reading.trim().is_empty()
                        && !text.contains(['\n', '\r', '\t', '\0'])
                        && text.chars().count() == reading.split_whitespace().count()
                        && reading.split_whitespace().all(crate::syllable::is_valid)
                })
                && parts
                    .iter()
                    .map(|(_, p)| p.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    == *key
        })
}
impl Engine {
    pub fn sentence_boundaries(&self) -> Boundaries {
        self.boundaries.read().unwrap().clone()
    }
    pub fn remember_boundary(&self, pinyin: &str, parts: Vec<(String, String)>) {
        let row = (parts, crate::now_ms());
        let key = pinyin.to_string();
        if !valid_boundaries(&BTreeMap::from([(key.clone(), row.clone())])) {
            return;
        }
        let mut rows = self.boundaries.write().unwrap();
        if rows.get(&key).is_some_and(|r| r.0 == row.0) {
            return;
        }
        if rows.len() >= 2000 && !rows.contains_key(&key) {
            return;
        }
        let before = rows.insert(key.clone(), row.clone());
        drop(rows);
        self.note_learning(crate::learn::Change::Boundary(key, before, row));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Focus { at: usize },
    Caret { at: usize },
    Move { backward: bool },
    Choose { index: usize },
    Resize { expand: bool },
    Page { next: bool },
    Cycle { backward: bool },
    Leave,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub from: usize,
    pub to: usize,
    pub text: String,
    pub pinyin: String,
    pub confirmed: bool,
    pub corrected: bool,
    pub focused: bool,
    pub uncertain: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alternative {
    pub text: String,
    pub pinyin: String,
    pub from: usize,
    pub to: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub text: String,
    pub raw: String,
    pub cursor: Option<usize>,
    pub blocks: Vec<Block>,
    pub alternatives: Vec<Alternative>,
    pub pending_from: Option<usize>,
    pub page: usize,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pub view: View,
    /// Each chosen interval is bound to its exact original keys.
    pins: Vec<(Block, String)>,
    pub edited: bool,
    pub boundaries_changed: bool,
    pub boundary_marks: Vec<usize>,
    pub merged: Vec<(String, String)>,
    model_words: bool,
    cycle: Option<Cycle>,
}
#[derive(Clone, Debug)]
struct Cycle {
    view: View,
    pins: Vec<(Block, String)>,
    choices: Vec<Alternative>,
    index: usize,
}

/// Align one candidate's readings to actual keys, including abbreviated, fuzzy
/// and typo edges. The bounded DP avoids exponential ambiguity in long input.
pub fn spans(raw: &str, pinyin: &str, engine: &Engine) -> Option<Vec<(usize, usize)>> {
    if !raw.is_ascii() || raw.len() > 1024 {
        return None;
    }
    let syllables: Vec<_> = pinyin.split_whitespace().collect();
    if syllables.is_empty() || syllables.len() > 256 {
        return None;
    }
    // Exact spelling is the common path: linear and bounded. Use the alignment
    // DP only for abbreviated/corrected input whose raw offsets differ.
    let mut exact = Vec::with_capacity(syllables.len());
    let mut at = 0;
    for (index, syllable) in syllables.iter().enumerate() {
        let start = at;
        at += raw[at..].bytes().take_while(|b| *b == b'\'').count();
        let n = if engine.config().shuangpin {
            crate::shuangpin::encode(engine.config().shuangpin_scheme, syllable)
                .iter()
                .find(|code| raw.as_bytes()[at..].starts_with(*code))
                .map(|_| 2)
        } else if raw[at..].starts_with(syllable) {
            Some(syllable.len())
        } else if index + 1 == syllables.len()
            && !raw[at..].is_empty()
            && syllable.starts_with(&raw[at..])
        {
            Some(raw.len() - at)
        } else {
            None
        };
        let Some(n) = n else { break };
        at += n;
        exact.push((start, at));
    }
    if exact.len() == syllables.len() && at == raw.len() {
        return Some(exact);
    }
    let mut states = vec![None; raw.len() + 1];
    states[0] = Some((0u32, Vec::new()));
    for syllable in syllables {
        let mut next = vec![None; raw.len() + 1];
        for (start, state) in states.into_iter().enumerate() {
            let Some((cost, path)) = state else { continue };
            let at = start + raw[start..].bytes().take_while(|b| *b == b'\'').count();
            for end in at + 1..=(at + 8).min(raw.len()) {
                let part = &raw[at..end];
                if part.contains('\'') {
                    break;
                }
                let expected: Vec<String> = if engine.config().shuangpin {
                    crate::shuangpin::encode(engine.config().shuangpin_scheme, syllable)
                        .iter()
                        .map(|s| String::from_utf8_lossy(s).into_owned())
                        .collect()
                } else {
                    vec![syllable.to_owned()]
                };
                let mut penalty = expected
                    .iter()
                    .map(|s| distance(part, s))
                    .min()
                    .unwrap_or(99);
                if !engine.config().shuangpin && syllable.starts_with(part) {
                    // Full spelling beats initials and an unfinished last syllable.
                    penalty = penalty.min(if part == syllable { 0 } else { 2 });
                }
                if penalty > 2 {
                    continue;
                }
                let score = cost + penalty;
                if next[end].as_ref().is_none_or(|(c, _)| score < *c) {
                    let mut p = path.clone();
                    p.push((start, end));
                    next[end] = Some((score, p));
                }
            }
        }
        states = next;
    }
    states.pop().flatten().map(|(_, p)| p)
}
fn distance(a: &str, b: &str) -> u32 {
    let mut row: Vec<u32> = (0..=b.len() as u32).collect();
    for (i, x) in a.bytes().enumerate() {
        let mut previous = row[0];
        row[0] = i as u32 + 1;
        for (j, y) in b.bytes().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (previous + u32::from(x != y)).min(row[j] + 1).min(old + 1);
            previous = old;
        }
    }
    row[b.len()]
}

impl State {
    pub fn rebuild(
        &mut self,
        raw: &str,
        text: &str,
        pinyin: &str,
        words: &[(String, String)],
        engine: &Engine,
    ) {
        let Some(ranges) = spans(raw, pinyin, engine) else {
            self.view = View::default();
            return;
        };
        if text.chars().count() != ranges.len() {
            self.view = View::default();
            return;
        }
        self.pins
            .retain(|(b, keys)| raw.get(b.from..b.to) == Some(keys));
        let mut blocks = Vec::new();
        let mut pos = 0;
        for (word, reading) in words {
            let n = word.chars().count();
            if n == 0 || pos + n > ranges.len() {
                blocks.clear();
                break;
            }
            blocks.push(Block {
                from: ranges[pos].0,
                to: ranges[pos + n - 1].1,
                text: word.clone(),
                pinyin: reading.clone(),
                ..Block::default()
            });
            pos += n;
        }
        if blocks.is_empty() || pos != ranges.len() {
            blocks = vec![Block {
                from: 0,
                to: raw.len(),
                text: text.into(),
                pinyin: pinyin.into(),
                ..Block::default()
            }];
        }
        if self.pins.is_empty() {
            let rows = engine.boundaries.read().unwrap();
            if !rows.is_empty() {
                let readings: Vec<_> = pinyin.split_whitespace().collect();
                for start in 0..readings.len() {
                    for end in (start + 2..=readings.len().min(start + 12)).rev() {
                        let key = readings[start..end].join(" ");
                        let Some((parts, _)) = rows.get(&key) else {
                            continue;
                        };
                        // Do not bind the final letter of an unfinished syllable:
                        // its range will grow on the next key (n -> ni).
                        let tail =
                            raw[ranges[end - 1].0..ranges[end - 1].1].trim_start_matches('\'');
                        if (engine.config().shuangpin && tail.len() < 2)
                            || (!engine.config().shuangpin
                                && tail.len() < readings[end - 1].len()
                                && readings[end - 1].starts_with(tail)
                                && !crate::syllable::is_valid(tail))
                        {
                            continue;
                        }
                        let from = ranges[start].0;
                        let to = ranges[end - 1].1;
                        if self.pins.iter().any(|(b, _)| b.from < to && b.to > from) {
                            continue;
                        }
                        let mut at = start;
                        for (word, reading) in parts {
                            let n = reading.split_whitespace().count();
                            let b = Block {
                                from: ranges[at].0,
                                to: ranges[at + n - 1].1,
                                text: word.clone(),
                                pinyin: reading.clone(),
                                ..Block::default()
                            };
                            self.pins.push((b.clone(), raw[b.from..b.to].into()));
                            at += n;
                        }
                        self.edited = true;
                        break;
                    }
                }
                self.pins.sort_by_key(|(b, _)| b.from);
            }
        }
        let cursor = self.view.cursor.filter(|at| *at <= raw.len());
        let last = pinyin.split_whitespace().last().unwrap_or("");
        let pending_from = ranges
            .last()
            .and_then(|(a, z)| {
                let tail = raw[*a..*z].trim_start_matches('\'');
                ((!engine.config().shuangpin
                    && tail.len() < last.len()
                    && last.starts_with(tail)
                    && !crate::syllable::is_valid(tail))
                    || (engine.config().shuangpin && tail.len() == 1))
                    .then_some(*a)
            })
            .filter(|at| *at > 0);
        if let Some(at) = pending_from {
            // The incomplete final syllable must not rewrite the complete prefix.
            if self.view.raw == raw[..at] && !self.view.blocks.is_empty() {
                blocks = self.view.blocks.clone();
            } else {
                let chars: Vec<_> = text.chars().collect();
                let p: Vec<_> = pinyin.split_whitespace().collect();
                blocks.retain(|b| b.from < at);
                if let Some(b) = blocks.last_mut().filter(|b| b.to > at) {
                    let start = ranges.iter().position(|(a, _)| *a == b.from).unwrap_or(0);
                    let end = ranges.len() - 1;
                    b.to = at;
                    b.text = chars[start..end].iter().collect();
                    b.pinyin = p[start..end].join(" ");
                }
            }
        }
        self.view = View {
            text: text.into(),
            raw: raw.into(),
            cursor,
            blocks,
            alternatives: Vec::new(),
            pending_from,
            page: 0,
        };
        self.update_text();
        if !self.pins.is_empty() && !self.model_words {
            self.apply_pins(engine);
        }
        if self.model_words {
            for b in &mut self.view.blocks {
                b.confirmed = self
                    .pins
                    .iter()
                    .any(|(p, _)| p.confirmed && p.from <= b.from && p.to >= b.to);
            }
        }
        self.focus(engine);
    }
    fn decode(&self, engine: &Engine, from: usize, to: usize) -> Vec<Block> {
        let Some(raw) = self.view.raw.get(from..to) else {
            return Vec::new();
        };
        if raw.is_empty() {
            return Vec::new();
        }
        let old = engine.context();
        let before = self
            .view
            .blocks
            .iter()
            .filter(|b| b.to <= from)
            .map(|b| b.text.as_str())
            .collect::<String>();
        engine.set_context(&(old.clone() + &before));
        let candidate = engine.try_input(raw).ok().and_then(|v| {
            v.into_iter()
                .find(|c| c.consumed == raw.len() && !c.long_completion)
        });
        engine.set_context(&old);
        let Some(c) = candidate else {
            return Vec::new();
        };
        let mut state = State::default();
        let words = c
            .words
            .iter()
            .map(|w| (w.text.clone(), w.pinyin.clone()))
            .collect::<Vec<_>>();
        state.rebuild(raw, &c.text, &c.pinyin, &words, engine);
        for b in &mut state.view.blocks {
            b.from += from;
            b.to += from;
        }
        state.view.blocks
    }
    fn apply_pins(&mut self, engine: &Engine) {
        let pins = self.pins.clone();
        let mut out = Vec::new();
        let mut at = 0;
        for (b, _) in pins {
            if b.from > at {
                out.extend(self.decode(engine, at, b.from));
            }
            at = b.to;
            out.push(b);
        }
        if at < self.view.raw.len() {
            out.extend(self.decode(engine, at, self.view.raw.len()));
        }
        // Never replace a valid sentence with a partial decode.
        if out.first().is_some_and(|b| b.from == 0)
            && out.last().is_some_and(|b| b.to == self.view.raw.len())
            && out.windows(2).all(|b| b[0].to == b[1].from)
        {
            self.view.blocks = out;
            self.update_text();
        }
    }
    fn update_text(&mut self) {
        self.view.text = self.view.blocks.iter().map(|b| b.text.as_str()).collect();
    }
    fn focus(&mut self, engine: &Engine) {
        self.view.alternatives.clear();
        let index = self.view.cursor.and_then(|at| {
            self.view
                .blocks
                .iter()
                .position(|b| at <= b.to && at > b.from)
                .or(Some(0))
        });
        for (i, b) in self.view.blocks.iter_mut().enumerate() {
            b.focused = Some(i) == index;
        }
        let Some(b) = index.and_then(|i| self.view.blocks.get(i)).cloned() else {
            return;
        };
        let Some(raw) = self.view.raw.get(b.from..b.to) else {
            return;
        };
        let mut ranges = vec![(b.from, b.to)];
        if let Some(chars) = spans(raw, &b.pinyin, engine) {
            let at = self.view.cursor.unwrap_or(b.to).saturating_sub(b.from);
            if let Some(&(a, z)) = chars.iter().find(|(a, z)| at > *a && at <= *z) {
                if z - a < raw.len() {
                    ranges.insert(0, (b.from + a, b.from + z));
                }
            }
        }
        let context = engine.context();
        let before = self
            .view
            .blocks
            .iter()
            .filter(|w| w.to <= b.from)
            .map(|w| w.text.as_str())
            .collect::<String>();
        engine.set_context(&(context.clone() + &before));
        for (a, z) in ranges {
            if let Ok(found) = engine.try_input(&self.view.raw[a..z]) {
                for c in found
                    .into_iter()
                    .filter(|c| c.consumed == z - a && !c.long_completion && !c.pinyin.is_empty())
                    .take(36)
                {
                    if c.text == b.text
                        || self
                            .view
                            .alternatives
                            .iter()
                            .any(|x| x.text == c.text && x.from == a && x.to == z)
                    {
                        continue;
                    }
                    self.view.alternatives.push(Alternative {
                        text: c.text,
                        pinyin: c.pinyin,
                        from: a,
                        to: z,
                    });
                }
            }
        }
        engine.set_context(&context);
    }
    fn pin(&mut self, b: Block) {
        let Some(keys) = self.view.raw.get(b.from..b.to).map(str::to_owned) else {
            return;
        };
        self.pins.retain(|(p, _)| p.to <= b.from || p.from >= b.to);
        self.pins.push((b, keys));
        self.pins.sort_by_key(|(b, _)| b.from);
    }
    pub fn action(&mut self, action: Action, engine: &Engine) -> bool {
        self.model_words = false;
        if self.view.blocks.is_empty() {
            return false;
        }
        if !matches!(action, Action::Cycle { .. }) {
            self.cycle = None;
        }
        match action {
            Action::Leave => {
                self.view.cursor = None;
                self.view.page = 0;
            }
            Action::Page { next } => {
                let last = self.view.alternatives.len().saturating_sub(1) / 9;
                self.view.page = if next {
                    (self.view.page + 1).min(last)
                } else {
                    self.view.page.saturating_sub(1)
                };
                return true;
            }
            Action::Cycle { backward } => {
                let Some(block) = self.view.blocks.iter().find(|b| b.focused).cloned() else {
                    return false;
                };
                let mut cycle = self.cycle.take().unwrap_or_else(|| {
                    let mut choices = vec![Alternative {
                        text: block.text.clone(),
                        pinyin: block.pinyin.clone(),
                        from: block.from,
                        to: block.to,
                    }];
                    choices.extend(self.view.alternatives.clone());
                    Cycle {
                        view: self.view.clone(),
                        pins: self.pins.clone(),
                        choices,
                        index: 0,
                    }
                });
                cycle.index = if backward {
                    cycle.index.saturating_sub(1)
                } else {
                    (cycle.index + 1).min(cycle.choices.len() - 1)
                };
                self.view = cycle.view.clone();
                self.pins = cycle.pins.clone();
                if cycle.index > 0 {
                    self.view.alternatives = vec![cycle.choices[cycle.index].clone()];
                    self.action(Action::Choose { index: 0 }, engine);
                } else {
                    self.update_text();
                    self.focus(engine);
                    self.edited = !self.pins.is_empty();
                }
                self.cycle = Some(cycle);
                return true;
            }
            Action::Caret { at } => self.view.cursor = Some(at.min(self.view.raw.len())),
            Action::Focus { at } => self.view.cursor = Some(at.min(self.view.raw.len())),
            Action::Move { backward } => {
                let at =
                    self.view
                        .cursor
                        .unwrap_or(if backward { self.view.raw.len() + 1 } else { 0 });
                self.view.cursor = if backward {
                    self.view
                        .blocks
                        .iter()
                        .rev()
                        .find(|b| b.to < at)
                        .map(|b| b.to)
                } else {
                    self.view.blocks.iter().find(|b| b.to > at).map(|b| b.to)
                };
            }
            Action::Choose { index } => {
                let Some(a) = self.view.alternatives.get(index).cloned() else {
                    return false;
                };
                // All text before the selected interval remains stable; later user
                // choices remain pinned, while unconfirmed suffixes can re-decode.
                for b in self
                    .view
                    .blocks
                    .clone()
                    .into_iter()
                    .filter(|b| b.to <= a.from)
                {
                    self.pin(b);
                }
                if let Some(b) = self
                    .view
                    .blocks
                    .iter()
                    .find(|b| b.from < a.from && b.to > a.from)
                    .cloned()
                {
                    if let Some(ranges) = spans(&self.view.raw[b.from..b.to], &b.pinyin, engine) {
                        let n = ranges.iter().filter(|(_, z)| b.from + z <= a.from).count();
                        self.pin(Block {
                            from: b.from,
                            to: a.from,
                            text: b.text.chars().take(n).collect(),
                            pinyin: b
                                .pinyin
                                .split_whitespace()
                                .take(n)
                                .collect::<Vec<_>>()
                                .join(" "),
                            confirmed: b.confirmed,
                            ..Block::default()
                        });
                    }
                }
                self.pin(Block {
                    from: a.from,
                    to: a.to,
                    text: a.text,
                    pinyin: a.pinyin,
                    confirmed: true,
                    ..Block::default()
                });
                self.edited = true;
                self.apply_pins(engine);
                self.view.cursor = Some(a.to);
            }
            Action::Resize { expand } => {
                let Some(i) = self.view.blocks.iter().position(|b| b.focused) else {
                    return false;
                };
                let b = self.view.blocks[i].clone();
                let j = if expand { i + 1 } else { i };
                let Some(source) = self.view.blocks.get(j).cloned() else {
                    return false;
                };
                let Some(ranges) = spans(
                    &self.view.raw[source.from..source.to],
                    &source.pinyin,
                    engine,
                ) else {
                    return false;
                };
                if !expand && ranges.len() < 2 {
                    return false;
                }
                let cut = if expand {
                    source.from + ranges[0].1
                } else {
                    source.from + ranges[ranges.len() - 2].1
                };
                let mut text = b.text.clone();
                let mut readings: Vec<String> =
                    b.pinyin.split_whitespace().map(str::to_owned).collect();
                if expand {
                    text.extend(source.text.chars().take(1));
                    readings.extend(source.pinyin.split_whitespace().take(1).map(str::to_owned));
                } else {
                    text.pop();
                    readings.pop();
                }
                let left = Block {
                    from: b.from,
                    to: cut,
                    text,
                    pinyin: readings.join(" "),
                    confirmed: true,
                    ..Block::default()
                };
                let end = if expand {
                    source.to
                } else {
                    self.view.blocks.get(i + 1).map_or(b.to, |x| x.to)
                };
                let whole = self.view.blocks[i..=if expand {
                    j
                } else {
                    (i + 1).min(self.view.blocks.len() - 1)
                }]
                    .iter()
                    .map(|x| x.text.as_str())
                    .collect::<String>();
                let allp = self.view.blocks[i..=if expand {
                    j
                } else {
                    (i + 1).min(self.view.blocks.len() - 1)
                }]
                    .iter()
                    .map(|x| x.pinyin.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let n = left.text.chars().count();
                let right = Block {
                    from: cut,
                    to: end,
                    text: whole.chars().skip(n).collect(),
                    pinyin: allp
                        .split_whitespace()
                        .skip(n)
                        .collect::<Vec<_>>()
                        .join(" "),
                    ..Block::default()
                };
                self.pin(left.clone());
                if !right.text.is_empty() {
                    self.pin(right);
                }
                self.merged.retain(|(word, _)| {
                    !left.text.starts_with(word) && !word.starts_with(&left.text)
                });
                if expand {
                    self.merged.push((left.text.clone(), left.pinyin.clone()));
                }
                self.boundary_marks.retain(|x| *x != b.to);
                self.boundary_marks.push(cut);
                self.edited = true;
                self.boundaries_changed = true;
                self.apply_pins(engine);
                self.view.cursor = Some(cut);
            }
        }
        self.view.page = 0;
        self.focus(engine);
        true
    }
    /// Shift surviving choices around edits; intersecting intervals are invalid.
    pub fn edit(&mut self, at: usize, removed: usize, added: usize) {
        self.cycle = None;
        self.model_words = false;
        self.pins.retain_mut(|(b, _)| {
            if b.to <= at {
                return true;
            }
            if b.from >= at + removed {
                b.from = b.from + added - removed;
                b.to = b.to + added - removed;
                return true;
            }
            false
        });
        self.view.cursor = None;
        self.boundaries_changed = false;
        self.boundary_marks.clear();
        self.merged.clear();
    }
    pub fn forget(&mut self, word: &str) {
        self.pins.retain(|(b, _)| b.text != word);
        self.merged.retain(|(w, _)| w != word);
        self.edited = !self.pins.is_empty();
        self.boundaries_changed = false;
    }
    pub fn constrain(&self, query: &mut crate::enhancement::Query, engine: &Engine) -> bool {
        query.variants.clear();
        let Some(ranges) = spans(&self.view.raw, &query.pinyin, engine) else {
            return false;
        };
        for (b, _) in &self.pins {
            let Some(start) = ranges.iter().position(|(a, _)| *a == b.from) else {
                return false;
            };
            let chars: Vec<_> = b.text.chars().collect();
            if start + chars.len() > ranges.len() || ranges[start + chars.len() - 1].1 != b.to {
                return false;
            }
            for (i, ch) in chars.into_iter().enumerate() {
                query.letters[start + i] = vec![ch];
            }
        }
        query.candidates.retain(|s| {
            s.chars()
                .enumerate()
                .all(|(i, c)| query.letters.get(i).is_some_and(|v| v.contains(&c)))
        });
        query.key.push_str(&format!("\u{1c}{:?}", query.letters));
        true
    }
    pub fn accept_model(
        &mut self,
        raw: &str,
        text: &str,
        pinyin: &str,
        words: &[(String, String)],
        engine: &Engine,
    ) {
        self.model_words = true;
        self.rebuild(raw, text, pinyin, words, engine);
    }
}

pub fn words(words: &[CandidateWord]) -> Vec<(String, String)> {
    words
        .iter()
        .map(|w| (w.text.clone(), w.pinyin.clone()))
        .collect()
}
