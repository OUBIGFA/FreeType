//! 水杉输入法（msime-windows）词库和快捷短语批量文件的读写。
//!
//! 只照公开的文件格式实现，不含水杉的代码：
//!
//! - 词库：每行 `词语<Tab>编码[<Tab>权重]`。编码是拼音，可以写成 `ni'hao`、`ni hao` 或
//!   `nihao`；`#` 开头的行是注释，`---` 到 `...` 之间的 YAML 头跳过；权重是非负整数，
//!   空着或含 `=` 时用默认值。
//! - 快捷短语：每行 `编码<Tab>短语[<Tab>权重]`。编码只能是英文字母（不分大小写，存成小写），
//!   短语里的换行、Tab、反斜杠写成 `\n`、`\t`、`\\`，默认权重 10，同一编码按权重从大到小排。
//!
//! 文件开头的 UTF-8 BOM 和 Windows 换行都能读。

use crate::lexicon::{parse_pinyin_keys, split_syllables};

/// 快捷短语没写权重时的默认值（和水杉一样）。
pub const QUICK_PHRASE_DEFAULT_WEIGHT: u32 = 10;
/// 词库没写权重时给的频次：比常用词低、比生僻词高。
pub const DICT_DEFAULT_WEIGHT: u32 = 10_000;

/// 词库里的一条。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictEntry {
    pub word: String,
    /// 空格分音节的拼音（ni hao）。
    pub pinyin: String,
    pub weight: u32,
}

/// 一条快捷短语。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickPhrase {
    /// 小写字母编码。
    pub code: String,
    /// 短语原文（真的换行、Tab）。
    pub text: String,
    pub weight: u32,
}

/// 读文件时跳过的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineError {
    /// 从 1 开始的行号。
    pub line: usize,
    pub message: String,
}

/// 读出来的条目和跳过的行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed<T> {
    pub items: Vec<T>,
    pub errors: Vec<LineError>,
}

impl<T> Default for Parsed<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            errors: Vec::new(),
        }
    }
}

fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .enumerate()
        .map(|(i, line)| (i + 1, line))
}

/// 按 Tab 切开，每段去掉两头空格。
fn fields(line: &str) -> Vec<&str> {
    line.split('\t').map(|f| f.trim_matches(' ')).collect()
}

/// 编码写法统一成「空格分音节」：ni'hao、ni hao、nihao、lü 都行。不是拼音返回 None。
pub fn normalize_pinyin(code: &str) -> Option<String> {
    let code = code.to_lowercase().replace('ü', "v");
    let mut parts = Vec::new();
    for token in code.split([' ', '\'', '\t']).filter(|t| !t.is_empty()) {
        if parse_pinyin_keys(token).is_ok() {
            parts.push(token.to_string());
        } else {
            parts.push(split_syllables(token)?);
        }
    }
    let pinyin = parts.join(" ");
    (!pinyin.is_empty() && parse_pinyin_keys(&pinyin).is_ok()).then_some(pinyin)
}

fn parse_weight(text: Option<&&str>, default: u32) -> Result<u32, &'static str> {
    match text {
        None => Ok(default),
        Some(t) if t.is_empty() || t.contains('=') => Ok(default),
        Some(t) if t.bytes().all(|b| b.is_ascii_digit()) => t.parse().map_err(|_| "权重数值无效"),
        Some(_) => Err("权重必须是非负整数"),
    }
}

/// 读词库批量文件。
pub fn parse_dictionary(text: &str) -> Parsed<DictEntry> {
    let mut parsed = Parsed::default();
    let mut yaml_header = false;
    for (line_no, line) in lines(text) {
        let trimmed = line.trim_matches([' ', '\t']);
        match trimmed {
            "" => continue,
            "---" => {
                yaml_header = true;
                continue;
            }
            "..." => {
                yaml_header = false;
                continue;
            }
            _ if yaml_header || trimmed.starts_with('#') => continue,
            _ => {}
        }
        let mut error = |message: &str| {
            parsed.errors.push(LineError {
                line: line_no,
                message: message.to_string(),
            })
        };
        let cols = fields(line);
        if cols.len() != 2 && cols.len() != 3 {
            error("格式错误，应为：词语<Tab>编码[<Tab>权重]");
            continue;
        }
        let (word, code) = (cols[0], cols[1]);
        if word.is_empty() || code.is_empty() {
            error("词语和编码不能为空");
            continue;
        }
        let weight = match parse_weight(cols.get(2), DICT_DEFAULT_WEIGHT) {
            Ok(w) => w,
            Err(message) => {
                error(message);
                continue;
            }
        };
        let Some(pinyin) = normalize_pinyin(code) else {
            error("编码不是有效的全拼");
            continue;
        };
        parsed.items.push(DictEntry {
            word: word.to_string(),
            pinyin,
            weight,
        });
    }
    parsed
}

/// 写成水杉能导入的词库文件：`词语<Tab>ni'hao<Tab>权重`。
pub fn format_dictionary(entries: &[DictEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            e.word.replace(['\t', '\n', '\r'], " "),
            e.pinyin.replace(' ', "'"),
            e.weight
        ));
    }
    out
}

/// 短语里的 `\n`、`\t`、`\\` 换成真的字符，其余反斜杠原样保留。
pub fn unescape_phrase(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    out.push('\n');
                    chars.next();
                }
                Some('t') => {
                    out.push('\t');
                    chars.next();
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    // Windows 换行统一成 \n
    out.replace("\r\n", "\n").replace('\r', "\n")
}

/// [`unescape_phrase`] 的反过程。
pub fn escape_phrase(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// 快捷短语编码：只能是英文字母，返回小写。
pub fn normalize_code(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_lowercase();
    (!code.is_empty() && code.bytes().all(|b| b.is_ascii_lowercase())).then_some(code)
}

/// 读快捷短语批量文件。`#` 开头的行当注释跳过（水杉导出的文件里没有这种行）。
pub fn parse_quick_phrases(text: &str) -> Parsed<QuickPhrase> {
    let mut parsed = Parsed::default();
    for (line_no, line) in lines(text) {
        if line.trim_matches([' ', '\t']).is_empty() || line.starts_with('#') {
            continue;
        }
        let mut error = |message: &str| {
            parsed.errors.push(LineError {
                line: line_no,
                message: message.to_string(),
            })
        };
        let cols: Vec<&str> = line.split('\t').map(|f| f.trim_matches([' ', '\t'])).collect();
        if cols.len() != 2 && cols.len() != 3 {
            error("格式错误，应为：编码<Tab>短语<Tab>权重；短语内的换行请写成 \\n");
            continue;
        }
        let text = unescape_phrase(cols[1]);
        let Some(code) = normalize_code(cols[0]).filter(|_| !text.trim().is_empty()) else {
            error("编码只能包含英文字母，短语不能为空");
            continue;
        };
        // 水杉对写了 Tab 却没写权重的行报错，这里放宽成默认值
        let weight = match parse_weight(cols.get(2), QUICK_PHRASE_DEFAULT_WEIGHT) {
            Ok(w) => w,
            Err(message) => {
                error(message);
                continue;
            }
        };
        parsed.items.push(QuickPhrase { code, text, weight });
    }
    parsed
}

/// 写成水杉能导入的快捷短语文件，按编码、权重从大到小排。
pub fn format_quick_phrases(phrases: &[QuickPhrase]) -> String {
    let mut sorted: Vec<&QuickPhrase> = phrases.iter().collect();
    sorted.sort_by(|a, b| a.code.cmp(&b.code).then(b.weight.cmp(&a.weight)));
    let mut out = String::new();
    for p in sorted {
        out.push_str(&format!("{}\t{}\t{}\n", p.code, escape_phrase(&p.text), p.weight));
    }
    out
}

/// 合并：同一编码、同一短语只留一条（后来的权重覆盖前面的），其余按出现顺序。
pub fn merge_quick_phrases(base: &mut Vec<QuickPhrase>, more: Vec<QuickPhrase>) -> usize {
    let mut added = 0;
    for p in more {
        if let Some(slot) = base.iter_mut().find(|q| q.code == p.code && q.text == p.text) {
            slot.weight = p.weight;
        } else {
            base.push(p);
            added += 1;
        }
    }
    added
}

/// 转成引擎自定义词的 TSV（`编码<Tab>短语`，短语保持转义）。编码正好是拼音时（nihao）
/// 引擎当拼音自定义词，否则当自定义编码，只在整个输入等于编码时排在最前面。
/// 同一编码按权重从大到小排在前面。
pub fn quick_phrases_to_custom_tsv(phrases: &[QuickPhrase]) -> String {
    let mut sorted: Vec<&QuickPhrase> = phrases.iter().collect();
    sorted.sort_by(|a, b| a.code.cmp(&b.code).then(b.weight.cmp(&a.weight)));
    let mut out = String::new();
    for p in sorted {
        out.push_str(&format!("{}\t{}\n", p.code, escape_phrase(&p.text)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_accepts_msime_spellings() {
        let text = "\u{feff}---\nname: x\n...\n# 注释\n你好\tni'hao\t300\r\n西安\txi an\n长城\tchangcheng\t\n绿色\tlü se\t=5\n";
        let parsed = parse_dictionary(text);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let got: Vec<(&str, &str, u32)> = parsed
            .items
            .iter()
            .map(|e| (e.word.as_str(), e.pinyin.as_str(), e.weight))
            .collect();
        assert_eq!(
            got,
            vec![
                ("你好", "ni hao", 300),
                ("西安", "xi an", DICT_DEFAULT_WEIGHT),
                ("长城", "chang cheng", DICT_DEFAULT_WEIGHT),
                ("绿色", "lv se", DICT_DEFAULT_WEIGHT),
            ]
        );
    }

    #[test]
    fn dictionary_reports_bad_lines_with_numbers() {
        let parsed = parse_dictionary("好\n词\tqqqx\n词\tci\t-1\n词\tci\t1\t2\n");
        let lines: Vec<usize> = parsed.errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, vec![1, 2, 3, 4]);
        assert!(parsed.items.is_empty());
    }

    #[test]
    fn dictionary_round_trip() {
        let parsed = parse_dictionary("你好\tni'hao\t300\n");
        let text = format_dictionary(&parsed.items);
        assert_eq!(text, "你好\tni'hao\t300\n");
        assert_eq!(parse_dictionary(&text).items, parsed.items);
    }

    #[test]
    fn quick_phrases_unescape_and_default_weight() {
        let parsed = parse_quick_phrases("YYJQ\t韵酒\nDZ\t四川省\\n宜宾市\\t\\\\x\t20\n\nbad1\tx\nok\t \n");
        assert_eq!(
            parsed.items,
            vec![
                QuickPhrase {
                    code: "yyjq".into(),
                    text: "韵酒".into(),
                    weight: 10
                },
                QuickPhrase {
                    code: "dz".into(),
                    text: "四川省\n宜宾市\t\\x".into(),
                    weight: 20
                },
            ]
        );
        assert_eq!(parsed.errors.len(), 2);
        assert_eq!(parsed.errors[0].line, 4);
    }

    #[test]
    fn quick_phrase_export_matches_msime_layout() {
        let phrases = vec![
            QuickPhrase {
                code: "dz".into(),
                text: "甲".into(),
                weight: 1,
            },
            QuickPhrase {
                code: "dz".into(),
                text: "乙\n丙".into(),
                weight: 9,
            },
        ];
        let text = format_quick_phrases(&phrases);
        assert_eq!(text, "dz\t乙\\n丙\t9\ndz\t甲\t1\n");
        assert_eq!(parse_quick_phrases(&text).items.len(), 2);
        assert_eq!(quick_phrases_to_custom_tsv(&phrases), "dz\t乙\\n丙\ndz\t甲\n");
    }

    #[test]
    fn merge_keeps_one_copy() {
        let mut base = parse_quick_phrases("a\tx\t1\n").items;
        let added = merge_quick_phrases(&mut base, parse_quick_phrases("a\tx\t5\nb\ty\n").items);
        assert_eq!(added, 1);
        assert_eq!(base[0].weight, 5);
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn quick_phrases_reach_engine_candidates() {
        let lexicon = crate::MemoryLexicon::from_tsv_str("ni hao\t你好\t100\nyun\t韵\t10\n").unwrap();
        let engine = crate::Engine::new(
            std::sync::Arc::new(lexicon),
            std::sync::Arc::new(crate::BigramLm::default()),
        );
        let phrases = parse_quick_phrases("yyjq\t韵酒\nnihao\t你好呀\n").items;
        engine
            .lexicon()
            .set_custom_tsv(&quick_phrases_to_custom_tsv(&phrases));
        assert_eq!(engine.input("yyjq")[0].text, "韵酒");
        assert_eq!(engine.input("nihao")[0].text, "你好呀");
    }

    #[test]
    fn imported_dictionary_words_are_candidates_but_not_user_words() {
        let lexicon = crate::MemoryLexicon::from_tsv_str("ni hao\t你好\t100\n").unwrap();
        let engine = crate::Engine::new(
            std::sync::Arc::new(lexicon),
            std::sync::Arc::new(crate::BigramLm::default()),
        );
        let entries = parse_dictionary("韵酒\tyun'jiu\t500\n").items;
        let words = entries
            .into_iter()
            .map(|e| (e.pinyin, e.word, f64::from(e.weight)))
            .collect();
        assert_eq!(engine.lexicon().set_extra_words(words), 1);
        assert!(engine.input("yunjiu").iter().any(|c| c.text == "韵酒"));
        assert!(engine.lexicon().words().is_empty(), "导入的词不进用户词库");
        engine.lexicon().set_extra_words(Vec::new());
        assert!(!engine.input("yunjiu").iter().any(|c| c.text == "韵酒"));
    }
}
