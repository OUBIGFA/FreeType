//! 引擎的选词学习、学习记录存取和个人词库。
use super::*;

impl Engine {
    /// 记录一次选词，用于历史特征。时间戳取当前时间。
    pub fn record_selection(&self, word: &str) {
        self.record_selection_at(word, now_ms());
    }

    /// 记录选词；当同一个词选够次数后自动加入用户词库。
    ///
    /// pinyin 是这次上屏时输入的拼音（用于生成用户词条）。词库里已有的词也会被
    /// 加成用户词——这等价于「记住用户的偏好」，不会覆盖原词条。
    pub fn record_selection_with_pinyin(&self, word: &str, pinyin: Option<&str>) {
        let word = &self.lexicon_form(word);
        let at_ms = now_ms();
        self.record_selection_at(word, at_ms);
        if !self.config.auto_learn || word.is_empty() {
            return;
        }
        let Some(pinyin) = pinyin.filter(|p| !p.trim().is_empty()) else {
            return;
        };
        let threshold = f64::from(self.config.auto_learn_threshold.max(1));
        let heat = self.history_heat(word, at_ms);
        // 给同一轮连续选择保留千分之一的序列化精度；不把半次热度进位。
        if self.confirm_pending(word, pinyin, at_ms, false) || heat + 0.001 >= threshold {
            self.add_learned_word(word, pinyin, heat.max(threshold));
        }
    }

    /// 学到的词收进用户词库。热度决定频次，不降低已显式设定的词频。
    pub(super) fn add_learned_word(&self, word: &str, pinyin: &str, heat: f64) {
        // 候选带的拼音已经是全拼（双拼也翻好了）；以前的前端会传双拼键位串（如 "vsgo"），
        // 那样的要先展开，否则用户词会带着一串无效音节存进去，永远也打不出来。
        let pinyin = if self.config.shuangpin
            && !word.chars().any(|c| c.is_ascii_alphabetic())
            && !pinyin.trim().contains(' ')
            && crate::lexicon::parse_pinyin_keys(pinyin.trim()).is_err()
        {
            crate::shuangpin::to_full_pinyin(self.config.shuangpin_scheme, pinyin.trim())
        } else {
            pinyin.to_string()
        };
        if pinyin.trim().is_empty() {
            return;
        }
        // 频次随选用次数增长但有上限，避免用户词压过通用高频词
        let freq = (1000.0 * heat).min(50_000.0);
        let before = self.lexicon.user_word(word);
        if before
            .as_ref()
            .is_some_and(|(_, existing)| *existing >= freq)
        {
            return;
        }
        if self.lexicon.add(&pinyin, word, freq).is_ok() {
            if let Some(after) = self.lexicon.user_word(word) {
                self.note_learning(learn::Change::UserWord(word.into(), before, after));
            }
        }
    }

    /// 第一次分段拼成短词只候选收录，30 天内用同一码再拼成或直接选中才正式收录。
    pub fn record_composed_word(&self, word: &str, pinyin: &str) {
        let word = &self.lexicon_form(word);
        let chars = word.chars().count();
        let mixed_short_word = word.chars().any(|c| c.is_ascii_alphabetic())
            && chars <= MIXED_WORD_MAX_CHARS
            && word
                .chars()
                .filter(|c| crate::autospace::is_han(*c))
                .count()
                <= COMPOSED_WORD_MAX_CHARS;
        if chars > COMPOSED_WORD_MAX_CHARS && !mixed_short_word {
            self.record_selection_with_pinyin(word, Some(pinyin));
            return;
        }
        let at_ms = now_ms();
        self.record_selection_at(word, at_ms);
        if !self.config.auto_learn || word.is_empty() || pinyin.trim().is_empty() {
            return;
        }
        if self.confirm_pending(word, pinyin, at_ms, true) || self.lexicon.user_word(word).is_some()
        {
            let heat = self
                .history_heat(word, at_ms)
                .max(f64::from(self.config.auto_learn_threshold.max(1)));
            self.add_learned_word(word, pinyin, heat);
        }
    }

    /// An explicit block merge always requires a second confirmation, including
    /// longer names; ordinary automatic sentence learning keeps its own policy.
    pub fn record_merged_word(&self, word: &str, pinyin: &str) {
        if !self.config.auto_learn || word.is_empty() || word.chars().count() > 32 {
            return;
        }
        let word = self.lexicon_form(word);
        let at = now_ms();
        self.record_selection_at(&word, at);
        if self.confirm_pending(&word, pinyin, at, true) || self.lexicon.user_word(&word).is_some()
        {
            self.add_learned_word(
                &word,
                pinyin,
                f64::from(self.config.auto_learn_threshold.max(1)),
            );
        }
    }

    pub(super) fn confirm_pending(
        &self,
        word: &str,
        code: &str,
        at_ms: u64,
        compose: bool,
    ) -> bool {
        let code = code.trim().to_ascii_lowercase();
        let Ok(mut words) = self.pending_words.write() else {
            return false;
        };
        let before = words.get(word).cloned();
        let confirmed = before.as_ref().is_some_and(|record| {
            record.pinyin == code && at_ms.saturating_sub(record.first_ms) <= learn::NEW_WORD_TTL_MS
        });
        if confirmed {
            words.remove(word);
            self.note_learning(learn::Change::Pending(word.into(), before, None));
        } else if compose {
            if words.len() >= learn::MAX_HISTORY_WORDS {
                words.retain(|_, record| {
                    at_ms.saturating_sub(record.first_ms) <= learn::NEW_WORD_TTL_MS
                });
            }
            if words.len() < learn::MAX_HISTORY_WORDS {
                let after = learn::NewWord {
                    pinyin: code,
                    first_ms: at_ms,
                };
                words.insert(word.into(), after.clone());
                self.note_learning(learn::Change::Pending(word.into(), before, Some(after)));
            }
        }
        confirmed
    }

    /// 带时间戳的选词记录（方便测试）。
    ///
    /// 记两样：这个词的热度，和它跟前后词的搭配（见 [`learn`]）。
    pub fn record_selection_at(&self, word: &str, at_ms: u64) {
        if word.is_empty() {
            return;
        }
        let word = &self.lexicon_form(word);
        if let Ok(mut history) = self.history.write() {
            let before = history.get(word).copied();
            let after = learn::bumped(before, at_ms);
            history.insert(word.to_owned(), after);
            self.note_learning(learn::Change::Word(word.to_owned(), before, after));
        }
        // 这个候选是哪几个词拼的（整句「我发给你」= 我 | 发给 | 你）
        let words: Vec<String> = self
            .last_words
            .read()
            .ok()
            .and_then(|m| m.get(word).cloned())
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| vec![word.to_string()]);
        let start = self.start_word();
        let mut need_prune = false;
        if let Ok(mut pairs) = self.pairs.write() {
            let mut bump = |prev, next| {
                if let Some(change) = pairs.bump(prev, next, at_ms) {
                    self.note_learning(change);
                }
            };
            if let Some(prev) = &start {
                bump(prev, &words[0]);
            }
            for pair in words.windows(2) {
                bump(&pair[0], &pair[1]);
            }
            need_prune = pairs.len() > crate::learn::MAX_USER_PAIRS;
        }
        if !need_prune {
            need_prune = self
                .history
                .read()
                .map(|h| h.len() > crate::learn::MAX_HISTORY_WORDS)
                .unwrap_or(false);
        }
        if need_prune {
            if let (Ok(mut history), Ok(mut pairs)) = (self.history.write(), self.pairs.write()) {
                crate::learn::prune_learning(&mut history, &mut pairs);
            }
        }
        if let Some(tail) = words.last() {
            self.record_last_word(tail);
        }
    }

    /// 光标前的最后一个词：上一次选的词，而且光标前的文本正好以它结尾
    /// （中间换了输入框、删了字，就不算）。
    pub(super) fn start_word(&self) -> Option<String> {
        let context = self.context.read().ok()?.clone();
        self.start_word_in(&context)
    }

    /// 同 [`Engine::start_word`]，光标前的文本由调用方给（上屏联想用上屏之后的文本）。
    pub(super) fn start_word_in(&self, context: &str) -> Option<String> {
        let last = self.last_word.read().ok()?.clone();
        if last.is_empty() {
            return None;
        }
        // 记的是词库里的写法，光标前是上屏的（开着繁体就是转换过的）字
        let shown = self.shown_text(&last);
        let matches = context.trim_end().ends_with(&shown)
            || crate::autospace::without_boundary_spaces(context.trim_end())
                .ends_with(&crate::autospace::without_boundary_spaces(&shown));
        matches.then_some(last)
    }

    /// 学到的搭配数。
    pub fn pair_count(&self) -> usize {
        self.pairs.read().map(|p| p.len()).unwrap_or(0)
    }

    /// 把用户输入习惯（词的热度、词搭配）存到 TSV。
    pub fn save_history<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let history = self
            .history
            .read()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        let pairs = self
            .pairs
            .read()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?;
        let pending = self
            .pending_words
            .read()
            .map_err(|_| anyhow::anyhow!("pending words lock poisoned"))?;
        learn::write_atomic(
            path.as_ref(),
            &(learn::history_text(&history, &pairs, &pending)
                + &learn::boundary_text(&self.sentence_boundaries())),
        )?;
        Ok(())
    }

    /// 读回 [`Engine::save_history`] 存的输入习惯，返回条数。
    pub fn load_history<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        // Validate the added section before mutating any learning state.
        let text = std::fs::read_to_string(path.as_ref())?;
        let mut boundaries = sentence::Boundaries::new();
        for line in text.lines().filter_map(|s| s.strip_prefix("b\t")) {
            boundaries = serde_json::from_str(line)?;
            anyhow::ensure!(
                sentence::valid_boundaries(&boundaries),
                "切分记录格式不正确"
            );
        }
        let mut history = self
            .history
            .write()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        let mut pairs = self
            .pairs
            .write()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?;
        let mut pending = self
            .pending_words
            .write()
            .map_err(|_| anyhow::anyhow!("pending words lock poisoned"))?;
        let n = learn::load_history(path, &mut history, &mut pairs, &mut pending)?;
        let count = boundaries.len();
        *self.boundaries.write().unwrap() = boundaries;
        Ok(n + count)
    }

    pub(super) fn history_heat(&self, word: &str, at_ms: u64) -> f64 {
        self.history
            .read()
            .ok()
            .and_then(|h| h.get(word).copied())
            .map_or(0.0, |e| learn::decayed(e, at_ms))
    }

    pub fn history_count(&self, word: &str) -> u32 {
        self.history
            .read()
            .ok()
            .and_then(|h| h.get(word).map(|e| e.count.round() as u32))
            .unwrap_or(0)
    }

    /// 添加用户词（拼音可以是 ni hao / nihao / xi'an）。
    pub fn add_user_word(&self, pinyin: &str, word: &str, freq: f64) -> Result<()> {
        self.lexicon.add(pinyin, word, freq)
    }

    /// 删除用户词，返回是否删掉了。
    pub fn remove_user_word(&self, word: &str) -> bool {
        self.lexicon.remove(word)
    }

    /// 忘掉学过的一个词（候选框里 Ctrl+Delete）：用户词库里的这个词、它的选词热度、
    /// 和它有关的搭配都去掉。词库里本来就有的词删不掉，去掉学到的加分后回到原来的位置。
    /// 设置里加的自定义词不动。返回是否真的忘掉了什么。
    pub fn forget_word(&self, word: &str) -> bool {
        let word = self.lexicon_form(word);
        let word = word.as_str();
        if word.is_empty() || self.lexicon.is_custom(word) {
            return false;
        }
        let mut boundaries = self.boundaries.write().unwrap();
        let before = boundaries.len();
        boundaries.retain(|_, (parts, _)| !parts.iter().any(|(w, _)| w == word));
        let mut forgot = self.lexicon.remove(word) || before != boundaries.len();
        drop(boundaries);
        if let Ok(mut history) = self.history.write() {
            forgot |= history.remove(word).is_some();
        }
        if let Ok(mut pairs) = self.pairs.write() {
            forgot |= pairs.remove_word(word) > 0;
        }
        if let Ok(mut pending) = self.pending_words.write() {
            forgot |= pending.remove(word).is_some();
        }
        self.clear_learning_journal();
        // 下一次上屏别再拿刚忘掉的词接搭配
        if let Ok(mut last) = self.last_word.write() {
            if *last == word {
                last.clear();
            }
        }
        forgot
    }

    /// 忘掉学到的全部东西：用户词库、选词热度和搭配（自定义词不动）。
    pub fn clear_learned(&self) {
        self.boundaries.write().unwrap().clear();
        self.lexicon.clear();
        if let Ok(mut pending) = self.pending_words.write() {
            pending.clear();
        }
        self.clear_learning_journal();
        if let Ok(mut history) = self.history.write() {
            history.clear();
        }
        if let Ok(mut pairs) = self.pairs.write() {
            *pairs = UserPairs::default();
        }
        if let Ok(mut last) = self.last_word.write() {
            last.clear();
        }
    }

    pub fn user_words(&self) -> Vec<(String, String, f64)> {
        self.lexicon.words()
    }

    pub fn save_user_dict<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        self.lexicon.save_tsv(path)
    }

    pub fn load_user_dict<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        self.lexicon.load_tsv(path)
    }

    /// 读自定义词文件（custom.tsv），整体替换上一次读到的；文件不存在就清空。
    pub fn load_custom_words<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        self.lexicon.load_custom(path)
    }
}
