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
        if prefix.len() < 4 || prefix.iter().any(|key| key.erhua) {
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
        let choices = self.lexicon.long_completions(&prefix, word_prefix, 16);
        // 诗句/专名按读音补全不以通用模型词表为门槛；仅有一个词库扩展也保留。
        // 整词有多个扩展时优先模型中真正收录的词，避免高词频切词残片占住补全位。
        let selected = if word_prefix.is_empty() || choices.len() <= 1 {
            choices.first()
        } else {
            choices.iter().find(|entry| self.lm.knows_word(&entry.word) == Some(true)).or_else(|| {
                if self.lm.knows_word(word_prefix).is_none() { return choices.first(); }
                let guard = self.nnlm.read().ok()?;
                let model = guard.as_ref()?;
                let context = model.context_tokens(word_prefix);
                let empty = model.context_tokens("");
                // 有限候选只比较补出的后缀相对无上下文的提升度，不按整句概率惩罚长词。
                choices.iter().take(8).filter(|entry| {
                    let suffix = &entry.word[word_prefix.len()..];
                    let Some(next) = suffix.chars().next().filter(|ch| model.knows(*ch)) else { return false; };
                    // 各备选共享两次上下文前向结果，只看新增首字的提升度；不对每条
                    // 长诗句执行完整神经推理，避免完整词库下按键耗时随后缀长度增长。
                    model.next_char_logprob(&context, next) > model.next_char_logprob(&empty, next)
                }).max_by(|a, b| a.freq.total_cmp(&b.freq))
            })
        };
        let Some(entry) = selected.cloned() else {
            return;
        };
        let text = self.shown_text(&entry.word);
        if candidates.iter().any(|candidate| candidate.text == text) {
            return;
        }
        let pinyin = entry
            .keys
            .iter()
            .map(|key| crate::Syllable::from_id(key.id).unwrap().text.to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let score = candidates
            .get(1)
            .map_or(top.score - 1.0, |next| (top.score + next.score) / 2.0);
        candidates.insert(
            1,
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
