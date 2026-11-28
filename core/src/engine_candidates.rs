//! 候选列表的组装：自定义编码、双拼、全拼、前缀补全与原样输入。
use super::*;

impl Engine {
    /// 输入正好是某个自定义编码（mail、wdyx）时，把对应的词放在最前面。
    pub(super) fn prepend_custom_codes(&self, raw: &str, at_ms: u64, result: &mut Vec<Candidate>) {
        let words = self.lexicon.custom_code_words(raw.trim());
        let mut learned = self.lexicon.learned_code_words(raw.trim());
        if words.is_empty() && learned.is_empty() {
            return;
        }
        // 输入码没有音节路径，沿用已有热度衰减和词搭配给同码的个人词排序。
        let start = self.start_word();
        let preference = |word: &str| {
            let hot = self
                .history
                .read()
                .ok()
                .and_then(|history| history.get(word).copied())
                .map(|entry| crate::learn::decayed(entry, at_ms))
                .unwrap_or(0.0);
            let pair = start
                .as_ref()
                .and_then(|previous| {
                    self.pairs
                        .read()
                        .ok()
                        .map(|pairs| pairs.bonus(previous, word, at_ms))
                })
                .unwrap_or(0.0);
            (1.0 + hot).ln() + pair
        };
        learned.sort_by(|a, b| {
            preference(&b.1)
                .total_cmp(&preference(&a.1))
                .then_with(|| b.2.total_cmp(&a.2))
        });
        learned.retain(|(_, text, _)| !words.contains(text));
        result.retain(|c| {
            !words.contains(&c.text) && !learned.iter().any(|(_, text, _)| text == &c.text)
        });
        let configured = words.into_iter().map(|text| Candidate {
            long_completion: false,
            pinyin: String::new(),
            score: 0.0,
            features: Feature::default(),
            words: vec![CandidateWord {
                text: text.clone(),
                pinyin: String::new(),
                freq: crate::lexicon::CUSTOM_WORD_FREQ,
                known: true,
            }],
            consumed: raw.len(),
            text,
        });
        let learned = learned.into_iter().map(|(input, text, freq)| Candidate {
            long_completion: false,
            pinyin: input.clone(),
            score: preference(&text),
            features: Feature::default(),
            words: vec![CandidateWord {
                text: text.clone(),
                pinyin: input,
                freq,
                known: true,
            }],
            consumed: raw.len(),
            text,
        });
        let front = configured.chain(learned);
        result.splice(0..0, front);
    }

    /// 双拼：先把键翻成全拼（`vsgo` -> `zhong'guo`），再走全拼那一套，
    /// 最后把候选用掉的全拼字节数换回双拼键的字节数。
    pub(super) fn shuangpin_candidates(&self, raw: &str, at_ms: u64) -> Result<Vec<Candidate>> {
        let display = raw.trim();
        let leading = raw.len() - raw.trim_start().len();
        let scheme = self.config.shuangpin_scheme;
        let variants = crate::shuangpin::decode_fuzzy(
            scheme,
            display,
            if self.config.fuzzy {
                self.config.fuzzy_groups
            } else {
                0
            },
        );
        let Some(first) = variants.first() else {
            // 翻不了（不是双拼）：只给原始串
            let mut result = Vec::new();
            self.append_raw_candidate(display, &mut result);
            for c in &mut result {
                c.consumed = raw.len();
            }
            return Ok(result);
        };
        // 双拼配齐的音节不是全拼前缀；只给未配齐的最后一键补全。
        let completion = display
            .bytes()
            .filter(|b| scheme.is_key(b.to_ascii_lowercase()))
            .count()
            % 2
            == 1;
        let mut result = self.pinyin_candidates(&first.pinyin, Some(display), completion, at_ms)?;
        result.retain_mut(|c| {
            let used = if is_raw_candidate(c) {
                display.len()
            } else {
                first.raw_bytes(c.consumed, display.len())
            };
            c.consumed = if used >= display.len() {
                raw.len()
            } else {
                leading + used
            };
            used > 0
        });
        // 重码读法一起比较整句分数，不让第一种切法的长尾吃掉后续读法。
        let mut extra = Vec::new();
        for other in variants.iter().skip(1) {
            for mut c in self.query_plain(&other.pinyin.to_ascii_lowercase(), completion, at_ms)? {
                c.text = self.shown_text(&c.text);
                c.consumed = raw.len();
                extra.push(c);
            }
        }
        if !extra.is_empty() {
            result.extend(extra);
            result = dedupe_by_text(result);
            result.sort_by(|a, b| {
                b.score
                    .total_cmp(&a.score)
                    .then_with(|| a.text.cmp(&b.text))
            });
            self.promote_custom(&mut result);
            result.truncate(self.config.max_candidates.max(1));
        }
        Ok(result)
    }

    /// 全拼的完整流程。`raw_display` 给了的话（双拼），原始串候选用它而不是 raw。
    pub(super) fn pinyin_candidates(
        &self,
        raw: &str,
        raw_display: Option<&str>,
        completion: bool,
        at_ms: u64,
    ) -> Result<Vec<Candidate>> {
        let display = raw.trim();
        let normalized = display.to_ascii_lowercase();
        let mut result = self.query_plain(&normalized, completion, at_ms)?;

        protect_exact_input(&mut result);
        promote_picked(&mut result);
        pin_single_initial(&normalized, &mut result);
        self.promote_custom(&mut result);
        self.append_raw_candidate(raw_display.unwrap_or(display), &mut result);
        let english_only = drop_forced_chinese(&mut result);
        if self.config.traditional {
            self.apply_traditional(&mut result);
        }
        for candidate in &mut result {
            candidate.consumed = raw.len();
        }
        if english_only {
            // 打的是英文词（iphone、hello）：硬解出来的中文都去掉了，也不补句首词
            return Ok(result);
        }
        if self.config.prefix_candidates > 0 {
            let leading = raw.len() - raw.trim_start().len();
            let mut prefix = self.prefix_candidates(&normalized)?;
            let _ = prefix_on_syllable_boundaries(&result, &normalized, &mut prefix);
            for candidate in &mut prefix {
                candidate.consumed = leading + letters_to_bytes(display, candidate.consumed);
                if self.config.traditional {
                    if let Ok(conv) = self.converter.read() {
                        if !conv.is_empty() {
                            candidate.text = conv.convert(&candidate.text);
                        }
                    }
                }
            }
            if let Some(head) = sentence_head(&result, &normalized) {
                let mut head = head;
                head.text = self.shown_text(&head.text);
                head.consumed = leading + letters_to_bytes(display, head.consumed);
                prefix.insert(0, head);
            }
            compose_candidates(&mut result, prefix, self.config.max_candidates.max(1));
        }
        if self.config.more_candidates {
            self.append_more_candidates(raw, display, &normalized, completion, &mut result)?;
        }
        // 双拼每个音节两个键，没有 yuan / yu'an 这种切法歧义
        if raw_display.is_none() {
            promote_split_reading(&mut result, raw.len());
        }
        if self.config.fuzzy {
            self.keep_literal_words(&normalized, raw.len(), at_ms, &mut result)?;
        }
        Ok(result)
    }

    /// 模糊音扩大召回时仍给完整拼写的整词保留位置。只查 2–4 音节的索引，
    /// 不再跑束搜索；保留原首选，最多补回三个精确词，避免低频词首次输入就消失。
    pub(super) fn keep_literal_words(
        &self,
        raw: &str,
        consumed: usize,
        at_ms: u64,
        found: &mut Vec<Candidate>,
    ) -> Result<()> {
        let parsed = parse(raw)?;
        let graph = SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: false,
                typo: false,
                completion: false,
            },
        );
        let mut paths = Vec::new();
        exact_segmentations(&graph, 0, 4, &mut Vec::new(), &mut paths);
        let mut entries = Vec::new();
        for path in paths.iter().filter(|p| (2..=4).contains(&p.len())) {
            let query: Vec<_> = path.iter().map(|edge| edge.query()).collect();
            for entry in self.lexicon.lookup_common(&query) {
                if entries
                    .iter()
                    .any(|(e, _): &(crate::lexicon::LexEntry, String)| e.word == entry.word)
                {
                    continue;
                }
                entries.push((
                    entry,
                    query
                        .iter()
                        .map(|q| q.display())
                        .collect::<Vec<_>>()
                        .join(" "),
                ));
            }
        }
        entries.sort_by(|(a, _), (b, _)| {
            self.history_heat(&b.word, at_ms)
                .total_cmp(&self.history_heat(&a.word, at_ms))
                .then_with(|| b.freq.total_cmp(&a.freq))
        });
        let mut slot = 1;
        for (entry, pinyin) in entries.into_iter().take(3) {
            let text = self.shown_text(&entry.word);
            if let Some(at) = found
                .iter()
                .position(|c| c.text == text && c.consumed == consumed && !c.long_completion)
            {
                if at <= slot {
                    slot = slot.max(at + 1);
                    continue;
                }
                let candidate = found.remove(at);
                found.insert(slot.min(found.len()), candidate);
            } else {
                found.insert(
                    slot.min(found.len()),
                    Candidate {
                        text,
                        pinyin: pinyin.clone(),
                        consumed,
                        score: 0.0,
                        long_completion: false,
                        features: Feature {
                            coverage_score: 1.0,
                            ..Default::default()
                        },
                        words: vec![CandidateWord {
                            text: entry.word,
                            pinyin,
                            freq: entry.freq,
                            known: true,
                        }],
                    },
                );
            }
            slot += 1;
        }
        // 保持后续合并所需的单调分数；原样字母候选仍留在末尾可选。
        for i in 1..found.len() {
            if found[i].score >= found[i - 1].score {
                found[i].score = found[i - 1].score - 1e-3;
            }
        }
        while found.len() > self.config.max_candidates.max(1) {
            if let Some(i) = found.iter().rposition(|c| !is_raw_candidate(c)) {
                found.remove(i);
            } else {
                break;
            }
        }
        Ok(())
    }

    /// 句首词候选：从输入开头起、但没有用完整个输入的词。
    ///
    /// candidate.consumed 先存「用掉的字母数」，由调用方换算成字节。
    pub(super) fn prefix_candidates(&self, normalized: &str) -> Result<Vec<Candidate>> {
        let Ok(parsed) = parse(normalized) else {
            return Ok(Vec::new());
        };
        if parsed.len() < 2 {
            return Ok(Vec::new());
        }
        let typo = self.config.typo_correction && !self.config.shuangpin;
        let full = self.full_input_graph(&parsed, typo, false);
        let graph = if full.has_exact_path() || !self.config.jianpin_fallback {
            full
        } else {
            let completion_explains = SyllableGraph::build(
                &parsed,
                GraphOptions {
                    mode: GraphMode::Full,
                    allow_erhua: true,
                    typo: false,
                    completion: true,
                },
            )
            .has_path_without_erhua_or_typo();
            abbrev_graph(&parsed, completion_explains).unwrap_or(full)
        };
        let fuzzy_view;
        let lexicon: &dyn Lexicon = if self.config.fuzzy {
            fuzzy_view = FuzzyLexicon::new(
                self.lexicon.as_ref(),
                self.config.fuzzy_groups,
                self.config.fuzzy_penalty,
            );
            &fuzzy_view
        } else {
            self.lexicon.as_ref()
        };
        let memo = self.memo_view(lexicon, self.config.fuzzy);
        let lexicon: &dyn Lexicon = &memo;
        let words = crate::beam::first_words(
            &graph,
            lexicon,
            &self.config.beam,
            self.config.prefix_candidates,
        );
        Ok(words
            .into_iter()
            .map(|(word, cost)| Candidate {
                long_completion: false,
                text: word.text.clone(),
                pinyin: word.pinyin.clone(),
                score: -cost,
                features: Feature {
                    typo_count: word.typos as f32,
                    typo: if word.typos > 0 { 1.0 } else { 0.0 },
                    ..Feature::default()
                },
                consumed: word.end,
                words: vec![CandidateWord {
                    text: word.text,
                    pinyin: word.pinyin,
                    freq: word.freq,
                    known: true,
                }],
            })
            .collect())
    }

    /// 翻页用的长尾：前面几页之后，把输入能对上的词全补上，直到 `max_candidates`。
    ///
    /// 照着微信输入法：先是整个输入的同音词（按词频），再是句首词（长的在前），
    /// 最后是第一个音节的单字。前面已经有的不重复；排第一的结果没纠错时不补纠错出来的词。
    pub(super) fn append_more_candidates(
        &self,
        raw: &str,
        display: &str,
        normalized: &str,
        completion: bool,
        result: &mut Vec<Candidate>,
    ) -> Result<()> {
        let max = self.config.max_candidates.max(1);
        if result.len() >= max {
            return Ok(());
        }
        let Ok(parsed) = parse(normalized) else {
            return Ok(());
        };
        if parsed.is_empty() {
            return Ok(());
        }
        let typo = self.config.typo_correction && !self.config.shuangpin;
        let full = self.full_input_graph(&parsed, typo, completion);
        let graph = if full.has_exact_path() || !self.config.jianpin_fallback {
            full
        } else {
            let completion_explains = full.has_path_without_erhua_or_typo();
            abbrev_graph(&parsed, completion_explains).unwrap_or(full)
        };
        let fuzzy_view;
        let lexicon: &dyn Lexicon = if self.config.fuzzy {
            fuzzy_view = FuzzyLexicon::new(
                self.lexicon.as_ref(),
                self.config.fuzzy_groups,
                self.config.fuzzy_penalty,
            );
            &fuzzy_view
        } else {
            self.lexicon.as_ref()
        };
        let memo = self.memo_view(lexicon, self.config.fuzzy);
        let lexicon: &dyn Lexicon = &memo;
        let words = crate::beam::all_first_words(&graph, lexicon, &self.config.beam, max);
        if words.is_empty() {
            return Ok(());
        }
        let total = graph.len();
        let exact_top = result
            .iter()
            .find(|c| !is_raw_candidate(c))
            .map(|c| c.features.typo_count == 0.0)
            .unwrap_or(false);
        let to_candidate = |word: BeamWord, cost: f64| Candidate {
            long_completion: false,
            text: word.text.clone(),
            pinyin: word.pinyin.clone(),
            score: -cost,
            features: Feature {
                typo_count: word.typos as f32,
                typo: if word.typos > 0 { 1.0 } else { 0.0 },
                ..Feature::default()
            },
            consumed: word.end,
            words: vec![CandidateWord {
                text: word.text,
                pinyin: word.pinyin,
                freq: word.freq,
                known: true,
            }],
        };
        let mut whole = Vec::new();
        let mut heads = Vec::new();
        for (word, cost) in words {
            if exact_top && word.typos > 0 {
                continue;
            }
            if word.end >= total {
                whole.push(to_candidate(word, cost));
            } else if word.completion == 0 {
                heads.push(to_candidate(word, cost));
            }
        }
        // 句首词只留在音节边界上的（不把一个音节拆开）
        let other_splits = prefix_on_syllable_boundaries(result, normalized, &mut heads);
        let leading = raw.len() - raw.trim_start().len();
        let mut seen: HashMap<String, ()> = result.iter().map(|c| (c.text.clone(), ())).collect();
        let converter = if self.config.traditional {
            self.converter.read().ok()
        } else {
            None
        };
        // 首音节常用字不要被几百个替代切分的生僻词压到长尾之后。
        let single = |c: &Candidate| c.text.chars().count() == 1;
        let (chars, words): (Vec<_>, Vec<_>) = heads.into_iter().partition(single);
        let (common, rare): (Vec<_>, Vec<_>) =
            chars.into_iter().partition(|c| c.words[0].freq >= 1.0);
        let (other_chars, other_words): (Vec<_>, Vec<_>) =
            other_splits.into_iter().partition(single);
        for mut candidate in whole
            .into_iter()
            .chain(words)
            .chain(common)
            .chain(other_words)
            .chain(rare)
            .chain(other_chars)
        {
            if result.len() >= max {
                break;
            }
            if let Some(conv) = converter.as_ref().filter(|c| !c.is_empty()) {
                candidate.text = conv.convert(&candidate.text);
            }
            if seen.insert(candidate.text.clone(), ()).is_some() {
                continue;
            }
            candidate.consumed = if candidate.consumed >= total {
                raw.len()
            } else {
                leading + letters_to_bytes(display, candidate.consumed)
            };
            result.push(candidate);
        }
        for i in 1..result.len() {
            if result[i].score >= result[i - 1].score {
                result[i].score = result[i - 1].score - 1e-3;
            }
        }
        Ok(())
    }

    /// 全拼下输入能否不靠纠错、补全切成完整音节。
    pub(super) fn is_plain_pinyin(&self, display: &str) -> bool {
        if self.config.shuangpin {
            return false;
        }
        let Ok(parsed) = parse(&display.to_ascii_lowercase()) else {
            return false;
        };
        SyllableGraph::build(
            &parsed,
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo: false,
                completion: false,
            },
        )
        .has_complete_path()
    }

    /// 英文/未登录输入：保证原始输入本身可以作为候选上屏。
    ///
    /// 放在第几位：
    /// - 拼音完整、没纠错的中文排前面，原始串垫底（she -> 社 设 ……）；
    /// - 输入是已知英文词，而中文要靠纠错才凑得出来：英文排第一
    ///   （hello、github、python 不该被改成乱码中文）；中文只差最后一个音节没打完
    ///   （bug、diy）时排第二；拼音本身就完整（she、long）时排第六；
    /// - 不认识的串：中文全是未登录兜底时排第一，中文纠了 2 处以上排第二，其余垫底。
    ///
    /// 原始串一定留在列表里，不会被截断挤掉。
    pub(super) fn append_raw_candidate(&self, display: &str, candidates: &mut Vec<Candidate>) {
        if !self.config.english_candidate || display.is_empty() || display.len() > 32 {
            return;
        }
        if !display.bytes().any(|b| b.is_ascii_alphabetic()) {
            return;
        }
        // 全小写输入且词表里有标准写法时用标准写法（github -> GitHub）
        let english = self
            .english
            .read()
            .ok()
            .and_then(|m| m.get(&display.to_ascii_lowercase()).cloned());
        let text = match &english {
            Some(word) if display.bytes().all(|b| !b.is_ascii_uppercase()) => word.clone(),
            _ => display.to_string(),
        };
        if candidates
            .iter()
            .any(|c| c.text.eq_ignore_ascii_case(&text))
        {
            return;
        }
        // 输入本身是不是一串完整的拼音（she、me、long），或者已经有不靠纠错和
        // 补全的完整中文候选（双拼下只能看后者）
        // 全拼混简拼（he l l o -> 合理了哦）是硬凑出来的，不算
        let exact = self.is_plain_pinyin(display)
            || candidates.iter().any(|c| {
                c.features.coverage_score >= 1.0
                    && c.features.typo_count == 0.0
                    && c.features.prefix_completion == 0.0
                    && !c.words.iter().any(|w| !w.known)
                    && !is_mixed_jianpin(c)
            });
        // 第一个中文候选在中间就有只打了声母的音节（hello -> he l l o）：
        // 不像是还在打拼音，英文排第一
        let inner_abbrev = candidates
            .first()
            .is_some_and(|c| is_mixed_jianpin(c) && abbrev_before_last(c));
        // 打了大写（Hello）却只能凑成混简拼（he l l o -> 合理了哦）：要的是原样上屏
        let literal = display.bytes().any(|b| b.is_ascii_uppercase())
            && candidates.first().is_some_and(is_mixed_jianpin);
        let top_typos = candidates
            .first()
            .map(|c| c.features.typo_count)
            .unwrap_or(0.0);
        let has_known_word = candidates
            .first()
            .map(|c| c.words.iter().any(|w| w.known))
            .unwrap_or(false);
        // 第一名是几个词凑的整句（ok -> 哦 | 可）：不是「一个词还没打完」
        let top_sentence = candidates.first().is_some_and(is_sentence);
        let position = if english.is_some() {
            if exact {
                // 本身就是拼音的英文词（fan、long、she）：前 5 个留给中文
                Some(5)
            } else if top_typos == 0.0 && has_known_word && !inner_abbrev && !top_sentence {
                // 中文只是最后一个音节没打完（bug -> 不过、diy -> 第一），
                // 用户多半还在打拼音，英文排第二
                Some(1)
            } else {
                Some(0)
            }
        } else if exact {
            // 拼音完整的中文，原始拼音串没必要出现
            return;
        } else if !has_known_word {
            Some(0)
        } else if literal {
            Some(0)
        } else if top_typos >= 2.0 {
            Some(1)
        } else {
            None
        };
        let max = self.config.max_candidates.max(1);
        let at = position.unwrap_or(candidates.len()).min(candidates.len());
        // 分数夹在前后两个候选之间，后面无论怎么重新排序（繁体转换会重排）位置都不变
        let score = match (
            at.checked_sub(1).and_then(|i| candidates.get(i)),
            candidates.get(at),
        ) {
            (None, None) => -1.0,
            (None, Some(next)) => next.score + 1.0,
            (Some(prev), None) => prev.score - 1.0,
            (Some(prev), Some(next)) => (prev.score + next.score) / 2.0,
        };
        let raw = Candidate {
            long_completion: false,
            text,
            pinyin: display.to_ascii_lowercase(),
            score,
            features: Feature::default(),
            words: vec![CandidateWord {
                text: display.to_string(),
                pinyin: String::new(),
                freq: 0.0,
                known: english.is_some(),
            }],
            consumed: display.len(),
        };
        candidates.insert(at, raw);
        if candidates.len() > max {
            // 截掉的是原始串以外的最后一个
            let drop = if at == candidates.len() - 1 {
                candidates.len() - 2
            } else {
                candidates.len() - 1
            };
            candidates.remove(drop);
        }
    }
}
