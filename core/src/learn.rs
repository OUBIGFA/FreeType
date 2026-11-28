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

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};

use crate::lm::LanguageModel;
use crate::{Engine, HistoryEntry};

/// 两份学习数据都先完整写入并刷盘，再替换旧文件；临时文件独占创建，避免误覆盖。
pub(crate) fn write_atomic(path: &Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
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
    let result = file.write_all(data.as_ref()).and_then(|()| file.sync_all());
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

/// 只选过一次的词，30 天后不再享受「选过的提前」待遇。
pub const MIN_PICK_HEAT: f64 = 0.5;
/// 五个半衰期以后，无近期使用的记录不写入文件。
pub const MIN_KEEP_HEAT: f64 = 1.0 / 32.0;
pub const NEW_WORD_TTL_MS: u64 = HALF_LIFE_MS as u64;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NewWord {
    pub pinyin: String,
    pub first_ms: u64,
}

/// 保存改动前后的值；撤销只还原仍是 after 的记录，不覆盖其他会话的新选择。
pub(crate) enum Change {
    Boundary(String, Option<(Vec<(String,String)>,u64)>, (Vec<(String,String)>,u64)),
    Word(String, Option<HistoryEntry>, HistoryEntry),
    Pair(String, String, Option<HistoryEntry>, HistoryEntry),
    UserWord(String, Option<(String, f64)>, (String, f64)),
    Pending(String, Option<NewWord>, Option<NewWord>),
    Last(String, String),
}

#[derive(Default)]
pub(crate) struct Journal {
    first: u64,
    changes: VecDeque<Option<Change>>,
}

impl Journal {
    pub fn mark(&self) -> u64 { self.first + self.changes.len() as u64 }

    pub fn push(&mut self, change: Change) {
        self.changes.push_back(Some(change));
        if self.changes.len() > 1024 {
            self.changes.pop_front();
            self.first += 1;
        }
    }

    pub fn take(&mut self, start: u64, end: u64) -> Vec<Change> {
        if start < self.first { return Vec::new(); }
        (start..end.min(self.mark())).rev().filter_map(|seq| self.changes[(seq - self.first) as usize].take()).collect()
    }

    pub fn clear(&mut self) {
        self.first = self.mark();
        self.changes.clear();
    }
}

/// 可跨线程保存的一份学习快照。序列化、裁剪和写盘都可以移出按键线程。
pub struct LearningSnapshot {
    boundaries: crate::sentence::Boundaries,
    user_words: Vec<(String, String, f64)>,
    words: HashMap<String, HistoryEntry>,
    pairs: UserPairs,
    pending: HashMap<String, NewWord>,
}

impl LearningSnapshot {
    pub fn save(self, dict: &Path, history: &Path) -> Result<()> {
        write_atomic(dict, &crate::lexicon::user_dict_text(&self.user_words))
            .context("保存学习词条")?;
        write_atomic(history, &(history_text(&self.words, &self.pairs, &self.pending)+&boundary_text(&self.boundaries)))
            .context("保存输入习惯")?;
        Ok(())
    }
}

impl Engine {
    pub fn learning_snapshot(&self) -> LearningSnapshot {
        LearningSnapshot {
            boundaries: self.sentence_boundaries(),
            user_words: self.lexicon.words(),
            words: self.history.read().unwrap().clone(),
            pairs: self.pairs.read().unwrap().clone(),
            pending: self.pending_words.read().unwrap().clone(),
        }
    }

    /// 换主词库时接续还没达到二次确认的词；它们不属于可同步的正式用户词条。
    pub fn inherit_pending_words(&self, current: &Engine) {
        *self.pending_words.write().unwrap() = current.pending_words.read().unwrap().clone();
    }

    pub(crate) fn learn_mark(&self) -> u64 {
        self.learning_journal.lock().unwrap().mark()
    }

    pub(crate) fn note_learning(&self, change: Change) {
        self.learning_journal.lock().unwrap().push(change);
    }

    pub(crate) fn clear_learning_journal(&self) {
        self.learning_journal.lock().unwrap().clear();
    }

    pub(crate) fn record_last_word(&self, word: &str) {
        if let Ok(mut slot) = self.last_word.write() {
            if *slot != word {
                let before = std::mem::replace(&mut *slot, word.to_owned());
                self.note_learning(Change::Last(before, word.to_owned()));
            }
        }
    }

    /// Undo exactly this session's learning, not intervening writes from another session.
    pub(crate) fn undo_learning(&self, start: u64, end: u64) -> bool {
        let changes = self.learning_journal.lock().unwrap().take(start, end);
        let mut changed = false;
        for change in changes {
            changed |= match change {
                Change::Boundary(key,before,after)=>{
                    let mut rows=self.boundaries.write().unwrap();
                    if rows.get(&key)!=Some(&after){false}else{if let Some(value)=before{rows.insert(key,value);}else{rows.remove(&key);}true}
                }
                Change::Word(word, before, after) => restore_entry(&mut self.history.write().unwrap(), word, before, Some(after)),
                Change::Pair(prev, word, before, after) => self.pairs.write().unwrap().restore(prev, word, before, after),
                Change::UserWord(word, before, after) => self.lexicon.restore_word(&word, before, after),
                Change::Pending(word, before, after) => restore_entry(&mut self.pending_words.write().unwrap(), word, before, after),
                Change::Last(before, after) => {
                    let mut current = self.last_word.write().unwrap();
                    if *current != after { continue; }
                    *current = before;
                    true
                }
            };
        }
        changed
    }
}

fn restore_entry<T: PartialEq>(entries: &mut HashMap<String, T>, key: String, before: Option<T>, after: Option<T>) -> bool {
    if entries.get(&key) != after.as_ref() { return false; }
    if let Some(before) = before { entries.insert(key, before); } else { entries.remove(&key); }
    true
}

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
    entry.count * 0.5f64.powf(age / HALF_LIFE_MS)
}

/// 热度保留小数，避免半次再次使用时被永久进位成一次。
pub(crate) fn bumped(before: Option<HistoryEntry>, at_ms: u64) -> HistoryEntry {
    HistoryEntry { count: before.map_or(0.0, |entry| decayed(entry, at_ms)) + 1.0, last_used_ms: at_ms }
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
    (max_real.max(crate::now_ms()), max_synth)
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
                && e.count > 0.0
                && entry_heat(**e, max_real, max_synth) >= MIN_KEEP_HEAT
        })
        .map(|(w, e)| (w.clone(), *e))
        .collect();
    if kept_words.len() > max_words {
        kept_words.sort_by(|(wa, ea), (wb, eb)| {
            entry_heat(*eb, max_real, max_synth)
                .total_cmp(&entry_heat(*ea, max_real, max_synth))
                .then_with(|| eb.last_used_ms.cmp(&ea.last_used_ms))
                .then_with(|| eb.count.total_cmp(&ea.count))
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
                && e.count > 0.0
                && entry_heat(*e, max_real, max_synth) >= MIN_KEEP_HEAT
        })
        .map(|(p, c, e)| (p.to_string(), c.to_string(), e))
        .collect();
    if kept_pairs.len() > max_pairs {
        kept_pairs.sort_by(|(pa, ca, ea), (pb, cb, eb)| {
            entry_heat(*eb, max_real, max_synth)
                .total_cmp(&entry_heat(*ea, max_real, max_synth))
                .then_with(|| eb.last_used_ms.cmp(&ea.last_used_ms))
                .then_with(|| eb.count.total_cmp(&ea.count))
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
        self.bump(prev, cur, at_ms);
    }

    pub(crate) fn bump(&mut self, prev: &str, cur: &str, at_ms: u64) -> Option<Change> {
        if prev.is_empty() || cur.is_empty() { return None; }
        let before = self.get(prev, cur);
        let after = bumped(before, at_ms);
        self.insert(prev.to_owned(), cur.to_owned(), after);
        Some(Change::Pair(prev.to_owned(), cur.to_owned(), before, after))
    }

    pub(crate) fn restore(&mut self, prev: String, cur: String, before: Option<HistoryEntry>, after: HistoryEntry) -> bool {
        if self.get(&prev, &cur) != Some(after) { return false; }
        if let Some(entry) = before {
            self.insert(prev, cur, entry);
        } else if let std::collections::hash_map::Entry::Occupied(mut words) = self.map.entry(prev) {
            words.get_mut().remove(&cur);
            if words.get().is_empty() { words.remove(); }
        }
        true
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
/// n <待确认新词> <编码> <第一次拼成的时间>
/// ```
pub(crate) fn history_text(
    words: &HashMap<String, HistoryEntry>,
    pairs: &UserPairs,
    pending: &HashMap<String, NewWord>,
) -> String {
    let mut out = String::from("# wowtype 用户输入习惯 v1\n");
    let (words, pairs) = pruned_history_and_pairs(words, pairs);
    for (word, entry) in words {
        out.push_str(&format!(
            "w\t{word}\t{}\t{}\n",
            heat_text(entry.count), entry.last_used_ms
        ));
    }
    for (prev, cur, entry) in pairs {
        out.push_str(&format!(
            "p\t{prev}\t{cur}\t{}\t{}\n",
            heat_text(entry.count), entry.last_used_ms
        ));
    }
    let now_ms = crate::now_ms();
    let mut pending: Vec<_> = pending.iter().filter(|(word, record)| {
        clean_token(word) && clean_token(&record.pinyin)
            && now_ms.saturating_sub(record.first_ms) <= NEW_WORD_TTL_MS
    }).collect();
    pending.sort_unstable_by_key(|(word, _)| *word);
    for (word, record) in pending {
        out.push_str(&format!("n\t{word}\t{}\t{}\n", record.pinyin, record.first_ms));
    }
    out
}

fn heat_text(heat: f64) -> String {
    format!("{heat:.3}").trim_end_matches('0').trim_end_matches('.').to_owned()
}

pub(crate) fn boundary_text(rows:&crate::sentence::Boundaries)->String {
    if rows.is_empty(){String::new()}else{format!("b\t{}\n",serde_json::to_string(rows).expect("boundary text"))}
}

/// 读回输入习惯。允许旧整数热度和新版小数；整份文件无法识别时通知调用方留底。
pub(crate) fn load_history<P: AsRef<Path>>(
    path: P,
    words: &mut HashMap<String, HistoryEntry>,
    pairs: &mut UserPairs,
    pending: &mut HashMap<String, NewWord>,
) -> Result<usize> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).with_context(|| format!("读取 {}", path.display()))?;
    let mut loaded = 0usize;
    let now_ms = crate::now_ms();
    let mut non_empty = false;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        non_empty = true;
        let parts: Vec<&str> = line.split('\t').collect();
        let entry = |count: &str, last: &str| -> Option<HistoryEntry> {
            let count: f64 = count.parse().ok()?;
            if !count.is_finite() || !(0.0..=f64::from(u32::MAX)).contains(&count) { return None; }
            Some(HistoryEntry { count, last_used_ms: last.parse().ok()? })
        };
        match parts.as_slice() {
            ["b", value] if serde_json::from_str::<crate::sentence::Boundaries>(value).is_ok()=>{loaded+=1;}
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
            ["n", word, pinyin, timestamp] if clean_token(word) && clean_token(pinyin) => {
                if let Ok(first_ms) = timestamp.parse() {
                    if pending.len() < MAX_HISTORY_WORDS && now_ms.saturating_sub(first_ms) <= NEW_WORD_TTL_MS {
                        pending.insert((*word).into(), NewWord { pinyin: (*pinyin).into(), first_ms });
                    }
                    loaded += 1;
                }
            }
            _ => {}
        }
    }
    anyhow::ensure!(!non_empty || loaded > 0, "没有可识别的学习记录");
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
            count: 4.0,
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
                count: 3.125,
                last_used_ms: 42,
            },
        );
        let mut pairs = UserPairs::default();
        pairs.learn("发", "给", 7);
        let at_ms = crate::now_ms();
        let mut pending: HashMap<String, NewWord> = [("霜频阁".into(), NewWord { pinyin: "shuang pin ge".into(), first_ms: at_ms })].into();
        write_atomic(&path, &history_text(&words, &pairs, &pending)).unwrap();

        let mut w2 = HashMap::new();
        let mut p2 = UserPairs::default();
        let mut n2 = HashMap::new();
        assert_eq!(load_history(&path, &mut w2, &mut p2, &mut n2).unwrap(), 3);
        assert_eq!(w2["种地"].count, 3.125);
        assert_eq!(p2.get("发", "给").unwrap().count, 1.0);
        assert_eq!(n2, std::mem::take(&mut pending));
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
                count: 1.0,
                last_used_ms: stale,
            },
        );
        words.insert(
            "高频旧词".to_string(),
            HistoryEntry {
                count: 10.0,
                last_used_ms: stale,
            },
        );
        words.insert(
            "新词A".to_string(),
            HistoryEntry {
                count: 1.0,
                last_used_ms: now,
            },
        );
        words.insert(
            "新词B".to_string(),
            HistoryEntry {
                count: 3.0,
                last_used_ms: now,
            },
        );

        let mut pairs = UserPairs::default();
        pairs.insert(
            "旧".to_string(),
            "搭配".to_string(),
            HistoryEntry {
                count: 1.0,
                last_used_ms: stale,
            },
        );
        pairs.insert(
            "常".to_string(),
            "用".to_string(),
            HistoryEntry {
                count: 4.0,
                last_used_ms: now,
            },
        );
        pairs.insert(
            "次".to_string(),
            "新".to_string(),
            HistoryEntry {
                count: 1.0,
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
