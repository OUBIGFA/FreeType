//! 整句编辑区与可选增强模型的异步结果。
use super::*;

impl<'e> Session<'e> {
    pub fn enhancement_pending(&self) -> bool {
        // Pending means the current visible frame has not consumed the result;
        // a cache hit still requires one refresh. Pause while the user navigates.
        self.sentence.view.cursor.is_none()
            && self.cursor == 0
            && self.page == 0
            && !self.expanded
            && self.enhancement_key.is_some()
            && self
                .engine
                .and_then(|e| e.enhancement_backend())
                .is_some_and(|b| b.available())
    }

    /// Apply only a ready result for the still-current input; explicit user
    /// confirmation and navigation always take precedence over delayed output.
    pub fn refresh_enhancement(&mut self) -> bool {
        if self.sentence.view.cursor.is_some()
            || !self.selections.is_empty()
            || self.cursor != 0
            || self.page != 0
            || self.expanded
        {
            return false;
        }
        let Some((engine, key)) = self.engine.zip(self.enhancement_key.as_ref()) else {
            return false;
        };
        let Some(answer) = engine.enhancement_backend().and_then(|b| b.answer(key)) else {
            return false;
        };
        self.enhancement_key = None;
        let Some(first) = self.candidates.first() else {
            return false;
        };
        // 用户固定的首选优先于模型，不能在异步刷新时改掉明确排位。
        if first.pin_position > 0 {
            return false;
        }
        if first.text == answer.text && first.words == answer.words {
            return false;
        }
        let previous: Vec<_> = first.text.chars().collect();
        let text = engine.shown_text(&answer.text);
        // 模型只能提升已有建议或加入新建议；原首选仍必须可选。按消费长度区分
        // 完整候选与同文本句首候选，避免替换上屏语义；复用已有词的读音与固定排位资格。
        let existing = self.candidates.iter().position(|c| {
            c.text == text
                && c.consumed == self.input.len()
                && c.kind == CandidateKind::Word
                && !c.long_completion
        });
        let mut suggestion = existing
            .map(|i| self.candidates.remove(i))
            .unwrap_or_else(|| Candidate {
                text: text.clone(),
                commit: text,
                consumed: self.input.len(),
                kind: CandidateKind::Word,
                ..Candidate::default()
            });
        suggestion.pinyin = answer.pinyin;
        suggestion.words = answer
            .words
            .into_iter()
            .map(|(w, p)| (engine.shown_text(&w), p))
            .collect();
        self.candidates.insert(0, suggestion);
        engine.apply_pins(&self.input, &mut self.candidates);
        let first = &self.candidates[0];
        self.sentence.accept_model(
            &self.input,
            &first.text,
            &first.pinyin,
            &first.words,
            engine,
        );
        let mut at = 0;
        for b in &mut self.sentence.view.blocks {
            let chars: Vec<_> = b.text.chars().collect();
            let end = at + chars.len();
            b.corrected = previous.get(at..end) != Some(chars.as_slice());
            b.uncertain = answer
                .uncertain
                .get(at..end)
                .is_some_and(|v| v.iter().any(|x| *x));
            at = end;
        }
        self.generation += 1;
        true
    }

    /// Sentence metadata is independent of the optional enhancement model.
    pub fn sentence_view(&mut self) -> Option<&crate::sentence::View> {
        if !self.config.sentence_editor {
            return None;
        }
        self.candidates();
        (!self.sentence.view.blocks.is_empty() && self.selections.is_empty())
            .then_some(&self.sentence.view)
    }

    /// Keyboard and pointer commands share one state transition. Picking a block
    /// keeps composing; only the ordinary commit path writes the final sentence.
    pub fn sentence_action(&mut self, action: crate::sentence::Action) -> Frame {
        if !self.config.sentence_editor {
            return self.frame(false);
        }
        self.candidates();
        if let crate::sentence::Action::Caret { at } = action {
            self.caret = Some(at.min(self.input.len()));
        }
        if matches!(action, crate::sentence::Action::Leave) {
            self.caret = None;
        }
        let handled = self
            .engine
            .is_some_and(|engine| self.sentence.action(action, engine));
        if handled && !self.sentence.view.text.is_empty() {
            if let Some(first) = self.candidates.first_mut() {
                first.text = self.sentence.view.text.clone();
                first.commit = first.text.clone();
                first.pinyin = self
                    .sentence
                    .view
                    .blocks
                    .iter()
                    .map(|b| b.pinyin.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                first.words = self
                    .sentence
                    .view
                    .blocks
                    .iter()
                    .map(|b| (b.text.clone(), b.pinyin.clone()))
                    .collect();
                first.pin_key = None;
            }
        }
        if handled {
            self.queue_enhancement();
        }
        self.frame(handled)
    }

    pub(super) fn queue_enhancement(&mut self) {
        self.enhancement_key = None;
        if self.preview.is_some()
            || !self.selections.is_empty()
            || self.no_learn
            || self.sentence.view.pending_from.is_some()
            || self.sentence.view.blocks.is_empty()
        {
            return;
        }
        if let (Some(engine), Some(top)) = (self.engine, self.candidates.first()) {
            let Some(backend) = engine.enhancement_backend() else {
                return;
            };
            let choices = self
                .candidates
                .iter()
                .filter(|c| {
                    c.consumed == self.input.len()
                        && c.pinyin == top.pinyin
                        && c.kind == CandidateKind::Word
                        && !c.long_completion
                })
                .take(8)
                .map(|c| engine.lexicon_form(&c.text))
                .collect();
            if let Some(mut query) = engine.enhancement_query(&self.input, &top.pinyin, choices) {
                query.blocks = top
                    .words
                    .iter()
                    .map(|(word, _)| engine.lexicon_form(word))
                    .collect();
                query.context = self
                    .context
                    .chars()
                    .rev()
                    .take(30)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                query.key = format!("{}\u{1d}{}", query.context, query.key);
                if self.sentence.edited && !self.sentence.constrain(&mut query, engine) {
                    return;
                }
                self.enhancement_key = Some(query.key.clone());
                backend.submit(query);
            }
        }
    }
}
