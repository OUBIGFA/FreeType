//! Precompile static dictionaries the way Rime deploys its tables. The server maps
//! the finished index read-only and allocates only the candidates returned by a query.
use super::*;
use anyhow::ensure;
use fst::{Map, MapBuilder};
use memmap2::Mmap;
use std::fs::File;
use std::io::Read;

// Header: magic, syllable/format version, source CRC, entry count, total frequency,
// end of entries, start of FST. The trailing CRC covers the header and all sections.
const MAGIC: &[u8; 4] = b"FTD1";
const HEADER: usize = 32;
const FULL: u8 = 1;
const INITIAL: u8 = 2;
const FUZZY: u8 = 3;
const PRIOR: u8 = 4;

struct IndexBytes {
    file: Mmap,
    start: usize,
}

impl AsRef<[u8]> for IndexBytes {
    fn as_ref(&self) -> &[u8] {
        &self.file[self.start..self.file.len() - 4]
    }
}

/// Immutable full, initial, fuzzy and prediction indexes over a single copy of each entry.
pub struct MappedLexicon {
    index: Map<IndexBytes>,
    entries_end: usize,
    count: usize,
    total: f64,
}

fn checksum(reader: impl Read) -> std::io::Result<u32> {
    let mut crc = crc32fast::Hasher::new();
    let mut reader = reader;
    let mut buffer = [0; 16384];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        crc.update(&buffer[..n]);
    }
    Ok(crc.finalize())
}

fn full_key(keys: impl Iterator<Item = SyllableKey>) -> Vec<u8> {
    let mut bytes = vec![FULL];
    for key in keys {
        bytes.extend((key.id | (u16::from(key.erhua) << 15)).to_be_bytes());
    }
    bytes
}

fn fuzzy_key(ids: impl Iterator<Item = u32>) -> Vec<u8> {
    let mut bytes = vec![FUZZY];
    for id in ids {
        bytes.extend(id.to_be_bytes());
    }
    bytes
}

fn prefixed(prefix: u8, key: &[u8]) -> Vec<u8> {
    std::iter::once(prefix).chain(key.iter().copied()).collect()
}

fn query_key(query: &[SyllableQuery]) -> Option<Vec<u8>> {
    if query.iter().any(|q| q.is_jianpin()) {
        return Some(prefixed(INITIAL, &query_initials(query)?));
    }
    Some(full_key(query.iter().filter_map(|q| match *q {
        SyllableQuery::Full { id, erhua } => Some(SyllableKey::new(id, erhua)),
        _ => None,
    })))
}

fn u32_at(bytes: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap())
}

impl MappedLexicon {
    /// Compile using the existing TSV parser and frequency estimator. Never replace
    /// a published index until the complete new file is ready.
    pub fn compile(source: &Path, target: &Path) -> Result<()> {
        let canonical_source = source.canonicalize()?;
        let canonical_target = target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf());
        ensure!(
            canonical_source != canonical_target,
            "输出索引不能覆盖源词库"
        );
        let text = fs::read_to_string(source)?;
        let source_crc = crc32fast::hash(text.as_bytes());
        let mut lex = MemoryLexicon::from_tsv_str(&text)?;
        drop(text);
        lex.estimate_placeholder_freqs(PLACEHOLDER_MIN_WORDS);
        let mut bytes = Vec::from(*MAGIC);
        bytes.extend(cache_version().to_le_bytes());
        bytes.extend(source_crc.to_le_bytes());
        bytes.extend(u32::try_from(lex.len())?.to_le_bytes());
        bytes.extend(lex.total_freq.to_le_bytes());
        bytes.resize(HEADER, 0);

        let mut offsets = Vec::with_capacity(lex.len());
        for entry in &lex.entries {
            offsets.push(u32::try_from(bytes.len())?);
            bytes.extend(u32::try_from(entry.word.len())?.to_le_bytes());
            bytes.extend(u16::try_from(entry.keys.len())?.to_le_bytes());
            bytes.extend(entry.freq.to_le_bytes());
            bytes.extend(entry.word.as_bytes());
            for key in &entry.keys {
                bytes.extend((key.id | (u16::from(key.erhua) << 15)).to_le_bytes());
            }
        }
        let entries_end = u32::try_from(bytes.len())?;
        bytes[24..28].copy_from_slice(&entries_end.to_le_bytes());
        let mut index = MapBuilder::memory();
        let fuzzy = lex.build_fuzzy_index();
        // Stable key order makes the file reproducible. Each posting list retains
        // its original order (particularly ties, rare characters and fuzzy matches).
        let mut write_groups = |mut groups: Vec<(Vec<u8>, Vec<usize>)>| -> Result<()> {
            groups.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            for (key, group) in groups {
                index.insert(key, u64::try_from(bytes.len())?)?;
                bytes.extend(u32::try_from(group.len())?.to_le_bytes());
                for id in group {
                    bytes.extend(offsets[id].to_le_bytes());
                }
            }
            Ok(())
        };
        write_groups(
            lex.full_index
                .into_iter()
                .map(|(keys, entries)| (full_key(keys.into_iter()), entries))
                .collect(),
        )?;
        write_groups(
            lex.jianpin_index
                .into_iter()
                .map(|(initials, entries)| (prefixed(INITIAL, &initials), entries))
                .collect(),
        )?;
        write_groups(
            fuzzy
                .into_iter()
                .map(|(keys, entries)| (fuzzy_key(keys.into_iter()), entries))
                .collect(),
        )?;

        // Sorted by word, then descending frequency: multiple readings share one prior.
        let mut words: Vec<_> = lex.entries.iter().filter(|e| e.freq >= 1.0).collect();
        words.sort_unstable_by(|first, second| {
            first
                .word
                .cmp(&second.word)
                .then_with(|| second.freq.total_cmp(&first.freq))
        });
        words.dedup_by(|first, second| first.word == second.word);
        for entry in words {
            index.insert(
                prefixed(PRIOR, entry.word.as_bytes()),
                (entry.freq.ln() as f32).to_bits() as u64,
            )?;
        }
        let index_start = u32::try_from(bytes.len())?;
        bytes[28..32].copy_from_slice(&index_start.to_le_bytes());
        bytes.extend(index.into_inner()?);
        bytes.extend(crc32fast::hash(&bytes).to_le_bytes());
        crate::learn::write_atomic(target, bytes)?;
        Ok(())
    }

    /// Validate through buffered reads so startup does not fault the entire mapping
    /// into the working set. Windows denies in-place writers while this file is mapped.
    pub fn open(source: &Path, index_path: &Path) -> Result<Self> {
        let mut options = File::options();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1 | 4); // FILE_SHARE_READ | FILE_SHARE_DELETE
        }
        let mut file = options.open(index_path)?;
        let len = file.metadata()?.len();
        ensure!(len >= (HEADER + 4) as u64, "词库索引不完整");
        let crc = checksum((&mut file).take(len - 4))?;
        let mut expected = [0; 4];
        file.read_exact(&mut expected)?;
        ensure!(crc == u32::from_le_bytes(expected), "词库索引校验失败");
        // Compilers publish immutable files by rename; no writable mapping is created.
        let file = unsafe { Mmap::map(&file)? };
        ensure!(
            &file[..4] == MAGIC && u32_at(&file, 4) == cache_version(),
            "词库索引版本不匹配"
        );
        ensure!(
            checksum(File::open(source)?)? == u32_at(&file, 8),
            "词库索引与源文件不匹配"
        );
        let count = u32_at(&file, 12) as usize;
        let total = f64::from_le_bytes(file[16..24].try_into()?);
        let entries_end = u32_at(&file, 24) as usize;
        let start = u32_at(&file, 28) as usize;
        ensure!(
            HEADER <= entries_end && entries_end <= start && start < file.len() - 4,
            "词库索引边界错误"
        );
        let index = Map::new(IndexBytes { file, start })?;
        Ok(Self {
            index,
            entries_end,
            count,
            total,
        })
    }

    fn entries(&self, key: &[u8]) -> impl Iterator<Item = LexEntry> + '_ {
        let data = self.index.as_fst().as_inner();
        let postings = self.index.get(key).map_or(&[][..], |offset| {
            let pos = offset as usize;
            let count = u32_at(&data.file, pos) as usize;
            &data.file[pos + 4..pos + 4 + count * 4]
        });
        postings.chunks_exact(4).map(|offset| {
            let pos = u32_at(offset, 0) as usize;
            let (word, freq, reading) = self.record(pos);
            let keys: Vec<_> = reading
                .chunks_exact(2)
                .map(|pair| {
                    let bits = u16::from_le_bytes([pair[0], pair[1]]);
                    SyllableKey::new(bits & 0x7fff, bits >> 15 != 0)
                })
                .collect();
            let initials = keys
                .iter()
                .map(|key| Syllable::from_id(key.id).unwrap().initial_letter())
                .collect();
            LexEntry {
                word: word.to_owned(),
                keys,
                initials,
                freq,
            }
        })
    }

    fn record(&self, pos: usize) -> (&str, f64, &[u8]) {
        let data = &self.index.as_fst().as_inner().file[pos..self.entries_end];
        let word_end = 14 + u32_at(data, 0) as usize;
        let syllables = u16::from_le_bytes(data[4..6].try_into().unwrap()) as usize;
        let freq = f64::from_le_bytes(data[6..14].try_into().unwrap());
        (
            std::str::from_utf8(&data[14..word_end]).unwrap(),
            freq,
            &data[word_end..word_end + 2 * syllables],
        )
    }

    fn lookup_impl(&self, query: &[SyllableQuery], common: bool) -> Vec<LexEntry> {
        let Some(key) = query_key(query) else {
            return vec![];
        };
        self.entries(&key)
            .take_while(|entry| !common || !entry.is_rare())
            .filter(|entry| key[0] == FULL || entry_matches(entry, query))
            .collect()
    }
}

impl Lexicon for MappedLexicon {
    fn lookup(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.lookup_impl(query, false)
    }
    fn lookup_common(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.lookup_impl(query, true)
    }
    fn len(&self) -> usize {
        self.count
    }
    fn total_freq(&self) -> f64 {
        self.total
    }

    fn has_longer(&self, query: &[SyllableQuery]) -> bool {
        if query.is_empty() {
            return false;
        }
        let Some(key) = query_key(query) else {
            return true;
        };
        let fst = self.index.as_fst();
        let mut node = fst.root();
        for byte in key {
            let Some(transition) = node.find_input(byte) else {
                return false;
            };
            node = fst.node(node.transition_addr(transition));
        }
        !node.is_empty()
    }

    fn lookup_fuzzy(&self, query: &[SyllableQuery], groups: u32) -> Vec<LexEntry> {
        if groups == 0 || query.is_empty() {
            return vec![];
        }
        let mut key = vec![FUZZY];
        for q in query {
            let SyllableQuery::Full { id, erhua: false } = q else {
                return vec![];
            };
            let Some(canon) = crate::syllable::fuzzy_key(*id) else {
                return vec![];
            };
            key.extend(canon.to_be_bytes());
        }
        self.entries(&key).filter(|entry| entry.keys.iter().zip(query).all(|(key, query)| {
            matches!(*query, SyllableQuery::Full { id, .. } if crate::syllable::fuzzy_groups_between(id, key.id)
                .is_some_and(|need| need & !groups == 0))
        })).collect()
    }

    fn for_each_word(&self, visit: &mut dyn FnMut(&str, f64)) {
        let mut pos = HEADER;
        while pos < self.entries_end {
            let (word, freq, reading) = self.record(pos);
            visit(word, freq);
            pos += 14 + word.len() + reading.len();
        }
    }

    fn prediction_index(&self) -> Option<&dyn PredictionIndex> {
        Some(self)
    }
}

impl PredictionIndex for MappedLexicon {
    fn log_frequency(&self, word: &str) -> Option<f32> {
        self.index
            .get(prefixed(PRIOR, word.as_bytes()))
            .map(|value| f32::from_bits(value as u32))
    }
}
