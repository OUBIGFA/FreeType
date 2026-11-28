//! Precompile static dictionaries the way Rime deploys its tables. The server maps
//! the finished index read-only and allocates only the candidates returned by a query.
use super::*;
use anyhow::ensure;
use fst::{IntoStreamer, Map, MapBuilder, Streamer};
use memmap2::Mmap;
use std::fs::File;
use std::io::Read;

// Header: magic, syllable/format version, source CRC, entry count, total frequency,
// end of entries, start of FST. FTD2 uses variable record lengths and signed posting
// deltas; its FST prior values point to a shared float table. FTD1 remains readable.
// The trailing CRC covers the header and all sections.
const MAGIC: &[u8; 4] = b"FTD2";
const LEGACY_MAGIC: &[u8; 4] = b"FTD1";
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
    compact: bool,
    completions: std::sync::Mutex<HashMap<(Vec<SyllableKey>, String), Vec<LexEntry>>>,
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

/// Deployed indexes stand alone; optional TSV identities still guard custom/build-time checks.
fn verified_identity(source: Option<&Path>, index: &Path) -> Option<String> {
    let stamp = |path: &Path| -> Option<String> {
        let meta = fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some(format!("{} {}", meta.len(), modified.as_nanos()))
    };
    let source = match source {
        Some(path) => stamp(path)?,
        None => "standalone".into(),
    };
    Some(format!(
        "FTD2 {} {} {}",
        cache_version(),
        stamp(index)?,
        source
    ))
}

fn u32_at(bytes: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap())
}

// Small record lengths and signed offset deltas need fewer bytes without losing precision or order.
fn write_integer(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        bytes.push(value as u8 | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn read_integer(bytes: &[u8], cursor: &mut usize) -> u64 {
    let mut value = 0_u64;
    for shift in (0..35).step_by(7) {
        let byte = bytes[*cursor];
        *cursor += 1;
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return value;
        }
    }
    unreachable!("validated immutable index offset")
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
            write_integer(&mut bytes, u64::from(u32::try_from(entry.word.len())?));
            write_integer(&mut bytes, u64::from(u16::try_from(entry.keys.len())?));
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
                let mut previous = 0_i64;
                for id in group {
                    let current = i64::from(offsets[id]);
                    let delta = current - previous;
                    write_integer(&mut bytes, ((delta << 1) ^ (delta >> 63)) as u64);
                    previous = current;
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
        // Equal frequencies share their exact f32 representation, keeping FST outputs small
        // without quantizing ranking scores or storing one float for every word.
        let mut priors = std::collections::BTreeMap::new();
        for entry in &words {
            priors.insert((entry.freq.ln() as f32).to_bits(), 0_u64);
        }
        for (bits, offset) in &mut priors {
            *offset = u64::try_from(bytes.len())?;
            bytes.extend(bits.to_le_bytes());
        }
        for entry in words {
            index.insert(
                prefixed(PRIOR, entry.word.as_bytes()),
                priors[&(entry.freq.ln() as f32).to_bits()],
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
        Self::open_checked(Some(source), index_path, true)
    }

    /// A deployed index needs no TSV. When the source exists, validate it too.
    /// After one complete CRC check, `record` stores the file identities. Later opens
    /// with the same identities only check the header and bounds;
    /// any replacement (installer, compiler or user) changes the identity and triggers
    /// the full check again. Silent in-place corruption with an unchanged timestamp is
    /// not detected; the shipped index is immutable and replaced only by rename.
    pub fn open_verified(source: &Path, index_path: &Path, record: &Path) -> Result<Self> {
        let source = source.try_exists()?.then_some(source);
        Self::open_remembered(source, index_path, record)
    }

    /// Installed indexes are authoritative even if an older installation left a TSV behind.
    pub fn open_deployed(index_path: &Path, record: &Path) -> Result<Self> {
        Self::open_remembered(None, index_path, record)
    }

    fn open_remembered(source: Option<&Path>, index_path: &Path, record: &Path) -> Result<Self> {
        let identity = verified_identity(source, index_path);
        let trusted = identity.as_ref().is_some_and(|identity| {
            fs::read_to_string(record).is_ok_and(|saved| saved.trim() == identity)
        });
        let lexicon = Self::open_checked(source, index_path, !trusted)?;
        if let (false, Some(identity)) = (trusted, identity) {
            // Failing to remember the result only costs another full check next time.
            if let Some(parent) = record.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = crate::learn::write_atomic(record, identity);
        }
        Ok(lexicon)
    }

    fn open_checked(source: Option<&Path>, index_path: &Path, full: bool) -> Result<Self> {
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
        if full {
            let crc = checksum((&mut file).take(len - 4))?;
            let mut expected = [0; 4];
            file.read_exact(&mut expected)?;
            ensure!(crc == u32::from_le_bytes(expected), "词库索引校验失败");
        }
        // Compilers publish immutable files by rename; no writable mapping is created.
        let file = unsafe { Mmap::map(&file)? };
        ensure!(
            (&file[..4] == MAGIC || &file[..4] == LEGACY_MAGIC)
                && u32_at(&file, 4) == cache_version(),
            "词库索引版本不匹配"
        );
        if let Some(source) = source.filter(|_| full) {
            ensure!(
                checksum(File::open(source)?)? == u32_at(&file, 8),
                "词库索引与源文件不匹配"
            );
        }
        let count = u32_at(&file, 12) as usize;
        let total = f64::from_le_bytes(file[16..24].try_into()?);
        let entries_end = u32_at(&file, 24) as usize;
        let start = u32_at(&file, 28) as usize;
        ensure!(
            HEADER <= entries_end && entries_end <= start && start < file.len() - 4,
            "词库索引边界错误"
        );
        let compact = &file[..4] == MAGIC;
        let index = Map::new(IndexBytes { file, start })?;
        Ok(Self {
            index,
            entries_end,
            count,
            total,
            compact,
            completions: Default::default(),
        })
    }

    fn entries(&self, key: &[u8]) -> impl Iterator<Item = LexEntry> + '_ {
        let data = self.index.as_fst().as_inner();
        let (mut cursor, mut remaining) = self.index.get(key).map_or((0, 0), |offset| {
            let pos = offset as usize;
            let count = u32_at(&data.file, pos) as usize;
            (pos + 4, count)
        });
        let mut previous = 0_i64;
        std::iter::from_fn(move || {
            if remaining == 0 {
                return None;
            }
            remaining -= 1;
            let pos = if self.compact {
                let value = read_integer(&data.file, &mut cursor);
                previous += (value >> 1) as i64 ^ -((value & 1) as i64);
                previous as usize
            } else {
                let offset = u32_at(&data.file, cursor) as usize;
                cursor += 4;
                offset
            };
            let (word, freq, reading, _) = self.record(pos);
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
            Some(LexEntry {
                word: word.to_owned(),
                keys,
                initials,
                freq,
            })
        })
    }

    fn record(&self, pos: usize) -> (&str, f64, &[u8], usize) {
        let data = &self.index.as_fst().as_inner().file[pos..self.entries_end];
        let (word_len, syllables, mut cursor) = if self.compact {
            let mut cursor = 0;
            let word_len = read_integer(data, &mut cursor) as usize;
            let syllables = read_integer(data, &mut cursor) as usize;
            (word_len, syllables, cursor)
        } else {
            (
                u32_at(data, 0) as usize,
                u16::from_le_bytes(data[4..6].try_into().unwrap()) as usize,
                6,
            )
        };
        let freq = f64::from_le_bytes(data[cursor..cursor + 8].try_into().unwrap());
        cursor += 8;
        let word_end = cursor + word_len;
        let record_end = word_end + 2 * syllables;
        (
            std::str::from_utf8(&data[cursor..word_end]).unwrap(),
            freq,
            &data[word_end..record_end],
            record_end,
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
    fn long_completions(
        &self,
        prefix: &[SyllableKey],
        word_prefix: &str,
        limit: usize,
    ) -> Vec<LexEntry> {
        if prefix.len() < 2 || limit == 0 {
            return Vec::new();
        }
        let cache_key = (prefix.to_vec(), word_prefix.to_owned());
        if let Some(found) = self.completions.lock().unwrap().get(&cache_key) {
            return found.iter().take(limit.min(32)).cloned().collect();
        }
        let prefix_key = full_key(prefix.iter().copied());
        let mut stream = self.index.range().ge(&prefix_key).into_stream();
        let mut best = Vec::new();
        while let Some((key, _)) = stream.next() {
            if !key.starts_with(&prefix_key) {
                break;
            }
            if key.len() == prefix_key.len() {
                continue;
            }
            for entry in self
                .entries(key)
                .filter(|entry| long_word_matches(entry, prefix, word_prefix))
            {
                best.push(entry);
                best.sort_unstable_by(|a, b| long_word_order(b, a));
                best.truncate(32);
            }
        }
        let mut cache = self.completions.lock().unwrap();
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.insert(cache_key, best.clone());
        best.truncate(limit.min(32));
        best
    }
    fn complete_long_word(&self, prefix: &[SyllableKey], word_prefix: &str) -> Option<LexEntry> {
        if prefix.len() < 2 {
            return None;
        }
        // 复用已部署的全拼 FST；只读取前缀范围，无须改文件格式或另建常驻索引。
        let prefix_key = full_key(prefix.iter().copied());
        let mut stream = self.index.range().ge(&prefix_key).into_stream();
        let mut best: Option<LexEntry> = None;
        while let Some((key, _)) = stream.next() {
            if !key.starts_with(&prefix_key) {
                break;
            }
            if key.len() == prefix_key.len() {
                continue;
            }
            for entry in self
                .entries(key)
                .filter(|entry| long_word_matches(entry, prefix, word_prefix))
            {
                if best
                    .as_ref()
                    .is_none_or(|old| long_word_order(&entry, old).is_gt())
                {
                    best = Some(entry);
                }
            }
        }
        best
    }

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

    fn has_longer_fuzzy(&self, query: &[SyllableQuery]) -> bool {
        if query.is_empty() {
            return false;
        }
        let Some(key) = crate::lexicon::fuzzy_query_key(query) else {
            return false;
        };
        let fst = self.index.as_fst();
        let mut node = fst.root();
        for byte in fuzzy_key(key.into_iter()) {
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
            let (word, freq, _, size) = self.record(pos);
            visit(word, freq);
            pos += size;
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
            .and_then(|value| {
                if !self.compact {
                    return Some(f32::from_bits(value as u32));
                }
                let data = self.index.as_fst().as_inner();
                let pos = usize::try_from(value).ok()?;
                let bytes = data
                    .file
                    .get(self.entries_end..data.start)?
                    .get(pos.checked_sub(self.entries_end)?..)?;
                Some(f32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
            })
    }
}
