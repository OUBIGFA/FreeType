//! 从现有词库补完整长词；在普通候选排好后插入，保留原首选及普通输入路径。
use crate::lexicon::{parse_pinyin_keys, Lexicon};
use crate::{Candidate, CandidateWord, Engine, Feature};

impl Engine {
    pub(crate) fn add_long_completion(&self, raw: &str, candidates: &mut Vec<Candidate>) {
        if !self.config.long_word_completion || self.config.max_candidates < 2 {
            return;
        }
        let Some(top) = candidates.first() else {
            return;
        };
        if top.consumed != raw.len()
            || top.features.typo_count > 0.0
            || top.features.prefix_completion > 0.0
            || top.pinyin.is_empty()
        {
            return;
        }
        let Ok(prefix) = parse_pinyin_keys(&top.pinyin) else {
            return;
        };
        if prefix.len() < 2 || prefix.iter().any(|key| key.erhua) {
            return;
        }
        // 简拼、末音节没打完和纠错都不扩写；双拼必须成对，并与实际解码读音一致。
        let complete = if self.config.shuangpin {
            let scheme = self.config.shuangpin_scheme;
            raw.bytes()
                .filter(|b| scheme.is_key(b.to_ascii_lowercase()))
                .count()
                % 2
                == 0
                && crate::shuangpin::decode(scheme, raw.trim())
                    .iter()
                    .any(|decoded| {
                        parse_pinyin_keys(&decoded.pinyin).is_ok_and(|keys| keys == prefix)
                    })
        } else {
            let letters = |text: &str| {
                text.bytes()
                    .filter(|b| b.is_ascii_alphabetic())
                    .map(|b| b.to_ascii_lowercase())
                    .collect::<Vec<_>>()
            };
            letters(raw) == letters(&top.pinyin)
        };
        if !complete {
            return;
        }
        // 首选是词库整词时按该词补；首选是组句时按读音找，允许“窗前明月”补出“床前明月光”。
        let word_prefix = if top.words.len() == 1 && top.words[0].known {
            top.words[0].text.as_str()
        } else {
            ""
        };
        let mut choices = self.lexicon.long_completions(&prefix, word_prefix, 16);
        // A learned phrase must not disappear because the current homophone is
        // 这时 rather than 这是. Personal words match the entered reading first.
        let personal = self.lexicon.personal_long_completions(&prefix, "");
        let personal_words: std::collections::HashSet<_> =
            personal.iter().map(|e| e.word.clone()).collect();
        choices.extend(personal);
        let at = crate::now_ms();
        choices.sort_by(|a, b| {
            self.history_heat(&b.word, at)
                .total_cmp(&self.history_heat(&a.word, at))
                .then_with(|| {
                    personal_words
                        .contains(&b.word)
                        .cmp(&personal_words.contains(&a.word))
                })
                .then_with(|| b.freq.total_cmp(&a.freq))
        });
        let mut seen = std::collections::HashSet::new();
        choices.retain(|c| seen.insert(c.word.clone()));
        // 诗句/专名按读音补全不以通用模型词表为门槛；仅有一个词库扩展也保留。
        // 整词有多个扩展时优先模型中真正收录的词，避免高词频切词残片占住补全位。
        let selected = if prefix.len() < 4
            || choices.first().is_some_and(|c| {
                personal_words.contains(&c.word) || self.history_heat(&c.word, at) > 0.0
            })
            || word_prefix.is_empty()
            || choices.len() <= 1
        {
            choices.first()
        } else {
            choices
                .iter()
                .find(|entry| self.lm.knows_word(&entry.word) == Some(true))
                .or_else(|| {
                    if self.lm.knows_word(word_prefix).is_none() {
                        return choices.first();
                    }
                    let guard = self.nnlm.read().ok()?;
                    let model = guard.as_ref()?;
                    let context = model.context_tokens(word_prefix);
                    let empty = model.context_tokens("");
                    // 有限候选只比较补出的后缀相对无上下文的提升度，不按整句概率惩罚长词。
                    choices
                        .iter()
                        .take(8)
                        .filter(|entry| {
                            let suffix = &entry.word[word_prefix.len()..];
                            let Some(next) = suffix.chars().next().filter(|ch| model.knows(*ch))
                            else {
                                return false;
                            };
                            // 各备选共享两次上下文前向结果，只看新增首字的提升度；不对每条
                            // 长诗句执行完整神经推理，避免完整词库下按键耗时随后缀长度增长。
                            model.next_char_logprob(&context, next)
                                > model.next_char_logprob(&empty, next)
                        })
                        .max_by(|a, b| a.freq.total_cmp(&b.freq))
                })
        };
        let Some(selected) = selected.cloned() else {
            return;
        };
        let mut entries = vec![selected];
        if prefix.len() < 4 {
            for entry in choices {
                if entries.len() >= 3 {
                    break;
                }
                if !entries.iter().any(|c| c.word == entry.word) {
                    entries.push(entry);
                }
            }
        }
        let top_heat = self.history_heat(&top.text, at);
        let top_score = top.score;
        // 已输入完整读音时，先保留同音整词，再放未经学习的长词扩展。
        // 组句和半词前缀仍可在第二位补全，用户反复选择的补全仍按热度提升。
        let mut position = if prefix.len() >= 4 {
            1
        } else {
            candidates
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    c.consumed == raw.len()
                        && c.pinyin == top.pinyin
                        && !c.long_completion
                        && c.features.typo_count == 0.0
                        && c.features.prefix_completion == 0.0
                        && c.words.len() == 1
                        && c.words[0].known
                })
                .map(|(i, _)| i + 1)
                .max()
                .unwrap_or(1)
        };
        let mut promoted = 0;
        for entry in entries {
            let text = self.shown_text(&entry.word);
            if candidates.iter().any(|candidate| candidate.text == text) {
                continue;
            }
            let pinyin = entry
                .keys
                .iter()
                .map(|key| crate::Syllable::from_id(key.id).unwrap().text.to_owned())
                .collect::<Vec<_>>()
                .join(" ");
            let score = candidates
                .get(1)
                .map_or(top_score - 1.0, |next| (top_score + next.score) / 2.0);
            let heat = self.history_heat(&entry.word, at);
            let insert = if heat >= 6.0 && heat > top_heat + 2.0 {
                let i = promoted;
                promoted += 1;
                i
            } else {
                position.min(candidates.len())
            };
            candidates.insert(
                insert,
                Candidate {
                    text,
                    pinyin: pinyin.clone(),
                    score,
                    features: Feature::default(),
                    words: vec![CandidateWord {
                        text: entry.word,
                        pinyin,
                        freq: entry.freq,
                        known: true,
                    }],
                    consumed: raw.len(),
                    long_completion: true,
                },
            );
            position += 1;
            if candidates.len() > self.config.max_candidates {
                // 原样字母始终可选；名额只够首选和原文时，宁可不显示补全。
                let drop = candidates
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(index, candidate)| *index > 1 && !crate::is_raw_candidate(candidate))
                    .map_or(1, |(index, _)| index);
                candidates.remove(drop);
            }
        }
    }

    pub(crate) fn add_english_prefix(&self, raw: &str, at: u64, candidates: &mut Vec<Candidate>) {
        if !self.config.long_word_completion
            || !self.config.english_candidate
            || raw.len() < 2
            || raw.len() > 31
            || !raw.bytes().all(|c| c.is_ascii_alphabetic())
            || self.config.max_candidates < 2
        {
            return;
        }
        let prefix = raw.to_ascii_lowercase();
        let mut words = self.lexicon.learned_code_prefix(&prefix);
        // Ordered range lookup avoids scanning the full bundled English table.
        if let Ok(table) = self.english.read() {
            for (code, text) in table
                .range(prefix.clone()..)
                .take_while(|(code, _)| code.starts_with(&prefix))
            {
                if code.len() > prefix.len() {
                    words.push((code.clone(), text.clone(), 1.0));
                }
            }
        }
        words.retain(|(_, word, _)| raw.len() >= 3 || self.history_heat(word, at) >= 3.0);
        words.sort_by(|a, b| {
            self.history_heat(&b.1, at)
                .total_cmp(&self.history_heat(&a.1, at))
                .then_with(|| a.0.len().cmp(&b.0.len()))
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut seen = std::collections::HashSet::new();
        words.retain(|(_, word, _)| seen.insert(word.clone()));
        words.truncate(3);
        let top_heat = candidates
            .first()
            .map_or(0.0, |c| self.history_heat(&c.text, at));
        let mut promoted = 0;
        for (position, (code, word, freq)) in words.into_iter().enumerate() {
            if candidates.iter().any(|c| c.text == word) {
                continue;
            }
            let heat = self.history_heat(&word, at);
            let index = if heat >= 6.0 && heat > top_heat + 2.0 {
                let i = promoted;
                promoted += 1;
                i
            } else {
                (position + 1).min(candidates.len())
            };
            candidates.insert(
                index,
                Candidate {
                    text: word.clone(),
                    pinyin: code.clone(),
                    score: heat,
                    features: Feature::default(),
                    words: vec![CandidateWord {
                        text: word,
                        pinyin: code,
                        freq,
                        known: true,
                    }],
                    consumed: raw.len(),
                    long_completion: true,
                },
            );
            if candidates.len() > self.config.max_candidates {
                if let Some(i) = candidates.iter().rposition(|c| !crate::is_raw_candidate(c)) {
                    candidates.remove(i);
                }
            }
        }
    }
}
