//! 候选翻页、光标移动、当前项上屏与删除学习。
use super::*;

impl<'e> Session<'e> {
    /// 收起展开面板：光标是面板里的光标，翻到它所在的那一页。
    pub(super) fn collapse(&mut self) {
        self.expanded = false;
        self.cursor = self.grid_cursor;
        self.page = self.cursor / self.page_size();
    }

    pub(super) fn page_size(&self) -> usize {
        self.config.page_size.max(1)
    }

    /// 当前页在全部候选里的起点和个数。
    pub(super) fn page_range(&self, n: usize) -> (usize, usize) {
        let start = self.page * self.page_size();
        (start, n.saturating_sub(start).min(self.page_size()))
    }

    /// 光标在当前页的第几个；不在这一页上是 None。
    pub(super) fn cursor_on_page(&self) -> Option<usize> {
        let size = self.page_size();
        (self.cursor / size == self.page).then_some(self.cursor % size)
    }

    pub(super) fn has_prev_page(&self) -> bool {
        self.page > 0
    }

    pub(super) fn has_next_page(&self, n: usize) -> bool {
        self.page + 1 < n.div_ceil(self.page_size())
    }

    /// 上一页。光标原来在这一页上的话跟着到新的一页的第一个（和 fcitx5 一样）。
    pub(super) fn prev_page(&mut self) {
        if self.has_prev_page() {
            let on_page = self.cursor_on_page().is_some();
            self.page -= 1;
            if on_page {
                self.cursor = self.page * self.page_size();
            }
        }
    }

    pub(super) fn next_page(&mut self) {
        if self.has_next_page(self.candidates.len()) {
            let on_page = self.cursor_on_page().is_some();
            self.page += 1;
            if on_page {
                self.cursor = self.page * self.page_size();
            }
        }
    }

    /// 移光标：光标不在这一页上时落到这一页的头 / 尾；否则走一格，跨页时翻到光标所在的页。
    /// 到了全部候选的两头就停住，不绕到另一头：第一个往前一按就跳到末页的生僻字，
    /// 再顺手按空格就上屏了它（展开面板、工具栏到头也是停住）。
    pub(super) fn move_cursor(&mut self, prev: bool) {
        let n = self.candidates.len();
        if n == 0 {
            return;
        }
        let (start, len) = self.page_range(n);
        if len == 0 {
            return;
        }
        if self.cursor_on_page().is_none() {
            self.cursor = start + if prev { len - 1 } else { 0 };
            return;
        }
        self.cursor = if prev {
            self.cursor.saturating_sub(1)
        } else {
            (self.cursor + 1).min(n - 1)
        };
        self.page = self.cursor / self.page_size();
    }

    /// 空格：选光标处的候选（光标不在这一页上时选这一页第一个），没有候选就原样上屏。
    pub(super) fn commit_current(&mut self) -> bool {
        let n = self.candidates().len();
        if n == 0 {
            self.commit_input_now();
            return true;
        }
        let index = self.page * self.page_size() + self.cursor_on_page().unwrap_or(0);
        self.choose_now(index.min(n - 1));
        true
    }

    /// 光标处的候选（空格选的那个）：展开面板时是面板的光标。没有候选是 None。
    pub(super) fn current_index(&mut self) -> Option<usize> {
        let n = self.candidates().len();
        if n == 0 {
            return None;
        }
        let index = if self.expanded {
            self.grid_cursor
        } else {
            self.page * self.page_size() + self.cursor_on_page().unwrap_or(0)
        };
        Some(index.min(n - 1))
    }

    /// Ctrl+Delete：让引擎忘掉光标处的候选（用户词库里的词、选词热度、搭配），
    /// 候选原地重算，编辑区不动，候选框停在原来的页和位置。只管引擎的词（计算结果、
    /// emoji、日期时间这类没学过）；什么也没学过、是自定义词时什么也不做。
    pub(super) fn forget_current(&mut self, editor_shortcut: bool) {
        let Some(engine) = self.engine else {
            return;
        };
        if let Some(word) = self
            .sentence
            .view
            .blocks
            .iter()
            .find(|b| editor_shortcut && b.focused)
            .map(|b| b.text.clone())
        {
            if self.defer_learning() {
                return;
            }
            let changed = engine.forget_word(&word);
            self.sentence.forget(&word);
            self.fresh = false;
            self.forgot |= changed;
            return;
        }
        let Some(index) = self.current_index() else {
            return;
        };
        if self.candidates[index].kind != CandidateKind::Word || self.defer_learning() {
            return;
        }
        if !engine.forget_word(&self.candidates[index].text) {
            return;
        }
        self.forgot = true;
        let (page, cursor, expanded, grid_cursor) =
            (self.page, self.cursor, self.expanded, self.grid_cursor);
        self.fresh = false;
        let n = self.candidates().len();
        if n == 0 {
            return;
        }
        let last_page = (n - 1) / self.page_size();
        self.page = page.min(last_page);
        self.cursor = cursor.min(n - 1);
        self.expanded = expanded && n > 1;
        self.grid_cursor = grid_cursor.min(n - 1);
    }

    /// 忘掉候选框里显示的第 index 个（一页，或展开面板的一屏），和 Ctrl+Delete 一样，
    /// 只是不看光标（鼠标长按候选时用）。忘掉了返回那个词；没学过、不是引擎的词返回 None。
    pub fn forget(&mut self, index: usize) -> Option<String> {
        let shown = self.frame(false);
        self.pending = shown.commit;
        self.forgot = shown.forgot;
        if index >= shown.cand_count {
            return None;
        }
        let global = shown.cand_start + index;
        let word = self.candidates[global].text.clone();
        // 光标挪到这个候选上再按 Ctrl+Delete 的规则忘
        if self.expanded {
            self.grid_cursor = global;
        } else {
            self.cursor = global;
            self.page = global / self.page_size();
        }
        let carried = std::mem::take(&mut self.forgot);
        // 鼠标菜单明确指向普通候选，不借用独立编辑区的键盘焦点。
        self.forget_current(false);
        let done = self.forgot;
        self.forgot |= carried;
        done.then_some(word)
    }
}
