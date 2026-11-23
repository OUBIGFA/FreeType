//! 独立查询模式的按键与分页。候选注解仅用于显示，所有提交只取 text。
use freetype_protocol::{keysym, modifier, Config, Frame, Key};
use wowtype_core::{input_tools::ToolCandidate, shuangpin::Scheme, Engine};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { English, Radical }

#[derive(Clone)]
pub struct LookupMode {
    pub kind: Kind,
    pub query: String,
    pub cursor: usize,
    pub candidates: Vec<ToolCandidate>,
}

pub enum Action { Stay, Close, Pass, Commit { text: String, learn: bool } }

impl LookupMode {
    pub fn new(kind: Kind) -> Self { Self { kind, query: String::new(), cursor: 0, candidates: Vec::new() } }

    pub fn refresh(&mut self, engine: &Engine, config: &Config) {
        let Some(tools) = engine.input_tools() else { return; };
        self.candidates = match self.kind {
            Kind::English => engine.english_tool_candidates(&self.query, 200, config.english_correction),
            Kind::Radical => tools.radical(&self.query, config.shuangpin.then(|| Scheme::from_name(&config.shuangpin_scheme)).flatten(), 200),
        };
        if self.kind == Kind::English && !self.query.is_empty() {
            let raw = ToolCandidate { text: self.query.clone(), reading: String::new(), gloss: "原样输入".into(), corrected: false };
            // 拼写建议不能替换原串成为空格默认项；正常补全也保留原样入口。
            if self.candidates.first().is_some_and(|e| e.corrected) || self.candidates.is_empty() {
                self.candidates.insert(0, raw);
            } else if !self.candidates.iter().any(|e| e.text == self.query) { self.candidates.push(raw); }
        }
        self.cursor = 0;
    }

    pub fn select(&self, index: usize, config: &Config, space: bool) -> Action {
        let Some(candidate) = self.candidates.get(index) else { return Action::Stay; };
        let mut text = candidate.text.clone();
        if self.kind == Kind::English && space && config.english_space && !text.ends_with(['\'', ' ']) { text.push(' '); }
        Action::Commit { text, learn: self.kind == Kind::English && !candidate.corrected }
    }

    pub fn key(&mut self, key: &Key, engine: &Engine, config: &Config) -> Action {
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
            self.query.clear(); self.candidates.clear(); self.cursor = 0;
            return Action::Pass;
        }
        let page = config.page_size as usize;
        match key.keysym {
            keysym::ESCAPE => return Action::Close,
            keysym::RETURN | keysym::KP_ENTER | 0x20 if self.query.is_empty() => return Action::Pass,
            keysym::RETURN | keysym::KP_ENTER if self.kind == Kind::English => return Action::Commit { text: self.query.clone(), learn: false },
            keysym::RETURN | keysym::KP_ENTER | 0x20 => return self.select(self.cursor, config, true),
            keysym::BACKSPACE => { if self.query.pop().is_none() { return Action::Close; } self.refresh(engine, config); }
            keysym::LEFT | keysym::UP => self.cursor = self.cursor.saturating_sub(1),
            keysym::RIGHT | keysym::DOWN | keysym::TAB => self.cursor = (self.cursor + 1).min(self.candidates.len().saturating_sub(1)),
            keysym::PAGE_UP => self.page(false, page),
            keysym::PAGE_DOWN => self.page(true, page),
            0x30..=0x39 if !self.candidates.is_empty() => {
                let slot = ((key.keysym - 0x30) as usize + 9) % 10;
                if slot < page { return self.select(self.cursor / page * page + slot, config, true); }
            }
            sym if (keysym::KP_0..=keysym::KP_9).contains(&sym) && !self.candidates.is_empty() => {
                let slot = ((sym - keysym::KP_0) as usize + 9) % 10;
                if slot < page { return self.select(self.cursor / page * page + slot, config, true); }
            }
            sym => if let Some(ch) = char::from_u32(sym).filter(|c| c.is_ascii_alphabetic() || *c == '\'' || (self.kind == Kind::Radical && *c == ';')) {
                if self.query.len() < 100 { self.query.push(ch); self.refresh(engine, config); }
            } else if self.kind == Kind::English && char::from_u32(sym).is_some_and(|c| c.is_ascii_punctuation()) {
                let selected = self.candidates.get(self.cursor).map_or(self.query.as_str(), |e| e.text.as_str());
                return Action::Commit { text: format!("{selected}{}", char::from_u32(sym).unwrap()), learn: false };
            } else { return Action::Pass; },
        }
        Action::Stay
    }

    pub fn page(&mut self, next: bool, page: usize) {
        self.cursor = if next { ((self.cursor / page + 1) * page).min(self.candidates.len().saturating_sub(1)) } else { self.cursor.saturating_sub(page) / page * page };
    }

    pub fn frame(&self, handled: bool, config: &Config) -> Frame {
        let page = config.page_size as usize;
        let start = self.cursor / page * page;
        let shown: Vec<_> = self.candidates.iter().skip(start).take(page).collect();
        Frame {
            handled,
            title: Some(if self.kind == Kind::English { "英文候选 · Esc 返回" } else { "拆字查字 · 输入部件拼音" }.into()),
            preedit: self.query.clone(), preedit_cursor: self.query.encode_utf16().count(),
            candidates: shown.iter().map(|e| e.text.clone()).collect(),
            annotations: shown.iter().map(|e| {
                if self.kind == Kind::English && !config.english_gloss { return String::new(); }
                let gloss: String = e.gloss.chars().take(75).collect();
                format!("{}{}{}", e.reading, if !e.reading.is_empty() && !gloss.is_empty() { "  " } else { "" }, gloss)
            }).collect(),
            badges: shown.iter().map(|e| if e.corrected && config.typo_mark { "*".into() } else { String::new() }).collect(),
            cursor: (!shown.is_empty()).then_some(self.cursor - start),
            has_prev: start > 0, has_next: start + shown.len() < self.candidates.len(),
            ..Frame::default()
        }
    }
}
