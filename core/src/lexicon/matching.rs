//! 查询匹配：长词补全顺序、声母过滤与模糊音键。
use super::*;

/// 补全只返回词库已有、一个音节对应一个汉字的更长词，避免混输或符号扩写。
pub(super) fn long_word_matches(
    entry: &LexEntry,
    prefix: &[SyllableKey],
    word_prefix: &str,
) -> bool {
    entry.keys.len() > prefix.len()
        && entry.keys.starts_with(prefix)
        && entry.word.starts_with(word_prefix)
        && entry.word.chars().count() == entry.keys.len()
        && entry.word.chars().all(crate::autospace::is_han)
}

/// 同频时先取短词，再按文字和读音稳定排序，内存与编译词库得到同一结果。
pub(super) fn long_word_order(a: &LexEntry, b: &LexEntry) -> std::cmp::Ordering {
    a.freq
        .total_cmp(&b.freq)
        .then_with(|| b.keys.len().cmp(&a.keys.len()))
        .then_with(|| b.word.cmp(&a.word))
        .then_with(|| b.keys.cmp(&a.keys))
}

/// 查询每个音节的声母（全拼音节取首字母，和词条的 initials 一样）。
pub(super) fn query_initials(query: &[SyllableQuery]) -> Option<Vec<u8>> {
    query
        .iter()
        .map(|q| match q {
            SyllableQuery::Full { id, .. } => Syllable::from_id(*id).map(|s| s.initial_letter()),
            SyllableQuery::Jianpin { letter } => {
                Some(crate::syllable::jianpin_first_letter(*letter))
            }
        })
        .collect()
}

/// 简拼查询要不要逐条核对声母（zh/ch/sh 的简拼只按首字母查不够）。
pub(super) fn needs_initial_check(query: &SyllableQuery) -> bool {
    matches!(query, SyllableQuery::Jianpin { letter }
        if crate::syllable::jianpin_required_initial(*letter).is_some())
}

/// 模糊音查询的归并读音。含简拼、儿化或不参与模糊音的音节时，lookup_fuzzy 什么也不返回，
/// 以它开头的更长查询同样如此，所以返回 None 表示「没有更长的模糊音词」。
pub(crate) fn fuzzy_query_key(query: &[SyllableQuery]) -> Option<Vec<u32>> {
    query
        .iter()
        .map(|q| match *q {
            SyllableQuery::Full { id, erhua: false } => crate::syllable::fuzzy_key(id),
            _ => None,
        })
        .collect()
}

pub(super) fn fuzzy_hash(key: &[u32]) -> u64 {
    key.iter().fold(KEY_HASH_SEED, |hash, id| {
        id.to_le_bytes().iter().fold(hash, |h, &byte| {
            (h ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3)
        })
    })
}

pub(super) fn initial_hash_step(hash: u64, letter: u8) -> u64 {
    (hash ^ u64::from(letter)).wrapping_mul(0x1000_0000_01b3)
}

pub(super) fn entry_matches(entry: &LexEntry, query: &[SyllableQuery]) -> bool {
    if entry.keys.len() != query.len() {
        return false;
    }
    query
        .iter()
        .zip(entry.keys.iter())
        .enumerate()
        .all(|(i, (q, key))| match q {
            SyllableQuery::Full { id, erhua } => key.id == *id && key.erhua == *erhua,
            SyllableQuery::Jianpin { letter } => {
                match crate::syllable::jianpin_required_initial(*letter) {
                    Some(initial) => {
                        Syllable::from_id(key.id).is_some_and(|s| s.initial == initial)
                    }
                    None => entry.initials.get(i) == Some(letter),
                }
            }
        })
}
