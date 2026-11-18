// Modified by FreeType; compare with WowType 5656e136 for the original implementation.
//! 从用户自己的输入里学：选过的词有多「热」，哪两个词常挨着用。
//!
//! 照着微信输入法的思路（`SelectedHistory`、`history_hot_val`、`context_bigram_score`）：
//!
//! - 热度：每选一次加 1，按 30 天半衰期衰减（WeType 超参数里的 2592000 秒）。
//!   排序用 ln(1+热度)，选得越多越靠前，但涨得越来越慢；很久不用就慢慢退回去。
//! - 搭配：用户上屏时记下「上一个词 -> 这个词」（同一句里相邻的词，以及上一次上屏的
//!   最后一个词 -> 这次的第一个词）。束搜索给这些搭配加分，所以 `fagei` 选过一次
//!   「发给」以后，`wofageini` 出「我发给你」而不是「我发个你」。
//!
//! 都存在 `history.tsv`（和用户词库放一起），重启输入法不会丢。

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::lm::LanguageModel;
use crate::HistoryEntry;

/// 两份学习数据都先完整写入并刷盘，再替换旧文件；临时文件独占创建，避免误覆盖。
pub(crate) fn write_atomic(path: &Path, data: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let filename = path.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "缺少文件名"))?;
    let (temporary, mut file) = loop {
        let mut name = std::ffi::OsString::from(".");
        name.push(filename);
        name.push(format!(".{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let temporary = path.with_file_name(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = file.write_all(data.as_bytes()).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = result {
        if let Err(cleanup) = std::fs::remove_file(&temporary) {
            return Err(std::io::Error::new(error.kind(), format!("{error}；临时文件清理失败：{cleanup}")));
        }
        return Err(error);
    }
    Ok(())
}

/// 半衰期：30 天（毫秒）。
pub const HALF_LIFE_MS: f64 = 30.0 * 24.0 * 3600.0 * 1000.0;

/// 单词热度记录上限：日常活跃词通常数千条，2 万条足以覆盖长期高频词汇。
pub const MAX_HISTORY_WORDS: usize = 20_000;

/// 上下文二元搭配上限：超出时按当前衰减热度与最近使用时间淘汰冷门搭配。
pub const MAX_USER_PAIRS: usize = 50_000;

/// 衰减淘汰阈值：`0.5^6 = 1/64`（仅选过 1 次且超过 6 个半衰期即 180 天未再使用的记录，
/// 衰减后热度低于此值，加分接近 0 且再次选中时保留次数已四舍五入为 0，存盘与导出时予以清理）。
pub const MIN_KEEP_HEAT: f64 = 1.0 / 64.0;

/// 区分真实 Unix 毫秒时间戳（2001-09-09 之后）与单测合成小整数时间戳，
/// 避免测试混用 `now_ms()` 与常量时间戳时把合成时间误判为几十年前的过期记录。
const REAL_EPOCH_FLOOR_MS: u64 = 1_000_000_000_000;

/// 一个学过的搭配给多少分（自然对数）：`PAIR_WEIGHT * ln(1+热度)`，最多 [`PAIR_BONUS_CAP`]。
///
/// 用过一次约 1.7，三次约 3.5——足够让常见的同音搭配（发给/发个、再/在）翻过来，
/// 又不至于让一次误选永远压住词库里差得很远的词。
pub const PAIR_WEIGHT: f64 = 2.5;
pub const PAIR_BONUS_CAP: f64 = 6.0;

/// 按半衰期衰减后的次数。
pub fn decayed(entry: HistoryEntry, at_ms: u64) -> f64 {
    let age = at_ms.saturating_sub(entry.last_used_ms) as f64;
    f64::from(entry.count) * 0.5f64.powf(age / HALF_LIFE_MS)
}

fn clean_token(s: &str) -> bool {
    !s.trim().is_empty() && !s.contains(['\t', '\n', '\r'])
}

fn reference_clocks(
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
) -> (u64, u64) {
    let mut max_real = 0u64;
    let mut max_synth = 0u64;
    for ts in words
        .values()
        .map(|e| e.last_used_ms)
        .chain(pairs.iter().map(|(_, _, e)| e.last_used_ms))
    {
        if ts >= REAL_EPOCH_FLOOR_MS {
            max_real = max_real.max(ts);
        } else {
            max_synth = max_synth.max(ts);
        }
    }
    (max_real, max_synth)
}

fn entry_heat(entry: HistoryEntry, max_real: u64, max_synth: u64) -> f64 {
    let at_ms = if entry.last_used_ms >= REAL_EPOCH_FLOOR_MS {
        max_real
    } else {
        max_synth
    };
    decayed(entry, at_ms)
}

/// 按衰减阈值与容量上限裁剪词热度与二元搭配，返回按字典序排好、可直接序列化或写盘的列表。
pub(crate) fn pruned_history_and_pairs(
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
) -> (
    Vec<(String, HistoryEntry)>,
    Vec<(String, String, HistoryEntry)>,
) {
    pruned_with_limits(words, pairs, MAX_HISTORY_WORDS, MAX_USER_PAIRS)
}

fn pruned_with_limits(
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
    max_words: usize,
    max_pairs: usize,
) -> (
    Vec<(String, HistoryEntry)>,
    Vec<(String, String, HistoryEntry)>,
) {
    let (max_real, max_synth) = reference_clocks(words, pairs);
    let mut kept_words: Vec<(String, HistoryEntry)> = words
        .iter()
        .filter(|(w, e)| {
            clean_token(w)
                && e.count > 0
                && entry_heat(**e, max_real, max_synth) >= MIN_KEEP_HEAT
        })
        .map(|(w, e)| (w.clone(), *e))
        .collect();
    if kept_words.len() > max_words {
        kept_words.sort_by(|(wa, ea), (wb, eb)| {
            entry_heat(*eb, max_real, max_synth)
                .total_cmp(&entry_heat(*ea, max_real, max_synth))
                .then_with(|| eb.last_used_ms.cmp(&ea.last_used_ms))
                .then_with(|| eb.count.cmp(&ea.count))
                .then_with(|| wa.cmp(wb))
        });
        kept_words.truncate(max_words);
    }
    kept_words.sort_by(|a, b| a.0.cmp(&b.0));

    let mut kept_pairs: Vec<(String, String, HistoryEntry)> = pairs
        .iter()
        .filter(|(p, c, e)| {
            clean_token(p)
                && clean_token(c)
                && e.count > 0
                && entry_heat(*e, max_real, max_synth) >= MIN_KEEP_HEAT
        })
        .map(|(p, c, e)| (p.to_string(), c.to_string(), e))
        .collect();
    if kept_pairs.len() > max_pairs {
        kept_pairs.sort_by(|(pa, ca, ea), (pb, cb, eb)| {
            entry_heat(*eb, max_real, max_synth)
                .total_cmp(&entry_heat(*ea, max_real, max_synth))
                .then_with(|| eb.last_used_ms.cmp(&ea.last_used_ms))
                .then_with(|| eb.count.cmp(&ea.count))
                .then_with(|| (pa, ca).cmp(&(pb, cb)))
        });
        kept_pairs.truncate(max_pairs);
    }
    kept_pairs.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    (kept_words, kept_pairs)
}

/// 就地清理已衰减过期的词热度和搭配，并将条目数限制在上限之内。
pub fn prune_learning(words: &mut HashMap<String, HistoryEntry>, pairs: &mut UserPairs) {
    let (kept_words, kept_pairs) = pruned_history_and_pairs(words, pairs);
    words.clear();
    words.extend(kept_words);
    *pairs = UserPairs::default();
    for (prev, cur, entry) in kept_pairs {
        pairs.insert(prev, cur, entry);
    }
}

/// 用户学到的词搭配：前一个词 -> 后一个词 -> 次数与上次使用时间。
#[derive(Default, Debug, Clone)]
pub struct UserPairs {
    map: HashMap<String, HashMap<String, HistoryEntry>>,
}

impl UserPairs {
    pub fn learn(&mut self, prev: &str, cur: &str, at_ms: u64) {
        if prev.is_empty() || cur.is_empty() {
            return;
        }
        let entry = self
            .map
            .entry(prev.to_string())
            .or_default()
            .entry(cur.to_string())
            .or_insert(HistoryEntry {
                count: 0,
                last_used_ms: at_ms,
            });
        // 次数先按时间衰减再加 1，这样很久以前用过很多次的搭配不会一直顶着
        let kept = decayed(*entry, at_ms).round() as u32;
        entry.count = kept.saturating_add(1);
        entry.last_used_ms = at_ms;
    }

    pub fn get(&self, prev: &str, cur: &str) -> Option<HistoryEntry> {
        self.map.get(prev).and_then(|next| next.get(cur)).copied()
    }

    /// 这个搭配的加分（没学过是 0）。
    pub fn bonus(&self, prev: &str, cur: &str, at_ms: u64) -> f64 {
        match self.get(prev, cur) {
            Some(entry) => (PAIR_WEIGHT * (1.0 + decayed(entry, at_ms)).ln()).min(PAIR_BONUS_CAP),
            None => 0.0,
        }
    }

    /// prev 后面学过的词（上屏联想按它们的热度加分）。
    pub fn after(&self, prev: &str) -> impl Iterator<Item = (&str, HistoryEntry)> {
        self.map
            .get(prev)
            .into_iter()
            .flat_map(|next| next.iter().map(|(cur, entry)| (cur.as_str(), *entry)))
    }

    /// 去掉和 word 有关的搭配（它在前或在后的都去掉），返回去掉了几条。
    pub fn remove_word(&mut self, word: &str) -> usize {
        let mut removed = self.map.remove(word).map_or(0, |next| next.len());
        for next in self.map.values_mut() {
            removed += usize::from(next.remove(word).is_some());
        }
        self.map.retain(|_, next| !next.is_empty());
        removed
    }

    pub fn len(&self) -> usize {
        self.map.values().map(HashMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, HistoryEntry)> {
        self.map.iter().flat_map(|(prev, next)| {
            next.iter()
                .map(move |(cur, entry)| (prev.as_str(), cur.as_str(), *entry))
        })
    }

    pub(crate) fn insert(&mut self, prev: String, cur: String, entry: HistoryEntry) {
        self.map.entry(prev).or_default().insert(cur, entry);
    }
}

/// 基础语言模型 + 用户学到的搭配。束搜索用它打分。
///
/// `start` 是光标前最后一个词（上一次上屏的），句首的第一个词用它查搭配。
pub struct LearnedLm<'a> {
    pub base: &'a dyn LanguageModel,
    pub pairs: &'a UserPairs,
    pub start: Option<&'a str>,
    pub at_ms: u64,
    /// 光标前已经有字（接着前文打）：第一个词不是句首，不加基础模型的句首分。
    pub mid_sentence: bool,
}

impl LanguageModel for LearnedLm<'_> {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        let base = if prev.is_none() && self.mid_sentence {
            0.0
        } else {
            self.base.score(prev, cur)
        };
        match prev.or(self.start) {
            Some(prev) => base + self.pairs.bonus(prev, cur, self.at_ms),
            None => base,
        }
    }

    fn has_bigram(&self, prev: &str, cur: &str) -> bool {
        self.base.has_bigram(prev, cur) || self.pairs.get(prev, cur).is_some()
    }

    fn top_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        self.base.top_next(prev, limit)
    }
}

/// 存盘格式（TSV，`#` 开头是注释，各列用 tab 分隔）：
///
/// ```text
/// w <词> <次数> <上次使用，epoch 毫秒>
/// p <前一个词> <后一个词> <次数> <上次使用>
/// ```
pub fn save_history<P: AsRef<Path>>(
    path: P,
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
) -> Result<()> {
    let path = path.as_ref();
    let mut out = String::from("# wowtype 用户输入习惯 v1\n");
    let (words, pairs) = pruned_history_and_pairs(words, pairs);
    for (word, entry) in words {
        out.push_str(&format!(
            "w\t{word}\t{}\t{}\n",
            entry.count, entry.last_used_ms
        ));
    }
    for (prev, cur, entry) in pairs {
        out.push_str(&format!(
            "p\t{prev}\t{cur}\t{}\t{}\n",
            entry.count, entry.last_used_ms
        ));
    }
    write_atomic(path, &out).with_context(|| format!("保存 {}", path.display()))?;
    Ok(())
}

/// 读回 [`save_history`] 写的文件，返回读到的条数。坏行跳过，读完后执行衰减裁剪与容量上限整理。
pub fn load_history<P: AsRef<Path>>(
    path: P,
    words: &mut HashMap<String, HistoryEntry>,
    pairs: &mut UserPairs,
) -> Result<usize> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).with_context(|| format!("读取 {}", path.display()))?;
    let mut loaded = 0usize;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        let entry = |count: &str, last: &str| -> Option<HistoryEntry> {
            Some(HistoryEntry {
                count: count.parse().ok()?,
                last_used_ms: last.parse().ok()?,
            })
        };
        match parts.as_slice() {
            ["w", word, count, last] if !word.is_empty() => {
                if let Some(e) = entry(count, last) {
                    words.insert((*word).to_string(), e);
                    loaded += 1;
                }
            }
            ["p", prev, cur, count, last] if !prev.is_empty() && !cur.is_empty() => {
                if let Some(e) = entry(count, last) {
                    pairs.insert((*prev).to_string(), (*cur).to_string(), e);
                    loaded += 1;
                }
            }
            _ => {}
        }
    }
    prune_learning(words, pairs);
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lm::BigramLm;

    #[test]
    fn heat_decays_with_half_life() {
        let e = HistoryEntry {
            count: 4,
            last_used_ms: 0,
        };
        assert!((decayed(e, 0) - 4.0).abs() < 1e-9);
        assert!((decayed(e, HALF_LIFE_MS as u64) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn learned_pair_adds_to_score_and_uses_start_word() {
        let mut pairs = UserPairs::default();
        pairs.learn("发", "给", 1000);
        let base = BigramLm::default();
        let lm = LearnedLm {
            base: &base,
            pairs: &pairs,
            start: Some("发"),
            at_ms: 1000,
            mid_sentence: false,
        };
        assert!(lm.score(Some("发"), "给") > lm.score(Some("发"), "个"));
        // 句首没有前一个词时用 start
        assert!(lm.score(None, "给") > lm.score(None, "个"));
    }

    #[test]
    fn history_round_trips() {
        let dir = std::env::temp_dir().join(format!("wowtype-learn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.tsv");
        let mut words = HashMap::new();
        words.insert(
            "种地".to_string(),
            HistoryEntry {
                count: 3,
                last_used_ms: 42,
            },
        );
        let mut pairs = UserPairs::default();
        pairs.learn("发", "给", 7);
        save_history(&path, &words, &pairs).unwrap();

        let mut w2 = HashMap::new();
        let mut p2 = UserPairs::default();
        assert_eq!(load_history(&path, &mut w2, &mut p2).unwrap(), 2);
        assert_eq!(w2["种地"].count, 3);
        assert_eq!(p2.get("发", "给").unwrap().count, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_learning_drops_decayed_entries_and_enforces_capacity_limits() {
        let now = 10 * HALF_LIFE_MS as u64;
        let stale = now - 7 * HALF_LIFE_MS as u64; // 7 个半衰期：1 次的热度 1/128 < 1/64
        let mut words = HashMap::new();
        words.insert(
            "过期偶发词".to_string(),
            HistoryEntry {
                count: 1,
                last_used_ms: stale,
            },
        );
        words.insert(
            "高频旧词".to_string(),
            HistoryEntry {
                count: 10,
                last_used_ms: stale,
            },
        );
        words.insert(
            "新词A".to_string(),
            HistoryEntry {
                count: 1,
                last_used_ms: now,
            },
        );
        words.insert(
            "新词B".to_string(),
            HistoryEntry {
                count: 3,
                last_used_ms: now,
            },
        );

        let mut pairs = UserPairs::default();
        pairs.insert(
            "旧".to_string(),
            "搭配".to_string(),
            HistoryEntry {
                count: 1,
                last_used_ms: stale,
            },
        );
        pairs.insert(
            "常".to_string(),
            "用".to_string(),
            HistoryEntry {
                count: 4,
                last_used_ms: now,
            },
        );
        pairs.insert(
            "次".to_string(),
            "新".to_string(),
            HistoryEntry {
                count: 1,
                last_used_ms: now,
            },
        );

        // 先验证衰减淘汰
        let (kept_w, kept_p) = pruned_history_and_pairs(&words, &pairs);
        let word_names: Vec<_> = kept_w.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(word_names, vec!["新词A", "新词B", "高频旧词"]);
        let pair_names: Vec<_> = kept_p.iter().map(|(p, c, _)| (p.as_str(), c.as_str())).collect();
        assert_eq!(pair_names, vec![("常", "用"), ("次", "新")]);

        // 再验证容量上限裁剪：只保留热度最高的条目
        let (capped_w, capped_p) = pruned_with_limits(&words, &pairs, 2, 1);
        let capped_word_names: Vec<_> = capped_w.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(capped_word_names, vec!["新词A", "新词B"]);
        assert_eq!(capped_p.len(), 1);
        assert_eq!((capped_p[0].0.as_str(), capped_p[0].1.as_str()), ("常", "用"));
    }
}
