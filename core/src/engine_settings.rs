//! 引擎设置：模型、模糊音、双拼、纠错、英文词表与简繁转换。
use super::*;

impl Engine {
    /// 加载微信输入法的字级神经语言模型（`nlrgpt2_nnlm.bin`），返回字表大小。
    /// 模型不随 WowType 分发，见 `docs/DATA.md`。
    pub fn load_nnlm<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let path = path.as_ref();
        let model = nnlm::Nnlm::load(path)?;
        let size = model.vocab_size();
        self.install_nnlm(Arc::new(model), path.to_path_buf());
        Ok(size)
    }

    pub fn set_nnlm(&self, model: Option<Arc<nnlm::Nnlm>>) {
        if let Ok(mut path) = self.nnlm_path.write() {
            *path = None;
        }
        if let Ok(mut slot) = self.nnlm.write() {
            *slot = model;
        }
    }

    pub fn model_path(&self) -> Option<std::path::PathBuf> {
        self.nnlm_path.read().ok()?.clone()
    }

    /// Windows 设置已完成文件校验与原子替换后，接续使用内存中的模型。
    pub fn install_nnlm(&self, model: Arc<nnlm::Nnlm>, path: std::path::PathBuf) {
        self.set_nnlm(Some(model));
        if let Ok(mut slot) = self.nnlm_path.write() {
            *slot = Some(path);
        }
    }

    pub fn has_nnlm(&self) -> bool {
        self.nnlm.read().map(|m| m.is_some()).unwrap_or(false)
    }

    pub fn with_config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_ranker(mut self, ranker: Arc<dyn Ranker>) -> Self {
        self.ranker = ranker;
        self
    }

    pub fn set_beam_size(&mut self, beam_size: usize) {
        self.config.beam.beam_size = beam_size.max(1);
    }

    /// 开关模糊音。
    pub fn set_fuzzy(&mut self, on: bool) {
        self.config.fuzzy = on;
    }

    /// 把模糊音索引先建好。模糊音打开或者兜底（默认开）都要用；不先建的话，第一次碰到
    /// 要查模糊音的输入时现建，会卡一下（55 万条约 0.2 秒）。载入词库的后台线程里调。
    pub fn prepare_fuzzy(&self) {
        self.base_lexicon.prepare_fuzzy();
    }

    /// 模糊音关着时的兜底（见 [`EngineConfig::fuzzy_fallback`]），默认开。
    pub fn set_fuzzy_fallback(&mut self, on: bool) {
        self.config.fuzzy_fallback = on;
    }

    /// 设置打开的模糊音分组（`syllable::fuzzy::*` 位掩码）。不用重建索引。
    pub fn set_fuzzy_groups(&mut self, groups: u32) {
        self.config.fuzzy_groups = groups & crate::syllable::fuzzy::ALL;
    }

    pub fn set_typo_correction(&mut self, on: bool) {
        self.config.typo_correction = on;
    }

    pub fn set_typo_kinds(&mut self, kinds: u8) {
        self.config.typo_kinds = kinds & crate::graph::typo::ALL;
    }

    /// 开关组字期间的长词补全，独立于上屏后的联想。
    pub fn set_long_word_completion(&mut self, on: bool) {
        self.config.long_word_completion = on;
    }

    /// 是否也出 CJK 扩展 B 区及以后的单字（默认不出，常见字体显示不了；通用规范汉字表里的照常）。
    pub fn set_rare_ext_chars(&mut self, on: bool) {
        self.config.beam.rare_ext_chars = on;
    }

    /// 开关双拼。
    pub fn set_shuangpin(&mut self, on: bool) {
        self.config.shuangpin = on;
    }

    /// 选双拼方案（不改开关）。
    pub fn set_shuangpin_scheme(&mut self, scheme: crate::shuangpin::Scheme) {
        self.config.shuangpin_scheme = scheme;
    }

    /// 编辑区里怎么显示还没选的输入：双拼翻成全拼（`vsgo` -> `zhong'guo`），全拼返回 None。
    pub fn shuangpin_display(&self, input: &str) -> Option<String> {
        self.config
            .shuangpin
            .then(|| crate::shuangpin::display(self.config.shuangpin_scheme, input))
    }

    /// 开关繁体输出。
    pub fn set_traditional(&mut self, on: bool) {
        self.config.traditional = on;
    }

    /// 载入简繁映射表，返回条目数。
    /// 载入英文词表，返回词条数。
    ///
    /// 每行一个词；也直接接受 rime 的 en.dict.yaml（「上屏<TAB>编码」，
    /// 注释和 YAML 头会被跳过）。只收 2~32 个字母的编码。
    pub fn load_english<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let text = std::fs::read_to_string(path)?;
        Ok(self.load_english_str(&text))
    }

    /// 从字符串载入英文词表（追加），返回累计词条数。
    pub fn load_english_str(&self, text: &str) -> usize {
        use std::collections::btree_map::Entry;
        let mut map: std::collections::BTreeMap<String, String> = self
            .english
            .read()
            .map(|m| m.as_ref().clone())
            .unwrap_or_default();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let mut cols = line.split('\t');
            let word = cols.next().unwrap_or("").trim();
            let code = cols.next().map(str::trim).unwrap_or(word);
            if code.len() < 2 || code.len() > 32 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
                continue;
            }
            if word.is_empty() || !word.eq_ignore_ascii_case(code) {
                // 缩写（January<TAB>Jan）只认上屏与编码一致的条目
                continue;
            }
            let key = code.to_ascii_lowercase();
            match map.entry(key) {
                Entry::Vacant(slot) => {
                    slot.insert(word.to_string());
                }
                Entry::Occupied(mut slot) => {
                    // 同一个词有大小写两种写法时，留全小写的（march 而不是 March）
                    if word.bytes().all(|b| !b.is_ascii_uppercase()) {
                        slot.insert(word.to_string());
                    }
                }
            }
        }
        let len = map.len();
        if let Ok(mut slot) = self.english.write() {
            *slot = Arc::new(map);
        }
        len
    }

    pub fn english_len(&self) -> usize {
        self.english.read().map(|m| m.len()).unwrap_or(0)
    }

    pub fn load_conversion<P: AsRef<std::path::Path>>(&self, path: P) -> Result<usize> {
        let conv = Converter::from_tsv_path(path)?;
        let len = conv.len();
        if let Ok(mut slot) = self.converter.write() {
            *slot = Arc::new(conv);
        }
        Ok(len)
    }

    /// 直接装载一张映射表（测试或程序内生成时用）。
    pub fn install_converter(&self, conv: Converter) {
        if let Ok(mut slot) = self.converter.write() {
            *slot = Arc::new(conv);
        }
    }

    pub fn conversion_len(&self) -> usize {
        self.converter.read().map(|c| c.len()).unwrap_or(0)
    }

    /// 把候选转成繁体，并合并转换后重复的候选。分词不转，还是词库里的写法（学词用）。
    pub(super) fn apply_traditional(&self, candidates: &mut Vec<Candidate>) {
        let conv = match self.converter.read() {
            Ok(slot) if !slot.is_empty() => slot.clone(),
            _ => return,
        };
        let mut best: HashMap<String, Candidate> = HashMap::new();
        for mut candidate in std::mem::take(candidates) {
            candidate.text = conv.convert(&candidate.text);
            match best.entry(candidate.text.clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(candidate);
                }
                Entry::Occupied(mut slot) => {
                    if candidate.score > slot.get().score {
                        slot.insert(candidate);
                    }
                }
            }
        }
        let mut list: Vec<Candidate> = best.into_values().collect();
        list.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.text.cmp(&b.text))
        });
        list.truncate(self.config.max_candidates.max(1));
        *candidates = list;
    }

    pub fn set_max_candidates(&mut self, max_candidates: usize) {
        self.config.max_candidates = max_candidates.max(1);
    }
}
