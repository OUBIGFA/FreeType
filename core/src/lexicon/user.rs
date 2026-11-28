//! 个人词库：学习词、自定义短语、导入的额外词与按编码的补全。
use super::*;

pub(crate) fn user_dict_text(words: &[(String, String, f64)]) -> String {
    let mut out = String::from("# 用户词库\n");
    for (code, word, freq) in words {
        out.push_str(&format!("{code}\t{word}\t{freq}\n"));
    }
    out
}

impl UserLexicon {
    pub fn new(base: Arc<dyn Lexicon>) -> Self {
        Self {
            base,
            words: RwLock::new(Vec::new()),
            custom: RwLock::new(Vec::new()),
            codes: RwLock::new(Vec::new()),
            learned_codes: RwLock::new(Vec::new()),
            phrases: RwLock::new(Vec::new()),
            extra: RwLock::new(Vec::new()),
            extra_snapshot: RwLock::new(Arc::new(MemoryLexicon::new())),
            snapshot: RwLock::new(Arc::new(MemoryLexicon::new())),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// 添加或更新一个用户词。拼音可以是 ni hao / nihao / xi'an。
    pub fn add(&self, pinyin: &str, word: &str, freq: f64) -> Result<()> {
        let pinyin = user_reading(pinyin, word, freq)?;

        {
            let mut words = self.words.write().unwrap();
            if let Some(slot) = words.iter_mut().find(|(_, w, _)| w == word) {
                if slot.0 == pinyin && slot.2 == freq {
                    return Ok(());
                }
                slot.0 = pinyin;
                slot.2 = freq;
            } else {
                words.push((pinyin, word.to_string(), freq));
            }
        }
        self.rebuild();
        Ok(())
    }

    /// 合并备份词条：先全部校验，已有词保留当前读音，权重只取较大值。
    pub(crate) fn merge_words(&self, incoming: &[(String, String, f64)]) -> Result<usize> {
        let mut checked = Vec::with_capacity(incoming.len());
        for (pinyin, word, frequency) in incoming {
            checked.push((
                user_reading(pinyin, word, *frequency)?,
                word.clone(),
                *frequency,
            ));
        }
        let mut changed = 0;
        {
            let mut words = self
                .words
                .write()
                .map_err(|_| anyhow!("user words lock poisoned"))?;
            // 整份备份只扫描一次现有词表，重复词仍保留本机读音。
            let mut positions: HashMap<String, usize> = words
                .iter()
                .enumerate()
                .map(|(index, (_, word, _))| (word.clone(), index))
                .collect();
            for (pinyin, word, frequency) in checked {
                if let Some(&index) = positions.get(&word) {
                    if frequency > words[index].2 {
                        words[index].2 = frequency;
                        changed += 1;
                    }
                } else {
                    positions.insert(word.clone(), words.len());
                    words.push((pinyin, word, frequency));
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.rebuild();
        }
        Ok(changed)
    }

    /// 删除用户词，返回是否删掉了东西。
    pub fn remove(&self, word: &str) -> bool {
        let removed = {
            let mut words = self.words.write().unwrap();
            let before = words.len();
            words.retain(|(_, w, _)| w != word);
            words.len() != before
        };
        if removed {
            self.rebuild();
        }
        removed
    }

    pub(crate) fn user_word(&self, word: &str) -> Option<(String, f64)> {
        self.words
            .read()
            .ok()?
            .iter()
            .find(|(_, text, _)| text == word)
            .map(|(code, _, freq)| (code.clone(), *freq))
    }

    /// 只在词条仍是这次自动学习写入的值时撤销，保留用户随后手工改的值。
    pub(crate) fn restore_word(
        &self,
        word: &str,
        before: Option<(String, f64)>,
        after: (String, f64),
    ) -> bool {
        if self.user_word(word) != Some(after) {
            return false;
        }
        match before {
            Some((code, freq)) => self.add(&code, word, freq).is_ok(),
            None => self.remove(word),
        }
    }

    pub fn words(&self) -> Vec<(String, String, f64)> {
        self.words.read().map(|w| w.clone()).unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut words) = self.words.write() {
            words.clear();
        }
        self.rebuild();
    }

    /// 把用户词写成 TSV（与 lexicon 格式一致）。空的也写（只有注释行）：
    /// 用户把学到的词都删掉以后，旧文件不能留着，不然重启又回来了。
    pub fn save_tsv<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        crate::learn::write_atomic(path.as_ref(), &user_dict_text(&self.words()))
            .with_context(|| format!("写入用户词库失败: {}", path.as_ref().display()))
    }

    /// 从 TSV 载入用户词，追加到现有内容。
    pub fn load_tsv<P: AsRef<Path>>(&self, path: P) -> Result<usize> {
        let data = fs::read_to_string(path.as_ref())
            .with_context(|| format!("读取用户词库失败: {}", path.as_ref().display()))?;
        let mut added = 0;
        let mut nonempty = false;
        // 批量合并后只建一次索引；逐条调用 add 会使启动耗时随学习词数平方增长。
        let mut words = self
            .words
            .write()
            .map_err(|_| anyhow!("user words lock poisoned"))?;
        let mut positions: HashMap<String, usize> = words
            .iter()
            .enumerate()
            .map(|(i, (_, word, _))| (word.clone(), i))
            .collect();
        for line in data.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            nonempty = true;
            let mut cols = line.split('\t');
            let (Some(pinyin), Some(word)) = (cols.next(), cols.next()) else {
                continue;
            };
            let freq = cols.next().and_then(|f| f.parse().ok()).unwrap_or(1000.0);
            if let Ok(pinyin) = user_reading(pinyin, word, freq) {
                if let Some(&index) = positions.get(word) {
                    words[index] = (pinyin, word.to_string(), freq);
                } else {
                    positions.insert(word.to_string(), words.len());
                    words.push((pinyin, word.to_string(), freq));
                }
                added += 1;
            }
        }
        drop(words);
        anyhow::ensure!(!nonempty || added > 0, "找不到可识别的用户词条");
        if added > 0 {
            self.rebuild();
        }
        Ok(added)
    }

    /// 用一份自定义词整体替换现有的。格式 `编码<TAB>词[<TAB>频次]`。
    ///
    /// 编码一般是拼音（可以不分音节）；不是拼音、但全是字母的（mail、wdyx）当成自定义编码，
    /// 只在整个输入正好等于它时给出。词里可以写 `\n`（换行）、`\t`（制表符），上屏时才换成真的。
    /// 其他行跳过。返回收下的条数。
    pub fn set_custom_tsv(&self, data: &str) -> usize {
        let mut custom = Vec::new();
        let mut codes = Vec::new();
        let mut phrases = Vec::new();
        for line in data.lines() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(pinyin), Some(word)) = (cols.next(), cols.next()) else {
                continue;
            };
            let (pinyin, word) = (pinyin.trim().to_ascii_lowercase(), word.trim());
            // 手写的 custom.tsv 可能不分音节（nihao）：自己切开
            if word.is_empty() {
                continue;
            }
            let code = pinyin.clone();
            let Some(pinyin) = (if parse_pinyin_keys(&pinyin).is_ok() {
                Some(pinyin)
            } else {
                split_syllables(&pinyin)
            }) else {
                let code: String = code.chars().filter(|c| *c != '\'').collect();
                if !code.is_empty() && code.bytes().all(|b| b.is_ascii_lowercase()) {
                    phrases.push((code.clone(), word.to_string()));
                    codes.push((code, word.to_string()));
                }
                continue;
            };
            phrases.push((code, word.to_string()));
            let freq = cols
                .next()
                .and_then(|f| f.trim().parse::<f64>().ok())
                .filter(|f| f.is_finite() && *f > 0.0)
                .unwrap_or(CUSTOM_WORD_FREQ);
            custom.push((pinyin, word.to_string(), freq));
        }
        let count = custom.len() + codes.len();
        let mut rebuild = false;
        if let Ok(mut slot) = self.custom.write() {
            if *slot != custom {
                *slot = custom;
                rebuild = true;
            }
        }
        if let Ok(mut slot) = self.codes.write() {
            *slot = codes;
        }
        if let Ok(mut slot) = self.phrases.write() {
            *slot = phrases;
        }
        if rebuild {
            self.rebuild();
        }
        count
    }

    /// 全部自定义词，按 custom.tsv 里的顺序：(编码, 词)。词还是转义过的（`\n`），上屏前要
    /// [`crate::unescape_custom`]。
    pub fn custom_phrases(&self) -> Vec<(String, String)> {
        self.phrases.read().map(|p| p.clone()).unwrap_or_default()
    }

    /// 从文件读自定义词（整体替换）；文件不存在就清空。
    pub fn load_custom<P: AsRef<Path>>(&self, path: P) -> Result<usize> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(data) => Ok(self.set_custom_tsv(&data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(self.set_custom_tsv("")),
            Err(e) => Err(e).with_context(|| format!("读取自定义词失败: {}", path.display())),
        }
    }

    /// 是不是设置里添加的自定义词。
    pub fn is_custom(&self, word: &str) -> bool {
        self.custom
            .read()
            .map(|c| c.iter().any(|(_, w, _)| w == word))
            .unwrap_or(false)
    }

    /// 自定义编码对应的词（按文件里的顺序）。code 不分大小写、忽略隔音符。
    pub fn custom_code_words(&self, code: &str) -> Vec<String> {
        let code: String = code
            .chars()
            .filter(|c| *c != '\'')
            .flat_map(char::to_lowercase)
            .collect();
        self.codes
            .read()
            .map(|codes| {
                codes
                    .iter()
                    .filter(|(c, _)| *c == code)
                    .map(|(_, w)| w.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 与自定义编码一样只命中完整输入；保留读音/输入码供后续选词继续计数。
    pub(crate) fn learned_code_words(&self, code: &str) -> Vec<(String, String, f64)> {
        let code: String = code
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '\'')
            .flat_map(char::to_lowercase)
            .collect();
        self.learned_codes
            .read()
            .map(|words| {
                words
                    .iter()
                    .filter(|(input, _, _)| input == &code)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn learned_code_prefix(&self, code: &str) -> Vec<(String, String, f64)> {
        let code = code.to_ascii_lowercase();
        self.learned_codes
            .read()
            .map(|words| {
                words
                    .iter()
                    .filter(|(input, word, _)| {
                        input.starts_with(&code)
                            && input.len() > code.len()
                            && word.bytes().all(|b| b.is_ascii_alphabetic())
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn personal_long_completions(
        &self,
        prefix: &[SyllableKey],
        word_prefix: &str,
    ) -> Vec<LexEntry> {
        self.snapshot
            .read()
            .map(|s| s.long_completions(prefix, word_prefix, 32))
            .unwrap_or_default()
    }

    pub fn custom_words(&self) -> Vec<(String, String, f64)> {
        self.custom.read().map(|w| w.clone()).unwrap_or_default()
    }

    /// 整份替换导入的词库：(空格分音节的拼音, 词, 频次)。拼音不合法的跳过，返回收下的条数。
    pub fn set_extra_words(&self, words: Vec<(String, String, f64)>) -> usize {
        let words: Vec<_> = words
            .into_iter()
            .filter(|(pinyin, word, _)| !word.is_empty() && parse_pinyin_keys(pinyin).is_ok())
            .collect();
        let count = words.len();
        if self.extra.read().is_ok_and(|existing| *existing == words) {
            return count;
        }
        let mut snapshot = MemoryLexicon::new();
        for (pinyin, word, freq) in &words {
            let _ = snapshot.insert(pinyin, word, *freq);
        }
        // 第一次模糊匹配也不能把导入词库的索引构建推给按键线程。
        snapshot.prepare_fuzzy();
        if let Ok(mut slot) = self.extra.write() {
            *slot = words;
        }
        if let Ok(mut slot) = self.extra_snapshot.write() {
            *slot = Arc::new(snapshot);
        }
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        count
    }

    pub fn extra_words(&self) -> Vec<(String, String, f64)> {
        self.extra.read().map(|w| w.clone()).unwrap_or_default()
    }

    pub(super) fn rebuild(&self) {
        let mut lexicon = MemoryLexicon::new();
        let mut learned_codes = Vec::new();
        // 数据在加入词库时已校验，直接构建索引，省去整表 TSV 往返。
        if let Ok(words) = self.words.read() {
            for (input, word, freq) in words.iter() {
                if lexicon.insert(input, word, *freq).is_err() {
                    learned_codes.push((input.clone(), word.clone(), *freq));
                }
            }
        }
        if let Ok(custom) = self.custom.read() {
            for (input, word, freq) in custom.iter() {
                let _ = lexicon.insert(input, word, *freq);
            }
        }
        if let Ok(mut slot) = self.snapshot.write() {
            *slot = Arc::new(lexicon);
        }
        if let Ok(mut slot) = self.learned_codes.write() {
            *slot = learned_codes;
        }
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// 词库内容的代数：学到的词、自定义词或导入词库变了就加一。
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// 中文词仍校验音节；英文、混输词无法拆成拼音时保存实际输入码。
/// 使用同一校验入口，保证直接添加、TSV 重载和 JSON 备份恢复接受相同词条。
pub(super) fn user_reading(input: &str, word: &str, frequency: f64) -> Result<String> {
    if word.trim().is_empty()
        || word.contains(['\t', '\r', '\n', '\0'])
        || input.trim().is_empty()
        || input.contains(['\t', '\r', '\n', '\0'])
    {
        bail!("用户词及其输入不能为空，也不能包含换行或制表符");
    }
    if !frequency.is_finite() || frequency < 0.0 {
        bail!("频次必须是非负有限数");
    }
    match parse_pinyin_keys(input.trim()) {
        Ok(_) => Ok(input.trim().to_string()),
        Err(error) => {
            let code: String = input
                .chars()
                .filter(|c| *c != ' ' && *c != '\'')
                .flat_map(char::to_lowercase)
                .collect();
            if word.chars().any(|c| c.is_ascii_alphabetic())
                && code.bytes().any(|b| b.is_ascii_alphanumeric())
                && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b';')
            {
                Ok(code)
            } else {
                Err(error).context("用户词拼音无法解析")
            }
        }
    }
}

impl Lexicon for UserLexicon {
    fn long_completions(
        &self,
        prefix: &[SyllableKey],
        word_prefix: &str,
        limit: usize,
    ) -> Vec<LexEntry> {
        let mut entries = self.base.long_completions(prefix, word_prefix, limit);
        if let Ok(snapshot) = self.snapshot.read() {
            entries.extend(snapshot.long_completions(prefix, word_prefix, limit));
        }
        if let Ok(snapshot) = self.extra_snapshot.read() {
            entries.extend(snapshot.long_completions(prefix, word_prefix, limit));
        }
        entries.sort_unstable_by(|a, b| long_word_order(b, a));
        let mut seen = std::collections::HashSet::new();
        entries.retain(|entry| seen.insert(entry.word.clone()));
        entries.truncate(limit.min(32));
        entries
    }
    fn complete_long_word(&self, prefix: &[SyllableKey], word_prefix: &str) -> Option<LexEntry> {
        // 学习和导入数据沿用各自的快照，删除或换词库后不留下过期补全。
        [
            self.base.complete_long_word(prefix, word_prefix),
            self.snapshot
                .read()
                .ok()
                .and_then(|s| s.complete_long_word(prefix, word_prefix)),
            self.extra_snapshot
                .read()
                .ok()
                .and_then(|s| s.complete_long_word(prefix, word_prefix)),
        ]
        .into_iter()
        .flatten()
        .max_by(long_word_order)
    }

    fn lookup(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        let mut result = self.base.lookup(query);
        if let Ok(snapshot) = self.snapshot.read() {
            result.extend(snapshot.lookup(query));
        }
        if let Ok(snapshot) = self.extra_snapshot.read() {
            result.extend(snapshot.lookup(query));
        }
        result
    }

    fn lookup_common(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        let mut result = self.base.lookup_common(query);
        if let Ok(snapshot) = self.snapshot.read() {
            result.extend(snapshot.lookup_common(query));
        }
        if let Ok(snapshot) = self.extra_snapshot.read() {
            result.extend(snapshot.lookup_common(query));
        }
        result
    }

    fn has_longer(&self, query: &[SyllableQuery]) -> bool {
        self.base.has_longer(query)
            || self
                .snapshot
                .read()
                .map(|s| s.has_longer(query))
                .unwrap_or(true)
            || self
                .extra_snapshot
                .read()
                .map(|s| s.has_longer(query))
                .unwrap_or(true)
    }

    fn has_longer_fuzzy(&self, query: &[SyllableQuery]) -> bool {
        self.base.has_longer_fuzzy(query)
            || self
                .snapshot
                .read()
                .map(|s| s.has_longer_fuzzy(query))
                .unwrap_or(true)
            || self
                .extra_snapshot
                .read()
                .map(|s| s.has_longer_fuzzy(query))
                .unwrap_or(true)
    }

    fn prepare_fuzzy(&self) {
        self.base.prepare_fuzzy();
    }

    fn lookup_fuzzy(&self, query: &[SyllableQuery], groups: u32) -> Vec<LexEntry> {
        let mut result = self.base.lookup_fuzzy(query, groups);
        if let Ok(snapshot) = self.snapshot.read() {
            result.extend(snapshot.lookup_fuzzy(query, groups));
        }
        if let Ok(snapshot) = self.extra_snapshot.read() {
            result.extend(snapshot.lookup_fuzzy(query, groups));
        }
        result
    }

    fn total_freq(&self) -> f64 {
        self.base.total_freq()
            + self.snapshot.read().map(|s| s.total_freq()).unwrap_or(0.0)
            + self
                .extra_snapshot
                .read()
                .map(|s| s.total_freq())
                .unwrap_or(0.0)
    }

    fn len(&self) -> usize {
        self.base.len()
            + self.snapshot.read().map(|s| s.len()).unwrap_or(0)
            + self.extra_snapshot.read().map(|s| s.len()).unwrap_or(0)
    }
}
