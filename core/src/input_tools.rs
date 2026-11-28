//! 独立英文候选和部件查字。查询只读固定本地数据，不请求网络，不改拼音分词规则。
use crate::{
    lookup_table::{LookupEntry, LookupTable},
    shuangpin::{self, Scheme},
    Engine,
};
use anyhow::{anyhow, Result};
use std::{collections::HashSet, path::Path};

pub struct InputTools {
    english: LookupTable,
    spelling: LookupTable,
    radical: LookupTable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCandidate {
    pub text: String,
    pub reading: String,
    pub gloss: String,
    pub corrected: bool,
}

impl From<LookupEntry> for ToolCandidate {
    fn from(entry: LookupEntry) -> Self {
        Self {
            text: entry.text,
            reading: entry.reading,
            gloss: entry.gloss,
            corrected: false,
        }
    }
}

pub fn letters(text: &str) -> String {
    text.bytes()
        .filter(u8::is_ascii_alphabetic)
        .map(|b| b.to_ascii_lowercase() as char)
        .collect()
}

/// 与生成器的分桶契约一致；桶只是缩小查询范围，实际建议还需编辑距离约束。
fn spelling_key(text: &str) -> String {
    let text = text
        .to_ascii_lowercase()
        .replace("ph", "f")
        .replace("ck", "k")
        .replace("qu", "k")
        .replace("gh", "g");
    let mut out = String::new();
    for c in text.chars() {
        let value = match c {
            'b' | 'p' | 'f' | 'v' => 'b',
            'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => 'k',
            'd' | 't' => 't',
            'm' | 'n' => 'n',
            'l' => 'l',
            'r' => 'r',
            _ => continue,
        };
        if !out.ends_with(value) {
            out.push(value);
        }
    }
    if out.is_empty() {
        out.push('a');
    }
    out
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len().abs_diff(b.len()) > 3 {
        return 4;
    }
    let mut previous: Vec<_> = (0..=b.len()).collect();
    let mut older = previous.clone();
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            row[j + 1] = (row[j] + 1)
                .min(previous[j + 1] + 1)
                .min(previous[j] + usize::from(ca != cb));
            if i > 0 && j > 0 && a[i] == b[j - 1] && a[i - 1] == b[j] {
                row[j + 1] = row[j + 1].min(older[j - 1] + 1);
            }
        }
        older = previous;
        previous = row;
    }
    previous[b.len()]
}

pub fn follow_case(text: &str, input: &str) -> String {
    let capitals = input.chars().filter(char::is_ascii_uppercase).count();
    let count = input.chars().filter(char::is_ascii_alphabetic).count();
    if count > 1 && capitals == count {
        return text.to_ascii_uppercase();
    }
    if input.starts_with(|c: char| c.is_ascii_uppercase()) && capitals == 1 {
        let mut out = text.to_ascii_lowercase();
        if let Some(first) = out.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        return out;
    }
    text.into()
}

impl InputTools {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        Ok(Self {
            english: LookupTable::open(root.join("english-lookup.ftd"))?,
            spelling: LookupTable::open(root.join("english-spelling.ftd"))?,
            radical: LookupTable::open(root.join("radical-lookup.ftd"))?,
        })
    }

    pub fn english(&self, input: &str, limit: usize, correction: bool) -> Vec<ToolCandidate> {
        let key = letters(input);
        if key.is_empty() || key.len() > 100 {
            return Vec::new();
        }
        let mut words: Vec<ToolCandidate> = self
            .english
            .search(&key, limit)
            .into_iter()
            .map(Into::into)
            .collect();
        if words.is_empty() && correction && key.len() >= 3 {
            let bucket = spelling_key(&key);
            let mut suggestions: Vec<_> = self
                .spelling
                .search(&bucket, 1000)
                .into_iter()
                .filter(|entry| entry.key == bucket)
                .filter_map(|entry| {
                    let d = distance(&key, &letters(&entry.text));
                    (d <= 3).then_some((d, entry))
                })
                .collect();
            // Neighboring-key substitutions, missing letters and transpositions can
            // change the phonetic bucket. Probe exact keys in the sorted index;
            // never scan the full dictionary for each mistyped character.
            let original = key.as_bytes();
            let mut variants = HashSet::new();
            for index in 0..=original.len() {
                for letter in b'a'..=b'z' {
                    let mut inserted = original.to_vec();
                    inserted.insert(index, letter);
                    variants.insert(inserted);
                    if index < original.len() {
                        let mut changed = original.to_vec();
                        changed[index] = letter;
                        variants.insert(changed);
                    }
                }
                if index < original.len() {
                    let mut removed = original.to_vec();
                    removed.remove(index);
                    variants.insert(removed);
                }
                if index + 1 < original.len() {
                    let mut swapped = original.to_vec();
                    swapped.swap(index, index + 1);
                    variants.insert(swapped);
                }
            }
            for variant in variants {
                let variant = std::str::from_utf8(&variant).unwrap();
                if variant != key {
                    if let Some(entry) = self.english.exact(variant) {
                        suggestions.push((1, entry));
                    }
                }
            }
            // 同距离、同词频的结果也固定排序，候选不能随散列表迭代顺序跳动。
            suggestions.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| b.1.rank.cmp(&a.1.rank))
                    .then_with(|| a.1.text.cmp(&b.1.text))
            });
            let mut seen = HashSet::new();
            for (_, suggestion) in suggestions
                .into_iter()
                .filter(|(_, entry)| seen.insert(entry.text.clone()))
                .take(limit.min(8))
            {
                if let Some(entry) = self
                    .english
                    .search(&letters(&suggestion.text), 20)
                    .into_iter()
                    .find(|e| e.text == suggestion.text)
                {
                    let mut candidate = ToolCandidate::from(entry);
                    candidate.corrected = true;
                    words.push(candidate);
                }
            }
        }
        for word in &mut words {
            word.text = follow_case(&word.text, input);
        }
        words
    }

    pub fn radical(&self, input: &str, scheme: Option<Scheme>, limit: usize) -> Vec<ToolCandidate> {
        let mut keys = vec![letters(input)];
        if let Some(scheme) = scheme {
            keys.extend(
                shuangpin::decode(scheme, input)
                    .into_iter()
                    .take(16)
                    .map(|d| letters(&d.pinyin)),
            );
        }
        keys.sort();
        keys.dedup();
        let mut candidates: Vec<_> = keys
            .iter()
            .flat_map(|key| self.radical.search(key, limit))
            .collect();
        candidates.sort_by_key(|e| {
            (
                std::cmp::Reverse(keys.contains(&e.key)),
                std::cmp::Reverse(e.rank),
                e.text.clone(),
            )
        });
        let mut seen = HashSet::new();
        candidates
            .into_iter()
            .filter(|e| seen.insert(e.text.clone()))
            .take(limit)
            .map(Into::into)
            .collect()
    }
}

impl Engine {
    pub fn english_tool_candidates(
        &self,
        input: &str,
        limit: usize,
        correction: bool,
    ) -> Vec<ToolCandidate> {
        let Some(tools) = self.input_tools() else {
            return Vec::new();
        };
        let key = letters(input);
        let mut candidates = tools.english(input, limit, correction);
        let mut learned = self.lexicon.learned_code_words(&key);
        learned.retain(|(_, word, _)| word.is_ascii());
        learned.sort_by(|a, b| b.2.total_cmp(&a.2));
        for (_, word, _) in learned.into_iter().rev() {
            let text = follow_case(&word, input);
            if let Some(index) = candidates
                .iter()
                .position(|candidate| candidate.text == text)
            {
                let candidate = candidates.remove(index);
                candidates.insert(0, candidate);
            } else {
                candidates.insert(
                    0,
                    ToolCandidate {
                        text,
                        reading: String::new(),
                        gloss: "个人词".into(),
                        corrected: false,
                    },
                );
            }
        }
        candidates.truncate(limit);
        candidates
    }

    pub fn load_input_tools(&self, root: impl AsRef<Path>) -> Result<()> {
        let tools = InputTools::open(root)?;
        self.input_tools
            .set(tools)
            .map_err(|_| anyhow!("查询词典已载入"))
    }

    pub fn input_tools(&self) -> Option<&InputTools> {
        self.input_tools.get()
    }
}
