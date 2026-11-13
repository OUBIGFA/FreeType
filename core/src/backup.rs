//! 完整学习备份：直接读取内存中的词、热度和搭配，不等待定时存盘。

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

use crate::{Engine, HistoryEntry};

fn valid_word(word: &str) -> bool {
    !word.trim().is_empty() && !word.contains(['\t', '\r', '\n'])
}

fn newer(incoming: HistoryEntry, current: Option<HistoryEntry>) -> bool {
    current.map_or(true, |current| {
        (incoming.last_used_ms, incoming.count) > (current.last_used_ms, current.count)
    })
}

impl Engine {
    /// 导出 UTF-8 JSON v1：用户词、选词次数/时间和词搭配；不含词库、设置和剪贴板。
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
        let mut history: Vec<_> = history
            .iter()
            .map(|(word, entry)| (word, entry.count, entry.last_used_ms))
            .collect();
        history.sort();
        let mut pairs: Vec<_> = pairs
            .iter()
            .map(|(previous, word, entry)| (previous, word, entry.count, entry.last_used_ms))
            .collect();
        pairs.sort();
        Ok(serde_json::to_string_pretty(&json!({
            "format": "freetype-learning",
            "version": 1,
            "words": words,
            "history": history,
            "pairs": pairs,
        }))? + "\n")
    }

    /// 校验整份备份后合并；较新的使用记录优先，相同记录不累加。返回更新的记录数。
    pub fn import_learning(&self, text: &str) -> Result<usize> {
        let mut data: Value = serde_json::from_str(text).context("学习备份不是有效的 JSON")?;
        ensure!(data["format"] == "freetype-learning", "不是 FreeType 学习备份");
        ensure!(data["version"] == 1, "不支持这个学习备份版本");
        let words: Vec<(String, String, f64)> = serde_json::from_value(data["words"].take())
            .context("学习备份中的词条格式不正确")?;
        let entries: Vec<(String, u32, u64)> = serde_json::from_value(data["history"].take())
            .context("学习备份中的使用记录格式不正确")?;
        let pairs: Vec<(String, String, u32, u64)> = serde_json::from_value(data["pairs"].take())
            .context("学习备份中的搭配格式不正确")?;
        ensure!(
            words.iter().all(|(pinyin, word, _)| valid_word(pinyin) && valid_word(word)),
            "词条不能为空，也不能包含换行或制表符"
        );
        ensure!(
            entries.iter().all(|(word, _, _)| valid_word(word)),
            "使用记录中的词不正确"
        );
        ensure!(
            pairs.iter().all(|(previous, word, _, _)| valid_word(previous) && valid_word(word)),
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
            let entry = HistoryEntry { count, last_used_ms };
            if newer(entry, history.get(&word).copied()) {
                history.insert(word, entry);
                changed += 1;
            }
        }
        for (previous, word, count, last_used_ms) in pairs {
            let entry = HistoryEntry { count, last_used_ms };
            if newer(entry, current_pairs.get(&previous, &word)) {
                current_pairs.insert(previous, word, entry);
                changed += 1;
            }
        }
        Ok(changed)
    }
}
