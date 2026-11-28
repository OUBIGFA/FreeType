//! 候选形态判断：音节边界、简拼、整句截取与去重。
use super::*;

/// 句首词只在多音节输入里有意义，而且只能在音节边界上切。
///
/// 单音节的 `dan` 不该出「大」（da + 剩个 n），`xian` 不该出「下」：
/// 那是把一个打完的音节拆开了，只会把第一页塞满没用的字。
/// 第一个整句候选是精确全拼时，句首词必须正好用完它的前几个音节；
/// 是简拼或纠错结果时没法对齐字母，只要求它至少两个音节。
///
/// 返回被筛掉、但换一种切法也说得通的词：词本身是完整全拼，剩下的字母也能整个切成
/// 音节（houxuanci 里的「后续」= hou xu | an ci）。它们不该挤进前几页，长尾最后补上。
pub(super) fn prefix_on_syllable_boundaries(
    result: &[Candidate],
    normalized: &str,
    prefix: &mut Vec<Candidate>,
) -> Vec<Candidate> {
    let Some(top) = result.iter().find(|c| !is_raw_candidate(c)) else {
        prefix.clear();
        return Vec::new();
    };
    let syllables: Vec<&str> = top.pinyin.split_whitespace().collect();
    if syllables.len() < 2 {
        prefix.clear();
        return Vec::new();
    }
    let letters: String = normalized
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    if top.features.full_jianpin > 0.0 || syllables.concat().len() > letters.len() + 2 {
        // 简拼（zgrm）字母和音节对不上，不按边界筛
        return Vec::new();
    }
    // 从头对齐：补全和纠错只改动对不上的那一段，前面对得上的音节边界照样可信
    let mut boundaries = Vec::new();
    let mut at = 0;
    for syllable in &syllables[..syllables.len() - 1] {
        if !letters[at..].starts_with(syllable) {
            break;
        }
        at += syllable.len();
        boundaries.push(at);
    }
    let exact = top.features.typo_count == 0.0;
    let mut other_splits = Vec::new();
    // consumed 此时还是字母数（隔音符也算一个字符）
    prefix.retain(|p| {
        let used = normalized
            .chars()
            .take(p.consumed)
            .filter(|c| c.is_ascii_alphabetic())
            .count();
        if exact && p.features.typo_count > 0.0 {
            return false;
        }
        if boundaries.contains(&used) {
            return true;
        }
        if exact
            && used < letters.len()
            && p.pinyin.split_whitespace().collect::<String>() == letters[..used]
            && splits_into_syllables(&letters[used..])
        {
            other_splits.push(p.clone());
        }
        false
    });
    // 整句候选本身的开头（你好中国 -> 你好）最可能是对的，越长越省事，排最前
    let head = |p: &Candidate| top.text.starts_with(&p.text);
    prefix.sort_by_key(|p| std::cmp::Reverse(if head(p) { p.text.chars().count() } else { 0 }));
    // 长的在前（同样长的保持束搜索给的顺序）
    other_splits.sort_by_key(|p| std::cmp::Reverse(p.text.chars().count()));
    other_splits
}

/// 这串字母能不能整个切成合法的全拼音节（an ci、ci）。
pub(super) fn splits_into_syllables(letters: &str) -> bool {
    let mut ok = vec![false; letters.len() + 1];
    ok[0] = true;
    for end in 1..=letters.len() {
        for len in 1..=6.min(end) {
            let syllable = &letters[end - len..end];
            if ok[end - len] && !matches!(syllable, "ng" | "hm" | "hng") && is_valid(syllable) {
                ok[end] = true;
                break;
            }
        }
    }
    ok[letters.len()]
}

/// 全拼对不上时用的简拼图；走不通时返回 None。
///
/// - 全是声母（zg、jzm）：纯简拼，和以前一样；
/// - 有韵母（kyi、yongzwen）：全拼混简拼，每个音节可以打全也可以只打声母。
///   全拼加补全已经能解释时（打到一半的 yanzhon）不用：结果一样，混简拼的路径却多得多，
///   长句逐键会从几毫秒涨到几十毫秒。`completion_explains` 是「全拼加补全、不靠纠错能走完」。
pub(super) fn abbrev_graph(
    parsed: &crate::syllable::ParsedInput,
    completion_explains: bool,
) -> Option<SyllableGraph> {
    let all_initials = parsed
        .letters
        .iter()
        .all(|&b| crate::syllable::is_jianpin_letter(b));
    if !all_initials && completion_explains {
        return None;
    }
    let graph = SyllableGraph::build(
        parsed,
        GraphOptions {
            mode: if all_initials {
                GraphMode::Jianpin
            } else {
                GraphMode::Mixed
            },
            allow_erhua: false,
            typo: false,
            completion: false,
        },
    );
    graph.has_complete_path().then_some(graph)
}

/// 全拼和只打声母的音节混着的候选（k yi -> 可以，yong z wen -> 用中文）。
pub(super) fn is_mixed_jianpin(c: &Candidate) -> bool {
    c.features.full_jianpin > 0.0
        && c.pinyin
            .split_whitespace()
            .any(|s| !crate::syllable::is_abbrev_syllable(s))
}

/// 最后一个音节之前就有只打了声母的音节。
pub(super) fn abbrev_before_last(c: &Candidate) -> bool {
    let syllables: Vec<&str> = c.pinyin.split_whitespace().collect();
    syllables.len() >= 2
        && syllables[..syllables.len() - 1]
            .iter()
            .any(|s| crate::syllable::is_abbrev_syllable(s))
}

/// 原始串 / 英文候选（append_raw_candidate 插进来的）。
pub(super) fn is_raw_candidate(c: &Candidate) -> bool {
    c.words.len() == 1 && c.words[0].pinyin.is_empty() && c.features.coverage_score == 0.0
}

/// 整个输入是不是只有一个音节、两个音节……（按第一个非原始串候选的拼音数）。
pub(super) fn syllable_count(result: &[Candidate]) -> usize {
    result
        .iter()
        .find(|c| !is_raw_candidate(c))
        .map(|c| c.pinyin.split_whitespace().count())
        .unwrap_or(0)
}

/// 整句候选：几个词拼起来的、用完整个输入的候选。
pub(super) fn is_sentence(c: &Candidate) -> bool {
    c.words.len() >= 2 && !is_raw_candidate(c)
}

/// 两个候选只差最后一个字（横向候选词 / 横向候选此）。
///
/// 这种候选没必要单独占位置：用户选整句的开头（横向候选），再选最后一个字就行。
pub(super) fn differs_only_at_tail(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    a.len() == b.len() && a.len() >= 2 && a[..a.len() - 1] == b[..b.len() - 1]
}

/// 整句候选最多留几个：越长的输入越可能要换个整句，短输入的「整句」多半是两个单字的拼凑。
pub(super) fn sentence_cap(syllables: usize) -> usize {
    match syllables {
        0..=2 => 1,
        3 => 2,
        _ => 3,
    }
}

/// 第一个整句候选去掉最后一个词（横向候选词 -> 横向候选），作为句首候选。
///
/// 整句只是最后一个词错了时，选它再选最后一个词，比翻页找整句快。
/// 只给精确全拼的整句，开头至少两个音节（一个音节的开头本来就在句首词里）。
/// consumed 先存「用掉的字符数」（隔音符算一个），由调用方换算成字节。
pub(super) fn sentence_head(result: &[Candidate], normalized: &str) -> Option<Candidate> {
    let top = result.iter().find(|c| !is_raw_candidate(c))?;
    if !is_sentence(top)
        || top.features.typo_count > 0.0
        || top.features.prefix_completion > 0.0
        || top.features.full_jianpin > 0.0
    {
        return None;
    }
    let head_words = &top.words[..top.words.len() - 1];
    let syllables: Vec<&str> = head_words
        .iter()
        .flat_map(|w| w.pinyin.split_whitespace())
        .collect();
    if syllables.len() < 2 || head_words.iter().any(|w| !w.known) {
        return None;
    }
    // 拼音要和输入从头对得上，才能知道切在第几个字母
    let letters: String = normalized
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let joined = syllables.concat();
    if !letters.starts_with(&joined) {
        return None;
    }
    let mut seen = 0usize;
    let mut consumed = 0usize;
    for (i, ch) in normalized.chars().enumerate() {
        if ch.is_ascii_alphabetic() {
            seen += 1;
        }
        if seen == joined.len() {
            consumed = i + 1;
            break;
        }
    }
    // 用分词（词库里的写法）拼出前半截，不按字数切整句：繁体词组转换可能改变字数
    // （计算机 -> 電腦）。开着繁体时由调用方再转一次
    Some(Candidate {
        text: head_words.iter().map(|w| w.text.as_str()).collect(),
        long_completion: false,
        pinyin: syllables.join(" "),
        score: top.score,
        features: Feature::default(),
        words: head_words.to_vec(),
        consumed,
    })
}

/// 把整句候选和句首候选排成最终列表，总数不超过 max（原始串/英文候选不会被截掉，位置不变）。
///
/// 照着微信输入法的做法：整句只留几个，而且只差最后一个字的整句不留；
/// 然后是整个输入正好是一个词的候选（同音词），再是句首候选——长的在前、单字在后。
/// 长输入（4 个音节以上）的第一个整句常常不全对，最好的句首候选放第二位。
pub(super) fn compose_candidates(result: &mut Vec<Candidate>, prefix: Vec<Candidate>, max: usize) {
    let syllables = syllable_count(result);
    let cap = sentence_cap(syllables);
    // 原始串记下原来的位置，排完再插回去
    let raw: Vec<(usize, Candidate)> = result
        .iter()
        .enumerate()
        .filter(|(_, c)| is_raw_candidate(c))
        .map(|(i, c)| (i, c.clone()))
        .collect();
    let whole: Vec<Candidate> = std::mem::take(result)
        .into_iter()
        .filter(|c| !is_raw_candidate(c))
        .collect();

    let mut kept: Vec<Candidate> = Vec::new();
    let mut sentences = 0usize;
    // 第一名是打全了的、词库里的整词（你好；纠错出来的也算：zhognguo -> 中国）时，比它差
    // JUNK_SENTENCE_MARGIN 以上的整句（泥 | 好、点 | 脑、钟 | 国）是硬凑的，不占位置；用户选过的照留
    let top_word = whole
        .first()
        .filter(|c| c.words.len() == 1 && c.words[0].known && c.features.prefix_completion == 0.0)
        .map(|c| c.score);
    for c in whole {
        if is_sentence(&c) {
            let junk = top_word.is_some_and(|top| top - c.score > JUNK_SENTENCE_MARGIN);
            if junk && c.features.history_use_count == 0.0 {
                continue;
            }
            let first = kept.is_empty();
            if !first
                && (sentences >= cap || kept.iter().any(|k| differs_only_at_tail(&k.text, &c.text)))
            {
                continue;
            }
            sentences += 1;
        }
        kept.push(c);
    }

    let mut seen: HashMap<String, ()> = HashMap::new();
    let prefix: Vec<Candidate> = prefix
        .into_iter()
        .filter(|p| !kept.iter().any(|r| r.text == p.text))
        .filter(|p| seen.insert(p.text.clone(), ()).is_none())
        .collect();

    // 排第一的没纠错时，纠错出来的整句/整词放到句首词后面：`bujianyi` 打的就是
    // 不建议，「古剑一」（b 当 g）不该排在「不见」前面
    let exact_first = kept.first().is_some_and(|c| c.features.typo <= 0.0);
    let (mut kept, mut corrected): (Vec<Candidate>, Vec<Candidate>) = if exact_first {
        kept.into_iter().partition(|c| c.features.typo <= 0.0)
    } else {
        (kept, Vec::new())
    };
    // 但第一名只是几个单字硬凑的整句（tianqu -> 天 | 去），而纠一处错就是个词（天气）时，
    // 这个词放第二位：打的多半是这个词，只是按错了一个键
    let weak_top = kept.first().is_some_and(|c| {
        is_sentence(c)
            && c.features.history_use_count == 0.0
            && c.words
                .iter()
                .all(|w| w.pinyin.split_whitespace().count() == 1)
    });
    if weak_top {
        let top = kept[0].score;
        let found = corrected.iter().position(|c| {
            c.words.len() == 1 && c.features.typo_count == 1.0 && top - c.score <= TYPO_WORD_GAP
        });
        if let Some(i) = found {
            let word = corrected.remove(i);
            kept.insert(1, word);
        }
    }
    let long_sentence = syllables >= 4 && kept.first().map(is_sentence).unwrap_or(false);
    let mut merged: Vec<Candidate> =
        Vec::with_capacity(kept.len() + prefix.len() + corrected.len());
    let mut prefix = prefix.into_iter();
    let mut kept = kept.into_iter();
    if long_sentence {
        merged.extend(kept.by_ref().take(1));
        merged.extend(prefix.by_ref().take(1));
    }
    merged.extend(kept);
    merged.extend(prefix);
    merged.extend(corrected);
    merged.truncate(max.saturating_sub(raw.len()).max(1));
    for (at, c) in raw {
        let at = at.min(merged.len());
        merged.insert(at, c);
    }
    // 分数改成单调递减，后面再按分数重排（繁体转换）顺序也不变
    for i in 1..merged.len() {
        if merged[i].score >= merged[i - 1].score {
            merged[i].score = merged[i - 1].score - 1e-3;
        }
    }
    *result = merged;
}

/// [`compose_candidates`]：整句比排第一的整词差多少（自然对数）就算硬凑的。
/// 实测 你好 −6.6 vs 泥 | 好 −15.5，电脑 −7.6 vs 点 | 脑 −14.8；整句评测不受影响。
pub(super) const JUNK_SENTENCE_MARGIN: f64 = 3.0;

/// [`compose_candidates`]：纠一处错得到的词最多比单字硬凑的第一名差多少，还能排第二。
/// 实测 tianqu：天 | 去 −11.1，天气（纠一处）−15.9，差 4.8。
pub(super) const TYPO_WORD_GAP: f64 = 6.5;

/// 第一个候选是英文词表里的词（iPhone、hello）时，去掉硬解出来的中文：两处纠错的（屁 | 好呢）、
/// 中间夹着只打了声母的音节的（合理[he l] | 了哦）。返回是不是只剩英文了。
pub(super) fn drop_forced_chinese(result: &mut Vec<Candidate>) -> bool {
    let english_first = result.len() > 1
        && result
            .first()
            .is_some_and(|c| is_raw_candidate(c) && c.words[0].known);
    if !english_first {
        return false;
    }
    result.retain(|c| {
        is_raw_candidate(c)
            || !(c.features.typo_count >= 2.0 || (is_mixed_jianpin(c) && abbrev_before_last(c)))
    });
    result.len() == 1
}

/// [`Engine::fuzzy_fallback`]：模糊音读出的词要比精确读音的第一名高出多少（自然对数）才给出来。
pub(super) const FUZZY_FALLBACK_LEAD: f64 = 2.0;

/// 只走精确全拼边（不纠错、不补全、不带儿化）、最多 depth 个音节的切法，最多 8 种。
pub(super) fn exact_segmentations(
    graph: &SyllableGraph,
    pos: usize,
    depth: usize,
    path: &mut Vec<crate::graph::Edge>,
    out: &mut Vec<Vec<crate::graph::Edge>>,
) {
    if pos == graph.len() {
        out.push(path.clone());
        return;
    }
    if depth == 0 || out.len() >= 8 {
        return;
    }
    for edge in graph.edges_from(pos) {
        let plain = matches!(edge.kind, crate::graph::EdgeKind::Full { erhua: false, .. });
        if edge.typo || edge.completion > 0 || !plain {
            continue;
        }
        path.push(*edge);
        exact_segmentations(graph, edge.end, depth - 1, path, out);
        path.pop();
    }
}

pub(super) fn dedupe_by_text(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut best: HashMap<String, Candidate> = HashMap::new();
    for candidate in candidates {
        match best.entry(candidate.text.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(candidate);
            }
            Entry::Occupied(mut slot) => {
                if candidate.score > slot.get().score {
                    slot.insert(candidate);
                }
            }
        }
    }
    best.into_values().collect()
}
