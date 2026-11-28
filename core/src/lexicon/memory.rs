//! 内存词库：TSV 解析、二进制缓存、占位词频估算、模糊音索引与查询。
use super::*;

/// 快照里存的是音节 ID（音节表的下标），音节表一改（比如补了 biang）旧快照就对不上了。
/// 所以写进快照的版本号混进音节表的指纹：表一变，旧快照自动作废重建。
pub(super) fn cache_version() -> u32 {
    // FNV-1a
    let mut hash: u32 = 0x811c_9dc5;
    for text in crate::syllable::SYLLABLES {
        for &byte in text.as_bytes().iter().chain(std::iter::once(&b',')) {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    CACHE_VERSION ^ (hash & 0xffff_ff00)
}

pub(super) fn cache_path(path: &Path) -> std::path::PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".wcache");
    std::path::PathBuf::from(os)
}

/// 快照可能落的位置：先放词库旁边，写不进去（比如 /usr/share 只读）
/// 再放 XDG 缓存目录。
pub(super) fn cache_paths(path: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![cache_path(path)];
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
        });
    if let Some(base) = base {
        // 用 FNV-1a 给路径算个短名字，避免不同词库撞在一起
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in path.to_string_lossy().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        out.push(base.join("wowtype").join(format!("{hash:016x}.wcache")));
    }
    out
}

/// 保留文件系统提供的时间精度，同一秒内覆盖等长词库也要重建快照。
pub(super) fn source_stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .try_into()
        .ok()?;
    Some((meta.len(), mtime))
}

impl MemoryLexicon {
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入一个词。pinyin 使用空格/隔音符分隔音节。
    pub fn insert(&mut self, pinyin: &str, word: &str, freq: f64) -> Result<()> {
        if word.is_empty() {
            bail!("词不能为空");
        }
        if !freq.is_finite() || freq < 0.0 {
            bail!("频次必须是非负有限数，收到 {freq}");
        }
        let keys = parse_pinyin_keys(pinyin)?;
        let initials = keys
            .iter()
            .map(|k| {
                Syllable::from_id(k.id)
                    .map(|s| s.initial_letter())
                    .ok_or_else(|| anyhow!("未知音节 ID: {}", k.id))
            })
            .collect::<Result<Vec<_>>>()?;

        self.add_entry(keys, initials, word.to_string(), freq);
        Ok(())
    }

    /// 已经解析好音节的词条入口（缓存加载用，跳过拼音解析）。
    pub(super) fn add_entry(
        &mut self,
        keys: Vec<SyllableKey>,
        initials: Vec<u8>,
        word: String,
        freq: f64,
    ) {
        let idx = self.entries.len();
        self.entries.push(LexEntry {
            word,
            freq,
            keys: keys.clone(),
            initials: initials.clone(),
        });
        let rare = self.entries[idx].is_rare();
        let entries = &self.entries;
        if keys.len() > 2 {
            self.long_prefix_index
                .entry(keys[..2].try_into().unwrap())
                .or_default()
                .push(idx);
        }
        // 每组里生僻字（LexEntry::is_rare）排在最后，lookup_common 读到它们就停
        let push = |list: &mut Vec<usize>| {
            list.push(idx);
            if !rare {
                let mut at = list.len() - 1;
                while at > 0 && entries[list[at - 1]].is_rare() {
                    list.swap(at - 1, at);
                    at -= 1;
                }
            }
        };
        let mut hash = KEY_HASH_SEED;
        for key in keys.iter().take(keys.len().saturating_sub(1)) {
            hash = key_hash_step(hash, *key);
            self.full_prefixes.insert(hash);
        }
        let mut hash = KEY_HASH_SEED;
        for &letter in initials.iter().take(initials.len().saturating_sub(1)) {
            hash = initial_hash_step(hash, letter);
            self.initial_prefixes.insert(hash);
        }
        push(self.full_index.entry(keys).or_default());
        push(self.jianpin_index.entry(initials).or_default());
        self.total_freq += freq;
    }

    /// 从 TSV 加载，并在旁边维护一份二进制快照。
    ///
    /// 55 万词库的 TSV 解析要 1.3 秒，快照能把这部分省掉。快照按
    /// 「源文件大小 + mtime + 格式版本」校验，词库更新后自动重建。
    pub fn from_tsv_path_cached<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let Some((len, mtime)) = source_stamp(path) else {
            return Self::from_tsv_path(path);
        };
        let candidates = cache_paths(path);
        for cache in &candidates {
            if let Ok(data) = fs::read(cache) {
                if let Some(lex) = Self::read_cache(&data, len, mtime) {
                    return Ok(lex);
                }
            }
        }
        let lex = Self::from_tsv_path(path)?;
        // 写失败不算错误：只读目录会退到 XDG 缓存目录，那里也不行就算了
        for cache in &candidates {
            if lex.write_cache(cache, len, mtime).is_ok() {
                break;
            }
        }
        Ok(lex)
    }

    pub(super) fn write_cache(&self, cache: &Path, src_len: u64, src_mtime: u64) -> Result<()> {
        let mut buf: Vec<u8> = Vec::with_capacity(self.entries.len() * 24);
        buf.extend_from_slice(CACHE_MAGIC);
        buf.extend_from_slice(&cache_version().to_le_bytes());
        buf.extend_from_slice(&src_len.to_le_bytes());
        buf.extend_from_slice(&src_mtime.to_le_bytes());
        buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        // 词频是估过的（见 estimate_placeholder_freqs），总频次单独存，别从估值里加
        buf.extend_from_slice(&self.total_freq.to_le_bytes());
        for entry in &self.entries {
            let word = entry.word.as_bytes();
            buf.extend_from_slice(&(word.len() as u16).to_le_bytes());
            buf.extend_from_slice(word);
            buf.extend_from_slice(&entry.freq.to_le_bytes());
            buf.push(entry.keys.len() as u8);
            for key in &entry.keys {
                let raw = (key.id & 0x7fff) | (u16::from(key.erhua) << 15);
                buf.extend_from_slice(&raw.to_le_bytes());
            }
            buf.push(entry.initials.len() as u8);
            buf.extend_from_slice(&entry.initials);
        }
        if let Some(parent) = cache.parent() {
            fs::create_dir_all(parent).ok();
        }
        let tmp = cache.with_extension("wcache.tmp");
        fs::write(&tmp, &buf).with_context(|| format!("写词库快照失败: {}", tmp.display()))?;
        fs::rename(&tmp, cache)
            .with_context(|| format!("替换词库快照失败: {}", cache.display()))?;
        Ok(())
    }

    pub(super) fn read_cache(data: &[u8], src_len: u64, src_mtime: u64) -> Option<MemoryLexicon> {
        if data.len() < 24 || &data[0..4] != CACHE_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(data[4..8].try_into().ok()?);
        if version != cache_version() {
            return None;
        }
        if u64::from_le_bytes(data[8..16].try_into().ok()?) != src_len
            || u64::from_le_bytes(data[16..24].try_into().ok()?) != src_mtime
        {
            return None;
        }
        let count = u32::from_le_bytes(data.get(24..28)?.try_into().ok()?) as usize;
        let total = f64::from_le_bytes(data.get(28..36)?.try_into().ok()?);
        let mut cursor = 36usize;
        let mut lex = MemoryLexicon::new();
        for _ in 0..count {
            let word_len =
                u16::from_le_bytes(data.get(cursor..cursor + 2)?.try_into().ok()?) as usize;
            cursor += 2;
            let word = std::str::from_utf8(data.get(cursor..cursor + word_len)?)
                .ok()?
                .to_string();
            cursor += word_len;
            let freq = f64::from_le_bytes(data.get(cursor..cursor + 8)?.try_into().ok()?);
            cursor += 8;
            let n_keys = *data.get(cursor)? as usize;
            cursor += 1;
            let mut keys = Vec::with_capacity(n_keys);
            for _ in 0..n_keys {
                let raw = u16::from_le_bytes(data.get(cursor..cursor + 2)?.try_into().ok()?);
                cursor += 2;
                keys.push(SyllableKey::new(raw & 0x7fff, raw & 0x8000 != 0));
            }
            let n_initials = *data.get(cursor)? as usize;
            cursor += 1;
            let initials = data.get(cursor..cursor + n_initials)?.to_vec();
            cursor += n_initials;
            lex.add_entry(keys, initials, word, freq);
        }
        lex.total_freq = total;
        Some(lex)
    }

    /// 从 TSV 文本加载。
    pub fn from_tsv_str(data: &str) -> Result<Self> {
        let mut lex = Self::new();
        Self::visit_tsv(data, |pinyin, word, freq| {
            // 音节不认识（各地方言音、冷僻拼音）时跳过这一行。
            if lex.insert(pinyin, word, freq).is_err() {
                lex.skipped += 1;
            }
        })?;
        Ok(lex)
    }

    /// 与载入使用同一格式校验；备份不查询词条，不必构建整份查询索引。
    /// 未知音节或非有限频次与 from_tsv_str 一样由载入时跳过，不拒绝整份文件。
    pub fn validate_tsv_str(data: &str) -> Result<()> {
        Self::visit_tsv(data, |_, _, _| {})
    }

    pub(super) fn visit_tsv(data: &str, mut visit: impl FnMut(&str, &str, f64)) -> Result<()> {
        for (line_no, raw) in data.lines().enumerate() {
            let line = raw.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let pinyin = cols.next().unwrap_or("");
            let word = cols.next().unwrap_or("");
            let freq = cols.next().unwrap_or("");
            if cols.next().is_some() || pinyin.is_empty() || word.is_empty() || freq.is_empty() {
                bail!("第 {} 行应为 3 列：拼音<TAB>词<TAB>频次", line_no + 1);
            }
            let freq: f64 = freq
                .parse()
                .with_context(|| format!("第 {} 行频次不是数字: {freq}", line_no + 1))?;
            visit(pinyin, word, freq);
        }
        Ok(())
    }

    /// 给「占位频次」的词估一个频次。
    ///
    /// rime-ice 的 ext / tencent 两个词表（加起来五十多万词，占整个词库一半以上）
    /// 所有词的频次都是同一个占位值 100。按字面当频次用，这些词就比任何常用字
    /// 拼出来的句子都冷门得多：`bujianyi` 里「不建议」排在「不见一」「部件」后面，
    /// 第一页根本看不到。
    ///
    /// 做法：找出多音节词里占比很高的那个频次（占位值），把这些词按自己的拼音
    /// 和字切成词库里已有的词，频次取「各部分概率相乘」再乘一个成词加成
    /// [`PLACEHOLDER_BONUS`]——词库收了它，说明这几个字常挨着用，相当于没有
    /// 二元模型时给它补上搭配分。不会低于原值，也不会高过最冷门的那一部分。
    pub(super) fn estimate_placeholder_freqs(&mut self, min_words: usize) {
        let mut hist: HashMap<u64, usize> = HashMap::new();
        let mut multi = 0usize;
        for entry in &self.entries {
            if entry.keys.len() >= 2 {
                multi += 1;
                *hist.entry(entry.freq.to_bits()).or_default() += 1;
            }
        }
        let Some((&bits, &count)) = hist.iter().max_by_key(|(_, &c)| c) else {
            return;
        };
        if count < min_words || count * 4 < multi {
            return;
        }
        let placeholder = f64::from_bits(bits);
        let bonus = std::env::var("WOWTYPE_PLACEHOLDER_BONUS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(PLACEHOLDER_BONUS);
        let total = self.total_freq.max(1.0);
        let mut updates = Vec::new();
        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.freq != placeholder || entry.keys.len() < 2 {
                continue;
            }
            let chars: Vec<char> = entry.word.chars().collect();
            let n = chars.len();
            if n != entry.keys.len() {
                continue; // 儿化、混了字母数字的词，字和音节对不齐
            }
            // best[j]：前 j 个字切成已有词的最小代价；rarest[j]：这种切法里最冷门部分的频次
            let mut best = vec![f64::INFINITY; n + 1];
            let mut rarest = vec![f64::INFINITY; n + 1];
            best[0] = 0.0;
            for i in 0..n {
                if !best[i].is_finite() {
                    continue;
                }
                for j in (i + 1)..=n.min(i + PLACEHOLDER_MAX_PART) {
                    if j - i == n {
                        continue; // 不能用自己
                    }
                    let Some(ids) = self.full_index.get(&entry.keys[i..j]) else {
                        continue;
                    };
                    let part: String = chars[i..j].iter().collect();
                    let freq = ids
                        .iter()
                        .map(|&k| &self.entries[k])
                        .filter(|e| e.word == part)
                        .map(|e| e.freq)
                        .fold(0.0f64, f64::max);
                    if freq <= 0.0 {
                        continue;
                    }
                    let cost = best[i] - (freq / total).ln();
                    if cost < best[j] {
                        best[j] = cost;
                        rarest[j] = rarest[i].min(freq);
                    }
                }
            }
            if !best[n].is_finite() {
                continue;
            }
            let estimate = (total * (bonus - best[n]).exp()).min(rarest[n]);
            if estimate > entry.freq {
                updates.push((idx, estimate));
            }
        }
        for (idx, freq) in updates {
            self.entries[idx].freq = freq;
        }
    }

    pub(super) fn build_fuzzy_prefixes(&self) -> HashSet<u64> {
        let mut set = HashSet::new();
        for entry in &self.entries {
            if let Some(fuzzy) = entry
                .keys
                .iter()
                .map(|k| crate::syllable::fuzzy_key(k.id))
                .collect::<Option<Vec<_>>>()
            {
                for end in 1..fuzzy.len() {
                    set.insert(fuzzy_hash(&fuzzy[..end]));
                }
            }
        }
        set
    }

    pub(super) fn build_fuzzy_index(&self) -> HashMap<Vec<u32>, Vec<usize>> {
        let mut map: HashMap<Vec<u32>, Vec<usize>> = HashMap::new();
        for (idx, entry) in self.entries.iter().enumerate() {
            if let Some(fuzzy) = entry
                .keys
                .iter()
                .map(|k| crate::syllable::fuzzy_key(k.id))
                .collect::<Option<Vec<_>>>()
            {
                map.entry(fuzzy).or_default().push(idx);
            }
        }
        map
    }

    /// 加载时被跳过的行数。
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// 从 TSV 文件加载。
    pub fn from_tsv_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let data = fs::read_to_string(path)
            .with_context(|| format!("读取词典失败: {}", path.display()))?;
        let mut lex = Self::from_tsv_str(&data)?;
        lex.estimate_placeholder_freqs(PLACEHOLDER_MIN_WORDS);
        Ok(lex)
    }
}

impl MemoryLexicon {
    /// 索引里每组的生僻字都排在最后（见 add_entry），不要它们时只取前面那段。
    pub(super) fn without_rare<'a>(&self, idxs: &'a [usize], skip_rare: bool) -> &'a [usize] {
        if skip_rare {
            &idxs[..idxs.partition_point(|&i| !self.entries[i].is_rare())]
        } else {
            idxs
        }
    }

    pub(super) fn lookup_impl(&self, query: &[SyllableQuery], skip_rare: bool) -> Vec<LexEntry> {
        let take = |&i: &usize| self.entries[i].clone();
        if query.is_empty() {
            return Vec::new();
        }

        let all_full = query
            .iter()
            .all(|q| matches!(q, SyllableQuery::Full { .. }));
        let all_jianpin = query
            .iter()
            .all(|q| matches!(q, SyllableQuery::Jianpin { .. }));

        if all_full {
            let key: Vec<SyllableKey> = query
                .iter()
                .map(|q| match q {
                    SyllableQuery::Full { id, erhua } => SyllableKey::new(*id, *erhua),
                    _ => unreachable!(),
                })
                .collect();
            return self
                .full_index
                .get(&key)
                .map(|idxs| {
                    self.without_rare(idxs, skip_rare)
                        .iter()
                        .map(take)
                        .collect()
                })
                .unwrap_or_default();
        }

        if all_jianpin && !query.iter().any(needs_initial_check) {
            let key: Vec<u8> = query
                .iter()
                .map(|q| match q {
                    SyllableQuery::Jianpin { letter } => *letter,
                    _ => unreachable!(),
                })
                .collect();
            return self
                .jianpin_index
                .get(&key)
                .map(|idxs| {
                    self.without_rare(idxs, skip_rare)
                        .iter()
                        .map(take)
                        .collect()
                })
                .unwrap_or_default();
        }

        // 全拼混简拼（k yi -> 可以）、zh/ch/sh 简拼：先按首字母查，再逐个比对
        let Some(initials) = query_initials(query) else {
            return Vec::new();
        };
        self.jianpin_index
            .get(&initials)
            .map(|idxs| {
                self.without_rare(idxs, skip_rare)
                    .iter()
                    .filter(|&&i| entry_matches(&self.entries[i], query))
                    .map(take)
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Lexicon for MemoryLexicon {
    fn long_completions(
        &self,
        prefix: &[SyllableKey],
        word_prefix: &str,
        limit: usize,
    ) -> Vec<LexEntry> {
        let Some(head) = prefix
            .get(..2)
            .and_then(|slice| <[SyllableKey; 2]>::try_from(slice).ok())
        else {
            return Vec::new();
        };
        let Some(indices) = self.long_prefix_index.get(&head) else {
            return Vec::new();
        };
        let mut entries: Vec<_> = indices
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|entry| long_word_matches(entry, prefix, word_prefix))
            .collect();
        entries.sort_unstable_by(|a, b| long_word_order(b, a));
        entries.into_iter().take(limit.min(32)).cloned().collect()
    }
    fn complete_long_word(&self, prefix: &[SyllableKey], word_prefix: &str) -> Option<LexEntry> {
        let head: [SyllableKey; 2] = prefix.get(..2)?.try_into().ok()?;
        self.long_prefix_index
            .get(&head)?
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|entry| long_word_matches(entry, prefix, word_prefix))
            .max_by(|a, b| long_word_order(a, b))
            .cloned()
    }

    fn lookup(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.lookup_impl(query, false)
    }

    fn lookup_common(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.lookup_impl(query, true)
    }

    fn has_longer(&self, query: &[SyllableQuery]) -> bool {
        if query.iter().any(|q| q.is_jianpin()) {
            // 简拼、混合查询按声母剪（只看声母会少剪一些，不会错剪）
            let Some(initials) = query_initials(query) else {
                return true;
            };
            let hash = initials
                .iter()
                .fold(KEY_HASH_SEED, |h, &letter| initial_hash_step(h, letter));
            return self.initial_prefixes.contains(&hash);
        }
        let mut hash = KEY_HASH_SEED;
        for q in query {
            if let SyllableQuery::Full { id, erhua } = q {
                hash = key_hash_step(hash, SyllableKey::new(*id, *erhua));
            }
        }
        self.full_prefixes.contains(&hash)
    }

    fn has_longer_fuzzy(&self, query: &[SyllableQuery]) -> bool {
        if query.is_empty() {
            return false;
        }
        let Some(key) = fuzzy_query_key(query) else {
            return false;
        };
        self.fuzzy_prefixes
            .get_or_init(|| self.build_fuzzy_prefixes())
            .contains(&fuzzy_hash(&key))
    }

    fn prepare_fuzzy(&self) {
        self.fuzzy_prefixes
            .get_or_init(|| self.build_fuzzy_prefixes());
        // 55 万条的词库约 0.2 秒
        self.fuzzy_index.get_or_init(|| self.build_fuzzy_index());
    }

    fn lookup_fuzzy(&self, query: &[SyllableQuery], groups: u32) -> Vec<LexEntry> {
        if query.is_empty() || groups == 0 {
            return Vec::new();
        }
        let mut ids = Vec::with_capacity(query.len());
        let mut key = Vec::with_capacity(query.len());
        for q in query {
            match q {
                SyllableQuery::Full { id, erhua } => {
                    if *erhua {
                        return Vec::new();
                    }
                    match crate::syllable::fuzzy_key(*id) {
                        Some(canon) => key.push(canon),
                        None => return Vec::new(),
                    }
                    ids.push(*id);
                }
                SyllableQuery::Jianpin { .. } => return Vec::new(),
            }
        }
        let index = self.fuzzy_index.get_or_init(|| self.build_fuzzy_index());
        let Some(idxs) = index.get(&key) else {
            return Vec::new();
        };
        // 索引按全部分组归并，这里只留下「用到的分组都打开了」的词
        idxs.iter()
            .map(|&i| &self.entries[i])
            .filter(|entry| {
                entry.keys.len() == ids.len()
                    && entry.keys.iter().zip(&ids).all(|(k, &q)| {
                        crate::syllable::fuzzy_groups_between(q, k.id)
                            .is_some_and(|need| need & !groups == 0)
                    })
            })
            .cloned()
            .collect()
    }

    fn total_freq(&self) -> f64 {
        self.total_freq
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn for_each_word(&self, visit: &mut dyn FnMut(&str, f64)) {
        for entry in &self.entries {
            visit(&entry.word, entry.freq);
        }
    }
}
