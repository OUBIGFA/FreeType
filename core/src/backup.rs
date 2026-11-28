//! 完整学习备份：直接读取内存中的词、热度和搭配，不等待定时存盘。

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

use crate::{Engine, HistoryEntry};

fn valid_word(word: &str) -> bool {
    !word.trim().is_empty() && !word.contains(['\t', '\r', '\n'])
}

fn heat_value(heat: f64) -> Value {
    let heat = (heat * 1000.0).round() / 1000.0;
    if heat.fract() == 0.0 {
        json!(heat as u64)
    } else {
        json!(heat)
    }
}

fn newer(incoming: HistoryEntry, current: Option<HistoryEntry>) -> bool {
    current.map_or(true, |current| {
        incoming.last_used_ms > current.last_used_ms
            || (incoming.last_used_ms == current.last_used_ms && incoming.count > current.count)
    })
}

impl Engine {
    /// 导出紧凑单行 UTF-8 JSON v1：用户词、选词次数/时间和词搭配；不含词库、设置和剪贴板。
    /// 导出前自动裁剪已衰减过期的冷门记录并限制最大条目数，避免文件膨胀和分行铺排。
    pub fn export_learning(&self) -> Result<String> {
        let mut words = self.lexicon.words();
        words.sort_by(|left, right| left.1.cmp(&right.1));
        let history = self
            .history
            .read()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        let pairs = self
            .pairs
            .read()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?;
        let (pruned_history, pruned_pairs) =
            crate::learn::pruned_history_and_pairs(&history, &pairs);
        let history: Vec<_> = pruned_history
            .into_iter()
            .map(|(word, entry)| (word, heat_value(entry.count), entry.last_used_ms))
            .collect();
        let pairs: Vec<_> = pruned_pairs
            .into_iter()
            .map(|(previous, word, entry)| {
                (previous, word, heat_value(entry.count), entry.last_used_ms)
            })
            .collect();
        Ok(serde_json::to_string(&json!({
            "format": "freetype-learning",
            "version": 1,
            "words": words,
            "history": history,
            "pairs": pairs,
            "boundaries": self.sentence_boundaries(),
            "pending": self.pending_words.read().unwrap().iter().map(|(word,row)|(word.clone(),(row.pinyin.clone(),row.first_ms))).collect::<std::collections::BTreeMap<_,_>>(),
        }))? + "\n")
    }

    /// 校验整份备份后合并；较新的使用记录优先，相同记录不累加。返回更新的记录数。
    pub fn import_learning(&self, text: &str) -> Result<usize> {
        let mut data: Value = serde_json::from_str(text).context("学习备份不是有效的 JSON")?;
        ensure!(
            data["format"] == "freetype-learning",
            "不是 FreeType 学习备份"
        );
        ensure!(data["version"] == 1, "不支持这个学习备份版本");
        let boundaries: crate::sentence::Boundaries = match data.get("boundaries") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => Default::default(),
        };
        ensure!(
            crate::sentence::valid_boundaries(&boundaries),
            "切分记录格式不正确"
        );
        let pending: std::collections::BTreeMap<String, (String, u64)> = match data.get("pending") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => Default::default(),
        };
        ensure!(
            pending.len() <= crate::learn::MAX_HISTORY_WORDS
                && pending
                    .iter()
                    .all(|(word, (code, _))| valid_word(word) && valid_word(code)),
            "待确认词记录不正确"
        );
        let words: Vec<(String, String, f64)> =
            serde_json::from_value(data["words"].take()).context("学习备份中的词条格式不正确")?;
        let entries: Vec<(String, f64, u64)> = serde_json::from_value(data["history"].take())
            .context("学习备份中的使用记录格式不正确")?;
        let pairs: Vec<(String, String, f64, u64)> =
            serde_json::from_value(data["pairs"].take()).context("学习备份中的搭配格式不正确")?;
        ensure!(
            words
                .iter()
                .all(|(pinyin, word, _)| valid_word(pinyin) && valid_word(word)),
            "词条不能为空，也不能包含换行或制表符"
        );
        ensure!(
            entries
                .iter()
                .all(|(word, count, _)| valid_word(word)
                    && (0.0..=f64::from(u32::MAX)).contains(count)),
            "使用记录中的词不正确"
        );
        ensure!(
            pairs
                .iter()
                .all(|(previous, word, count, _)| valid_word(previous)
                    && valid_word(word)
                    && (0.0..=f64::from(u32::MAX)).contains(count)),
            "搭配中的词不正确"
        );

        let mut history = self
            .history
            .write()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        let mut current_pairs = self
            .pairs
            .write()
            .map_err(|_| anyhow::anyhow!("pairs lock poisoned"))?;
        // merge_words 先校验全部读音与权重，再一次更新；其后不再有可能拒绝的输入。
        let mut changed = self.lexicon.merge_words(&words)?;
        for (word, count, last_used_ms) in entries {
            let entry = HistoryEntry {
                count,
                last_used_ms,
            };
            if newer(entry, history.get(&word).copied()) {
                history.insert(word, entry);
                changed += 1;
            }
        }
        for (previous, word, count, last_used_ms) in pairs {
            let entry = HistoryEntry {
                count,
                last_used_ms,
            };
            if newer(entry, current_pairs.get(&previous, &word)) {
                current_pairs.insert(previous, word, entry);
                changed += 1;
            }
        }
        crate::learn::prune_learning(&mut history, &mut current_pairs);
        let mut current = self.boundaries.write().unwrap();
        for (key, row) in boundaries {
            if current.get(&key).is_none_or(|v| row.1 > v.1)
                && (current.len() < 2000 || current.contains_key(&key))
            {
                current.insert(key, row);
                changed += 1;
            }
        }
        let mut current_pending = self.pending_words.write().unwrap();
        for (word, (pinyin, first_ms)) in pending {
            if crate::now_ms().saturating_sub(first_ms) > crate::learn::NEW_WORD_TTL_MS {
                continue;
            }
            if current_pending
                .get(&word)
                .is_none_or(|r| first_ms > r.first_ms)
                && current_pending.len() < crate::learn::MAX_HISTORY_WORDS
            {
                current_pending.insert(word, crate::learn::NewWord { pinyin, first_ms });
                changed += 1;
            }
        }
        if changed > 0 {
            self.clear_learning_journal();
        }
        Ok(changed)
    }
}
