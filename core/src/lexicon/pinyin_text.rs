//! 拼音文本工具：切分音节、规范编码与读音、按词库猜读音。
use super::*;

/// 解析词表里的拼音字段。
///
/// 允许：
/// - 单音节：xian
/// - 多音节：xi an / xi'an
/// - 儿化：hua r
///
/// 多音节不允许写成无分隔的连写，因为无法唯一切分（如 fangan）。
/// 把连着写的拼音切成音节（`nihao` -> `ni hao`）：音节数最少，一样多时前面的音节取长的
/// （`fangan` -> `fang an`，想要 fan gan 就自己加空格或隔音符）。切不开返回 None。
pub fn split_syllables(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    let n = text.len();
    // best[i]：text[i..] 最少切成几个音节，以及第一个音节的长度
    let mut best: Vec<Option<(usize, usize)>> = vec![None; n + 1];
    best[n] = Some((0, 0));
    for i in (0..n).rev() {
        for len in (1..=crate::syllable::MAX_SYLLABLE_LEN.min(n - i)).rev() {
            if !crate::syllable::is_valid(&text[i..i + len]) {
                continue;
            }
            if let Some((count, _)) = best[i + len] {
                if best[i].map_or(true, |(c, _)| count + 1 < c) {
                    best[i] = Some((count + 1, len));
                }
            }
        }
    }
    best[0]?;
    let mut parts = Vec::new();
    let mut i = 0;
    while i < n {
        let (_, len) = best[i]?;
        parts.push(&text[i..i + len]);
        i += len;
    }
    Some(parts.join(" "))
}

/// 自定义词的编码（设置界面加词、导入用）：拼音整理成空格分开的音节（见 [`normalize_pinyin`]）；
/// 不是拼音但全是字母的（mail）去掉隔音符、小写。都不是返回 None。
pub fn normalize_code(text: &str) -> Option<String> {
    if let Some(pinyin) = normalize_pinyin(text) {
        return Some(pinyin);
    }
    let code: String = text
        .trim()
        .to_lowercase()
        .chars()
        .filter(|&c| c != '\'')
        .collect();
    (!code.is_empty() && code.bytes().all(|b| b.is_ascii_lowercase())).then_some(code)
}

/// 把打的拼音整理成空格分开的音节：Ni Hǎo、nihao、ni'hao、ni3hao3 -> ni hao。
/// ü、u:、u 加分音符都写成 v；数字声调、单独的声调符号去掉；单独一个 r 跟在音节后面是儿化。
/// 连着写的照 [`split_syllables`] 切。有一段切不开（不是拼音的字母、符号）返回 None。
pub fn normalize_pinyin(text: &str) -> Option<String> {
    let mut plain = String::new();
    for c in text.trim().to_lowercase().chars() {
        if let Some(base) = toneless(c) {
            plain.push(base);
        } else if (c == '\u{308}' || c == ':') && plain.ends_with('u') {
            plain.pop();
            plain.push('v');
        } else if c.is_ascii_digit() || ('\u{300}'..='\u{36f}').contains(&c) {
            continue;
        } else {
            plain.push(c);
        }
    }
    let mut parts: Vec<String> = Vec::new();
    let tokens = plain
        .split(|c: char| c == '\'' || c == '\u{2019}' || c.is_whitespace())
        .filter(|t| !t.is_empty());
    for token in tokens {
        if token.bytes().all(|b| b.is_ascii_lowercase()) && crate::syllable::is_valid(token) {
            parts.push(token.to_string());
        } else if token == "r" && !parts.is_empty() {
            parts.push("r".to_string());
        } else {
            parts.push(split_syllables(token)?);
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// 在词库原文（拼音<TAB>词<TAB>频次，一行一条）里猜词的拼音（设置界面加词时自动填编码）：
/// 整个词有就取频次最高的那条，没有就一个字一个字找。猜不出（有词库里没有的字、英文）返回 None。
/// 结果照 [`normalize_pinyin`] 整理过。
pub fn guess_pinyin(lexicon: &[u8], word: &str) -> Option<String> {
    if word.is_empty() {
        return None;
    }
    let chars: Vec<&str> = word
        .char_indices()
        .map(|(i, c)| &word[i..i + c.len_utf8()])
        .collect();
    // 词里出现过的不同的字：叠字（妈妈咪、试试看）共用一个槽位，按位置分槽的话第二个「妈」永远空着
    let mut uniq: Vec<&str> = Vec::new();
    for c in &chars {
        if !uniq.contains(c) {
            uniq.push(c);
        }
    }
    // best[0] 是整个词，best[1..] 是每个不同的字：(频次, 拼音)
    let mut best: Vec<Option<(f64, &[u8])>> = vec![None; uniq.len() + 1];
    for line in lexicon.split(|&b| b == b'\n') {
        if line.first() == Some(&b'#') {
            continue;
        }
        let mut cols = line.splitn(3, |&b| b == b'\t');
        let (Some(reading), Some(text), Some(freq)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let slot = if text == word.as_bytes() {
            0
        } else if let Some(i) = uniq.iter().position(|c| c.as_bytes() == text) {
            i + 1
        } else {
            continue;
        };
        let freq = std::str::from_utf8(freq)
            .ok()
            .and_then(|f| f.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
        if best[slot].map_or(true, |(old, _)| freq > old) {
            best[slot] = Some((freq, reading));
        }
    }
    let text = |reading: &[u8]| String::from_utf8_lossy(reading).into_owned();
    if let Some((_, reading)) = best[0] {
        return normalize_pinyin(&text(reading));
    }
    let readings: Option<Vec<String>> = chars
        .iter()
        .map(|c| {
            let slot = uniq.iter().position(|u| u == c)? + 1;
            best[slot].map(|(_, r)| text(r))
        })
        .collect();
    normalize_pinyin(&readings?.join(" "))
}

/// 带声调的字母 -> 不带的（ü 是 v）
pub(super) fn toneless(c: char) -> Option<char> {
    Some(match c {
        'ā' | 'á' | 'ǎ' | 'à' => 'a',
        'ē' | 'é' | 'ě' | 'è' | 'ê' => 'e',
        'ī' | 'í' | 'ǐ' | 'ì' => 'i',
        'ō' | 'ó' | 'ǒ' | 'ò' => 'o',
        'ū' | 'ú' | 'ǔ' | 'ù' => 'u',
        'ü' | 'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' => 'v',
        'ń' | 'ň' | 'ǹ' => 'n',
        'ḿ' => 'm',
        _ => return None,
    })
}

pub fn parse_pinyin_keys(pinyin: &str) -> Result<Vec<SyllableKey>> {
    let pinyin = pinyin.trim();
    if pinyin.is_empty() {
        bail!("拼音不能为空");
    }

    // 单音节优先：xian 必须解析成一个音节，而不是 xi + an。
    if !pinyin.contains([' ', '\t', '\'']) {
        let syl = Syllable::from_text(pinyin)
            .ok_or_else(|| anyhow!("未知音节: {pinyin}（多音节请用空格分隔）"))?;
        return Ok(vec![SyllableKey::new(syl.id, false)]);
    }

    let mut keys: Vec<SyllableKey> = Vec::new();
    for token in pinyin.split([' ', '\t', '\'']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if token == "r" {
            match keys.last_mut() {
                Some(last) if !last.erhua => last.erhua = true,
                Some(_) => bail!("重复的儿化标记"),
                None => bail!("儿化标记 r 前面没有音节"),
            }
            continue;
        }
        let syl = Syllable::from_text(token).ok_or_else(|| anyhow!("未知音节: {token}"))?;
        keys.push(SyllableKey::new(syl.id, false));
    }
    if keys.is_empty() {
        bail!("拼音不能为空");
    }
    Ok(keys)
}
