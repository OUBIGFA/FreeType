//! 按键分发：组字、表情网格、以词定字、选词键、双拼分号与编辑区光标。
use super::*;

impl<'e> Session<'e> {
    /// 处理一次按键。
    pub fn key(&mut self, key: &Key) -> Frame {
        let traced = self.trace_key_before(key);
        if !key.release && (key.keysym != keysym::BACKSPACE || self.composing()) {
            self.committed_learning.clear();
        }
        self.last_choice = None;
        let handled = self.handle_key(key);
        self.trace_key_after(key, traced);
        self.frame(handled)
    }

    pub(super) fn handle_key(&mut self, key: &Key) -> bool {
        if key.release {
            return false;
        }
        if self.composing() && key.is_forget() {
            self.forget_current(true);
            return true;
        }
        if key.has_command_modifier() {
            return false;
        }
        if self.composing() {
            if let Some(handled) = self.composing_key(key) {
                return handled;
            }
        }
        let sym = key.keysym;
        if (u32::from(b'a')..=u32::from(b'z')).contains(&sym)
            || (u32::from(b'A')..=u32::from(b'Z')).contains(&sym)
        {
            // 大写字母原样保留：Hello、iPhone 这类英文候选要用到
            self.push_input(char::from(sym as u8));
            return true;
        }
        if self.composing() && sym == keysym::APOSTROPHE {
            // 隔音符（xi'an）；开头的隔音符没有意义
            self.push_input('\'');
            return true;
        }
        false
    }

    /// 正在输入时的键（字母、隔音符除外）。Some：处理完了（handled 是里面的值）；
    /// None：接着当字母、隔音符看，都不是就交给外壳。
    pub(super) fn composing_key(&mut self, key: &Key) -> Option<bool> {
        let sym = key.keysym;
        // 先把候选算好：候选重算时候选框回到开头，别在读页和光标以后才重算
        let n = self.candidates().len();
        if self.config.sentence_editor
            && !self.calc_active()
            && !self.expanded
            && !self.sentence.view.blocks.is_empty()
        {
            if matches!(sym, keysym::TAB | keysym::ISO_LEFT_TAB)
                && self.sentence.view.blocks.len() > 1
                && self.config.page_keys & page_keys::TAB == 0
            {
                self.sentence_action(crate::sentence::Action::Move {
                    backward: sym == keysym::ISO_LEFT_TAB || key.modifiers & modifier::SHIFT != 0,
                });
                return Some(true);
            }
            if self.sentence.view.cursor.is_some() {
                if sym == u32::from(b'0') {
                    return Some(true);
                }
                if sym == keysym::SEMICOLON && !self.semicolon_is_key() {
                    let index = 9 * self.sentence.view.page;
                    self.sentence_action(crate::sentence::Action::Choose { index });
                    return Some(true);
                }
                if let Some(index) = (u32::from(b'1')..=u32::from(b'9'))
                    .contains(&sym)
                    .then_some(sym.saturating_sub(u32::from(b'1')) as usize)
                {
                    let index = index + 9 * self.sentence.view.page;
                    self.sentence_action(crate::sentence::Action::Choose { index });
                    return Some(true);
                }
                if matches!(sym, keysym::BRACKET_LEFT | keysym::BRACKET_RIGHT) {
                    self.sentence_action(crate::sentence::Action::Resize {
                        expand: sym == keysym::BRACKET_RIGHT,
                    });
                    return Some(true);
                }
                if sym == keysym::END {
                    self.sentence_action(crate::sentence::Action::Leave);
                    return Some(true);
                }
                if matches!(
                    sym,
                    keysym::PAGE_UP | keysym::PAGE_DOWN | keysym::MINUS | keysym::EQUAL
                ) {
                    self.sentence_action(crate::sentence::Action::Page {
                        next: matches!(sym, keysym::PAGE_DOWN | keysym::EQUAL),
                    });
                    return Some(true);
                }
                if matches!(sym, keysym::UP | keysym::DOWN) {
                    self.sentence_action(crate::sentence::Action::Cycle {
                        backward: sym == keysym::UP,
                    });
                    return Some(true);
                }
            }
            // 空格沿用普通候选的提交路径；辅助区不能把简拼尾音拆成两次上屏。
        }
        if self.calc_active() && self.input.len() > 1 {
            let expr = &self.input[1..];
            if matches!(sym, 0x24 | 0xa5 | 0xffe5) {
                if let Some(money) = crate::calc::money_text(expr) {
                    self.finish(money);
                }
                // 算式不完整或不能转金额时保留输入，不能落入普通标点上屏。
                return Some(true);
            }
            if let Some(value) = crate::calc::evaluate(expr) {
                if matches!(sym, keysym::RETURN | keysym::KP_ENTER) {
                    self.finish(crate::calc::format(value));
                    return Some(true);
                }
                if let Some(letter) = char::from_u32(sym).filter(char::is_ascii_alphabetic) {
                    let index = usize::from(letter.to_ascii_lowercase() as u8 - b'a');
                    let (start, len) = self.page_range(n);
                    if index < len {
                        self.choose_now(start + index);
                    }
                    return Some(true);
                }
            }
        }
        if self.expanded {
            if n == 0 {
                self.expanded = false;
            } else if let Some(handled) = self.grid_key(key, n) {
                return Some(handled);
            }
        }
        match sym {
            keysym::BACKSPACE => {
                // 选过词先撤销最后一次选词，拼音回到编辑区（后面还没选的拼音不动），
                // 和微信输入法一样；没有选词了才删字母
                let at = self.caret();
                if at < self.input.len() {
                    // 光标在拼音中间（#129）：删光标前一个；光标在最前面时撤销选词，没有就不动
                    if at > self.rest_start() {
                        let prev = self.input[..at]
                            .chars()
                            .next_back()
                            .map_or(1, char::len_utf8);
                        self.sentence.edit(at - prev, prev, 0);
                        self.input.replace_range(at - prev..at, "");
                        self.caret = Some(at - prev);
                    } else {
                        self.pop_selection();
                    }
                } else if self.pop_selection().is_none() {
                    if !self.input.is_empty() {
                        self.sentence.edit(self.input.len() - 1, 1, 0);
                    }
                    self.input.pop();
                }
                self.fresh = false;
                if self.input.is_empty() {
                    self.clear();
                }
                return Some(true);
            }
            keysym::ESCAPE => {
                self.clear();
                return Some(true);
            }
            keysym::SPACE => return Some(self.commit_current()),
            keysym::RETURN | keysym::KP_ENTER => {
                // 回车：已选的词 + 原始字母上屏
                self.commit_input_now();
                return Some(true);
            }
            _ => {}
        }
        if self.config.arrow_keys == ArrowKeys::Caret || self.sentence.view.cursor.is_some() {
            if let Some(handled) = self.caret_key(sym) {
                return Some(handled);
            }
        }
        if self.config.calculator {
            // V 模式：v 后面的数字、运算符进编辑区（不选词、不翻页、不当标点）。双拼时是大写 V
            if sym == keysym::EQUAL && self.calc_active() && self.input.len() > 1 {
                // = 上屏计算结果（第一个候选）
                return Some(self.commit_current());
            }
            if let Some(c) = calc_char(sym) {
                if self.calc_active() {
                    self.push_input(c);
                    return Some(true);
                }
            }
        }
        if let Some(nth) = self.select_key_index(sym) {
            // 分号选第 2 个、引号选第 3 个；这一页没这么多就什么都不做
            let (start, len) = self.page_range(n);
            if nth <= len {
                self.choose_now(start + nth - 1);
            }
            return Some(true);
        }
        if let Some(last) = self.pick_char_key(sym) {
            // 以词定字：没有能拆的候选时交给外壳（照常当标点）
            if self.pick_char(last) {
                return Some(true);
            }
            return None;
        }
        if let Some(digit) = key.digit() {
            // 数字按当前页选词；Windows 可显示 10 个，0 对应第 10 个。
            // 不足 10 个时仍然只吃掉 0，计算器已在前面的分支处理。
            let digit = if digit == 0 { 10 } else { digit };
            let (start, len) = self.page_range(n);
            if (1..=len).contains(&digit) {
                self.choose_now(start + digit - 1);
            }
            return Some(true);
        }
        if sym == keysym::SEMICOLON && self.semicolon_is_key() {
            // 微软、搜狗、紫光双拼用分号当 ing：等第二个键时分号进编辑区，不当标点
            self.push_input(';');
            return Some(true);
        }
        if n == 0 {
            return None;
        }
        let arrow = matches!(
            sym,
            keysym::UP | keysym::DOWN | keysym::LEFT | keysym::RIGHT
        );
        if !arrow && key.action != KeyAction::None {
            // 全局的翻页、移光标热键：翻不动也吃掉，不让它变成标点把字上屏
            match key.action {
                KeyAction::PrevPage => self.prev_page(),
                KeyAction::NextPage => self.next_page(),
                KeyAction::PrevCandidate => self.move_cursor(true),
                KeyAction::NextCandidate => self.move_cursor(false),
                KeyAction::None => {}
            }
            return Some(true);
        }
        // 横排时左右键移动候选、上下键翻页；竖排反过来。左右键用来移拼音里的光标时（上面 caret_key
        // 处理完了，到不了这里）和竖排一样上下键选词，下键也不展开面板
        let horizontal = self.config.horizontal && self.config.arrow_keys == ArrowKeys::Candidates;
        let (prev_key, next_key, prev_page_key, next_page_key) = if horizontal {
            (keysym::LEFT, keysym::RIGHT, keysym::UP, keysym::DOWN)
        } else {
            (keysym::UP, keysym::DOWN, keysym::LEFT, keysym::RIGHT)
        };
        if horizontal && sym == keysym::DOWN && n > 1 && !self.calc_active() {
            // 横排时下键展开候选面板（照微信输入法），光标停在当前选中的词上
            self.expanded = true;
            self.grid_cursor = self.cursor;
            return Some(true);
        }
        if sym == prev_key {
            self.move_cursor(true);
            return Some(true);
        }
        if sym == next_key {
            self.move_cursor(false);
            return Some(true);
        }
        // 设置里能选的翻页键（- =、[ ]、, .、Shift+Tab Tab）
        let keys = self.config.page_keys;
        let shift = key.modifiers & modifier::SHIFT != 0;
        let tab = keys & page_keys::TAB != 0;
        let char_prev = (keys & page_keys::MINUS != 0 && sym == keysym::MINUS)
            || (keys & page_keys::BRACKET != 0 && sym == keysym::BRACKET_LEFT)
            || (keys & page_keys::COMMA != 0 && sym == keysym::COMMA)
            || (tab && (sym == keysym::ISO_LEFT_TAB || (sym == keysym::TAB && shift)));
        let char_next = (keys & page_keys::MINUS != 0 && sym == keysym::EQUAL)
            || (keys & page_keys::BRACKET != 0 && sym == keysym::BRACKET_RIGHT)
            || (keys & page_keys::COMMA != 0 && sym == keysym::PERIOD)
            || (tab && sym == keysym::TAB && !shift);
        let (has_prev, has_next) = (self.has_prev_page(), self.has_next_page(n));
        if (sym == keysym::PAGE_UP || sym == prev_page_key || char_prev) && has_prev {
            self.prev_page();
            return Some(true);
        }
        if (sym == keysym::PAGE_DOWN || sym == next_page_key || char_next) && has_next {
            self.next_page();
            return Some(true);
        }
        if (char_prev || char_next) && (has_prev || has_next) {
            // 已经在第一页 / 最后一页：吃掉，不当标点上屏（只有一页时照常当标点）
            return Some(true);
        }
        None
    }

    /// 展开面板里的键。Some：处理完了；None：收起了面板，接着照常处理这个键。
    pub(super) fn grid_key(&mut self, key: &Key, n: usize) -> Option<bool> {
        let mut cursor = self.grid_cursor.min(n - 1);
        match key.keysym {
            keysym::LEFT => cursor = cursor.saturating_sub(1),
            keysym::RIGHT => {
                if cursor + 1 < n {
                    cursor += 1;
                }
            }
            keysym::UP => {
                if cursor < GRID_COLS {
                    // 第一行再往上：收起
                    self.collapse();
                    return Some(true);
                }
                cursor -= GRID_COLS;
            }
            keysym::DOWN => {
                if cursor + GRID_COLS < n {
                    cursor += GRID_COLS;
                } else if cursor / GRID_COLS + 1 < n.div_ceil(GRID_COLS) {
                    cursor = n - 1; // 最后一行不满时落到最后一个
                }
            }
            keysym::PAGE_DOWN | keysym::EQUAL => cursor = (cursor + GRID_SCREEN).min(n - 1),
            keysym::PAGE_UP | keysym::MINUS => {
                cursor = if cursor >= GRID_SCREEN {
                    cursor - GRID_SCREEN
                } else {
                    cursor % GRID_COLS
                };
            }
            keysym::ESCAPE => {
                self.collapse();
                return Some(true);
            }
            keysym::SPACE => {
                self.choose_now(cursor);
                return Some(true);
            }
            _ => {
                if let Some(digit) = key.digit() {
                    // 数字选光标所在行的第几个
                    if (1..=GRID_COLS).contains(&digit) {
                        let index = cursor / GRID_COLS * GRID_COLS + digit - 1;
                        if index < n {
                            self.choose_now(index);
                        }
                    }
                    return Some(true);
                }
                // 打字、退格、回车、标点：先收起，再照常处理
                self.collapse();
                return None;
            }
        }
        self.grid_cursor = cursor;
        Some(true)
    }

    /// 以词定字的键：`[` 是 Some(false)（第一个字），`]` 是 Some(true)（最后一个字）。
    pub(super) fn pick_char_key(&self, sym: u32) -> Option<bool> {
        if !self.config.pick_char || self.config.page_keys & page_keys::BRACKET != 0 {
            return None;
        }
        match sym {
            keysym::BRACKET_LEFT => Some(false),
            keysym::BRACKET_RIGHT => Some(true),
            _ => None,
        }
    }

    /// 以词定字：光标处的候选只取第一个（last 时最后一个）字。没用完输入的候选（句首词）
    /// 取的字进已选栈，剩下的拼音接着选；用完输入的连同已选的词上屏。取出来的字不记选词
    /// （学的是词，不是这个字）。光标处不是引擎的多字词时返回 false。
    pub(super) fn pick_char(&mut self, last: bool) -> bool {
        let Some(index) = self.current_index() else {
            return false;
        };
        let chosen = self.candidates[index].clone();
        let chars: Vec<char> = chosen.text.chars().collect();
        if chosen.kind != CandidateKind::Word || chars.len() < 2 {
            return false;
        }
        self.trace_character_selection(index, &chosen, last);
        let pick = if last {
            chars[chars.len() - 1]
        } else {
            chars[0]
        };
        // 开着繁体时也取词库写法里的同一个字（字数对不上就不知道是哪个，留空）
        let plain: Vec<char> = self.plain_of(&chosen.text).chars().collect();
        let plain = if plain.len() == chars.len() {
            (if last {
                plain[plain.len() - 1]
            } else {
                plain[0]
            })
            .to_string()
        } else {
            String::new()
        };
        // 拼音按音节对上字数时带上这个字的读音（以后拼成词能学），对不上就不带
        let syllables: Vec<&str> = chosen.pinyin.split_whitespace().collect();
        let pinyin = if syllables.len() == chars.len() {
            (if last {
                syllables[syllables.len() - 1]
            } else {
                syllables[0]
            })
            .to_string()
        } else {
            String::new()
        };
        self.last_choice = None;
        if chosen.consumed < self.remaining().len() {
            self.push_selection(Selection {
                text: pick.to_string(),
                plain,
                pinyin,
                bytes: chosen.consumed,
            });
        } else {
            let text = self.selected_text() + &pick.to_string();
            self.finish(text);
        }
        true
    }

    /// 分号、引号选第几个；不是选词键是 None。
    pub(super) fn select_key_index(&self, sym: u32) -> Option<usize> {
        match (self.config.select_keys, sym) {
            (SelectKeys::Off, _) => None,
            // 微软、搜狗、紫光双拼里分号是 ing 键，不能拿来选词
            (_, keysym::SEMICOLON) => (!self.shuangpin_semicolon()).then_some(2),
            (SelectKeys::SemicolonQuote, keysym::APOSTROPHE) => Some(3),
            _ => None,
        }
    }

    pub(super) fn shuangpin(&self) -> bool {
        self.engine.is_some_and(|e| e.config().shuangpin)
    }

    /// 双拼方案拿分号当韵母键。
    pub(super) fn shuangpin_semicolon(&self) -> bool {
        self.engine.is_some_and(|e| {
            let config = e.config();
            config.shuangpin && config.shuangpin_scheme.uses_semicolon()
        })
    }

    /// 双拼等第二个键（还没选词的部分按了奇数个键）时，分号是韵母键。
    pub(super) fn semicolon_is_key(&self) -> bool {
        self.shuangpin_semicolon()
            && self.remaining().bytes().filter(|&c| c != b'\'').count() % 2 == 1
    }

    /// 左右键移动拼音里的光标（[`ArrowKeys::Caret`]），Home、End、Delete 也管光标；到了两头照样吃掉。
    /// Some：处理完了；None：不是这几个键。
    pub(super) fn caret_key(&mut self, sym: u32) -> Option<bool> {
        let sentence_focus = self.sentence.view.cursor.is_some();
        if !matches!(
            sym,
            keysym::LEFT
                | keysym::RIGHT
                | keysym::HOME
                | keysym::END
                | keysym::DELETE
                | keysym::KP_DELETE
        ) {
            return None;
        }
        let start = self.rest_start();
        let at = self.caret();
        let len = self.input.len();
        let prev = self.input[..at]
            .chars()
            .next_back()
            .map_or(0, char::len_utf8);
        let next = self.input[at..].chars().next().map_or(0, char::len_utf8);
        match sym {
            keysym::LEFT if at > start => self.caret = Some(at - prev),
            keysym::RIGHT if at < len => self.caret = (at + next < len).then_some(at + next),
            keysym::HOME => self.caret = Some(start),
            keysym::END => self.caret = None,
            keysym::DELETE | keysym::KP_DELETE if at < len => {
                self.sentence.edit(at, next, 0);
                self.input.replace_range(at..at + next, "");
                self.caret = (at < self.input.len()).then_some(at);
                self.fresh = false;
                if self.input.is_empty() {
                    self.clear();
                }
            }
            _ => {} // 到了两头
        }
        if sentence_focus && !matches!(sym, keysym::DELETE | keysym::KP_DELETE) {
            if let Some(engine) = self.engine {
                let action = self.caret.map_or(crate::sentence::Action::Leave, |at| {
                    crate::sentence::Action::Focus { at }
                });
                self.sentence.action(action, engine);
            }
        }
        Some(true)
    }

    /// 光标在拼音里的位置换算到显示的样子（ni hao 这种分了音节的）里：显示的字是原始输入按顺序
    /// 加上分隔符时才换得出来，换不出来（双拼显示成全拼）返回 None。
    pub(super) fn shown_caret(raw: &str, shown: &str, caret: usize) -> Option<usize> {
        if caret >= raw.len() {
            return Some(shown.len());
        }
        let mut consumed = 0;
        for (i, ch) in shown.char_indices() {
            if consumed == caret {
                return Some(i);
            }
            if raw[consumed..].starts_with(ch) {
                consumed += ch.len_utf8();
            }
        }
        None
    }
}
