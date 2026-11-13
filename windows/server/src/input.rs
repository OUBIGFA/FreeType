//! 一个输入框的状态和按键规则。
//!
//! 核心的会话（`wowtype_core::session::Session`）管拼音、候选、选词和学习；这里补上外壳的规则
//! （Linux 版在 fcitx5 外壳里）：
//!
//! - 中 / 英两种状态，英文状态下按键交给应用（自动加空格打开时除外）；
//! - 中文标点：没在组字时按规则转换（数字后面原样、引号左右交替）；组字时先上屏候选再上屏标点；
//!   成对标点补全（（ “ 《 【 补上另一半，光标停在中间）；
//! - 工具栏（v + 空格）、表情面板（[`crate::tools`]）；
//! - 邮箱、网址补全（[`crate::raw`]）；
//! - 中英文之间自动加空格；
//! - 组字时会话不认的键一律吃掉，不让它漏给应用。

use freetype_protocol::{keysym, modifier, Config, Frame, Key, Mode, Panel};
use wowtype_core::autospace;
use wowtype_core::session::{self, CandidateKind, KeyAction, Session};
use wowtype_core::symbols;
use wowtype_core::Engine;

use crate::raw::{self, Raw};
use crate::tools::{self, Outcome, Page, Sources, Tool};

/// 光标前记多少个字当上文。
const CONTEXT_CHARS: usize = 16;
/// 标点上屏前最多连按几次空格把句首词选完。
const MAX_HEAD_COMMITS: usize = 32;

/// 成对标点：左半边 → 右半边。
const PAIRS: [(&str, &str); 7] = [
    ("（", "）"),
    ("【", "】"),
    ("《", "》"),
    ("「", "」"),
    ("“", "”"),
    ("‘", "’"),
    ("『", "』"),
];

/// 处理按键时要用的东西（宿主每次给）。
pub struct Env<'a> {
    pub config: &'a Config,
    pub mode: Mode,
    pub engine: Option<&'a Engine>,
    /// 常用语：(编码, 上屏的文字)。
    pub phrases: &'a [(String, String)],
    pub recent_emoji: &'a [String],
}

impl Env<'_> {
    fn sources(&self) -> Sources<'_> {
        Sources {
            engine: self.engine,
            phrases: self.phrases,
            recent: self.recent_emoji,
            clipboard_on: self.config.clipboard_history,
        }
    }

    fn autospace_flags(&self) -> u32 {
        if !self.config.autospace {
            return 0;
        }
        let mut flags = 0;
        if self.config.autospace_letters {
            flags |= autospace::LETTERS;
        }
        if self.config.autospace_digits {
            flags |= autospace::DIGITS;
        }
        flags
    }

    fn shuangpin(&self) -> bool {
        self.engine.is_some_and(|e| e.config().shuangpin)
    }
}

/// 按键的结果：给 DLL 的应答，加上宿主要做的事。
#[derive(Default)]
pub struct Out {
    pub frame: Frame,
    /// 上屏了一个 emoji（记进最近用过的）。
    pub emoji: Option<String>,
    /// 忘掉了学过的词（马上存盘）。
    pub forgot: bool,
}

#[derive(Clone)]
pub struct ClientState {
    pub session: Session<'static>,
    /// 这个输入框里光标前的字（上屏的字，或 DLL 读到的）。
    pub context: String,
    /// 双引号、单引号上一次给的是不是左引号（没开成对补全时左右交替）。
    quote_open: [bool; 2],
    pub tool: Option<Tool>,
    pub raw: Option<Raw>,
    /// 应用的程序名（按应用记中英文时用）。
    pub app: String,
}

fn shown_text(text: &str) -> String {
    text.replace('\n', "⏎").replace('\t', "⇥")
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn printable(key: &Key) -> Option<char> {
    if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
        return None;
    }
    char::from_u32(key.keysym).filter(|c| c.is_ascii_graphic())
}

/// 可见的 ASCII 标点（字母、数字以外）。
fn punct_char(key: &Key) -> Option<char> {
    printable(key).filter(|c| !c.is_ascii_alphanumeric())
}

fn is_letter(key: &Key) -> bool {
    char::from_u32(key.keysym).is_some_and(|c| c.is_ascii_alphabetic())
}

/// 小键盘数字键对应的字符。
fn keypad_digit(key: &Key) -> Option<char> {
    (keysym::KP_0..=keysym::KP_9)
        .contains(&key.keysym)
        .then(|| char::from(b'0' + (key.keysym - keysym::KP_0) as u8))
}

pub fn session_config(config: &Config) -> session::Config {
    session::Config {
        page_size: config.page_size as usize,
        calculator: config.calculator,
        date_time: config.date_time,
        emoji_suggest: config.emoji_suggest,
        horizontal: config.horizontal,
        page_keys: config.page_keys,
        select_keys: match config.select_keys.as_str() {
            "semicolon" => session::SelectKeys::Semicolon,
            "quote" => session::SelectKeys::SemicolonQuote,
            _ => session::SelectKeys::Off,
        },
        pick_char: config.pick_char,
    }
}

impl ClientState {
    pub fn new(engine: Option<&'static Engine>, config: &Config, app: String) -> Self {
        let mut session = Session::new();
        session.set_engine(engine);
        session.configure(session_config(config));
        Self {
            session,
            context: String::new(),
            quote_open: [false; 2],
            tool: None,
            raw: None,
            app,
        }
    }

    /// 在组字、开着面板或邮箱补全。
    pub fn busy(&self) -> bool {
        self.session.composing() || self.tool.is_some() || self.raw.is_some()
    }

    fn prev(&self) -> Option<char> {
        self.context.chars().last()
    }

    /// 光标前的字（DLL 读到的）。
    pub fn set_before(&mut self, before: &str) {
        let n = before.chars().count();
        self.context = before.chars().skip(n.saturating_sub(CONTEXT_CHARS)).collect();
        self.session.set_context(&self.context.clone());
    }

    pub fn remember(&mut self, text: &str) {
        self.context.push_str(text);
        let n = self.context.chars().count();
        if n > CONTEXT_CHARS {
            self.context = self.context.chars().skip(n - CONTEXT_CHARS).collect();
        }
        self.session.set_context(&self.context.clone());
    }

    /// 上屏一段字：交界和段内的中英混输共用同一规则，再记进上文。
    fn emit(&mut self, text: &str, env: &Env) -> String {
        let flags = env.autospace_flags();
        let mut out = String::new();
        let mut prev = self.prev();
        for next in text.chars() {
            if prev.is_some_and(|p| autospace::needs_space(p, next, flags)) {
                out.push(' ');
            }
            out.push(next);
            prev = Some(next);
        }
        self.remember(&out);
        out
    }

    /// 关掉面板和邮箱补全，清空编辑区。
    pub fn clear(&mut self) {
        self.tool = None;
        self.raw = None;
        if self.session.composing() {
            self.session.reset(false);
        }
    }

    /// 核心的结果翻成应答（不带样式），记下上屏的字。
    fn core_frame(&mut self, core: session::Frame, env: &Env) -> Out {
        let mut out = Out {
            forgot: core.forgot,
            ..Out::default()
        };
        if let Some(text) = &core.commit {
            if let Some(choice) = self.session.last_choice() {
                if choice.kind == CandidateKind::Emoji && !choice.emoji.is_empty() {
                    out.emoji = Some(choice.emoji.clone());
                }
            }
            out.frame.commit = Some(self.emit(text, env));
        }
        let candidates: Vec<String> = if core.cand_count > 0 {
            self.session.candidates()[core.cand_start..core.cand_start + core.cand_count]
                .iter()
                .map(|c| shown_text(&c.text))
                .collect()
        } else {
            Vec::new()
        };
        let expanded = core.panel == session::Panel::Expanded;
        if expanded {
            let row = core.cand_cursor.map(|c| c / session::GRID_COLS);
            out.frame.labels = (0..candidates.len())
                .map(|i| {
                    if Some(i / session::GRID_COLS) == row {
                        (i % session::GRID_COLS + 1).to_string()
                    } else {
                        String::new()
                    }
                })
                .collect();
            out.frame.panel = Panel::Grid;
            out.frame.columns = session::GRID_COLS;
        }
        out.frame.forgettable = !candidates.is_empty();
        out.frame.handled = core.handled;
        out.frame.preedit_cursor = utf16_len(&core.preedit[..core.preedit_cursor.min(core.preedit.len())]);
        out.frame.preedit = core.preedit;
        out.frame.candidates = candidates;
        out.frame.cursor = core.cand_cursor;
        out.frame.has_prev = core.has_prev;
        out.frame.has_next = core.has_next;
        out
    }

    /// 现在的样子（不按键）。
    pub fn current(&mut self, handled: bool, env: &Env) -> Out {
        if let Some(tool) = &self.tool {
            return Out {
                frame: tool_frame(tool, handled),
                ..Out::default()
            };
        }
        if let Some(raw) = &self.raw {
            return Out {
                frame: raw_frame(raw, handled, env.config),
                ..Out::default()
            };
        }
        let core = self.session.frame(handled);
        self.core_frame(core, env)
    }

    /// 打开工具栏或表情面板（结束正在打的字）。
    pub fn open_tool(&mut self, page: Page, env: &Env) -> Out {
        self.raw = None;
        if self.session.composing() {
            self.session.reset(false);
        }
        self.tool = Some(Tool::open(page, &env.sources()));
        self.current(true, env)
    }

    /// 中文标点（数字后面原样；引号左右交替）。None：原样上屏 ASCII。
    fn punct_text(&mut self, c: char, env: &Env, prev: Option<char>) -> Option<String> {
        if !env.config.fullwidth_punct || prev.is_some_and(|p| p.is_ascii_digit()) {
            return None;
        }
        if c == '"' || c == '\'' {
            let slot = usize::from(c == '\'');
            let (open, close) = if c == '"' { ("“", "”") } else { ("‘", "’") };
            if env.config.auto_pair {
                return Some(open.to_string());
            }
            let want_open = prev.is_some_and(|p| p.is_whitespace() || "(（[【<《{「『".contains(p)) || !self.quote_open[slot];
            self.quote_open[slot] = want_open;
            return Some(if want_open { open } else { close }.to_string());
        }
        symbols::fullwidth_punct(c).map(str::to_string)
    }

    /// 标点要上屏的字和光标往回挪几个（成对补全）。
    fn punct_commit(&mut self, c: char, env: &Env, prev: Option<char>) -> (String, usize) {
        let text = self.punct_text(c, env, prev).unwrap_or_else(|| c.to_string());
        if env.config.auto_pair {
            if let Some((_, close)) = PAIRS.iter().find(|(open, _)| *open == text) {
                return (format!("{text}{close}"), utf16_len(close));
            }
        }
        (text, 0)
    }

    pub fn key(&mut self, key: &Key, env: &Env) -> Out {
        if self.tool.is_some() {
            return self.tool_key(key, env);
        }
        if self.raw.is_some() {
            return self.raw_key(key, env);
        }
        if env.mode == Mode::English {
            return self.pass_or_space(key, env);
        }
        let composing = self.session.composing();
        if key.modifiers & modifier::CAPS != 0 && !composing && is_letter(key) {
            // 大写锁定：字母原样交给应用
            return self.pass_or_space(key, env);
        }
        let plain = key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) == 0;
        // v + 空格：工具栏
        if plain
            && key.keysym == 0x20
            && env.config.toolbar
            && !env.shuangpin()
            && self.session.input() == "v"
        {
            return self.open_tool(Page::Menu, env);
        }
        if plain {
            if let Some(out) = self.maybe_raw(key, env) {
                return out;
            }
        }
        let prev = self.prev();
        let core_key = session::Key {
            keysym: key.keysym,
            modifiers: key.modifiers,
            release: false,
            prev_char: prev,
            action: KeyAction::None,
        };
        let core = self.session.key(&core_key);
        if core.handled || !composing {
            if core.handled {
                return self.core_frame(core, env);
            }
            // 没在组字：中文标点
            if let Some(c) = punct_char(key) {
                let (text, back) = self.punct_commit(c, env, prev);
                let mut out = self.core_frame(core, env);
                out.frame.handled = true;
                out.frame.commit = Some(self.emit(&text, env));
                out.frame.caret_back = back;
                return out;
            }
            return self.pass_or_space(key, env);
        }
        // 组字时会话不认的键
        let Some(c) = punct_char(key) else {
            // 方向键以外的功能键、Ctrl 组合键：吃掉，不漏给应用
            let mut out = self.core_frame(core, env);
            out.frame.handled = true;
            return out;
        };
        // 标点：先把候选选完上屏，再上屏标点
        let mut text = String::new();
        let mut emoji = None;
        for _ in 0..MAX_HEAD_COMMITS {
            if !self.session.composing() {
                break;
            }
            let space = self.session.key(&session::Key::press(session::keysym::SPACE));
            if let Some(choice) = self.session.last_choice() {
                if choice.kind == CandidateKind::Emoji {
                    emoji = Some(choice.emoji.clone());
                }
            }
            if let Some(commit) = space.commit {
                text.push_str(&commit);
            }
            if !space.handled {
                break;
            }
        }
        if self.session.composing() {
            // 没有候选可选：原样上屏字母
            if let Some(commit) = self.session.commit_input().commit {
                text.push_str(&commit);
            }
        }
        let head = self.emit(&text, env);
        let prev = head.chars().last().or(prev);
        let (mark, back) = self.punct_commit(c, env, prev);
        self.remember(&mark);
        let mut out = self.current(true, env);
        out.frame.commit = Some(head + &mark);
        out.frame.caret_back = back;
        out.emoji = emoji.filter(|e| !e.is_empty());
        out
    }

    /// 交给应用的键；自动加空格时字母、数字前面可能要先补一个空格（自己上屏）。
    fn pass_or_space(&mut self, key: &Key, env: &Env) -> Out {
        let mut out = self.current(false, env);
        let Some(c) = printable(key).or_else(|| keypad_digit(key)) else {
            return out;
        };
        let flags = env.autospace_flags();
        if let Some(prev) = self.prev() {
            if flags != 0 && c.is_ascii_alphanumeric() && autospace::needs_space(prev, c, flags) {
                out.frame.handled = true;
                out.frame.commit = Some(self.emit(&c.to_string(), env));
                return out;
            }
        }
        // 交给应用的字也记进上文（下一个键判断要不要加空格、标点用）
        self.remember(&c.to_string());
        out
    }

    /// 要不要进邮箱、网址补全，或小键盘的点直接上屏。
    fn maybe_raw(&mut self, key: &Key, env: &Env) -> Option<Out> {
        let composing = self.session.composing();
        let plain_input = composing
            && self.session.selections().is_empty()
            && self.session.input().bytes().all(|b| b.is_ascii_alphanumeric());
        if key.keysym == keysym::KP_DECIMAL && env.config.numpad_dot {
            if plain_input {
                return Some(self.start_raw(format!("{}.", self.session.input()), env));
            }
            if !composing {
                let mut out = self.current(true, env);
                out.frame.commit = Some(self.emit(".", env));
                return Some(out);
            }
            return None;
        }
        if !env.config.email_completion {
            return None;
        }
        if key.keysym == u32::from(b'@') {
            if plain_input {
                return Some(self.start_raw(format!("{}@", self.session.input()), env));
            }
            if !composing {
                return Some(self.start_raw("@".to_string(), env));
            }
        }
        if key.keysym == u32::from(b'.') && plain_input {
            let input = self.session.input().to_string();
            let english = self
                .session
                .candidates()
                .first()
                .is_some_and(|c| c.text.is_ascii() && c.text.eq_ignore_ascii_case(&input));
            if raw::wants_completion(&input, english) {
                return Some(self.start_raw(format!("{input}."), env));
            }
        }
        None
    }

    fn start_raw(&mut self, text: String, env: &Env) -> Out {
        if self.session.composing() {
            self.session.reset(false);
        }
        self.raw = Some(Raw::new(text));
        self.current(true, env)
    }

    fn finish_raw(&mut self, text: String, env: &Env) -> Out {
        self.raw = None;
        let mut out = self.current(true, env);
        out.frame.commit = Some(self.emit(&text, env));
        out
    }

    fn raw_key(&mut self, key: &Key, env: &Env) -> Out {
        let Some(raw) = self.raw.as_mut() else {
            return self.current(false, env);
        };
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
            // 组字时的组合键：吃掉
            return self.current(true, env);
        }
        match key.keysym {
            0x20 => {
                let text = raw.current();
                return self.finish_raw(text, env);
            }
            keysym::RETURN | keysym::KP_ENTER => {
                let text = raw.text.clone();
                return self.finish_raw(text, env);
            }
            keysym::ESCAPE => {
                self.raw = None;
                return self.current(true, env);
            }
            keysym::BACKSPACE => {
                if !raw.pop() {
                    self.raw = None;
                }
            }
            keysym::LEFT | keysym::UP => raw.move_cursor(true),
            keysym::RIGHT | keysym::DOWN | keysym::TAB => raw.move_cursor(false),
            // 这些键没有邮箱正文含义，和候选框的翻页箭头使用同一入口。
            keysym::PAGE_UP | keysym::PAGE_DOWN => return self.page(key.keysym == keysym::PAGE_DOWN, env),
            keysym::KP_DECIMAL => raw.push('.'),
            sym => {
                let c = char::from_u32(sym)
                    .filter(|c| c.is_ascii_graphic())
                    .or_else(|| keypad_digit(key));
                match c {
                    Some(c) if raw::accepts(c) => raw.push(c),
                    Some(c) => {
                        // 别的标点：原样上屏编辑区，再照常上屏这个标点
                        let text = raw.text.clone();
                        self.raw = None;
                        let head = self.emit(&text, env);
                        let (mark, back) = self.punct_commit(c, env, head.chars().last());
                        self.remember(&mark);
                        let mut out = self.current(true, env);
                        out.frame.commit = Some(head + &mark);
                        out.frame.caret_back = back;
                        return out;
                    }
                    None => {}
                }
            }
        }
        self.current(true, env)
    }

    fn tool_key(&mut self, key: &Key, env: &Env) -> Out {
        let sources = env.sources();
        let Some(tool) = self.tool.as_mut() else {
            return self.current(false, env);
        };
        let outcome = tool.key(key, &sources);
        self.tool_outcome(outcome, env)
    }

    fn tool_outcome(&mut self, outcome: Outcome, env: &Env) -> Out {
        match outcome {
            Outcome::Stay { .. } => self.current(true, env),
            Outcome::Close => {
                self.tool = None;
                self.current(true, env)
            }
            Outcome::Pass => {
                self.tool = None;
                self.current(false, env)
            }
            Outcome::Commit { text, emoji } => {
                self.tool = None;
                let mut out = self.current(true, env);
                out.frame.commit = Some(self.emit(&text, env));
                if emoji {
                    out.emoji = Some(text);
                }
                out
            }
        }
    }

    /// 鼠标点了这一屏的第 index 个。
    pub fn select(&mut self, index: usize, env: &Env) -> Out {
        if let Some(tool) = self.tool.as_mut() {
            let outcome = tool.click(index, &env.sources());
            return self.tool_outcome(outcome, env);
        }
        if let Some(raw) = &self.raw {
            let all = raw.candidates();
            let page = env.config.page_size.max(1) as usize;
            let start = raw.cursor / page * page;
            if let Some(text) = all.get(start + index).cloned() {
                return self.finish_raw(text, env);
            }
            return self.current(true, env);
        }
        let core = self.session.select(index);
        self.core_frame(core, env)
    }

    pub fn page(&mut self, next: bool, env: &Env) -> Out {
        if let Some(tool) = self.tool.as_mut() {
            tool.page(next);
            return self.current(true, env);
        }
        if let Some(raw) = self.raw.as_mut() {
            let page = env.config.page_size.max(1) as usize;
            let n = raw.candidates().len();
            raw.cursor = if next {
                ((raw.cursor / page + 1) * page).min(n - 1)
            } else {
                (raw.cursor / page).saturating_sub(1) * page
            };
            return self.current(true, env);
        }
        let action = if next {
            KeyAction::NextPage
        } else {
            KeyAction::PrevPage
        };
        let core = self.session.key(&session::Key::action(action));
        self.core_frame(core, env)
    }

    /// 输入框没了焦点、应用要结束组字：`commit` 时原样上屏。
    pub fn reset(&mut self, commit: bool, env: &Env) -> Out {
        self.tool = None;
        if let Some(raw) = self.raw.take() {
            let mut out = self.current(true, env);
            if commit {
                out.frame.commit = Some(self.emit(&raw.text, env));
            }
            return out;
        }
        let core = if commit && self.session.composing() {
            self.session.commit_input()
        } else {
            self.session.reset(false)
        };
        self.core_frame(core, env)
    }
}

fn tool_frame(tool: &Tool, handled: bool) -> Frame {
    let view: tools::View = tool.view();
    Frame {
        handled,
        preedit_cursor: utf16_len(&view.preedit),
        preedit: view.preedit,
        title: Some(view.title),
        candidates: view.items,
        labels: view.labels,
        cursor: view.cursor,
        has_prev: view.has_prev,
        has_next: view.has_next,
        panel: view.panel,
        columns: view.columns,
        ..Frame::default()
    }
}

fn raw_frame(raw: &Raw, handled: bool, config: &Config) -> Frame {
    let all = raw.candidates();
    let page = config.page_size.max(1) as usize;
    let cursor = raw.cursor.min(all.len() - 1);
    let start = cursor / page * page;
    let end = (start + page).min(all.len());
    let candidates: Vec<String> = all[start..end].to_vec();
    Frame {
        handled,
        preedit_cursor: utf16_len(&raw.text),
        preedit: raw.text.clone(),
        title: Some("空格选中 · 回车原样上屏 · 方向键挑选".into()),
        labels: vec![String::new(); candidates.len()],
        candidates,
        cursor: Some(cursor - start),
        has_prev: start > 0,
        has_next: end < all.len(),
        ..Frame::default()
    }
}
