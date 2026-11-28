//! 选词、上屏与上屏后的学习。
use super::*;

impl<'e> Session<'e> {
    /// 选第 index 个候选（全部候选里的序号，不分页）。
    ///
    /// - emoji：上屏已选的词 + emoji（推荐在句尾对上时是整句 + emoji），不学习；
    /// - 计算结果、日期时间这类用完输入的：已选的词 + 它原样上屏，不学习；
    /// - 句首词（没用完输入）：进已选栈，记一次选词，剩下的接着选；
    /// - 其余：已选的词 + 它上屏，记选词（分几次选出来的整句也记一次，下次能一次打出来）。
    ///
    /// 序号不对时什么也不做（handled = false）。过渡：外壳的鼠标点候选、展开面板点一行用。
    pub fn choose(&mut self, index: usize) -> Frame {
        self.trace_via("click");
        self.last_choice = None;
        let handled = self.choose_now(index);
        self.frame(handled)
    }

    /// 选候选框里显示的第 index 个（一页，或展开面板的一屏），见 [`Session::choose`]。
    pub fn select(&mut self, index: usize) -> Frame {
        self.trace_via("click");
        self.last_choice = None;
        let shown = self.frame(false);
        self.pending = shown.commit;
        self.forgot = shown.forgot;
        let handled = index < shown.cand_count && self.choose_now(shown.cand_start + index);
        self.frame(handled)
    }

    pub(super) fn choose_now(&mut self, index: usize) -> bool {
        let Some(chosen) = self.candidates().get(index).cloned() else {
            return false;
        };
        self.trace_step(Some(index), Some(&chosen));
        let remaining = self.remaining().len();
        match chosen.kind {
            CandidateKind::Emoji => {
                let text = self.selected_text() + &chosen.commit;
                self.finish(text);
                self.last_choice = Some(chosen);
            }
            CandidateKind::Plain if chosen.consumed >= remaining => {
                let text = self.selected_text() + &chosen.text;
                self.finish(text);
                self.last_choice = Some(chosen);
            }
            _ if chosen.consumed < remaining => {
                // 句首词：先放进已选栈，剩下的拼音继续选
                let mark = self.engine.map(Engine::learn_mark);
                self.learn(&chosen.text, &chosen.pinyin, chosen.consumed);
                let changed = mark
                    .zip(self.engine.map(Engine::learn_mark))
                    .filter(|(start, end)| start < end);
                self.push_selection(Selection {
                    plain: self.plain_of(&chosen.text),
                    text: chosen.text,
                    pinyin: chosen.pinyin,
                    bytes: chosen.consumed,
                });
                *self.selection_learning.last_mut().unwrap() = changed;
                self.learning_changed |= changed.is_some();
            }
            _ => {
                self.commit_with(&chosen.text, &chosen.pinyin);
                self.last_choice = Some(chosen);
            }
        }
        if self.pending.is_some() {
            self.pending_rank = index + 1;
        }
        true
    }

    /// 上屏已选的词 + 还没选的原始输入，原始输入也记一次选词（和回车、`reset(true)` 一样）。
    pub fn commit_input(&mut self) -> Frame {
        self.trace_via("raw");
        self.last_choice = None;
        self.commit_input_now();
        self.frame(true)
    }

    pub(super) fn commit_input_now(&mut self) {
        self.trace_step(None, None);
        let tail = self.remaining().to_owned();
        self.commit_with(&tail, "");
    }

    /// 已选的词 + tail 上屏，记选词，清空。
    pub(super) fn commit_with(&mut self, tail: &str, tail_pinyin: &str) {
        let text = self.selected_text() + tail;
        let mark = self.engine.map(Engine::learn_mark);
        if self.sentence.edited
            && tail == self.sentence.view.text
            && !self.no_learn
            && !self.defer_learning()
        {
            if let Some(engine) = self.engine {
                if self.sentence.boundaries_changed {
                    for pair in self
                        .sentence
                        .view
                        .blocks
                        .windows(2)
                        .filter(|p| self.sentence.boundary_marks.contains(&p[0].to))
                    {
                        let parts = pair
                            .iter()
                            .map(|b| (self.plain_of(&b.text), b.pinyin.clone()))
                            .collect::<Vec<_>>();
                        engine.remember_boundary(
                            &format!("{} {}", pair[0].pinyin, pair[1].pinyin),
                            parts,
                        );
                    }
                }
                for (word, reading) in self.sentence.merged.clone() {
                    engine.record_merged_word(&self.plain_of(&word), &reading);
                }
            }
        }
        if self.engine.is_some() && !self.no_learn && !text.is_empty() {
            if !tail.is_empty() && !self.sentence.merged.iter().any(|(word, _)| word == tail) {
                // 带上拼音：选够次数后引擎会自动把这个词收进用户词库
                self.learn(tail, tail_pinyin, self.remaining().len());
            }
            // 以词定字取出来的字对不上读音时拼音是空的，这样拼成的不学
            if !self.selections.is_empty()
                && !tail.is_empty()
                && !tail_pinyin.is_empty()
                && self.selections.iter().all(|s| !s.pinyin.is_empty())
            {
                // 短词需再次确认；更长的整句仍按普通次数学习。
                let mut pinyin = String::new();
                for selection in &self.selections {
                    pinyin.push_str(&selection.pinyin);
                    pinyin.push(' ');
                }
                pinyin.push_str(tail_pinyin);
                // 学词库里的写法（开着繁体时上屏的是转换过的字）
                let mut plain: String = self
                    .selections
                    .iter()
                    .map(|s| {
                        if s.plain.is_empty() {
                            s.text.as_str()
                        } else {
                            s.plain.as_str()
                        }
                    })
                    .collect();
                plain += &self.plain_of(tail);
                self.learn_composed(&plain, &pinyin);
            }
        }
        self.finish(text);
        if let Some((start, end)) = mark
            .zip(self.engine.map(Engine::learn_mark))
            .filter(|(a, b)| a < b)
        {
            self.committed_learning.push((start, end));
            self.learning_changed = true;
        }
    }

    /// 候选在词库里的写法（开着繁体时候选是转换过的），见 [`Engine::lexicon_form`]。
    pub(super) fn plain_of(&self, shown: &str) -> String {
        self.engine
            .map_or_else(|| shown.to_string(), |engine| engine.lexicon_form(shown))
    }

    /// 记一次选词。上文和算候选时一样（已上屏的字 + 已选的词）。
    pub(super) fn learn(&mut self, word: &str, pinyin: &str, input_bytes: usize) {
        let Some(engine) = self.engine else {
            return;
        };
        if self.no_learn || self.defer_learning() {
            return;
        }
        let context = self.context.clone() + &self.selected_text();
        engine.set_context(utf8_tail(&context, CONTEXT_BYTES));
        // 英文和混输使用真正按下的输入码，双拼、简拼时也不能只保留中文读音。
        let completed = self
            .candidates
            .iter()
            .any(|c| c.long_completion && c.text == word && c.pinyin == pinyin);
        let reading = if word.chars().any(|c| c.is_ascii_alphabetic()) && !completed {
            &self.remaining()[..input_bytes.min(self.remaining().len())]
        } else {
            pinyin
        };
        engine.record_selection_with_pinyin(word, Some(reading).filter(|p| !p.is_empty()));
    }

    /// 记分几次选出来拼成的词，见 [`Engine::record_composed_word`]。上文是还没选词时的上文。
    pub(super) fn learn_composed(&mut self, word: &str, pinyin: &str) {
        let Some(engine) = self.engine else {
            return;
        };
        if self.no_learn || self.defer_learning() {
            return;
        }
        engine.set_context(utf8_tail(&self.context, CONTEXT_BYTES));
        let reading = if word.chars().any(|c| c.is_ascii_alphabetic()) {
            &self.input
        } else {
            pinyin
        };
        engine.record_composed_word(word, reading);
    }

    /// 上屏 text（空的不上屏，frame 取走），清空编辑区。
    pub(super) fn finish(&mut self, text: String) {
        self.trace_commit(&text);
        let marks = std::mem::take(&mut self.selection_learning);
        self.clear();
        self.committed_learning = marks.into_iter().flatten().collect();
        self.pending = (!text.is_empty()).then_some(text);
    }

    /// 选了句首词，压进已选栈（测试里不带引擎时用）。
    pub fn push_selection(&mut self, selection: Selection) {
        self.selections.push(selection);
        self.selection_learning.push(None);
        self.caret = None;
        self.fresh = false;
    }

    /// 过渡：外壳往原始输入后面加字（计算器的算式、双拼的分号、表情面板的搜索词）。
    pub fn append(&mut self, text: &str) {
        self.input.push_str(text);
        self.fresh = false;
    }
}
