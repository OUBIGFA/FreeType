//! 计算当前输入的候选：日期时间、计算器、表情与键帽建议、分段显示。
use super::*;

impl<'e> Session<'e> {
    /// 还没选词那部分输入的候选（全部，不分页）。没有引擎、没在打字时为空。
    /// 重算时候选框回到第一页、光标在第一个、收起展开面板。
    pub fn candidates(&mut self) -> &[Candidate] {
        if !self.fresh {
            self.candidates = self.compute_candidates();
            let editable = self.selections.is_empty()
                && self.candidates.first().is_some_and(|c| {
                    c.kind == CandidateKind::Word
                        && c.consumed == self.input.len()
                        && !c.long_completion
                });
            if !editable {
                if !self.sentence.view.blocks.is_empty()
                    && self.input.starts_with(&self.sentence.view.raw)
                    && self.selections.is_empty()
                    && !self
                        .candidates
                        .first()
                        .is_some_and(|c| c.kind == CandidateKind::Plain || c.long_completion)
                {
                    let from = self
                        .sentence
                        .view
                        .pending_from
                        .unwrap_or(self.sentence.view.raw.len());
                    self.sentence.view.raw = self.input.clone();
                    self.sentence.view.pending_from = Some(from);
                } else {
                    self.sentence.view = crate::sentence::View::default();
                }
            }
            if self.selections.is_empty() {
                if let (Some(engine), Some(first)) = (self.engine, self.candidates.first()) {
                    if first.kind == CandidateKind::Word
                        && first.consumed == self.input.len()
                        && !first.long_completion
                    {
                        self.sentence.rebuild(
                            &self.input,
                            &first.text,
                            &first.pinyin,
                            &first.words,
                            engine,
                        );
                        if self.sentence.edited
                            && self.sentence.view.pending_from.is_none()
                            && !self.sentence.view.text.is_empty()
                        {
                            let first = &mut self.candidates[0];
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
                        }
                    }
                }
            }
            self.fresh = true;
            self.generation += 1;
            self.page = 0;
            self.cursor = 0;
            self.expanded = false;
            self.grid_cursor = 0;
            self.queue_enhancement();
        }
        &self.candidates
    }

    pub(super) fn compute_candidates(&self) -> Vec<Candidate> {
        let input = self.remaining();
        let Some(engine) = self.engine else {
            return Vec::new();
        };
        if input.is_empty() {
            return Vec::new();
        }
        if self.calc_active() && input.len() > 1 {
            // V 模式：v 后面是算式，候选是结果和「算式=结果」
            return crate::calc::candidates(&input[1..])
                .into_iter()
                .map(|text| Candidate::plain(text, input.len()))
                .collect();
        }

        // 已经选了开头的词时，把它当上文，剩下的部分排序更准
        let context = self.context.clone() + &self.selected_text();
        engine.set_context(utf8_tail(&context, CONTEXT_BYTES));
        let Ok(found) = engine.try_input(input) else {
            return Vec::new();
        };
        let mut all: Vec<Candidate> = found
            .into_iter()
            .map(|c| {
                let consumed = if c.consumed == 0 || c.consumed > input.len() {
                    input.len()
                } else {
                    c.consumed
                };
                // 自定义词里带换行的，不往用户词库里记（词库是一行一条）
                let kind = if c.text.contains(['\n', '\t']) {
                    CandidateKind::Plain
                } else {
                    CandidateKind::Word
                };
                let corrected = c.features.typo_count > 0.0;
                let pin_key = (kind == CandidateKind::Word
                    && !corrected
                    && !c.long_completion
                    && consumed == input.len()
                    && c.words.len() == 1
                    && c.words[0].known
                    && !c.pinyin.is_empty())
                .then(|| format!("{}\t{}", c.pinyin, c.words[0].text));
                Candidate {
                    words: if consumed == input.len() {
                        crate::sentence::words(&c.words)
                    } else {
                        Vec::new()
                    },
                    commit: c.text.clone(),
                    text: c.text,
                    pinyin: c.pinyin,
                    consumed,
                    kind,
                    corrected,
                    long_completion: c.long_completion,
                    pin_key,
                    ..Candidate::default()
                }
            })
            .collect();
        if self.config.emoji_suggest {
            add_emoji_suggestion(&mut all, input.len());
            add_keycap(&mut all, input.len(), self.config.page_size);
        }
        if self.config.date_time && self.selections.is_empty() {
            self.add_date_time(&mut all, input, engine.config().shuangpin);
        }
        engine.apply_pins(input, &mut all);
        all
    }

    /// 日期时间：首选还是那个词（日期、时间），各种写法从第二个开始。
    pub(super) fn add_date_time(&self, all: &mut Vec<Candidate>, input: &str, shuangpin: bool) {
        let Some(first) = all.first() else {
            return;
        };
        // 全拼看打的字母（riqi、rq、date 等）；双拼看首选的拼音
        let key = if shuangpin {
            if first.consumed < input.len() {
                return;
            }
            first.pinyin.clone()
        } else {
            input.to_owned()
        };
        let Some(now) = (self.clock)() else {
            return;
        };
        let extra: Vec<Candidate> = crate::datetime::candidates(&key, &now)
            .into_iter()
            .map(|text| Candidate::plain(text, input.len()))
            .collect();
        if extra.is_empty() {
            return;
        }
        // 写法是给「时间」这个词的：接着前文打时它可能不在第一（日期后面接 shijian，
        // 语料里「…日事件」多，事件排第一），先把它挪到第一，写法再跟在后面
        if let Some(word) = crate::datetime::trigger_word(&key) {
            if let Some(at) = all.iter().position(|c| c.text == word).filter(|&at| at > 0) {
                let trigger = all.remove(at);
                all.insert(0, trigger);
            }
        }
        all.splice(1..1, extra);
    }

    /// V 模式（计算器）：没选过词、v 开头、后面只有数字和运算符。
    /// 双拼时 v 是声母键（v1 是「选第一个候选」），改用大写 V（#160）。
    /// 外壳用同一判定避免邮箱、网址补全抢走算式的小数点。
    pub fn calc_active(&self) -> bool {
        if !self.config.calculator || !self.selections.is_empty() {
            return false;
        }
        let prefix = if self.shuangpin() { 'V' } else { 'v' };
        let Some(rest) = self.input.strip_prefix(prefix) else {
            return false;
        };
        rest.bytes()
            .all(|c| c.is_ascii_digit() || b"+-*/%^.()".contains(&c))
    }

    /// 还没选词那部分输入的显示：全拼分音节（nihao → ni hao），双拼显示成全拼
    /// （vsgo → zhong'guo），和微信输入法一样；设置了 `shuangpin_keys` 时双拼显示打的键。
    /// 分不出来就原样。连同光标在显示里的字节位置一起给。
    pub(super) fn segmented_remaining(&mut self) -> (String, usize) {
        let shown = self.segmented_remaining_shown();
        let input = self.remaining();
        let rest_caret = self.caret() - self.rest_start();
        match Self::shown_caret(input, &shown, rest_caret) {
            Some(at) => (shown, at),
            // 光标在拼音中间、显示的样子换不回光标位置（双拼显示成全拼）：显示打的键
            None => (input.to_owned(), rest_caret),
        }
    }

    pub(super) fn segmented_remaining_shown(&mut self) -> String {
        let input = self.remaining().to_owned();
        if self.calc_active() {
            // 算式原样显示（双拼时不能当成按键换成全拼）
            return input;
        }
        if let Some(engine) = self.engine {
            if engine.config().shuangpin {
                if self.config.shuangpin_keys {
                    return input;
                }
                return engine
                    .shuangpin_display(&input)
                    .filter(|shown| !shown.is_empty())
                    .unwrap_or(input);
            }
        }
        let full = self
            .candidates()
            .iter()
            .find(|c| c.consumed >= input.len())
            .map(|c| c.pinyin.clone());
        full.and_then(|pinyin| crate::preedit::segment_input(&input, &pinyin))
            .unwrap_or(input)
    }
}

/// 首选是整句时按它推荐一个 emoji，插在第 [`EMOJI_SLOT`] 个。
pub(super) fn add_emoji_suggestion(all: &mut Vec<Candidate>, input_len: usize) {
    let Some(first) = all.first() else {
        return;
    };
    if first.consumed < input_len {
        return;
    }
    let top = first.text.clone();
    let emoji: Vec<Candidate> = crate::emoji::recommend(&top, 1)
        .into_iter()
        .map(|s| Candidate {
            // 句尾对上时 commit 是「整句 + emoji」
            emoji: s
                .commit
                .strip_prefix(top.as_str())
                .unwrap_or(&s.commit)
                .to_owned(),
            text: s.display,
            pinyin: String::new(),
            consumed: input_len,
            kind: CandidateKind::Emoji,
            commit: s.commit,
            ..Candidate::default()
        })
        .collect();
    let at = EMOJI_SLOT.min(all.len());
    all.splice(at..at, emoji);
}

/// 第一页里有数字字就在它后面跟按键 emoji：打 ba 是「吧 八 8️⃣ 把」，和手机输入法一样。
pub(super) fn add_keycap(all: &mut Vec<Candidate>, input_len: usize, page_size: usize) {
    let window = all.len().min(page_size);
    for i in 0..window {
        let c = &all[i];
        if c.kind == CandidateKind::Emoji || c.consumed < input_len {
            continue;
        }
        let Some(keycap) = crate::emoji::keycap(&c.text) else {
            continue;
        };
        all.insert(
            i + 1,
            Candidate {
                text: keycap.to_owned(),
                pinyin: String::new(),
                consumed: input_len,
                kind: CandidateKind::Emoji,
                commit: keycap.to_owned(),
                emoji: keycap.to_owned(),
                ..Candidate::default()
            },
        );
        return;
    }
}
