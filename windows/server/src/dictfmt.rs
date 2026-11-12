//! 认别的输入法的词库、短语文件，统一成水杉格式的条目（导入时用）。
//!
//! 只照公开的文件格式写，没有使用这些输入法或转换工具的代码：
//!
//! | 文件 | 样子 | 编码 |
//! | --- | --- | --- |
//! | 搜狗细胞词库 `.scel` | 二进制：0x120 处是词组数，0x1540 起是拼音表，后面是词 | UTF-16LE |
//! | 搜狗文本词库 | `'ni'hao 你好` | 常见 GBK，也有 UTF-16、UTF-8 |
//! | 水杉、Rime 词库 | `你好<Tab>ni hao[<Tab>权重]` | UTF-8 |
//! | WowType 词库 | `ni hao<Tab>你好<Tab>词频` | UTF-8 |
//! | 只有词语的词表（微信输入法等导出的） | `你好` 一行一个 | 任意上面几种 |
//! | 搜狗自定义短语、微信输入法常用语导出 | `编码,位置=短语` | 同上 |
//!
//! 文本按 BOM、能否当 UTF-8 读、零字节的位置判断编码，都不是就当 GBK。
//! 只有词语、没有拼音的行由 [`Reading`] 按常用读音注音（多音字可能注错）。

use std::collections::HashMap;

use wowtype_core::msime::{self, DictEntry, LineError, Parsed, QuickPhrase};

/// 没写权重的词条用这个（和水杉一样）。
const DEFAULT_WEIGHT: u32 = 10000;

/// 认出来的格式（导入报告里说给用户听）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    SogouScel,
    SogouText,
    Text,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::SogouScel => "搜狗细胞词库",
            Format::SogouText => "搜狗文本词库",
            Format::Text => "文本词库",
        }
    }
}

/// 词语的读音（给只有词语的行注音）。
pub trait Reading {
    /// 词语的拼音（空格分音节）；不认识的字返回 None。
    fn pinyin(&self, word: &str) -> Option<String>;
}

/// 从词库文件建的读音表：整词优先，否则逐字取频次最高的读音。
#[derive(Default)]
pub struct LexiconReading {
    words: HashMap<String, (String, f64)>,
}

impl LexiconReading {
    /// 只收 `wanted` 里的词和它们的字（词库很大，全收太占内存）。
    pub fn from_tsv(text: &str, wanted: &[String]) -> Self {
        let mut need: std::collections::HashSet<String> = wanted.iter().cloned().collect();
        for word in wanted {
            for c in word.chars() {
                need.insert(c.to_string());
            }
        }
        let mut words: HashMap<String, (String, f64)> = HashMap::new();
        for line in text.lines() {
            let mut cols = line.split('\t');
            let (Some(pinyin), Some(word)) = (cols.next(), cols.next()) else {
                continue;
            };
            if !need.contains(word) {
                continue;
            }
            let freq: f64 = cols.next().and_then(|f| f.trim().parse().ok()).unwrap_or(0.0);
            let better = words.get(word).is_none_or(|(_, f)| freq > *f);
            if better {
                words.insert(word.to_string(), (pinyin.trim().to_string(), freq));
            }
        }
        Self { words }
    }
}

impl Reading for LexiconReading {
    fn pinyin(&self, word: &str) -> Option<String> {
        if let Some((pinyin, _)) = self.words.get(word) {
            return Some(pinyin.clone());
        }
        let mut out = Vec::new();
        for c in word.chars() {
            out.push(self.words.get(&c.to_string())?.0.clone());
        }
        (!out.is_empty()).then(|| out.join(" "))
    }
}

/// 一份导入的结果。
pub struct Imported<T> {
    pub parsed: Parsed<T>,
    pub format: Format,
    /// 自动注音的条数。
    pub annotated: usize,
}

/// 文件内容按编码读成文字。
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return utf16(rest, false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return utf16(rest, true);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    // 没有 BOM 的 UTF-16：ASCII 字符的高字节是 0
    let odd_zeros = bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    if bytes.len() >= 4 && odd_zeros * 4 >= bytes.len() / 2 {
        return utf16(bytes, false);
    }
    gbk(bytes)
}

fn utf16(bytes: &[u8], big_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| if big_endian { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

/// GBK（代码页 936）读成文字。
#[cfg(windows)]
fn gbk(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::{MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
    if bytes.is_empty() {
        return String::new();
    }
    unsafe {
        let n = MultiByteToWideChar(936, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; n as usize];
        MultiByteToWideChar(936, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide));
        String::from_utf16_lossy(&wide)
    }
}

#[cfg(not(windows))]
fn gbk(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn is_scel(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x40, 0x15, 0x00, 0x00]) && bytes.len() > 0x1544
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?, *bytes.get(at + 2)?, *bytes.get(at + 3)?]))
}

fn utf16_at(bytes: &[u8], at: usize, len: usize) -> Option<String> {
    let slice = bytes.get(at..at + len)?;
    Some(utf16(slice, false))
}

/// 读搜狗细胞词库。
fn parse_scel(bytes: &[u8]) -> Parsed<DictEntry> {
    let mut parsed = Parsed::default();
    let fail = |parsed: &mut Parsed<DictEntry>, message: &str| {
        parsed.errors.push(LineError {
            line: 0,
            message: message.to_string(),
        });
    };
    let Some(groups) = u32_at(bytes, 0x120) else {
        fail(&mut parsed, "文件太短，不是完整的细胞词库");
        return parsed;
    };
    // 拼音表：个数，然后每项是 序号、字节数、拼音
    let mut pos = 0x1540;
    let Some(count) = u32_at(bytes, pos) else {
        fail(&mut parsed, "读不到拼音表");
        return parsed;
    };
    pos += 4;
    let mut table: HashMap<u16, String> = HashMap::new();
    for _ in 0..count.min(1000) {
        let (Some(index), Some(len)) = (u16_at(bytes, pos), u16_at(bytes, pos + 2)) else {
            break;
        };
        let Some(pinyin) = utf16_at(bytes, pos + 4, len as usize) else {
            break;
        };
        table.insert(index, pinyin);
        pos += 4 + len as usize;
    }
    // 词组：同音词个数、拼音序号的字节数、拼音序号，然后每个词是 字节数、词、扩展字节数、扩展（开头两字节是词频）
    for _ in 0..groups {
        let (Some(same), Some(py_len)) = (u16_at(bytes, pos), u16_at(bytes, pos + 2)) else {
            break;
        };
        pos += 4;
        let mut syllables = Vec::new();
        let mut ok = true;
        for i in 0..(py_len as usize / 2) {
            match u16_at(bytes, pos + i * 2).and_then(|k| table.get(&k)) {
                Some(s) => syllables.push(s.clone()),
                None => ok = false,
            }
        }
        pos += py_len as usize;
        for _ in 0..same {
            let Some(len) = u16_at(bytes, pos) else {
                return parsed;
            };
            let word = utf16_at(bytes, pos + 2, len as usize).unwrap_or_default();
            pos += 2 + len as usize;
            let Some(ext_len) = u16_at(bytes, pos) else {
                return parsed;
            };
            let freq = u16_at(bytes, pos + 2).unwrap_or(0);
            pos += 2 + ext_len as usize;
            let pinyin = msime::normalize_pinyin(&syllables.join(" "));
            match (ok, pinyin) {
                (true, Some(pinyin)) if !word.is_empty() => parsed.items.push(DictEntry {
                    word,
                    pinyin,
                    weight: if freq == 0 { DEFAULT_WEIGHT } else { u32::from(freq) },
                }),
                _ => parsed.errors.push(LineError {
                    line: parsed.items.len() + parsed.errors.len() + 1,
                    message: format!("「{word}」的拼音读不出来"),
                }),
            }
        }
    }
    parsed
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && !text.is_ascii() && !text.chars().any(char::is_control)
}

/// 一行文本词库：认出词语、拼音、权重。Ok(None) 表示只有词语（等注音）。
fn parse_text_line(line: &str) -> Result<(String, Option<String>, u32), &'static str> {
    // 搜狗：'ni'hao 你好
    if let Some(rest) = line.strip_prefix('\'') {
        let (code, word) = rest.split_once([' ', '\t']).ok_or("格式错误，应为：'ni'hao 你好")?;
        let pinyin = msime::normalize_pinyin(code).ok_or("拼音无效")?;
        let word = word.trim();
        return if is_word(word) { Ok((word.to_string(), Some(pinyin), DEFAULT_WEIGHT)) } else { Err("词语为空") };
    }
    let cols: Vec<&str> = if line.contains('\t') {
        line.split('\t').map(str::trim).filter(|c| !c.is_empty()).collect()
    } else {
        line.split([' ', ',', '，']).map(str::trim).filter(|c| !c.is_empty()).collect()
    };
    let mut word = None;
    let mut pinyin = None;
    let mut weight = DEFAULT_WEIGHT;
    let mut codes: Vec<&str> = Vec::new();
    for col in &cols {
        if is_word(col) && word.is_none() {
            word = Some(col.to_string());
        } else if col.bytes().all(|b| b.is_ascii_digit() || b == b'.') && col.bytes().any(|b| b.is_ascii_digit()) {
            weight = col.parse::<f64>().map(|f| f.clamp(1.0, f64::from(u32::MAX)) as u32).unwrap_or(DEFAULT_WEIGHT);
        } else if col.is_ascii() {
            codes.push(col);
        }
    }
    if !codes.is_empty() {
        // 空格分开的拼音（ni hao）会切成几段，拼回去
        pinyin = Some(msime::normalize_pinyin(&codes.join(" ")).ok_or("拼音无效")?);
    }
    let word = word.ok_or("没有找到词语")?;
    Ok((word, pinyin, weight))
}

/// 读词库文件（任意上面的格式）。
pub fn parse_dictionary(bytes: &[u8], reading: &dyn Fn(&[String]) -> Box<dyn Reading>) -> Imported<DictEntry> {
    if is_scel(bytes) {
        return Imported {
            parsed: parse_scel(bytes),
            format: Format::SogouScel,
            annotated: 0,
        };
    }
    let text = decode(bytes);
    let mut parsed = Parsed::default();
    let mut sogou = 0usize;
    let mut bare: Vec<(usize, String, u32)> = Vec::new();
    let mut yaml = false;
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.trim_matches(['\r', ' ', '\t', '\u{feff}']);
        match line {
            "" => continue,
            "---" => {
                yaml = true;
                continue;
            }
            "..." => {
                yaml = false;
                continue;
            }
            _ if yaml || line.starts_with('#') || line.starts_with(';') => continue,
            _ => {}
        }
        match parse_text_line(line) {
            Ok((word, Some(pinyin), weight)) => {
                if line.starts_with('\'') {
                    sogou += 1;
                }
                parsed.items.push(DictEntry { word, pinyin, weight });
            }
            Ok((word, None, weight)) => bare.push((i + 1, word, weight)),
            Err(message) => parsed.errors.push(LineError {
                line: i + 1,
                message: message.to_string(),
            }),
        }
    }
    let mut annotated = 0;
    if !bare.is_empty() {
        let words: Vec<String> = bare.iter().map(|(_, w, _)| w.clone()).collect();
        let reader = reading(&words);
        for (line, word, weight) in bare {
            match reader.pinyin(&word).and_then(|p| msime::normalize_pinyin(&p)) {
                Some(pinyin) => {
                    annotated += 1;
                    parsed.items.push(DictEntry { word, pinyin, weight });
                }
                None => parsed.errors.push(LineError {
                    line,
                    message: format!("「{word}」里有不认识读音的字，没有拼音"),
                }),
            }
        }
    }
    let format = if sogou * 2 > parsed.items.len() { Format::SogouText } else { Format::Text };
    Imported {
        parsed,
        format,
        annotated,
    }
}

/// 读快捷短语文件：水杉 `编码<Tab>短语[<Tab>权重]`、Rime `短语<Tab>编码[<Tab>权重]`、
/// 搜狗自定义短语和微信输入法常用语导出的 `编码,位置=短语`。位置 1 排最前。
pub fn parse_quick_phrases(bytes: &[u8]) -> Parsed<QuickPhrase> {
    let text = decode(bytes);
    if !text.lines().any(|l| sogou_phrase(l.trim()).is_some()) {
        // 不是搜狗那种写法：先照水杉读，Rime 的列反过来的行换一下
        let swapped: String = text
            .lines()
            .map(|l| {
                let cols: Vec<&str> = l.split('\t').collect();
                if cols.len() >= 2 && !cols[0].is_ascii() && msime::normalize_code(cols[1]).is_some() {
                    let mut out = vec![cols[1], cols[0]];
                    out.extend(&cols[2..]);
                    out.join("\t")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        return msime::parse_quick_phrases(&swapped);
    }
    let mut parsed = Parsed::default();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_matches(['\r', ' ', '\u{feff}']);
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
            continue;
        }
        match sogou_phrase(line) {
            Some((code, position, phrase)) => parsed.items.push(QuickPhrase {
                code,
                text: msime::unescape_phrase(&phrase),
                // 位置小的排前面：换成权重，位置 1 最大
                weight: 100u32.saturating_sub(position.min(99)),
            }),
            None => parsed.errors.push(LineError {
                line: i + 1,
                message: "格式错误，应为：编码,位置=短语".into(),
            }),
        }
    }
    parsed
}

/// `编码,位置=短语`。
fn sogou_phrase(line: &str) -> Option<(String, u32, String)> {
    let (head, phrase) = line.split_once('=')?;
    let (code, position) = head.split_once(',')?;
    let code = msime::normalize_code(code)?;
    let position: u32 = position.trim().parse().ok()?;
    let phrase = phrase.trim();
    (!phrase.is_empty()).then(|| (code, position, phrase.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;
    impl Reading for Fixed {
        fn pinyin(&self, word: &str) -> Option<String> {
            match word {
                "你好" => Some("ni hao".into()),
                "微信" => Some("wei xin".into()),
                _ => None,
            }
        }
    }

    fn read(bytes: &[u8]) -> Imported<DictEntry> {
        parse_dictionary(bytes, &|_| Box::new(Fixed))
    }

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xff, 0xfe] } else { Vec::new() };
        for u in text.encode_utf16() {
            out.extend(u.to_le_bytes());
        }
        out
    }

    #[test]
    fn sogou_text_in_utf16_and_utf8() {
        let text = "'ni'hao 你好\r\n'wei'xin 微信\r\n";
        for bytes in [utf16le(text, true), utf16le(text, false), text.as_bytes().to_vec()] {
            let got = read(&bytes);
            assert_eq!(got.format, Format::SogouText);
            assert_eq!(got.parsed.items.len(), 2, "{:?}", got.parsed.errors);
            assert_eq!(got.parsed.items[0].word, "你好");
            assert_eq!(got.parsed.items[0].pinyin, "ni hao");
        }
    }

    #[cfg(windows)]
    #[test]
    fn sogou_text_in_gbk() {
        // 「'ni'hao 你好」的 GBK 字节
        let mut bytes = b"'ni'hao ".to_vec();
        bytes.extend([0xc4, 0xe3, 0xba, 0xc3]);
        let got = read(&bytes);
        assert_eq!(got.parsed.items[0].word, "你好");
    }

    #[test]
    fn mixed_text_columns_and_bare_words() {
        let text = "# 注释\n你好\tni'hao\t500\nwei xin\t微信\t300\n你好 nihao\n微信\n生僻词\nabc\n";
        let got = read(text.as_bytes());
        let words: Vec<(&str, &str, u32)> = got
            .parsed
            .items
            .iter()
            .map(|e| (e.word.as_str(), e.pinyin.as_str(), e.weight))
            .collect();
        assert_eq!(words[0], ("你好", "ni hao", 500));
        assert_eq!(words[1], ("微信", "wei xin", 300));
        assert_eq!(words[2], ("你好", "ni hao", DEFAULT_WEIGHT));
        assert_eq!(words[3], ("微信", "wei xin", DEFAULT_WEIGHT), "只有词语的按读音注音");
        assert_eq!(got.annotated, 1);
        assert_eq!(got.parsed.errors.len(), 2, "{:?}", got.parsed.errors);
        assert!(got.parsed.errors.iter().any(|e| e.message.contains("生僻词")));
    }

    fn scel_bytes(entries: &[(&[u16], &str, u16)], table: &[&str]) -> Vec<u8> {
        let mut b = vec![0u8; 0x1540];
        b[..4].copy_from_slice(&[0x40, 0x15, 0x00, 0x00]);
        b[0x120..0x124].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        b.extend((table.len() as u32).to_le_bytes());
        for (i, py) in table.iter().enumerate() {
            let s = utf16le(py, false);
            b.extend((i as u16).to_le_bytes());
            b.extend((s.len() as u16).to_le_bytes());
            b.extend(s);
        }
        for (py, word, freq) in entries {
            b.extend(1u16.to_le_bytes());
            b.extend(((py.len() * 2) as u16).to_le_bytes());
            for k in *py {
                b.extend(k.to_le_bytes());
            }
            let w = utf16le(word, false);
            b.extend((w.len() as u16).to_le_bytes());
            b.extend(w);
            b.extend(10u16.to_le_bytes());
            b.extend(freq.to_le_bytes());
            b.extend([0u8; 8]);
        }
        b
    }

    #[test]
    fn sogou_scel() {
        let bytes = scel_bytes(&[(&[0, 1], "你好", 321), (&[2, 3], "微信", 0)], &["ni", "hao", "wei", "xin"]);
        let got = read(&bytes);
        assert_eq!(got.format, Format::SogouScel);
        assert_eq!(got.parsed.items.len(), 2, "{:?}", got.parsed.errors);
        assert_eq!(got.parsed.items[0].pinyin, "ni hao");
        assert_eq!(got.parsed.items[0].weight, 321);
        assert_eq!(got.parsed.items[1].word, "微信");
        assert_eq!(got.parsed.items[1].weight, DEFAULT_WEIGHT);
    }

    #[test]
    fn lexicon_reading_prefers_whole_word_then_chars() {
        let lexicon = "hao\t好\t100\nhao\t号\t50\nni\t你\t90\nni hao\t你好\t800\nhang\t行\t10\nxing\t行\t80\n";
        let reading = LexiconReading::from_tsv(lexicon, &["你好".into(), "你行".into(), "龘".into()]);
        assert_eq!(reading.pinyin("你好").as_deref(), Some("ni hao"));
        assert_eq!(reading.pinyin("你行").as_deref(), Some("ni xing"), "多音字取频次高的");
        assert_eq!(reading.pinyin("龘"), None);
    }

    #[test]
    fn quick_phrases_in_sogou_rime_and_msime_forms() {
        let sogou = "[header]\ndz,1=北京市海淀区\nyx,2=me@example.com\nyx,1=hi@example.com\n";
        let got = parse_quick_phrases(sogou.as_bytes());
        assert_eq!(got.items.len(), 3, "{:?}", got.errors);
        assert_eq!((got.items[0].code.as_str(), got.items[0].text.as_str()), ("dz", "北京市海淀区"));
        assert!(got.items[2].weight > got.items[1].weight, "位置 1 排前面");
        let rime = "北京市\tbj\n上海市\tsh\t5\n";
        let got = parse_quick_phrases(&utf16le(rime, true));
        assert_eq!(got.items.len(), 2, "{:?}", got.errors);
        assert_eq!((got.items[1].code.as_str(), got.items[1].text.as_str(), got.items[1].weight), ("sh", "上海市", 5));
        let msime_form = "bj\t北京市\n";
        assert_eq!(parse_quick_phrases(msime_form.as_bytes()).items[0].text, "北京市");
    }
}
