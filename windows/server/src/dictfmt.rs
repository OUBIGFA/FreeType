//! 认别的输入法的词库、短语文件，统一成水杉格式的条目（导入时用）。
//!
//! 只照公开的文件格式写，没有使用这些输入法或转换工具的代码：
//!
//! | 文件 | 样子 | 编码 |
//! | --- | --- | --- |
//! | 搜狗细胞词库 `.scel` | 二进制：0x120 处是词组数，0x1540 起是拼音表，后面是词 | UTF-16LE |
//! | 搜狗文本词库 | `'ni'hao 你好` | 常见 GBK，也有 UTF-16、UTF-8 |
//! | 水杉词库 | `你好<Tab>ni hao[<Tab>权重]` | UTF-8 |
//! | Rime 词典 `.dict.yaml` | YAML 文件头（`columns`、`import_tables`）加 `你好<Tab>ni hao[<Tab>权重]`，见 [`rime`] | UTF-8 |
//! | WowType 词库 | `ni hao<Tab>你好<Tab>词频` | UTF-8 |
//! | 只有词语的词表（微信输入法等导出的） | `你好` 一行一个 | 任意上面几种 |
//! | 搜狗自定义短语、微信输入法常用语导出 | `编码,位置=短语` | 同上 |
//!
//! 文本按 BOM、能否当 UTF-8 读、零字节的位置判断编码，都不是就当 GBK。
//! 只有词语、没有拼音的行由 [`Reading`] 按常用读音注音（多音字可能注错）。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use wowtype_core::msime::{self, DictEntry, LineError, Parsed, QuickPhrase};

/// 没写权重的词条用这个（和水杉一样）。
const DEFAULT_WEIGHT: u32 = 10000;

/// 认出来的格式（导入报告里说给用户听）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    SogouScel,
    SogouText,
    Rime,
    Text,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::SogouScel => "搜狗细胞词库",
            Format::SogouText => "搜狗文本词库",
            Format::Rime => "Rime 词典",
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
            let freq: f64 = cols
                .next()
                .and_then(|f| f.trim().parse().ok())
                .unwrap_or(0.0);
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
    /// 不是中文词语的条数（Rime 词典里的英文、符号），没有收。
    pub foreign: usize,
    /// 跟着 Rime 主词典的 `import_tables` 读进来的词典个数。
    pub tables: usize,
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
        .map(|p| {
            if big_endian {
                u16::from_be_bytes([p[0], p[1]])
            } else {
                u16::from_le_bytes([p[0], p[1]])
            }
        })
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
        MultiByteToWideChar(
            936,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(&mut wide),
        );
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
    Some(u32::from_le_bytes([
        *bytes.get(at)?,
        *bytes.get(at + 1)?,
        *bytes.get(at + 2)?,
        *bytes.get(at + 3)?,
    ]))
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
                    weight: if freq == 0 {
                        DEFAULT_WEIGHT
                    } else {
                        u32::from(freq)
                    },
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
        let (code, word) = rest
            .split_once([' ', '\t'])
            .ok_or("格式错误，应为：'ni'hao 你好")?;
        let pinyin = msime::normalize_pinyin(code).ok_or("拼音无效")?;
        let word = word.trim();
        return if is_word(word) {
            Ok((word.to_string(), Some(pinyin), DEFAULT_WEIGHT))
        } else {
            Err("词语为空")
        };
    }
    let cols: Vec<&str> = if line.contains('\t') {
        line.split('\t')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect()
    } else {
        line.split([' ', ',', '，'])
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect()
    };
    let mut word = None;
    let mut pinyin = None;
    let mut weight = DEFAULT_WEIGHT;
    let mut codes: Vec<&str> = Vec::new();
    for col in &cols {
        if is_word(col) && word.is_none() {
            word = Some(col.to_string());
        } else if col.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            && col.bytes().any(|b| b.is_ascii_digit())
        {
            weight = col
                .parse::<f64>()
                .map(|f| f.clamp(1.0, f64::from(u32::MAX)) as u32)
                .unwrap_or(DEFAULT_WEIGHT);
        } else if col.is_ascii() || msime::normalize_pinyin(col).is_some() {
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

/// 读文本词库时攒下来的东西：有拼音的直接进 `parsed`，只有词语的等最后一起注音。
#[derive(Default)]
struct Collected {
    parsed: Parsed<DictEntry>,
    /// 只有词语的行：(在哪一行, 词语, 权重)。
    bare: Vec<(Place, String, u32)>,
    sogou: usize,
    foreign: usize,
}

/// 一行在哪：第几个文件（0 是用户选的那个，其余是 `import_tables` 里的第几个）的第几行。
#[derive(Clone, Copy)]
struct Place {
    table: usize,
    line: usize,
}

impl Collected {
    fn error(&mut self, tables: &[String], place: Place, message: &str) {
        self.parsed.errors.push(
            match place.table.checked_sub(1).and_then(|i| tables.get(i)) {
                // 导入表里的行：行号前面带上是哪个词典
                Some(table) => LineError {
                    line: 0,
                    message: format!("{table} 第 {} 行：{message}", place.line),
                },
                None => LineError {
                    line: place.line,
                    message: message.to_string(),
                },
            },
        );
    }
}

/// 一般的文本词库（搜狗文本、水杉、WowType、只有词语的词表），逐行认。
fn collect_text(text: &str, out: &mut Collected) {
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
        let place = Place {
            table: 0,
            line: i + 1,
        };
        match parse_text_line(line) {
            Ok((word, Some(pinyin), weight)) => {
                if line.starts_with('\'') {
                    out.sogou += 1;
                }
                out.parsed.items.push(DictEntry {
                    word,
                    pinyin,
                    weight,
                });
            }
            Ok((word, None, weight)) => out.bare.push((place, word, weight)),
            Err(message) => out.error(&[], place, message),
        }
    }
}

/// Rime 词典文件（`*.dict.yaml`）。规则照 librime 的 `DictSettings`、`EntryCollector`：
///
/// - 文件头是 `---` 到 `...` 之间的 YAML。`columns` 说各列是什么（没写就是 词语、编码、权重），
///   `import_tables` 列出要一起读的词典：主词典常常自己没有词条，全在这些词典里。
///   导入表的路径相对主词典所在的目录，只跟一层（导入表自己的 `import_tables` 不看）。
/// - 正文一行一条，用 Tab 分列。`#` 开头是注释，直到出现一行 `# no comment` 为止。
/// - 编码可以不写（按常用读音注音）；权重可以不写，也可以写成百分比（`50%`）。
///
/// 和 librime 不一样的地方：FreeType 的词库只收带拼音的中文词条，英文、符号词条跳过并计数；
/// 编码里的辅助码（万象的 `ni;rx hao;nz`）去掉；百分比权重按默认权重折算（没有预置词频表）。
mod rime {
    use super::*;

    /// 文件头里导入要用的两项。
    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct Header {
        pub columns: Vec<String>,
        pub import_tables: Vec<String>,
    }

    /// 词语、编码、权重各在第几列。
    struct Columns {
        text: usize,
        code: Option<usize>,
        weight: Option<usize>,
    }

    impl Header {
        fn list(&mut self, list: List) -> &mut Vec<String> {
            match list {
                List::Columns => &mut self.columns,
                List::ImportTables => &mut self.import_tables,
            }
        }

        fn layout(&self) -> Columns {
            let at = |label: &str| self.columns.iter().position(|c| c == label);
            match at("text") {
                Some(text) => Columns {
                    text,
                    code: at("code"),
                    weight: at("weight"),
                },
                // 没写 columns（或者没有 text 列，librime 会报错）：按默认的三列
                None => Columns {
                    text: 0,
                    code: Some(1),
                    weight: Some(2),
                },
            }
        }
    }

    /// 去掉 YAML 注释（行首的 `#`，或者空白后面的 `#`）。
    fn uncomment(line: &str) -> &str {
        if line.trim_start().starts_with('#') {
            return "";
        }
        match line.find(" #").or_else(|| line.find("\t#")) {
            Some(at) => &line[..at],
            None => line,
        }
    }

    fn scalar(text: &str) -> String {
        let text = text.trim();
        let unquoted = text
            .strip_prefix('"')
            .and_then(|t| t.strip_suffix('"'))
            .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')));
        unquoted.unwrap_or(text).to_string()
    }

    /// 文件头里的一个列表（`columns`、`import_tables`）。
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum List {
        Columns,
        ImportTables,
    }

    /// 从文件头的几行里读出 `columns` 和 `import_tables`（列表写成 `- 项` 或 `[项, 项]` 都认）。
    fn header(lines: &[&str]) -> Header {
        let mut header = Header::default();
        // 正在读哪个键下面的 `- 项`
        let mut open: Option<List> = None;
        for raw in lines {
            let line = uncomment(raw);
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(item) = trimmed.strip_prefix('-') {
                if let Some(list) = open {
                    header.list(list).push(scalar(item));
                }
                continue;
            }
            if line.starts_with([' ', '\t']) {
                continue;
            }
            // 顶层的键
            open = None;
            let Some((key, value)) = trimmed.split_once(':') else {
                continue;
            };
            let list = match key.trim() {
                "columns" => List::Columns,
                "import_tables" => List::ImportTables,
                _ => continue,
            };
            match value
                .trim()
                .strip_prefix('[')
                .and_then(|v| v.strip_suffix(']'))
            {
                Some(inline) => header.list(list).extend(
                    inline
                        .split(',')
                        .map(scalar)
                        .filter(|item| !item.is_empty()),
                ),
                None => open = Some(list),
            }
        }
        header
    }

    /// 是 Rime 词典就返回文件头和正文从第几行开始（从 0 数）。
    ///
    /// 第一行有内容的（注释不算）必须是 `---`，后面要有 `...`；不是的话按一般文本词库读。
    pub fn split(text: &str) -> Option<(Header, usize)> {
        let lines: Vec<&str> = text
            .split('\n')
            .map(|l| {
                l.trim_start_matches('\u{feff}')
                    .trim_end_matches(['\r', ' ', '\t'])
            })
            .collect();
        let open = lines
            .iter()
            .position(|l| !l.trim().is_empty() && !l.starts_with('#'))?;
        if lines[open].trim() != "---" {
            return None;
        }
        let close = open + 1 + lines[open + 1..].iter().position(|l| *l == "...")?;
        Some((header(&lines[open + 1..close]), close + 1))
    }

    /// 权重：数字或百分比；空着、写错了用默认权重。
    fn weight(text: &str) -> u32 {
        let text = text.trim();
        let (number, percent) = match text.strip_suffix('%') {
            Some(number) => (number.trim_end(), true),
            None => (text, false),
        };
        let Some(value) = number.parse::<f64>().ok().filter(|v| v.is_finite()) else {
            return DEFAULT_WEIGHT;
        };
        let value = if percent {
            f64::from(DEFAULT_WEIGHT) * value / 100.0
        } else {
            value
        };
        value.clamp(1.0, f64::from(u32::MAX)) as u32
    }

    /// 编码里每个音节后面 `;` 起的辅助码去掉。
    fn code(text: &str) -> String {
        text.split_whitespace()
            .map(|syllable| syllable.split(';').next().unwrap_or(syllable))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// 读一个 Rime 词典的正文。`table` 是第几个文件（见 [`Place`]）。
    pub fn collect(
        text: &str,
        header: &Header,
        body: usize,
        table: usize,
        tables: &[String],
        out: &mut Collected,
    ) {
        let layout = header.layout();
        let mut comments = true;
        for (i, raw) in text.split('\n').enumerate().skip(body) {
            let line = raw.trim_end_matches(['\r', ' ', '\t']);
            if line.is_empty() {
                continue;
            }
            if comments && line.starts_with('#') {
                comments = line != "# no comment";
                continue;
            }
            let place = Place { table, line: i + 1 };
            let cols: Vec<&str> = line.split('\t').map(|c| c.trim_matches(' ')).collect();
            let word = cols.get(layout.text).copied().unwrap_or("");
            if word.is_empty() {
                out.error(tables, place, "没有找到词语");
                continue;
            }
            if !is_word(word) {
                out.foreign += 1;
                continue;
            }
            let weight = layout
                .weight
                .and_then(|at| cols.get(at))
                .map_or(DEFAULT_WEIGHT, |w| weight(w));
            match layout
                .code
                .and_then(|at| cols.get(at))
                .filter(|c| !c.is_empty())
            {
                None => out.bare.push((place, word.to_string(), weight)),
                Some(text) => match msime::normalize_pinyin(&code(text)) {
                    Some(pinyin) => out.parsed.items.push(DictEntry {
                        word: word.to_string(),
                        pinyin,
                        weight,
                    }),
                    None => out.error(tables, place, "拼音无效"),
                },
            }
        }
    }

    /// 找 `import_tables` 里的词典文件：相对主词典的目录；主词典自己在子目录里的话
    /// （用户选了 `cn_dicts\x.dict.yaml`）再往上找两层，librime 是从用户目录的根算的。
    pub fn read_table(main: &Path, table: &str) -> Option<Vec<u8>> {
        // import_tables 只接受包内相对名称；拒绝父目录、盘符和绝对路径，
        // 免得导入外部词典时读入该词典指定的其他本机文件。
        if table.is_empty()
            || table.contains(':')
            || !Path::new(table)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            return None;
        }
        let relative = format!("{table}.dict.yaml");
        main.parent()?
            .ancestors()
            .take(3)
            .map(|dir| dir.join(&relative))
            .find(|path| path.is_file())
            .and_then(|path| std::fs::read(path).ok())
    }
}

/// 读词库文件（任意上面的格式）。Rime 主词典的 `import_tables` 这里读不到（没有路径），
/// 会报在出错的行里；从文件导入用 [`parse_dictionary_file`]。
pub fn parse_dictionary(
    bytes: &[u8],
    reading: &dyn Fn(&[String]) -> Box<dyn Reading>,
) -> Imported<DictEntry> {
    parse_dictionary_with(bytes, reading, &|_| None)
}

/// 读词库文件；Rime 主词典 `import_tables` 里的词典跟着一起读。
pub fn parse_dictionary_file(
    path: &Path,
    reading: &dyn Fn(&[String]) -> Box<dyn Reading>,
) -> std::io::Result<Imported<DictEntry>> {
    let bytes = std::fs::read(path)?;
    Ok(parse_dictionary_with(&bytes, reading, &|table| {
        rime::read_table(path, table)
    }))
}

fn parse_dictionary_with(
    bytes: &[u8],
    reading: &dyn Fn(&[String]) -> Box<dyn Reading>,
    read_table: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Imported<DictEntry> {
    if is_scel(bytes) {
        return Imported {
            parsed: parse_scel(bytes),
            format: Format::SogouScel,
            annotated: 0,
            foreign: 0,
            tables: 0,
        };
    }
    let text = decode(bytes);
    let mut out = Collected::default();
    let mut tables: Vec<String> = Vec::new();
    let mut read = 0;
    let rime = rime::split(&text);
    match &rime {
        Some((header, body)) => {
            tables.clone_from(&header.import_tables);
            rime::collect(&text, header, *body, 0, &tables, &mut out);
            let whole = Place { table: 0, line: 0 };
            for (i, table) in tables.iter().enumerate() {
                let Some(bytes) = read_table(table) else {
                    out.error(
                        &[],
                        whole,
                        &format!("未找到 {table}.dict.yaml；请检查 import_tables 中的相对路径"),
                    );
                    continue;
                };
                let text = decode(&bytes);
                match rime::split(&text) {
                    Some((header, body)) => {
                        rime::collect(&text, &header, body, i + 1, &tables, &mut out);
                        read += 1;
                    }
                    None => out.error(
                        &[],
                        whole,
                        &format!(
                            "{table}.dict.yaml 不是 Rime 词典；缺少由 --- 和 ... 包围的文件头"
                        ),
                    ),
                }
            }
        }
        None => collect_text(&text, &mut out),
    }
    let mut annotated = 0;
    let bare = std::mem::take(&mut out.bare);
    if !bare.is_empty() {
        let words: Vec<String> = bare.iter().map(|(_, w, _)| w.clone()).collect();
        let reader = reading(&words);
        for (place, word, weight) in bare {
            match reader
                .pinyin(&word)
                .and_then(|p| msime::normalize_pinyin(&p))
            {
                Some(pinyin) => {
                    annotated += 1;
                    out.parsed.items.push(DictEntry {
                        word,
                        pinyin,
                        weight,
                    });
                }
                None => out.error(
                    &tables,
                    place,
                    &format!("「{word}」里有不认识读音的字，没有拼音"),
                ),
            }
        }
    }
    let Collected {
        mut parsed,
        sogou,
        foreign,
        ..
    } = out;
    if rime.is_some() {
        // 几个词典里重复的词条只留最前面的（Rime：有重复词条时，最上面的权重生效）
        let mut seen = HashSet::new();
        parsed.items.retain(|e| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (e.word.as_str(), e.pinyin.as_str()).hash(&mut hasher);
            seen.insert(hasher.finish())
        });
    }
    let format = match rime {
        Some(_) => Format::Rime,
        None if sogou * 2 > parsed.items.len() => Format::SogouText,
        None => Format::Text,
    };
    Imported {
        parsed,
        format,
        annotated,
        foreign,
        tables: read,
    }
}

/// 读快捷短语文件：水杉 `编码<Tab>短语[<Tab>权重]`、Rime `短语<Tab>编码[<Tab>权重]`、
/// 搜狗自定义短语和微信输入法常用语导出的 `编码,位置=短语`。位置 1 排最前。
pub fn parse_quick_phrases(bytes: &[u8]) -> Parsed<QuickPhrase> {
    let text = decode(bytes);
    if text.lines().any(|l| sogou_phrase(l.trim()).is_some()) {
        return parse_sogou_phrases(&text);
    }
    // 不是搜狗那种写法：照水杉读。Rime 的列是反的（短语在前），整份文件按同一个方向换过来——
    // 逐行猜的话，短语本身是英文的行（邮箱、网址）看不出来该不该换。
    if !rime_phrase_table(&text) {
        return msime::parse_quick_phrases(&text);
    }
    let swapped: String = text
        .lines()
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            if l.starts_with('#') || cols.len() < 2 {
                return l.to_string();
            }
            let mut out = vec![cols[1], cols[0]];
            out.extend(&cols[2..]);
            out.join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    msime::parse_quick_phrases(&swapped)
}

/// 这份 Tab 分列的短语文件是不是 Rime 的写法（`短语<Tab>编码`）。
///
/// Rime 的短语表开头有 `# Rime table` 或 `#@/db_type<Tab>tabledb`；没有文件头的，看哪种读法
/// 说得通的行多：第二列是编码而第一列不是，就是 Rime 的方向。
fn rime_phrase_table(text: &str) -> bool {
    let (mut rime, mut msime_form) = (0usize, 0usize);
    for line in text.lines() {
        let line = line.trim_matches(['\r', '\u{feff}']);
        if line.starts_with('#') {
            if line.trim_end() == "# Rime table" || line.starts_with("#@/db_type") {
                return true;
            }
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        if cols.len() < 2 {
            continue;
        }
        let code = |col: &str| msime::normalize_code(col).is_some();
        match (code(cols[0]), code(cols[1])) {
            (false, true) => rime += 1,
            (true, false) => msime_form += 1,
            _ => {}
        }
    }
    rime > msime_form
}

/// 搜狗自定义短语（微信输入法常用语导出也是这个写法）：`编码,位置=短语`，`;` 开头是注释。
/// 短语可以有多行：`编码,位置=` 后面的行都属于它，直到下一条。
fn parse_sogou_phrases(text: &str) -> Parsed<QuickPhrase> {
    let mut parsed = Parsed::default();
    // 正在读的一条：(开头在第几行, 编码, 位置, 各行)
    let mut open: Option<(usize, String, u32, Vec<String>)> = None;
    let close = |parsed: &mut Parsed<QuickPhrase>,
                 entry: Option<(usize, String, u32, Vec<String>)>| {
        let Some((line, code, position, lines)) = entry else {
            return;
        };
        let phrase = lines.join("\n");
        let phrase = phrase.trim_matches(['\n', ' ', '\t']);
        if phrase.is_empty() {
            parsed.errors.push(LineError {
                line,
                message: "短语不能为空".into(),
            });
            return;
        }
        parsed.items.push(QuickPhrase {
            code,
            text: msime::unescape_phrase(phrase),
            // 位置小的排前面：换成权重，位置 1 最大
            weight: 100u32.saturating_sub(position.min(99)),
        });
    };
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_matches(['\r', ' ', '\u{feff}']);
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some((code, position, phrase)) = sogou_phrase(line) {
            close(&mut parsed, open.take());
            open = Some((i + 1, code, position, vec![phrase]));
            continue;
        }
        match open.as_mut() {
            // 多行短语的后面几行（空行也算，最后再去掉首尾的）
            Some((_, _, _, lines)) => lines.push(line.to_string()),
            None if line.is_empty() || line.starts_with('[') => {}
            None => parsed.errors.push(LineError {
                line: i + 1,
                message: "格式错误，应为：编码,位置=短语".into(),
            }),
        }
    }
    close(&mut parsed, open.take());
    parsed
}

/// `编码,位置=短语` 的开头一行；短语可以空着（多行短语从下一行开始）。
fn sogou_phrase(line: &str) -> Option<(String, u32, String)> {
    let (head, phrase) = line.split_once('=')?;
    let (code, position) = head.split_once(',')?;
    let code = msime::normalize_code(code)?;
    let position: u32 = position.trim().parse().ok()?;
    Some((code, position, phrase.trim().to_string()))
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
        for bytes in [
            utf16le(text, true),
            utf16le(text, false),
            text.as_bytes().to_vec(),
        ] {
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
        assert_eq!(
            words[3],
            ("微信", "wei xin", DEFAULT_WEIGHT),
            "只有词语的按读音注音"
        );
        assert_eq!(got.annotated, 1);
        assert_eq!(got.parsed.errors.len(), 2, "{:?}", got.parsed.errors);
        assert!(got
            .parsed
            .errors
            .iter()
            .any(|e| e.message.contains("生僻词")));
    }

    fn items(got: &Imported<DictEntry>) -> Vec<(&str, &str, u32)> {
        got.parsed
            .items
            .iter()
            .map(|e| (e.word.as_str(), e.pinyin.as_str(), e.weight))
            .collect()
    }

    fn errors<T>(parsed: &Parsed<T>) -> Vec<(usize, &str)> {
        parsed
            .errors
            .iter()
            .map(|e| (e.line, e.message.as_str()))
            .collect()
    }

    #[test]
    fn rime_dictionary_follows_header_columns_comments_and_weights() {
        // 列的顺序照文件头；英文词条不收；辅助码和声调去掉；百分比权重；空编码按读音注音
        let text = concat!(
            "# Rime dictionary\n# encoding: utf-8\n\n---\nname: demo\nversion: \"2026-01-01\"\nsort: by_weight\n",
            "columns:\n  - code   # 编码在前\n  - text\n  - weight\n...\n\n# 注释\n",
            "ni hao\t拟好\t500\r\nwei xin\t围新\t50%\nhello\tHello\t9\nnǐ;rx hǎo;nz\t你好\n\t\t3\n\t微信\n",
        );
        let got = read(text.as_bytes());
        assert_eq!(got.format, Format::Rime);
        assert_eq!(
            items(&got),
            vec![
                ("拟好", "ni hao", 500),
                ("围新", "wei xin", DEFAULT_WEIGHT / 2),
                ("你好", "ni hao", DEFAULT_WEIGHT),
                ("微信", "wei xin", DEFAULT_WEIGHT),
            ]
        );
        assert_eq!((got.foreign, got.annotated, got.tables), (1, 1, 0));
        assert_eq!(errors(&got.parsed), vec![(19, "没有找到词语")]);
    }

    #[test]
    fn rime_no_comment_line_turns_hash_lines_into_entries() {
        let text = "---\nname: demo\nversion: \"1\"\ncolumns: [text, weight, code]\n...\n# 注释\n# no comment\n#号\t7\tjing hao\n";
        let got = read(text.as_bytes());
        assert_eq!(
            items(&got),
            vec![("#号", "jing hao", 7)],
            "{:?}",
            got.parsed.errors
        );
    }

    #[test]
    fn rime_main_dictionary_reads_its_import_tables() {
        // 主词典自己只有一条，其余都在 import_tables 里（雾凇、白霜都是这样）
        let main = concat!(
            "---\nname: main\nversion: \"1\"\nimport_tables:\n  - cn_dicts/base   # 基础词库\n",
            "  # - cn_dicts/off\n  - cn_dicts/missing\n  - cn_dicts/plain\n...\n你好\tni hao\t9\n",
        );
        let base =
            "---\nname: base\nversion: \"1\"\n...\n你好\tni hao\t1\n微信\twei xin\t2\n坏\tb@d\n";
        let got = parse_dictionary_with(
            main.as_bytes(),
            &|_| Box::new(Fixed),
            &|table| match table {
                "cn_dicts/base" => Some(base.as_bytes().to_vec()),
                "cn_dicts/plain" => Some("你好\tni hao\n".as_bytes().to_vec()),
                _ => None,
            },
        );
        assert_eq!(got.format, Format::Rime);
        assert_eq!(
            items(&got),
            vec![("你好", "ni hao", 9), ("微信", "wei xin", 2)],
            "重复的词条留最前面的"
        );
        assert_eq!(got.tables, 1);
        assert_eq!(
            errors(&got.parsed),
            vec![
                (0, "cn_dicts/base 第 7 行：拼音无效"),
                (
                    0,
                    "未找到 cn_dicts/missing.dict.yaml；请检查 import_tables 中的相对路径"
                ),
                (
                    0,
                    "cn_dicts/plain.dict.yaml 不是 Rime 词典；缺少由 --- 和 ... 包围的文件头"
                ),
            ]
        );
        // 没有路径可找时（只给了内容）：报出来，不当成功
        let alone = read(main.as_bytes());
        assert_eq!(
            (alone.parsed.items.len(), alone.parsed.errors.len()),
            (1, 3)
        );
    }

    #[test]
    fn text_with_dashes_but_no_rime_header_is_plain_text() {
        // 有 --- 但没有 ... 结尾的文件头：不是 Rime 词典，照旧按文本词库读
        let got = read("你好\tni hao\t5\n---\n".as_bytes());
        assert_eq!(got.format, Format::Text);
        assert_eq!(items(&got), vec![("你好", "ni hao", 5)]);
    }

    #[test]
    fn sogou_phrase_file_with_comments_and_multiline_phrases() {
        // 搜狗「自定义短语」导出的样子：; 注释，多行短语从 = 后面的下一行开始
        let text = concat!(
            "; 搜狗拼音输入法--自定义短语配置文件\r\n\r\n; 单行的格式：\r\n; 字符串+英文逗号+数字（指定排序位置）=短语\r\n\r\n",
            "haha,1=哈哈哈\r\nrq,2=#$year年$month月$day_cn日\r\n\r\n; 多行\r\ndz,1=\r\n北京市海淀区\r\n\r\n中关村大街 1 号\r\n\r\n",
            "qm,3=此致\r\n敬礼\r\nkong,1=\r\n",
        );
        let got = parse_quick_phrases(text.as_bytes());
        let phrases: Vec<(&str, &str)> = got
            .items
            .iter()
            .map(|p| (p.code.as_str(), p.text.as_str()))
            .collect();
        assert_eq!(
            phrases,
            vec![
                ("haha", "哈哈哈"),
                ("rq", "#$year年$month月$day_cn日"),
                ("dz", "北京市海淀区\n\n中关村大街 1 号"),
                ("qm", "此致\n敬礼"),
            ]
        );
        assert_eq!(errors(&got), vec![(17, "短语不能为空")]);
    }

    #[test]
    fn rime_phrase_table_swaps_every_line_including_ascii_phrases() {
        // 邮箱、网址这种英文短语单看一行分不出哪列是编码：按文件头，整份都是 短语<Tab>编码
        let text = concat!(
            "# Rime table\n# coding: utf-8\n#@/db_name\tcustom_phrase.txt\n#@/db_type\ttabledb\n#\n",
            "有\tu\t3\nme@example.com\tyx\t2\nhttps://example.com\twz\n",
        );
        let got = parse_quick_phrases(text.as_bytes());
        assert!(got.errors.is_empty(), "{:?}", got.errors);
        let phrases: Vec<(&str, &str, u32)> = got
            .items
            .iter()
            .map(|p| (p.code.as_str(), p.text.as_str(), p.weight))
            .collect();
        assert_eq!(phrases[0], ("u", "有", 3));
        assert_eq!(phrases[1], ("yx", "me@example.com", 2));
        assert_eq!((phrases[2].0, phrases[2].1), ("wz", "https://example.com"));
        // 没有文件头：看多数行的方向
        let got = parse_quick_phrases("北京市\tbj\nme@example.com\tyx\n上海市\tsh\n".as_bytes());
        assert_eq!(got.items.len(), 3, "{:?}", got.errors);
        assert_eq!(got.items[1].text, "me@example.com");
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
        let bytes = scel_bytes(
            &[(&[0, 1], "你好", 321), (&[2, 3], "微信", 0)],
            &["ni", "hao", "wei", "xin"],
        );
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
        let reading =
            LexiconReading::from_tsv(lexicon, &["你好".into(), "你行".into(), "龘".into()]);
        assert_eq!(reading.pinyin("你好").as_deref(), Some("ni hao"));
        assert_eq!(
            reading.pinyin("你行").as_deref(),
            Some("ni xing"),
            "多音字取频次高的"
        );
        assert_eq!(reading.pinyin("龘"), None);
    }

    #[test]
    fn quick_phrases_in_sogou_rime_and_msime_forms() {
        let sogou = "[header]\ndz,1=北京市海淀区\nyx,2=me@example.com\nyx,1=hi@example.com\n";
        let got = parse_quick_phrases(sogou.as_bytes());
        assert_eq!(got.items.len(), 3, "{:?}", got.errors);
        assert_eq!(
            (got.items[0].code.as_str(), got.items[0].text.as_str()),
            ("dz", "北京市海淀区")
        );
        assert!(got.items[2].weight > got.items[1].weight, "位置 1 排前面");
        let rime = "北京市\tbj\n上海市\tsh\t5\n";
        let got = parse_quick_phrases(&utf16le(rime, true));
        assert_eq!(got.items.len(), 2, "{:?}", got.errors);
        assert_eq!(
            (
                got.items[1].code.as_str(),
                got.items[1].text.as_str(),
                got.items[1].weight
            ),
            ("sh", "上海市", 5)
        );
        let msime_form = "bj\t北京市\n";
        assert_eq!(
            parse_quick_phrases(msime_form.as_bytes()).items[0].text,
            "北京市"
        );
    }

    #[test]
    fn text_dictionary_normalizes_tones_and_uppercase() {
        let got =
            read("你好\tNi Hǎo\t300\n白色 bai2se4 120\n绿色\tlǜ\tsè\n'nǚ'xiá 女侠\n".as_bytes());
        assert!(got.parsed.errors.is_empty(), "{:?}", got.parsed.errors);
        let items: Vec<(&str, &str, u32)> = got
            .parsed
            .items
            .iter()
            .map(|e| (e.word.as_str(), e.pinyin.as_str(), e.weight))
            .collect();
        assert_eq!(
            items,
            vec![
                ("你好", "ni hao", 300),
                ("白色", "bai se", 120),
                ("绿色", "lv se", DEFAULT_WEIGHT),
                ("女侠", "nv xia", DEFAULT_WEIGHT),
            ]
        );
    }
}
