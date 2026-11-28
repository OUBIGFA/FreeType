// Modified by FreeType; compare with WowType 5656e136 for the original implementation.
//! 词典抽象与内存实现。
//!
//! 真实 WeType 词库是 LOUDS trie + 音节 ID 编码，后续解析器只需实现
//! Lexicon trait 即可接入；这里的 MemoryLexicon 同时提供 TSV 加载，便于
//! 先用导出的词表跑通整条链路。
//!
//! TSV 格式：拼音<TAB>词<TAB>频次
//! - 多音节拼音用空格或隔音符分隔，例如：xi an / xi'an
//! - 儿化用独立的 r 标记，例如：hua r
//! - 以 # 开头的行和空行会被忽略。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::{Arc, RwLock};

use anyhow::{anyhow, bail, Context, Result};

use crate::syllable::{Syllable, SyllableId, SyllableQuery};

#[path = "mapped_lexicon.rs"]
mod mapped;
pub use mapped::MappedLexicon;
mod memory;
use memory::*;
mod user;
pub(crate) use user::user_dict_text;
mod matching;
pub(crate) use matching::fuzzy_query_key;
use matching::*;
mod pinyin_text;
pub use pinyin_text::{
    guess_pinyin, normalize_code, normalize_pinyin, parse_pinyin_keys, split_syllables,
};

/// 词库快照的魔数与格式版本。格式一变就改版本号，旧快照自动失效。
const CACHE_MAGIC: &[u8; 4] = b"WLC1";
const CACHE_VERSION: u32 = 2;

/// 占位频次的词估频时给的成词加成（自然对数，约 ×20）。见 `estimate_placeholder_freqs`。
const PLACEHOLDER_BONUS: f64 = 3.0;
/// 同一个频次至少有这么多多音节词才当占位值（小词库、用户词库不处理）。
const PLACEHOLDER_MIN_WORDS: usize = 1000;
/// 切分时每一部分最多几个音节。
const PLACEHOLDER_MAX_PART: usize = 6;

const KEY_HASH_SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// 音节键序列的滚动哈希（FNV-1a）。
fn key_hash_step(hash: u64, key: SyllableKey) -> u64 {
    let raw = u64::from(key.id) | (u64::from(key.erhua) << 16);
    let mut h = hash;
    for byte in raw.to_le_bytes().iter().take(3) {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// 扩展 B 区以后、常见中文字体（Noto CJK、思源）带的字：通用规范汉字表里的 196 个加雾凇
/// 另收的几个（𰻝）。按码位排好序。其余的见 [`LexEntry::is_ext_char`]。
const COMMON_EXT_CHARS: &str = concat!(
    "𠅤𠙶𠳐𡎚𡐓𣗋𣲗𣲘𣸣𤆵𤧛𤩽𤫉𥔲𥕢𥖨𥻗𦈡𦒍𦙶𦝼𦭜𦰡𧿹𨐈𨙸𨚕𨟠𨭉𨱇𨱏𨱑𨱔𨺙𩽾𩾃𩾌𪟝𪣻𪤗",
    "𪨰𪨶𪩘𪾢𫄧𫄨𫄷𫄸𫇭𫌀𫍣𫍯𫍲𫍽𫐄𫐐𫐓𫑡𫓧𫓯𫓶𫓹𫔍𫔎𫔶𫖮𫖯𫖳𫗧𫗴𫘜𫘝𫘦𫘧𫘨𫘪𫘬𫚕𫚖𫚭",
    "𫛭𫞩𫟅𫟦𫟷𫟹𫟼𫠆𫠊𫠜𫢸𫫇𫭟𫭢𫭼𫮃𫰛𫵷𫶇𫷷𫸩𬀩𬀪𬂩𬃊𬇕𬇙𬇹𬉼𬊈𬊤𬌗𬍛𬍡𬍤𬒈𬒔𬒗𬕂𬘓",
    "𬘘𬘡𬘩𬘫𬘬𬘭𬘯𬙂𬙊𬙋𬜬𬜯𬞟𬟁𬟽𬣙𬣞𬣡𬣳𬤇𬤊𬤝𬨂𬨎𬩽𬪩𬬩𬬭𬬮𬬱𬬸𬬹𬬻𬬿𬭁𬭊𬭎𬭚𬭛𬭤",
    "𬭩𬭬𬭯𬭳𬭶𬭸𬭼𬮱𬮿𬯀𬯎𬱖𬱟𬳵𬳶𬳽𬳿𬴂𬴃𬴊𬶋𬶍𬶏𬶐𬶟𬶠𬶨𬶭𬶮𬷕𬸘𬸚𬸣𬸦𬸪𬹼𬺈𬺓𰻝",
);

/// 词典条目。
#[derive(Clone, Debug, PartialEq)]
pub struct LexEntry {
    pub word: String,
    pub freq: f64,
    /// 规范音节键（全拼，含儿化标记）。
    pub keys: Vec<SyllableKey>,
    /// 每个音节的首字母，用于简拼匹配。
    pub initials: Vec<u8>,
}

impl LexEntry {
    /// 生僻字：频次不到 1 的单字（大字表补进来的读音，见 tools/rime2tsv.py --extra-chars）。
    ///
    /// 它们不参与组句、不占前几页，只在翻页的长尾里列出来，保证一个音节的字都找得到。
    /// 一个音节动辄几百个这样的字（yi 有六百多），让束搜索每一步都去排它们会慢很多。
    pub fn is_rare(&self) -> bool {
        self.freq < 1.0 && self.word.chars().count() == 1
    }

    /// CJK 扩展 B 区及以后（U+20000 起）的单字，通用规范汉字表里的除外（[`COMMON_EXT_CHARS`]）。
    ///
    /// 常见中文字体都不带这些字，列出来是方块，默认不出，见
    /// [`crate::BeamConfig::rare_ext_chars`]。不看频次：万象词库把它们都记成普通单字。
    /// 含这些字的词（𩽾𩾌）不算，那是词库特意收的。
    pub fn is_ext_char(&self) -> bool {
        // 四字节 UTF-8 且只有一个字
        if self.word.len() != 4 || self.word.as_bytes()[0] < 0xF0 {
            return false;
        }
        let c = self.word.chars().next().unwrap_or(' ');
        u32::from(c) >= 0x20000 && !COMMON_EXT_CHARS.contains(c)
    }
}

/// 全拼音节键。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SyllableKey {
    pub id: SyllableId,
    pub erhua: bool,
}

impl SyllableKey {
    pub fn new(id: SyllableId, erhua: bool) -> Self {
        Self { id, erhua }
    }
}

/// 词典接口。
///
/// query 是一段连续音节；返回所有“音节序列恰好等于 query”的词。
/// 简拼查询用 SyllableQuery::Jianpin，匹配每个音节的首字母。
pub trait Lexicon: Send + Sync {
    /// 至少两个完整音节起，取词频最高的更长汉字词；word_prefix 非空时还要匹配字面前缀。
    fn complete_long_word(&self, _prefix: &[SyllableKey], _word_prefix: &str) -> Option<LexEntry> {
        None
    }
    /// 有限的备选集合供引擎筛选质量，避免词频最高的切词残片挡住真实长词。
    fn long_completions(
        &self,
        prefix: &[SyllableKey],
        word_prefix: &str,
        _limit: usize,
    ) -> Vec<LexEntry> {
        self.complete_long_word(prefix, word_prefix)
            .into_iter()
            .collect()
    }
    /// Static dictionaries can supply the same prediction priors directly from disk.
    fn prediction_index(&self) -> Option<&dyn PredictionIndex> {
        None
    }
    /// 返回所有权值而非引用：这样「基础词库 + 用户词库」可以合并返回，
    /// 也能在运行时安全地增删用户词。命中数很小，克隆成本可接受。
    fn lookup(&self, query: &[SyllableQuery]) -> Vec<LexEntry>;

    /// 同 [`Lexicon::lookup`]，但不要生僻字（[`LexEntry::is_rare`]）。组句用这个。
    fn lookup_common(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        let mut found = self.lookup(query);
        found.retain(|e| !e.is_rare());
        found
    }

    /// 模糊音查询（z/zh、an/ang 等）。groups 是打开的分组（`syllable::fuzzy::*` 位掩码）。
    /// 默认不支持。
    fn lookup_fuzzy(&self, _query: &[SyllableQuery], _groups: u32) -> Vec<LexEntry> {
        Vec::new()
    }

    /// 把模糊音查询要用的索引先建好（载入词库的后台线程里调，免得第一次按键时现建）。默认什么都不做。
    fn prepare_fuzzy(&self) {}

    /// 是否存在比 query 更长、且以 query 开头的词。
    ///
    /// 束搜索靠它剪掉不可能成词的音节串；不确定时返回 true（只影响速度，不影响结果）。
    fn has_longer(&self, _query: &[SyllableQuery]) -> bool {
        true
    }

    /// 同 [`Lexicon::has_longer`]，但看模糊音读音：是否存在比 query 更长、模糊音归并后
    /// 以 query 开头的词。[`FuzzyLexicon`] 靠它剪枝，否则开着模糊音时束搜索完全不剪，
    /// 每键的查询次数多出六七倍。不确定时返回 true。
    fn has_longer_fuzzy(&self, _query: &[SyllableQuery]) -> bool {
        true
    }

    fn total_freq(&self) -> f64;

    fn len(&self) -> usize;

    /// 逐个给出词库里的 (词, 词频)（同一个词多个读音会给多次）。上屏联想建词频表用，默认没有。
    fn for_each_word(&self, _visit: &mut dyn FnMut(&str, f64)) {}

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 词的一元代价（越大越不可能）。缺省用 -ln(freq/total)。
    fn cost(&self, entry: &LexEntry) -> f64 {
        let total = self.total_freq().max(1.0);
        -((entry.freq.max(0.5)) / total).ln()
    }

    /// 束搜索用的查表：`all` 时含生僻字（[`Lexicon::lookup`]），否则 [`Lexicon::lookup_common`]；
    /// `skip_ext` 时去掉扩展区单字。按（代价, 文字）取前 `limit` 个不同的词，附上代价。
    ///
    /// 束搜索对同一结束位置也按这个顺序只留 `limit` 个不同的词，所以先截断结果不变；
    /// 单音节常有上千条、模糊音再翻几倍，省掉逐条建立候选。[`MemoLexicon`] 跨按键缓存它。
    fn lookup_ranked(
        &self,
        query: &[SyllableQuery],
        all: bool,
        skip_ext: bool,
        limit: usize,
    ) -> Vec<(f64, LexEntry)> {
        let mut entries = if all {
            self.lookup(query)
        } else {
            self.lookup_common(query)
        };
        if skip_ext {
            entries.retain(|e| !e.is_ext_char());
        }
        let mut costed: Vec<(f64, LexEntry)> =
            entries.into_iter().map(|e| (self.cost(&e), e)).collect();
        if costed.len() > limit {
            costed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.word.cmp(&b.1.word)));
            let mut seen = HashSet::new();
            costed.retain(|(_, e)| seen.insert(e.word.clone()));
            costed.truncate(limit);
        }
        costed
    }
}

/// 跨按键复用 [`Lexicon::lookup_ranked`] 的结果。逐键输入时前面的音节不变，
/// 同一段查询每键要重查好几遍（整句、句首词、翻页长尾）；结果只取决于查询和词库内容。
/// `salt` 由词库代数和查询视图（模糊音分组、惩罚）组成，变了就整份清空。
#[derive(Default)]
pub struct LookupMemo {
    state: std::sync::Mutex<MemoState>,
}

#[derive(Default)]
struct MemoState {
    salt: u64,
    entries: usize,
    map: HashMap<
        u64,
        Vec<(
            Vec<SyllableQuery>,
            bool,
            bool,
            usize,
            Arc<Vec<(f64, LexEntry)>>,
        )>,
    >,
}

impl LookupMemo {
    /// 上限：缓存的词条总数约 6 万条（几 MB），超过就整份清空重来。
    const MAX_ENTRIES: usize = 60_000;
    /// 单次结果超过这么多条不缓存。
    const MAX_RESULT: usize = 64;

    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = MemoState::default();
        }
    }
}

/// 带缓存的查询视图；除 [`Lexicon::lookup_ranked`] 外都直接转给 `inner`。
pub struct MemoLexicon<'a> {
    pub inner: &'a dyn Lexicon,
    pub memo: &'a LookupMemo,
    pub salt: u64,
}

impl Lexicon for MemoLexicon<'_> {
    fn long_completions(
        &self,
        prefix: &[SyllableKey],
        word_prefix: &str,
        limit: usize,
    ) -> Vec<LexEntry> {
        self.inner.long_completions(prefix, word_prefix, limit)
    }
    fn complete_long_word(&self, prefix: &[SyllableKey], word_prefix: &str) -> Option<LexEntry> {
        self.inner.complete_long_word(prefix, word_prefix)
    }
    fn prediction_index(&self) -> Option<&dyn PredictionIndex> {
        self.inner.prediction_index()
    }
    fn lookup(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.inner.lookup(query)
    }
    fn lookup_common(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.inner.lookup_common(query)
    }
    fn lookup_fuzzy(&self, query: &[SyllableQuery], groups: u32) -> Vec<LexEntry> {
        self.inner.lookup_fuzzy(query, groups)
    }
    fn prepare_fuzzy(&self) {
        self.inner.prepare_fuzzy()
    }
    fn has_longer(&self, query: &[SyllableQuery]) -> bool {
        self.inner.has_longer(query)
    }
    fn has_longer_fuzzy(&self, query: &[SyllableQuery]) -> bool {
        self.inner.has_longer_fuzzy(query)
    }
    fn total_freq(&self) -> f64 {
        self.inner.total_freq()
    }
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn for_each_word(&self, visit: &mut dyn FnMut(&str, f64)) {
        self.inner.for_each_word(visit)
    }
    fn cost(&self, entry: &LexEntry) -> f64 {
        self.inner.cost(entry)
    }

    fn lookup_ranked(
        &self,
        query: &[SyllableQuery],
        all: bool,
        skip_ext: bool,
        limit: usize,
    ) -> Vec<(f64, LexEntry)> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (query, all, skip_ext, limit).hash(&mut hasher);
        let hash = hasher.finish();
        let same = |item: &&(
            Vec<SyllableQuery>,
            bool,
            bool,
            usize,
            Arc<Vec<(f64, LexEntry)>>,
        )| {
            item.0 == query && item.1 == all && item.2 == skip_ext && item.3 == limit
        };
        if let Ok(state) = self.memo.state.lock() {
            if state.salt == self.salt {
                if let Some(found) = state
                    .map
                    .get(&hash)
                    .and_then(|bucket| bucket.iter().find(same))
                {
                    return found.4.as_ref().clone();
                }
            }
        }
        // 查表不持锁；同一时刻只有服务进程的引擎线程在用，重复计算也只是多做一次。
        let found = self.inner.lookup_ranked(query, all, skip_ext, limit);
        // 翻页长尾的整表（几百上千条）复制和重新解码差不多贵，只缓存截断后的小结果。
        if found.len() > LookupMemo::MAX_RESULT {
            return found;
        }
        if let Ok(mut state) = self.memo.state.lock() {
            if state.salt != self.salt || state.entries + found.len() > LookupMemo::MAX_ENTRIES {
                *state = MemoState {
                    salt: self.salt,
                    ..MemoState::default()
                };
            }
            state.entries += found.len().max(1);
            state.map.entry(hash).or_default().push((
                query.to_vec(),
                all,
                skip_ext,
                limit,
                Arc::new(found.clone()),
            ));
        }
        found
    }
}

/// Read-only word frequencies used by prediction without an extra resident copy.
pub trait PredictionIndex {
    fn log_frequency(&self, word: &str) -> Option<f32>;
}

/// 内存词典。
#[derive(Default)]
pub struct MemoryLexicon {
    entries: Vec<LexEntry>,
    /// 只按前四个音节分桶，每个长词存一次；查询不会遍历整份词库。
    long_prefix_index: HashMap<[SyllableKey; 2], Vec<usize>>,
    full_index: HashMap<Vec<SyllableKey>, Vec<usize>>,
    jianpin_index: HashMap<Vec<u8>, Vec<usize>>,
    /// 模糊音索引：键是各音节在全部分组都打开时的归并键（`syllable::fuzzy_key`），
    /// 查到以后再按实际打开的分组过滤。
    ///
    /// 默认不构建——模糊音默认关闭，而 55 万词库下建这张表要占掉不少加载时间。
    /// 第一次模糊查询时再建。
    fuzzy_index: OnceLock<HashMap<Vec<u32>, Vec<usize>>>,
    /// 模糊音归并读音的所有「真前缀」哈希，供 has_longer_fuzzy 剪枝；和 fuzzy_index 一起建。
    fuzzy_prefixes: OnceLock<HashSet<u64>>,
    /// 全拼词条所有「真前缀」的哈希，供 has_longer 剪枝（哈希碰撞只会少剪，不会错剪）。
    full_prefixes: HashSet<u64>,
    /// 同上，按声母：简拼和「全拼混简拼」（kyi、yongzwen）的查询用它剪枝。
    initial_prefixes: HashSet<u64>,
    total_freq: f64,
    /// 加载时被跳过的行数（未知音节等）。
    skipped: usize,
}

/// 给束搜索用的视图：精确读音之外，再加上模糊音读音的词。
///
/// 以前只在精确读音一个词都没有时才退到模糊音，可 zong、si、dan si 这些本身就有词
/// （总、四、单丝），模糊音永远轮不到，打不出中、是、但是（#93）。
/// 现在模糊音读音和精确读音一起参与组句、排序，只是每个模糊词的代价多 `penalty`
/// （自然对数），差不多常用时精确读音在前。
pub struct FuzzyLexicon<'a> {
    inner: &'a dyn Lexicon,
    groups: u32,
    penalty: f64,
}

impl<'a> FuzzyLexicon<'a> {
    /// groups：打开的模糊音分组（`syllable::fuzzy::*` 位掩码）；penalty：模糊词多出的代价。
    pub fn new(inner: &'a dyn Lexicon, groups: u32, penalty: f64) -> Self {
        Self {
            inner,
            groups,
            penalty,
        }
    }

    /// 精确结果后面接上只有模糊音才对得上的词：每用一处模糊音代价多 penalty（频次打折）。
    fn with_fuzzy(
        &self,
        query: &[SyllableQuery],
        mut found: Vec<LexEntry>,
        common: bool,
    ) -> Vec<LexEntry> {
        let penalty = self.penalty.max(0.0);
        for mut entry in self.inner.lookup_fuzzy(query, self.groups) {
            if common && entry.is_rare() {
                continue;
            }
            // 用了几处模糊音：san -> shang 是声母、韵母各一处
            let used: u32 = entry
                .keys
                .iter()
                .zip(query)
                .map(|(k, q)| match q {
                    SyllableQuery::Full { id, .. } => {
                        crate::syllable::fuzzy_groups_between(*id, k.id).map_or(0, u32::count_ones)
                    }
                    SyllableQuery::Jianpin { .. } => 0,
                })
                .sum();
            // lookup_fuzzy 也返回读音完全相同的词，精确那份已经有了
            if used == 0 {
                continue;
            }
            entry.freq *= (-penalty * f64::from(used)).exp();
            found.push(entry);
        }
        found
    }
}

impl Lexicon for FuzzyLexicon<'_> {
    fn lookup(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.with_fuzzy(query, self.inner.lookup(query), false)
    }

    fn lookup_common(&self, query: &[SyllableQuery]) -> Vec<LexEntry> {
        self.with_fuzzy(query, self.inner.lookup_common(query), true)
    }

    fn lookup_fuzzy(&self, query: &[SyllableQuery], groups: u32) -> Vec<LexEntry> {
        self.inner.lookup_fuzzy(query, groups)
    }

    // 精确或模糊读音任一能接成更长的词才继续往后搜；只看归并读音、不看分组，只会少剪。
    fn has_longer(&self, query: &[SyllableQuery]) -> bool {
        self.inner.has_longer(query) || (self.groups != 0 && self.inner.has_longer_fuzzy(query))
    }

    fn has_longer_fuzzy(&self, query: &[SyllableQuery]) -> bool {
        self.inner.has_longer_fuzzy(query)
    }

    fn total_freq(&self) -> f64 {
        self.inner.total_freq()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

/// 用户词库：在基础词库之上叠加用户自己添加的词。
///
/// 用户词以 (拼音, 词, 频次) 保存，改动时重建一份小的 MemoryLexicon；
/// 用户词通常只有几百到几千条，重建成本可以忽略。
pub struct UserLexicon {
    base: Arc<dyn Lexicon>,
    words: RwLock<Vec<(String, String, f64)>>,
    /// 自定义词（设置里添加的，存在 custom.tsv）：只读，整份替换，不写回用户词库。
    custom: RwLock<Vec<(String, String, f64)>>,
    /// 自定义词里编码不是拼音的（mail -> 邮箱地址）：只在整个输入正好等于编码时给出。
    codes: RwLock<Vec<(String, String)>>,
    /// 学到的英文或混输输入码；由 words 重建，不另存一份用户数据。
    learned_codes: RwLock<Vec<(String, String, f64)>>,
    /// 全部自定义词按文件里的顺序（编码、词，词保留转义），给工具栏的「常用语」列表用。
    phrases: RwLock<Vec<(String, String)>>,
    /// 导入的词库（比如水杉格式的批量文件）：只读，整份替换，不写回用户词库，也不提前排。
    extra: RwLock<Vec<(String, String, f64)>>,
    // 导入词库只在导入/删除时重建，不能随每次选词学习重建几十万条数据。
    extra_snapshot: RwLock<Arc<MemoryLexicon>>,
    snapshot: RwLock<Arc<MemoryLexicon>>,
    /// 两份快照每换一次加一；查询缓存（[`LookupMemo`]）据此失效。
    generation: std::sync::atomic::AtomicU64,
}

/// 自定义词默认频次：和最常用的词差不多，所以排在同音词前面。
pub const CUSTOM_WORD_FREQ: f64 = 5_000_000.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn lex() -> MemoryLexicon {
        MemoryLexicon::from_tsv_str(
            "xian\t先\t1000\nxi an\t西安\t500\nzhong guo\t中国\t800\nhua r\t花儿\t100\n",
        )
        .unwrap()
    }

    #[test]
    fn full_lookup() {
        let lex = lex();
        let xian = Syllable::from_text("xian").unwrap();
        let found = lex.lookup(&[xian.query(false)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].word, "先");
    }

    #[test]
    fn learned_and_imported_layers_keep_independent_lifetimes() {
        let lex = UserLexicon::new(Arc::new(MemoryLexicon::new()));
        let q = [Syllable::from_text("ni").unwrap().query(false)];
        lex.set_extra_words(vec![("ni".into(), "泥".into(), 100.0)]);
        lex.set_custom_tsv("ni\t妮\t200\n");
        lex.add("ni", "你", 300.0).unwrap();
        assert_eq!(lex.lookup(&q).len(), 3);
        assert_eq!(lex.total_freq(), 600.0);
        lex.add("ni", "你", 400.0).unwrap();
        assert_eq!(lex.total_freq(), 700.0);
        lex.clear();
        assert_eq!(
            lex.lookup(&q).len(),
            2,
            "clearing learning must keep imported and custom words"
        );
        lex.set_extra_words(Vec::new());
        assert_eq!(
            lex.lookup(&q)
                .iter()
                .map(|e| e.word.as_str())
                .collect::<Vec<_>>(),
            vec!["妮"]
        );
        lex.set_custom_tsv("");
        assert!(lex.lookup(&q).is_empty());
    }

    #[test]
    fn backup_validation_matches_dictionary_loading_without_building_indexes() {
        for text in [
            "ni\t你\t3\n",
            "unknown\t字\t3\n",
            "ni\t你\tNaN\n",
            "# comment\n",
            "ni\t你\tbad\n",
            "ni\t你\n",
            "ni\t你\t3\textra\n",
        ] {
            assert_eq!(
                MemoryLexicon::validate_tsv_str(text).is_ok(),
                MemoryLexicon::from_tsv_str(text).is_ok(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn apostrophe_and_multi_syllable() {
        let lex = lex();
        let xi = Syllable::from_text("xi").unwrap();
        let an = Syllable::from_text("an").unwrap();
        let found = lex.lookup(&[xi.query(false), an.query(false)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].word, "西安");
    }

    #[test]
    fn mixed_lookup_and_prune() {
        let mut lex = MemoryLexicon::new();
        lex.insert("zhong wen", "中文", 100.0).unwrap();
        lex.insert("zheng wen", "正文", 50.0).unwrap();
        lex.insert("zhong guo ren", "中国人", 80.0).unwrap();
        let wen = Syllable::from_text("wen").unwrap().query(false);
        let guo = Syllable::from_text("guo").unwrap().query(false);
        let z = SyllableQuery::Jianpin { letter: b'z' };
        let words: Vec<String> = lex.lookup(&[z, wen]).into_iter().map(|e| e.word).collect();
        assert_eq!(words.len(), 2, "{words:?}");
        assert!(words.contains(&"中文".to_string()));
        // 剪枝按声母：z guo 后面还有中国人，z wen 后面没有更长的词
        assert!(lex.has_longer(&[z, guo]));
        assert!(!lex.has_longer(&[z, wen]));
        assert!(lex.has_longer(&[z]));
        assert!(!lex.has_longer(&[SyllableQuery::Jianpin { letter: b'q' }]));
    }

    #[test]
    fn jianpin_lookup() {
        let lex = lex();
        let found = lex.lookup(&[
            SyllableQuery::Jianpin { letter: b'z' },
            SyllableQuery::Jianpin { letter: b'g' },
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].word, "中国");
    }

    #[test]
    fn digraph_jianpin_lookup() {
        let mut lex = MemoryLexicon::new();
        lex.insert("zhi chi", "支持", 100.0).unwrap();
        lex.insert("zi chi", "自持", 10.0).unwrap();
        lex.insert("zhi ci", "至此", 50.0).unwrap();
        let words = |q: &[SyllableQuery]| {
            let mut w: Vec<String> = lex.lookup(q).into_iter().map(|e| e.word).collect();
            w.sort();
            w
        };
        let zh = SyllableQuery::Jianpin { letter: b'Z' };
        let ch = SyllableQuery::Jianpin { letter: b'C' };
        let z = SyllableQuery::Jianpin { letter: b'z' };
        let c = SyllableQuery::Jianpin { letter: b'c' };
        let chi = Syllable::from_text("chi").unwrap().query(false);
        assert_eq!(words(&[zh, ch]), vec!["支持"]);
        assert_eq!(words(&[zh, c]), vec!["支持", "至此"]);
        assert_eq!(words(&[z, c]), vec!["支持", "自持", "至此"]);
        assert_eq!(words(&[zh, chi]), vec!["支持"]);
        assert_eq!(words(&[z, chi]), vec!["支持", "自持"]);
        assert!(lex.has_longer(&[zh]));
    }

    #[test]
    fn erhua_lookup() {
        let lex = lex();
        let hua = Syllable::from_text("hua").unwrap();
        let found = lex.lookup(&[hua.query(true)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].word, "花儿");
        assert!(lex.lookup(&[hua.query(false)]).is_empty());
    }

    #[test]
    fn rejects_ambiguous_concatenation() {
        assert!(parse_pinyin_keys("fangan").is_err());
    }

    #[test]
    fn normalize_custom_codes() {
        for (input, want) in [
            ("nihao", "ni hao"),
            ("Ni Hǎo", "ni hao"),
            ("ni3hao3", "ni hao"),
            ("xi'an", "xi an"),
            ("xi\u{2019}an", "xi an"),
            ("xian", "xian"),
            ("Lǜ shī", "lv shi"),
            ("nu:", "nv"),
            ("nu\u{308}3", "nv"),
            ("hua r", "hua r"),
            ("zhongguo", "zhong guo"),
            ("  biang biang mian ", "biang biang mian"),
        ] {
            assert_eq!(normalize_pinyin(input).as_deref(), Some(want), "{input}");
        }
        assert_eq!(normalize_pinyin("mail"), None);
        assert_eq!(normalize_pinyin("r"), None);
        assert_eq!(normalize_pinyin(""), None);
        assert_eq!(normalize_code("mail").as_deref(), Some("mail"));
        assert_eq!(normalize_code("Addr").as_deref(), Some("addr"));
        assert_eq!(normalize_code("o'k").as_deref(), Some("ok"));
        assert_eq!(normalize_code("a b?"), None);
        assert_eq!(normalize_code("  "), None);
    }

    #[test]
    fn guess_pinyin_from_lexicon_text() {
        let lexicon = "# 注释\tshi\t是\t1\nni hao\t你好\t500\nnihao\t你好\t10\nzhong\t重\t100\n\
                       chong\t重\t300\nni\t你\t1\nhao\t好\t1\nshi\t是\t9\r\nlv\t绿\t5";
        let guess = |word: &str| guess_pinyin(lexicon.as_bytes(), word);
        assert_eq!(guess("你好").as_deref(), Some("ni hao"));
        assert_eq!(guess("重").as_deref(), Some("chong"));
        assert_eq!(guess("你是好").as_deref(), Some("ni shi hao"));
        assert_eq!(guess("绿").as_deref(), Some("lv"));
        assert_eq!(guess("你x"), None);
        // 叠字：整词不在词库里，逐字找时两个相同的字都有读音
        assert_eq!(guess("你你好").as_deref(), Some("ni ni hao"));
        assert_eq!(guess("好好").as_deref(), Some("hao hao"));
        assert_eq!(guess("你你x"), None);
        assert_eq!(guess(""), None);
    }

    #[test]
    fn total_freq_and_cost() {
        let lex = lex();
        assert_eq!(lex.total_freq(), 2400.0);
        let entry = &lex.entries[0];
        assert!(lex.cost(entry) > 0.0);
    }

    #[test]
    fn lenient_loading_skips_unknown_syllables() {
        // 「qqq」不是音节，整行跳过而不是让整个词库失败
        let lex = MemoryLexicon::from_tsv_str("ni hao\t你好\t100\nqqq\t呲\t5\n").unwrap();
        assert_eq!(lex.len(), 1);
        assert_eq!(lex.skipped(), 1);
    }

    #[test]
    fn structural_errors_still_fail() {
        assert!(MemoryLexicon::from_tsv_str("只有两列\t词\n").is_err());
        assert!(MemoryLexicon::from_tsv_str("ni hao\t你好\tabc\n").is_err());
    }

    fn temp_lexicon(tag: &str, body: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("wowtype_lex_{}_{}.tsv", tag, std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(cache_path(&path));
        std::fs::write(&path, body).expect("写临时词库失败");
        path
    }

    #[test]
    fn saving_user_words_replaces_the_file_instead_of_truncating_it() {
        use std::io::Read;
        let old = "ni\t你\t1000\n";
        let path = temp_lexicon("atomic-user", old);
        let mut previous = std::fs::File::open(&path).unwrap();
        let lex = UserLexicon::new(Arc::new(MemoryLexicon::new()));
        lex.add("hao", "好", 2000.0).unwrap();
        lex.save_tsv(&path).unwrap();
        let mut text = String::new();
        previous.read_to_string(&mut text).unwrap();
        assert_eq!(
            text, old,
            "an existing reader must retain the complete previous file"
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("好"));
        drop(previous);
        cleanup(&path);
    }

    #[test]
    fn failed_atomic_learning_save_preserves_target_and_unrelated_temp() {
        let root = std::env::temp_dir().join(format!("learning-atomic-{}", std::process::id()));
        std::fs::create_dir_all(root.join("user.tsv")).unwrap();
        let other = root.join("user.tsv.tmp");
        std::fs::write(&other, "unrelated").unwrap();
        let lex = UserLexicon::new(Arc::new(MemoryLexicon::new()));
        assert!(lex.save_tsv(root.join("user.tsv")).is_err());
        assert!(root.join("user.tsv").is_dir());
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "unrelated");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_file(other).unwrap();
        std::fs::remove_dir(root.join("user.tsv")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(cache_path(path));
    }

    #[test]
    fn split_syllables_prefers_fewest_and_longest_first() {
        assert_eq!(split_syllables("nihao").as_deref(), Some("ni hao"));
        assert_eq!(split_syllables("fangan").as_deref(), Some("fang an"));
        assert_eq!(split_syllables("xian").as_deref(), Some("xian"));
        assert_eq!(
            split_syllables("zhongguoren").as_deref(),
            Some("zhong guo ren")
        );
        assert_eq!(split_syllables("github"), None);
        assert_eq!(split_syllables("Ni"), None);
    }

    #[test]
    fn cache_roundtrip_matches_tsv() {
        let path = temp_lexicon("rt", "ni hao\t你好\t100\nzhong guo\t中国\t50\n");
        let first = MemoryLexicon::from_tsv_path_cached(&path).unwrap();
        assert!(cache_path(&path).exists(), "第一次加载应当写出快照");
        let second = MemoryLexicon::from_tsv_path_cached(&path).unwrap();
        assert_eq!(first.len(), second.len());
        assert_eq!(first.entries, second.entries);
        assert_eq!(first.total_freq, second.total_freq);
        cleanup(&path);
    }

    #[test]
    fn stale_cache_is_ignored() {
        let path = temp_lexicon("stale", "ni hao\t你好\t100\n");
        let first = MemoryLexicon::from_tsv_path_cached(&path).unwrap();
        assert_eq!(first.len(), 1);
        // 源文件变了（内容更长），旧快照必须失效
        std::fs::write(
            &path,
            "ni hao\t你好\t100\nzhong guo\t中国\t50\nshi jie\t世界\t30\n",
        )
        .unwrap();
        let second = MemoryLexicon::from_tsv_path_cached(&path).unwrap();
        assert_eq!(second.len(), 3, "源文件更新后应当重新解析");
        cleanup(&path);
    }

    #[test]
    fn same_length_rewrite_in_one_second_invalidates_snapshot() {
        use std::time::{Duration, UNIX_EPOCH};
        let path = temp_lexicon("same-size", "ni\t你\t100\n");
        let base = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for (offset, word) in [(100, "你"), (200, "泥")] {
            fs::write(&path, format!("ni\t{word}\t100\n")).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(base + Duration::from_millis(offset))
                .unwrap();
            let lex = MemoryLexicon::from_tsv_path_cached(&path).unwrap();
            assert_eq!(lex.entries[0].word, word);
        }
        cleanup(&path);
    }

    #[test]
    fn cache_from_other_syllable_table_is_ignored() {
        // 音节表改了（版本号还是旧的那个）：快照里的音节 ID 对不上，必须重新解析
        let path = temp_lexicon("table", "ni hao\t你好\t100\n");
        let lex = MemoryLexicon::from_tsv_path(&path).unwrap();
        let (len, mtime) = source_stamp(&path).unwrap();
        let cache = cache_path(&path);
        lex.write_cache(&cache, len, mtime).unwrap();
        let mut data = std::fs::read(&cache).unwrap();
        assert!(MemoryLexicon::read_cache(&data, len, mtime).is_some());
        data[4..8].copy_from_slice(&CACHE_VERSION.to_le_bytes());
        assert!(MemoryLexicon::read_cache(&data, len, mtime).is_none());
        cleanup(&path);
    }

    #[test]
    fn corrupt_cache_falls_back_to_tsv() {
        let path = temp_lexicon("corrupt", "ni hao\t你好\t100\n");
        let _ = MemoryLexicon::from_tsv_path_cached(&path).unwrap();
        std::fs::write(cache_path(&path), b"not a cache").unwrap();
        let lex = MemoryLexicon::from_tsv_path_cached(&path).unwrap();
        assert_eq!(lex.len(), 1, "快照损坏时应回退到 TSV");
        cleanup(&path);
    }
    #[test]
    fn placeholder_weight_words_get_estimated_from_their_parts() {
        let mut lex = MemoryLexicon::from_tsv_str(
            "bu\t不\t3000000\njian yi\t建议\t500000\nbu jian\t不见\t250000\nyi\t一\t35000000\n\
             bu jian yi\t不建议\t100\nbu jian ying\t不坚硬\t100\nhao\t好\t9000\n\
             zhe ge\t这个\t100\n",
        )
        .unwrap();
        let total = lex.total_freq;
        lex.estimate_placeholder_freqs(2);
        let freq = |w: &str| lex.entries.iter().find(|e| e.word == w).unwrap().freq;
        // 不建议 = 不 × 建议 × 加成，高过「不见 + 一」这句
        let cost = |f: f64| -(f / total).ln();
        assert!(cost(freq("不建议")) < cost(freq("不见")) + cost(freq("一")));
        // 不会高过最冷门的那部分
        assert!(freq("不建议") <= 500000.0);
        // 切不开的词保持原样；总频次不变
        assert_eq!(freq("不坚硬"), 100.0);
        assert_eq!(freq("这个"), 100.0);
        assert_eq!(lex.total_freq, total);
    }
}
