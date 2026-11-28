//! 候选顺序的保护规则：完整音节、分词读音、单声母首字与已选词前移。
use super::*;

/// 输入正好是一个完整音节（xian、ji、zhe）时：本音节的字排在补全出来的字（想、就、真）前面，
/// 纠错出来的（gei → 被、飞；si → 是，漏按 h）再往后。
///
/// 补全每个字母只罚 1.4 左右，高频字补全以后常常比本音节的字分还高；已经打全的音节，
/// 用户要的是这个音节的字。多音节输入、没打全的音节不动。
pub(super) fn exact_syllable_first(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let exact = |c: &Candidate| c.features.typo_count == 0.0 && c.features.prefix_completion == 0.0;
    let one_syllable = candidates
        .iter()
        .any(|c| exact(c) && c.words.len() == 1 && c.words[0].known && !c.pinyin.contains(' '));
    if !one_syllable {
        return candidates;
    }
    let (mut out, rest): (Vec<Candidate>, Vec<Candidate>) = candidates.into_iter().partition(exact);
    lift_split_word(&mut out);
    // 补全的在前、纠错的在后，各自按分数
    let (completions, typos): (Vec<Candidate>, Vec<Candidate>) =
        rest.into_iter().partition(|c| c.features.typo_count == 0.0);
    // 分数也压到前面的下面，后面按分数合并、排序的地方不会再把它们换上来
    for c in completions.into_iter().chain(typos) {
        let floor = out.last().map_or(0.0, |last| last.score);
        out.push(Candidate {
            score: c.score.min(floor - 1e-3),
            ..c
        });
    }
    out
}

/// 一个音节也能切成两个音节的整词（xian → 西安）：分数和第一名差得不多时，至少排到第 [`SPLIT_WORD_SLOT`] 位。
/// 单字太多，按分数它常常排在十几位以后，第一页看不到。
pub(super) fn lift_split_word(exact: &mut Vec<Candidate>) {
    let Some(top) = exact.first().map(|c| c.score) else {
        return;
    };
    let found = exact.iter().position(|c| {
        c.words.len() == 1 && c.pinyin.contains(' ') && top - c.score <= SPLIT_WORD_MARGIN
    });
    let Some(at) = found.filter(|&at| at > SPLIT_WORD_SLOT) else {
        return;
    };
    let mut word = exact.remove(at);
    // 分数夹在前后两个之间，后面再按分数排也不会变
    let (above, below) = (
        exact[SPLIT_WORD_SLOT - 1].score,
        exact[SPLIT_WORD_SLOT].score,
    );
    word.score = (above + below) / 2.0;
    exact.insert(SPLIT_WORD_SLOT, word);
}

/// [`lift_split_word`]：最多和第一名差多少（自然对数）。实测 xian 的「西安」差 3.0，
/// 别的音节能切出来的整词（提案、彼岸、立案、堤岸）差 4 以上，不抬。
pub(super) const SPLIT_WORD_MARGIN: f64 = 3.5;

/// [`lift_split_word`]：抬到第几位（从 0 数，4 是第五个）。
pub(super) const SPLIT_WORD_SLOT: usize = 4;

/// 第 k 个字母之后在原始串里的字节位置（跳过紧随其后的隔音符）。
pub(super) fn letters_to_bytes(raw: &str, letters: usize) -> usize {
    let bytes = raw.as_bytes();
    let mut seen = 0usize;
    let mut idx = 0usize;
    while idx < bytes.len() && seen < letters {
        if bytes[idx].is_ascii_alphabetic() {
            seen += 1;
        }
        idx += 1;
    }
    while idx < bytes.len() && bytes[idx] == b'\'' {
        idx += 1;
    }
    idx
}

/// 把句首词候选并入整句候选。
///
/// 整句是一个词时（shijie -> 世界）句首词放在最后；整句由多个词拼成时
/// （长句）第二名往后的整句只是小改动，价值不如句首词，所以句首词紧跟第一名。
/// 词频低于这个数的词不受「精确优先」保护（rime-ice 里约排在 20 万名以后，多是生僻或凑出来的条目）。
pub(super) const EXACT_PROTECT_MIN_FREQ: f64 = 1000.0;

/// 神经语言模型特征最低到多少（自然对数），免得一条特别离谱的把分数拉得太开。
pub(super) const NNLM_FLOOR: f64 = 20.0;

/// 拼音打全了、而且正好是一个实在的词时，这个词必须排第一。
///
/// 补全和纠错的分数里带着词频，高频词补出来以后可能比用户打全了的低频词还高：
/// `zhongdi` 会出「重点」（zhong dian）而不是「种地」，`ka` 出「看」，`beiji` 出「北京」。
/// 用户按空格上屏的是第一个——打到一半时补全排第二只是多按一个键，
/// 打全了却被换成别的词就是打错字。所以排在这个词前面的补全/纠错候选都挪到它后面。
///
/// 同理，整个输入就是一个词时，几个单字硬凑出来的候选（`shidan` 拆成「是 | 但」）
/// 也不该压过它（「实弹」）。
///
/// 「实在的词」：精确匹配（不靠纠错和补全）、要么整个输入是一个词，要么最后一个词
/// 至少两个字（这时只和「前面的词一样、只有最后一个词不同」的候选比）；每个词的词频不低于 [`EXACT_PROTECT_MIN_FREQ`]，或者是用户选过的。
/// 最后是个单字的精确结果（`zhonggu` 拆成「中 | 骨」）不算，用户多半还没打完，
/// 补全照常可以排第一。
/// 拆开读法的词排在前几个以内就不用再提。
pub(super) const SPLIT_READING_FIRST_PAGE: usize = 5;

/// 拆开读法的词频至少是第一个候选的 1/这么多，才算真歧义（西安 对 先）而不是罕见重码（激昂 对 将）。
pub(super) const SPLIT_READING_RATIO: f64 = 100.0;

/// 第一个候选超过这么多音节就不管：长输入的别种切法几乎都是凑出来的。
pub(super) const SPLIT_READING_MAX_SYLLABLES: usize = 4;

/// 同一串字母还能切成别的音节时（yuan 也是 yu'an、xian 也是 xi'an），把这种读法里
/// 最好的整词放到第 2 位，不用翻页或改打隔音符才找得到「预案」「西安」。
///
/// 做法参考水杉输入法（msime-windows）的「备选切分保护位」，没有用它的代码：
/// 主切分（最长音节）照旧排第一；只提精确的单个整词（不是纠错、补全、拼凑的），
/// 词频要过 [`SPLIT_READING_RATIO`] 的门槛。打了隔音符（yu'an）时整音节读法不存在，
/// 这里自然不动。
pub(super) fn promote_split_reading(candidates: &mut Vec<Candidate>, input_len: usize) {
    let exact_word = |c: &Candidate| {
        c.consumed == input_len
            && !is_raw_candidate(c)
            && c.features.typo_count == 0.0
            && c.features.prefix_completion == 0.0
            && c.words.len() == 1
            && c.words[0].known
    };
    let Some(top) = candidates.first().filter(|c| exact_word(c)) else {
        return;
    };
    if top.pinyin.split_whitespace().count() > SPLIT_READING_MAX_SYLLABLES {
        return;
    }
    let letters = |pinyin: &str| pinyin.split_whitespace().collect::<String>();
    let (top_pinyin, top_letters, top_freq) =
        (top.pinyin.clone(), letters(&top.pinyin), top.words[0].freq);
    // 字母一样、音节边界不一样，就是另一种切法
    let split = |c: &Candidate| {
        exact_word(c) && c.pinyin != top_pinyin && letters(&c.pinyin) == top_letters
    };
    let Some(at) = candidates.iter().position(split) else {
        return;
    };
    if at < SPLIT_READING_FIRST_PAGE
        || candidates[at].words[0].freq * SPLIT_READING_RATIO < top_freq
    {
        return;
    }
    let word = candidates.remove(at);
    candidates.insert(1, word);
    // 分数改成单调递减，后面再按分数重排（繁体转换）顺序也不变
    for i in 1..candidates.len() {
        if candidates[i].score >= candidates[i - 1].score {
            candidates[i].score = candidates[i - 1].score - 1e-3;
        }
    }
}

pub(super) fn protect_exact_input(candidates: &mut Vec<Candidate>) {
    let multi = |w: &CandidateWord| w.pinyin.split_whitespace().count() >= 2;
    let is_exact = |c: &Candidate| {
        c.features.coverage_score >= 1.0
            && c.features.typo_count == 0.0
            && c.features.prefix_completion == 0.0
            && !c.words.is_empty()
            && c.words.iter().all(|w| w.known)
    };
    let solid = |c: &Candidate| {
        is_exact(c)
            && (c.words.len() == 1 || c.words.last().map(multi).unwrap_or(false))
            && (c.features.history_count > 0.0
                || c.words.iter().all(|w| w.freq >= EXACT_PROTECT_MIN_FREQ))
    };
    let Some(anchor) = candidates.iter().position(solid) else {
        return;
    };
    if anchor == 0 {
        return;
    }
    let whole_word = candidates[anchor].words.len() == 1;
    // 精确结果是几个词拼的（我在 | 种地）时，只管开头一样、只是后面换了词的拼凑结果
    // （我 | 在重点）；一个整词盖住全部输入的（从善如流 vs 从善 | 入理）是更强的证据，不动
    let head_text: String = {
        let words = &candidates[anchor].words;
        words[..words.len() - 1]
            .iter()
            .map(|w| w.text.as_str())
            .collect()
    };
    let same_head =
        |c: &Candidate| whole_word || (c.words.len() >= 2 && c.text.starts_with(&head_text));
    let yields = |c: &Candidate| {
        let corrected = c.features.typo_count > 0.0 || c.features.prefix_completion > 0.0;
        (corrected && same_head(c))
            || (whole_word && c.words.len() >= 2 && !c.words.iter().any(multi))
    };
    if !candidates[..anchor].iter().any(yields) {
        return;
    }
    let mut ahead = Vec::new();
    let mut keep = Vec::with_capacity(candidates.len());
    for (i, c) in candidates.drain(..).enumerate() {
        if i < anchor && yields(&c) {
            ahead.push(c);
        } else {
            keep.push(c);
        }
    }
    let at = keep
        .iter()
        .position(solid)
        .map(|i| i + 1)
        .unwrap_or(keep.len());
    keep.splice(at..at, ahead);
    // 分数改成单调递减，后面再按分数重排（繁体转换）顺序也不变
    for i in 1..keep.len() {
        if keep[i].score >= keep[i - 1].score {
            keep[i].score = keep[i - 1].score - 1e-3;
        }
    }
    *candidates = keep;
}

impl Engine {
    pub(super) fn promote_custom(&self, candidates: &mut [Candidate]) {
        let custom = |c: &Candidate| c.words.len() == 1 && self.lexicon.is_custom(&c.text);
        if candidates.iter().any(custom) {
            // 稳定排序：自定义词之间、其余候选之间都保持原来的顺序
            candidates.sort_by_key(|c| !custom(c));
        }
    }
}

/// 自定义词的转义：`\n` 换行、`\t` 制表符、`\\` 反斜杠，其余原样。
pub fn unescape_custom(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// 只打一个声母（l、b、zh）时固定排第一的字：词库按读音分的词频把它们排得太后
/// （了 le 348 万，来 1484 万；不 299 万，把 380 万），而且第一个候选不该随上文、选过的字变来变去
/// （#156）。没列的声母照词频排。
pub(super) const SINGLE_INITIAL_TOP: &[(&str, &str)] = &[
    ("b", "不"),
    ("d", "的"),
    ("g", "个"),
    ("h", "和"),
    ("j", "就"),
    ("l", "了"),
    ("n", "你"),
    ("q", "去"),
    ("r", "人"),
    ("s", "是"),
    ("sh", "是"),
    ("t", "他"),
    ("w", "我"),
    ("y", "一"),
    ("z", "在"),
    ("zh", "这"),
];

/// 输入只是一个声母时，把 [`SINGLE_INITIAL_TOP`] 里的字挪到第一个（在选过最多的字前面；
/// 自定义词、自定义编码之后还会排到它前面）。候选里没有这个字就不动。
pub(super) fn pin_single_initial(input: &str, candidates: &mut Vec<Candidate>) {
    let Some(&(_, top)) = SINGLE_INITIAL_TOP.iter().find(|(key, _)| *key == input) else {
        return;
    };
    if let Some(i) = candidates
        .iter()
        .position(|c| c.text == top && c.words.len() == 1)
    {
        let pinned = candidates.remove(i);
        candidates.insert(0, pinned);
    }
}

pub(super) fn promote_picked(candidates: &mut Vec<Candidate>) {
    let picked = |c: &Candidate| {
        c.features.history_use_count >= learn::MIN_PICK_HEAT as f32
            && c.features.typo_count == 0.0
            && c.features.prefix_completion == 0.0
            && c.features.full_jianpin == 0.0
    };
    let strongest = |single: bool| {
        let mut indices: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| picked(c) && (c.text.chars().count() == 1) == single)
            .map(|(index, _)| index)
            .collect();
        indices.sort_by(|&a, &b| {
            candidates[b]
                .features
                .history_use_count
                .total_cmp(&candidates[a].features.history_use_count)
                .then(a.cmp(&b))
        });
        indices
    };
    let word = strongest(false).first().copied().filter(|&i| {
        i != 0
            && candidates[i].features.history_use_count > candidates[0].features.history_use_count
    });
    let mut singles = strongest(true);
    // 候选首位不被单字提权替换；其后最多提升三个足够热的单字。
    singles.retain(|&index| index != 0 || word.is_some());
    singles.truncate(3);
    if word.is_none() && singles.is_empty() {
        return;
    }
    let mut order = Vec::with_capacity(candidates.len());
    order.push(word.unwrap_or(0));
    order.extend(singles);
    let mut taken = vec![false; candidates.len()];
    for &index in &order {
        taken[index] = true;
    }
    order.extend((0..candidates.len()).filter(|&index| !taken[index]));
    let mut old: Vec<_> = std::mem::take(candidates).into_iter().map(Some).collect();
    candidates.extend(order.into_iter().map(|index| old[index].take().unwrap()));
    for i in 1..candidates.len() {
        if candidates[i].score >= candidates[i - 1].score {
            candidates[i].score = candidates[i - 1].score - 1e-3;
        }
    }
}
