//! 一次查询：音节图、解码、模糊音兜底和候选打分特征。
use super::*;

impl Engine {
    /// 单条全拼输入的完整查询（精确 + 可选模糊音）。
    ///
    /// 模糊音视图里精确读音和模糊读音都有（模糊词扣 `fuzzy_penalty`），一次搜索就够了，
    /// 两种读音的词按各自的分数排在一起。
    pub(super) fn query_plain(
        &self,
        normalized: &str,
        completion: bool,
        at_ms: u64,
    ) -> Result<Vec<Candidate>> {
        self.run_query(normalized, completion, at_ms, self.config.fuzzy)
    }

    pub(super) fn full_input_graph(
        &self,
        input: &crate::syllable::ParsedInput,
        typo: bool,
        completion: bool,
    ) -> SyllableGraph {
        let mut graph = SyllableGraph::build_with_typos(
            input,
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: true,
                typo,
                completion,
            },
            self.config.typo_kinds,
        );
        if self.config.fuzzy {
            graph.add_fuzzy_spellings(self.config.fuzzy_groups);
        }
        graph
    }

    /// 跑一次查询；fuzzy=true 时精确读音和模糊音读音的词一起搜。
    ///
    /// 末音节补全（zhongg -> 中国、zhonggu -> 中国）和纠错都做成了音节图里的边，
    /// 一次搜索就能同时给出精确、补全、纠错三类候选。
    pub(super) fn run_query(
        &self,
        normalized: &str,
        completion: bool,
        at_ms: u64,
        fuzzy: bool,
    ) -> Result<Vec<Candidate>> {
        let parsed = parse(normalized)?;
        if parsed.is_empty() {
            return Ok(Vec::new());
        }
        let typo = self.config.typo_correction && !self.config.shuangpin;
        let full = self.full_input_graph(&parsed, typo, completion);
        let mut full_found = if full.has_complete_path() {
            self.query_once(&full, at_ms, fuzzy)?
        } else {
            Vec::new()
        };
        // 不靠补全、纠错就能完整解析：直接用全拼。
        // 但能切成音节不等于词库里有词：hm、hng 这类叹词音节词库不收，以前只给出原样的 hm，
        // 打 hm（后面、好吗）一个中文候选都没有。一个认识的词都凑不出来就接着试简拼。
        if full.has_exact_path() {
            self.keep_exact_syllable(&full, &mut full_found, at_ms, fuzzy)?;
            if !fuzzy && self.config.fuzzy_fallback && !self.config.shuangpin {
                self.fuzzy_fallback(&full, &mut full_found);
            }
            let any_known = full_found.iter().any(|c| {
                c.features.typo_count == 0.0
                    && c.features.prefix_completion == 0.0
                    && !c.words.is_empty()
                    && c.words.iter().all(|w| w.known)
            });
            if !self.config.jianpin_fallback || any_known {
                return Ok(full_found);
            }
        }
        // 否则简拼（zg、jzm）、全拼混简拼（kyi、yongzwen）与全拼补全/纠错都试，按分数合并
        let mut merged: Vec<Candidate> = Vec::new();
        if self.config.jianpin_fallback {
            // 图上能用“未知儿化 + 补全”走通，不等于词库能解释输入。仅当完整候选确有
            // 不靠纠错的已知词时省掉混简拼查询，否则 shurf 会永远失去 shu+r+f 这条路。
            let completion_explains = full.has_path_without_erhua_or_typo()
                && full_found.iter().any(|c| {
                    c.features.typo_count == 0.0
                        && !c.words.is_empty()
                        && c.words.iter().all(|w| w.known)
                });
            if let Some(jianpin) = abbrev_graph(&parsed, completion_explains) {
                let mut found = self.query_once(&jianpin, at_ms, fuzzy)?;
                if jianpin.mode() == GraphMode::Mixed {
                    // 混简拼里拿声母、单个字母兜底凑出来的（h e l l o -> "hello"）没有意义
                    found.retain(|c| c.words.iter().all(|w| w.known));
                }
                merged.extend(found);
            }
        }
        merged.append(&mut full_found);
        let mut result = dedupe_by_text(merged);
        result.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.text.cmp(&b.text))
        });
        result.truncate(self.config.max_candidates.max(1));
        Ok(result)
    }

    /// 当补全耗尽束搜索名额时，单独补回打全的常用单字（WowType 59b3c91）。
    pub(super) fn keep_exact_syllable(
        &self,
        graph: &SyllableGraph,
        found: &mut Vec<Candidate>,
        at_ms: u64,
        fuzzy: bool,
    ) -> Result<()> {
        let one = graph.edges_from(0).iter().any(|edge| {
            edge.end == graph.len()
                && !edge.typo
                && edge.completion == 0
                && matches!(edge.kind, EdgeKind::Full { erhua: false, .. })
        });
        let exact = |c: &Candidate| {
            c.features.typo_count == 0.0
                && c.features.prefix_completion == 0.0
                && c.words.len() == 1
                && c.words[0].known
                && !c.pinyin.contains(' ')
        };
        if !one || found.iter().any(exact) {
            return Ok(());
        }
        let graph = SyllableGraph::build(
            graph.input(),
            GraphOptions {
                mode: GraphMode::Full,
                allow_erhua: false,
                typo: false,
                completion: false,
            },
        );
        let mut extra: Vec<_> = self
            .query_once(&graph, at_ms, fuzzy)?
            .into_iter()
            .filter(|c| {
                exact(c) && c.words[0].freq >= 1.0 && !found.iter().any(|item| item.text == c.text)
            })
            .collect();
        if let Some(top) = found.first().map(|c| c.score) {
            let count = extra.len();
            for (i, candidate) in extra.iter_mut().enumerate() {
                candidate.score = top + 1e-3 * (count - i) as f64;
            }
        }
        found.splice(0..0, extra);
        found.truncate(self.config.max_candidates.max(1));
        Ok(())
    }

    /// 模糊音关着时的兜底：精确读音的第一名偏弱（几个词硬凑的、纠错或补全出来的），
    /// 而输入按模糊音读是个常用词（yingai -> 应该、shengme -> 什么、zidao -> 知道、
    /// penyou -> 朋友）时，把这个词（最多两个）加进候选，比第一名高出
    /// [`FUZZY_FALLBACK_LEAD`] 才加。
    ///
    /// 只查 2–4 个音节的精确切分，每种切分一次模糊查表，不再跑一遍束搜索。
    pub(super) fn fuzzy_fallback(&self, graph: &SyllableGraph, found: &mut Vec<Candidate>) {
        let weak = found.first().is_none_or(|c| {
            c.words.len() >= 2
                || c.features.typo_count > 0.0
                || c.features.prefix_completion > 0.0
                || c.words.iter().any(|w| !w.known)
        });
        let syllables = found
            .first()
            .map_or(0, |c| c.pinyin.split_whitespace().count());
        if !weak || !(2..=4).contains(&syllables) {
            return;
        }
        let top = found.first().map_or(f64::NEG_INFINITY, |c| c.score);
        let view = FuzzyLexicon::new(
            self.lexicon.as_ref(),
            self.config.fuzzy_groups,
            self.config.fuzzy_penalty,
        );
        // 接着前文打时第一个词不在句首，和精确候选一样不加句首分
        let mid_sentence = !self.context().trim().is_empty();
        let mut segmentations = Vec::new();
        exact_segmentations(graph, 0, 4, &mut Vec::new(), &mut segmentations);
        let mut extra: Vec<Candidate> = Vec::new();
        for segmentation in segmentations.iter().filter(|s| (2..=4).contains(&s.len())) {
            let query: Vec<SyllableQuery> = segmentation.iter().map(|e| e.query()).collect();
            let exact = self.lexicon.lookup_common(&query);
            let pinyin = query
                .iter()
                .map(|q| q.display())
                .collect::<Vec<_>>()
                .join(" ");
            for entry in view.lookup_common(&query) {
                // 精确读音也有这个词、而且更常用：不是靠模糊音才有的
                if exact
                    .iter()
                    .any(|e| e.word == entry.word && e.freq >= entry.freq)
                {
                    continue;
                }
                if found.iter().chain(&extra).any(|c| c.text == entry.word) {
                    continue;
                }
                let lm = if mid_sentence {
                    0.0
                } else {
                    self.lm.score(None, &entry.word)
                };
                let feature = Feature {
                    coverage_score: 1.0,
                    lm_score_with_previous_txt: lm as f32,
                    ..Feature::default()
                };
                let score = -view.cost(&entry)
                    + self.config.beam.lm_weight * lm
                    + self.ranker.score(&feature) as f64;
                if score < top + FUZZY_FALLBACK_LEAD {
                    continue;
                }
                extra.push(Candidate {
                    long_completion: false,
                    text: entry.word.clone(),
                    pinyin: pinyin.clone(),
                    score,
                    features: feature,
                    words: vec![CandidateWord {
                        text: entry.word.clone(),
                        pinyin: pinyin.clone(),
                        freq: entry.freq,
                        known: true,
                    }],
                    consumed: 0,
                });
            }
        }
        if extra.is_empty() {
            return;
        }
        extra.sort_by(|a, b| b.score.total_cmp(&a.score));
        extra.truncate(2);
        found.extend(extra);
        found.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.text.cmp(&b.text))
        });
        found.truncate(self.config.max_candidates.max(1));
    }

    pub(super) fn query_once(
        &self,
        graph: &SyllableGraph,
        at_ms: u64,
        fuzzy: bool,
    ) -> Result<Vec<Candidate>> {
        // fuzzy 时用包装视图：精确读音之外再加上模糊音读音的词。
        let fuzzy_view;
        let lexicon: &dyn Lexicon = if fuzzy {
            fuzzy_view = FuzzyLexicon::new(
                self.lexicon.as_ref(),
                self.config.fuzzy_groups,
                self.config.fuzzy_penalty,
            );
            &fuzzy_view
        } else {
            self.lexicon.as_ref()
        };
        let memo = self.memo_view(lexicon, fuzzy);
        let lexicon: &dyn Lexicon = &memo;
        let start = self.start_word();
        let pairs = self
            .pairs
            .read()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?;
        // 接着前文打：第一个词不在句首，不加句首分（上文搭配由排序特征 context_bigram_score 管）
        let mid_sentence = !self.context().trim().is_empty();
        let lm = LearnedLm {
            base: self.lm.as_ref(),
            pairs: &pairs,
            start: start.as_deref(),
            at_ms,
            mid_sentence,
        };
        let mut paths = beam_search(graph, lexicon, &lm, &self.config.beam);
        drop(pairs);
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        // 有完整覆盖路径时只保留完整覆盖，避免 西an / xi安 这类半拼音候选。
        // 否则退而求其次，保留有部分词典命中的路径；全未登录时保留兜底路径。
        if paths.iter().any(|p| p.coverage >= 1.0) {
            paths.retain(|p| p.coverage >= 1.0);
        } else if paths.iter().any(|p| p.coverage > 0.0) {
            paths.retain(|p| p.coverage > 0.0);
        }

        let best_score = paths
            .iter()
            .map(|p| p.score)
            .fold(f64::NEG_INFINITY, f64::max);

        let nnlm = self.nnlm_scores(&paths);
        let candidates: Vec<Candidate> = paths
            .into_iter()
            .zip(nnlm)
            .map(|(path, nnlm)| self.make_candidate(path, best_score, at_ms, nnlm))
            .collect();

        let mut deduped = dedupe_by_text(candidates);
        deduped.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.text.cmp(&b.text))
        });
        let mut deduped = exact_syllable_first(deduped);
        deduped.truncate(self.config.max_candidates.max(1));
        Ok(deduped)
    }

    /// 每条路径的神经语言模型特征：log P(整句 | 光标前的文本) 减去这批里最好的那条，≤ 0。
    ///
    /// 只算束搜索分数最高的 `nnlm_rerank` 条（模型一步约 1 ms，同一前缀会缓存）；
    /// 其余的给比算过的最差那条还低一点的值。没有模型时全是 0。
    pub(super) fn nnlm_scores(&self, paths: &[BeamPath]) -> Vec<f32> {
        let mut out = vec![0.0f32; paths.len()];
        let limit = self.config.nnlm_rerank;
        if limit == 0 || paths.len() < 2 {
            return out;
        }
        let Some(model) = self.nnlm.read().ok().and_then(|m| m.clone()) else {
            return out;
        };
        let mut order: Vec<usize> = (0..paths.len()).collect();
        order.sort_by(|&a, &b| paths[b].score.total_cmp(&paths[a].score));
        let preceding = self.context();
        // 无上文的短词保留词频排序，避免逐键跑神经模型。
        if preceding.trim().is_empty()
            && paths.first().is_none_or(|path| {
                path.words
                    .iter()
                    .map(|word| word.syllable_count)
                    .sum::<usize>()
                    < 5
            })
        {
            return out;
        }
        let context = model.context_tokens(&preceding);
        let scored: Vec<(usize, f64)> = order
            .iter()
            .take(limit)
            .map(|&i| (i, model.score(&context, &paths[i].text)))
            .collect();
        let best = scored.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
        let worst = scored.iter().map(|s| s.1).fold(f64::INFINITY, f64::min);
        for &(i, s) in &scored {
            out[i] = (s - best).max(-NNLM_FLOOR) as f32;
        }
        let rest = ((worst - best) - 2.0).max(-NNLM_FLOOR) as f32;
        for &i in order.iter().skip(limit) {
            out[i] = rest;
        }
        out
    }

    /// 词级语言模型下，光标前的文字接这个词的分数（上文排序特征）。
    ///
    /// 上文末尾 1–4 个字里，模型认识的每个后缀都当一次前一个词，取最高分。以前只试「整段上文」
    /// 和「最后一个字」：整段上文模型不认识时得 0，反倒把负分抹平了（「今天我」碰巧正常、「我」不正常）。
    ///
    /// 「后缀 + 这个词」本身是模型里的一个词时（我 + 要 = 我要、不 + 知道 = 不知道），
    /// 语料分词时它们被并成了一个词，这个搭配从没被计过数，模型给的是「没见过」的负分，
    /// 还不如偶然见过几次的「我腰」「我悔」。这时按见过的常用搭配算，至少给 [`MERGED_PAIR_SCORE`]。
    pub(super) fn word_lm_context_score(&self, context: &str, word: &str) -> f32 {
        let chars: Vec<char> = context.chars().collect();
        let merged_floor = MERGED_PAIR_SCORE;
        let mut best: Option<f64> = None;
        for n in 1..=chars.len().min(4) {
            let prev: String = chars[chars.len() - n..].iter().collect();
            if self.lm.knows_word(&prev) == Some(true) {
                let s = self.lm.score(Some(&prev), word);
                best = Some(best.map_or(s, |b| b.max(s)));
            }
            if self.lm.knows_word(&format!("{prev}{word}")) == Some(true) {
                best = Some(best.map_or(merged_floor, |b| b.max(merged_floor)));
            }
        }
        best.unwrap_or(0.0) as f32
    }

    pub(super) fn make_candidate(
        &self,
        path: BeamPath,
        best_score: f64,
        at_ms: u64,
        nnlm: f32,
    ) -> Candidate {
        let mut feature = self.make_feature(&path, best_score, at_ms);
        feature.head_word_nnlm = nnlm;
        let rank_score = self.ranker.score(&feature) as f64;
        // 单个音节打全时只有一个字：句首 LM 分反映的是语料里谁常出现在句首，
        // 没有上下文时不该压过字频（否则 he 会先出「何」）。
        // 只减基础模型的句首分：path.lm_score 里还有用户学过的搭配加成，学过的字要照样靠前。
        let single_word = path.words.len() == 1 && path.words[0].syllable_count == 1;
        // 接着前文打时句首分本来就没加（见 run_query 的 mid_sentence），不用再减
        let mid_sentence = !self.context().trim().is_empty();
        let score = if single_word && !mid_sentence {
            let start = self.lm.score(None, &path.words[0].text);
            path.score - self.config.beam.lm_weight * start + rank_score
        } else {
            path.score + rank_score
        };
        Candidate {
            text: path.text,
            long_completion: false,
            pinyin: path.pinyin,
            score,
            features: feature,
            words: path
                .words
                .into_iter()
                .map(|w| CandidateWord {
                    text: w.text,
                    pinyin: w.pinyin,
                    freq: w.freq,
                    known: w.known,
                })
                .collect(),
            consumed: 0,
        }
    }

    pub(super) fn make_feature(&self, path: &BeamPath, best_score: f64, at_ms: u64) -> Feature {
        // 历史：整句命中与末词命中都算。
        //
        // 排序用热度（30 天半衰期衰减后的次数，取 ln(1+热度)），见 [`learn`]。
        // 末词只算多字词，而且算一半：选过「种地」，「我想种地」也该靠前；
        // 选过「你」不能把所有以「你」结尾的整句都抬上来。
        let history_hot_val = self
            .history
            .read()
            .ok()
            .map(|history| {
                let exact = history
                    .get(&path.text)
                    .map(|e| crate::learn::decayed(*e, at_ms))
                    .unwrap_or(0.0);
                let tail = path
                    .words
                    .last()
                    .filter(|w| path.words.len() >= 2 && w.text.chars().count() >= 2)
                    .and_then(|w| history.get(&w.text))
                    .map(|e| crate::learn::decayed(*e, at_ms))
                    .unwrap_or(0.0);
                ((1.0 + exact + 0.5 * tail).ln() as f32, exact as f32)
            })
            .unwrap_or((0.0, 0.0));
        let (history_hot_val, picked) = history_hot_val;
        let (history_count, history_time_diff) = self
            .history
            .read()
            .ok()
            .map(|history| {
                let exact = history.get(&path.text).copied();
                let tail = path
                    .words
                    .last()
                    .and_then(|w| history.get(&w.text))
                    .copied();
                let merged = match (exact, tail) {
                    (Some(a), Some(b)) if b.count > a.count => b,
                    (Some(a), _) => a,
                    (None, Some(b)) => b,
                    (None, None) => HistoryEntry {
                        count: 0.0,
                        last_used_ms: at_ms,
                    },
                };
                let seconds = at_ms.saturating_sub(merged.last_used_ms) as f64 / 1000.0;
                (merged.count as f32, (1.0 + seconds).ln() as f32)
            })
            .unwrap_or((0.0, 0.0));

        // 上下文：用候选首词去查二元分。
        //
        // 取键要同时试「上一个词」和「上一个字」：词级 LM 用前者，
        // 字级 LM（例如从语料统计的字二元）用后者。中文上下文通常没有空格，
        // 只取 split_whitespace().last() 会把整段文本当成一个词，查不到任何 bigram。
        let (with_context, context_bigram_score) = self
            .context
            .read()
            .map(|context| {
                let context = context.trim();
                let Some(candidate) = path.words.first() else {
                    return (0.0, 0.0);
                };
                if context.is_empty() {
                    return (0.0, 0.0);
                }
                if self.lm.knows_word(&candidate.text).is_some() {
                    return (1.0, self.word_lm_context_score(context, &candidate.text));
                }
                let mut best: Option<f32> = None;
                let mut consider = |prev: &str| {
                    if prev.is_empty() {
                        return;
                    }
                    let score = self.lm.score(Some(prev), &candidate.text) as f32;
                    best = Some(best.map_or(score, |b: f32| b.max(score)));
                };
                if let Some(word) = context.split_whitespace().last() {
                    consider(word);
                }
                let mut buf = [0u8; 4];
                if let Some(ch) = context.chars().last() {
                    consider(ch.encode_utf8(&mut buf));
                }
                (1.0, best.unwrap_or(0.0))
            })
            .unwrap_or((0.0, 0.0));

        Feature {
            kind: if path.words.len() <= 1 { 0.0 } else { 1.0 },
            prob: (path.score - best_score).exp().clamp(0.0, 1.0) as f32,
            coverage_score: path.coverage,
            lm_score_with_previous_txt: path.lm_score as f32,
            full_jianpin: if path.jianpin { 1.0 } else { 0.0 },
            ltr_valid: if self.ranker.has_ltr() { 1.0 } else { 0.0 },
            history_count,
            // 这个候选本身（整个文本）被选过的热度，不含末词命中
            history_use_count: picked,
            history_time_diff,
            history_hot_val,
            with_context,
            context_bigram_score,
            typo_count: path.typos as f32,
            prefix_completion: (path.completion as f32).min(3.0),
            typo: if path.typos > 0 { 1.0 } else { 0.0 },
            // 用户词等特征还没有数据源，保持 0；NNLM 在 make_candidate 里填。
            ..Feature::default()
        }
    }
}
