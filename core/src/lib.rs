// Modified by FreeType; compare with WowType 5656e136 for the original implementation.
//! WowType 拼音引擎骨架。
//!
//! 流水线：
//! 输入预处理 -> 音节图 -> 束搜索（词典 + 二元 LM） -> 特征/线性打分 -> 候选。
//! LTR / 神经重排 / MixRank 只在接口上预留，尚未接入真实模型。

pub mod autospace;
mod backup;
pub mod beam;
pub mod calc;
pub mod clipboard;
pub mod convert;
pub mod datetime;
pub mod emoji;
pub mod enhancement;
pub mod ffi;
pub mod graph;
pub mod input_tools;
pub mod learn;
pub mod lexicon;
pub mod llama;
pub mod lm;
mod long_completion;
pub mod lookup_table;
pub mod msime;
pub mod nnlm;
pub mod predict;
pub mod preedit;
pub mod rank;
pub mod sentence;
pub mod session;
pub mod session_ffi;
pub mod shuangpin;
pub mod syllable;
pub mod symbols;
pub mod train;
pub mod translate;
pub mod translate_ffi;
#[cfg(feature = "voice")]
pub mod voice_ffi;
#[cfg(not(feature = "voice"))]
pub mod voice_stub;
pub mod wordlm;

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

pub use crate::beam::{beam_search, BeamConfig, BeamPath, BeamWord};
pub use crate::convert::Converter;
pub use crate::graph::{Edge, EdgeKind, GraphMode, GraphOptions, SyllableGraph};
pub use crate::learn::{LearnedLm, UserPairs};
pub use crate::lexicon::{
    FuzzyLexicon, LexEntry, Lexicon, MemoryLexicon, SyllableKey, UserLexicon,
};
pub use crate::lm::{ArpaLm, BigramLm, LanguageModel};
pub use crate::rank::{
    Feature, FeatureWeights, LinearRanker, LtrModel, Ranker, FEATURE_COUNT, FEATURE_NAMES,
};
pub use crate::syllable::{
    is_valid, parse, split, ParseError, ParsedInput, Syllable, SyllableId, SyllableQuery, SYLLABLES,
};

/// 最终候选。
#[derive(Clone, Debug)]
pub struct Candidate {
    pub text: String,
    pub pinyin: String,
    pub score: f64,
    pub features: Feature,
    pub words: Vec<CandidateWord>,
    /// 这个候选用掉了原始输入的前多少字节。
    ///
    /// 整句候选等于输入长度；句首词候选（长句先选「你好」，剩下的接着打）更短，
    /// 前端上屏后应把剩余部分留在输入框里继续转换。
    pub consumed: usize,
    /// 补出了尚未输入的完整音节；按普通候选显示，不把它固定在前缀编码下。
    pub long_completion: bool,
}

/// 候选内的一个分词。
#[derive(Clone, Debug)]
pub struct CandidateWord {
    pub text: String,
    pub pinyin: String,
    pub freq: f64,
    pub known: bool,
}

/// 引擎配置。
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub beam: BeamConfig,
    pub max_candidates: usize,
    /// 全拼走不通时是否尝试简拼（zg -> 中国）。
    pub jianpin_fallback: bool,
    /// 是否启用模糊音（总开关）。
    pub fuzzy: bool,
    /// 打开的模糊音分组（`syllable::fuzzy::*` 位掩码），默认平翘舌 + 前后鼻音。
    pub fuzzy_groups: u32,
    /// 每用一处模糊音（一个声母或韵母）多出的代价（自然对数），越大越靠后。
    pub fuzzy_penalty: f64,
    /// 模糊音关着时的兜底：精确读音只能凑出硬拼的结果（yingai -> 因该），而按模糊音
    /// 是个常用词（应该）时，把这个词也给出来。见 [`Engine::fuzzy_fallback`]。
    pub fuzzy_fallback: bool,
    /// 是否使用双拼。
    pub shuangpin: bool,
    /// 双拼方案（自然码、小鹤……）。
    pub shuangpin_scheme: crate::shuangpin::Scheme,
    /// 是否在候选末尾追加原始输入（英文/未登录串）。
    pub english_candidate: bool,
    /// 是否把候选转成繁体。
    pub traditional: bool,
    /// 选词达到阈值后自动加入用户词库。
    pub auto_learn: bool,
    /// 自动学习阈值（选多少次后进用户词库）。
    pub auto_learn_threshold: u32,
    /// 按错键纠错（相邻键、两键按反、多按/漏按）。双拼下不生效。
    pub typo_correction: bool,
    /// 各类全拼错因，可沿用既有总开关单独暂停所有纠错。
    pub typo_kinds: u8,
    /// 句首词候选最多几个（0 表示不给）。长句打错时可以先选开头的词。
    pub prefix_candidates: usize,
    /// 前面几页之后把剩下的词全补上（整个输入的同音词、句首词、单字），直到
    /// `max_candidates`。关掉时只给最可能的那几页。
    pub more_candidates: bool,
    /// 至少四个完整音节后补一个词库长词；由支持此功能的外壳显式开启。
    pub long_word_completion: bool,
    /// 有神经语言模型时，按束搜索分数取前几条整句用它重排（0 表示不用）。
    pub nnlm_rerank: usize,
}

/// 上文末尾和候选连起来本身是一个词（我 + 要 = 我要）时，上文排序特征至少给这么多
/// （自然对数，和语言模型的分数同一个尺度）。见 [`Engine::word_lm_context_score`]。
pub const MERGED_PAIR_SCORE: f64 = 1.5;

/// 分几次选出来的词，多长以内按二次确认收录（人名、术语、成语大多在四个字以内）。
pub const COMPOSED_WORD_MAX_CHARS: usize = 4;
/// 混输短词按汉字数判断，同时沿用英文候选的长度范围，避免把长句当作造词。
const MIXED_WORD_MAX_CHARS: usize = 32;

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            beam: BeamConfig::default(),
            max_candidates: 10,
            jianpin_fallback: true,
            fuzzy: false,
            fuzzy_groups: crate::syllable::fuzzy::DEFAULT,
            fuzzy_penalty: 2.5,
            fuzzy_fallback: true,
            shuangpin: false,
            shuangpin_scheme: crate::shuangpin::Scheme::Ziranma,
            english_candidate: true,
            traditional: false,
            auto_learn: true,
            auto_learn_threshold: 3,
            typo_correction: true,
            typo_kinds: crate::graph::typo::ALL,
            prefix_candidates: 16,
            more_candidates: true,
            long_word_completion: false,
            nnlm_rerank: 8,
        }
    }
}

/// 用户历史里的一条记录。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistoryEntry {
    pub count: f64,
    /// 上次使用时间（epoch 毫秒）。
    pub last_used_ms: u64,
}

/// 当前时间（epoch 毫秒）。
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 拼音引擎。
pub struct Engine {
    /// 原始词库（只读）。
    base_lexicon: Arc<dyn Lexicon>,
    /// 基础词库 + 用户词的合并视图，束搜索实际用它。
    lexicon: Arc<UserLexicon>,
    lm: Arc<dyn LanguageModel>,
    ranker: Arc<dyn Ranker>,
    config: EngineConfig,
    /// 用户选词历史：候选文本 -> (次数, 上次时间)。
    history: RwLock<HashMap<String, HistoryEntry>>,
    pending_words: RwLock<HashMap<String, learn::NewWord>>,
    boundaries: RwLock<sentence::Boundaries>,
    enhancement: RwLock<Option<Arc<dyn enhancement::Backend>>>,
    learning_journal: std::sync::Mutex<learn::Journal>,
    /// 光标前已上屏的文本，用于上下文 bigram 特征。
    context: RwLock<String>,
    /// 用户学到的词搭配（上一个词 -> 这个词）。
    pairs: RwLock<UserPairs>,
    /// 最近一次选的词的最后一个词；光标前的文本以它结尾时，下一个词和它组成搭配。
    last_word: RwLock<String>,
    /// 上一次 input() 给出的候选：文本 -> 分词，选词时用它知道整句是哪几个词。
    last_words: RwLock<HashMap<String, Vec<String>>>,
    /// 上屏联想用的词频表（词的哈希 -> ln 词频），第一次联想时建，见 [`predict`]。
    word_prior: OnceLock<HashMap<u64, f32>>,
    /// 显式固定排位与自动选词热度互不影响。
    manual_pins: RwLock<session::ManualPins>,
    /// 简繁字符映射表。
    converter: RwLock<Arc<Converter>>,
    /// 英文词表：小写编码 -> 上屏形式（github -> GitHub）。
    english: RwLock<Arc<std::collections::BTreeMap<String, String>>>,
    /// 字级神经语言模型（微信输入法的 nlrgpt2，本机有才加载），给整句候选重排。
    nnlm: RwLock<Option<Arc<nnlm::Nnlm>>>,
    nnlm_path: RwLock<Option<std::path::PathBuf>>,
    /// 束搜索查表结果的跨按键缓存，见 [`lexicon::MemoLexicon`]。
    lookup_memo: lexicon::LookupMemo,
    input_tools: OnceLock<input_tools::InputTools>,
}

impl Engine {
    pub fn new(lexicon: Arc<dyn Lexicon>, lm: Arc<dyn LanguageModel>) -> Self {
        let merged = Arc::new(UserLexicon::new(lexicon.clone()));
        Self {
            base_lexicon: lexicon,
            lexicon: merged,
            lm,
            ranker: Arc::new(LinearRanker::default()),
            config: EngineConfig::default(),
            history: RwLock::new(HashMap::new()),
            pending_words: RwLock::new(HashMap::new()),
            boundaries: RwLock::new(sentence::Boundaries::new()),
            enhancement: RwLock::new(None),
            learning_journal: std::sync::Mutex::new(learn::Journal::default()),
            context: RwLock::new(String::new()),
            pairs: RwLock::new(UserPairs::default()),
            last_word: RwLock::new(String::new()),
            last_words: RwLock::new(HashMap::new()),
            word_prior: OnceLock::new(),
            input_tools: OnceLock::new(),
            manual_pins: RwLock::new(session::ManualPins::new()),
            converter: RwLock::new(Arc::new(Converter::new())),
            english: RwLock::new(Arc::new(std::collections::BTreeMap::new())),
            nnlm: RwLock::new(None),
            nnlm_path: RwLock::new(None),
            lookup_memo: lexicon::LookupMemo::default(),
        }
    }

    /// 束搜索用的带缓存视图。缓存键含词库代数和视图参数（模糊音分组、惩罚），
    /// 学到新词、导入词库或改模糊音设置后旧结果自动作废。
    fn memo_view<'a>(&'a self, inner: &'a dyn Lexicon, fuzzy: bool) -> lexicon::MemoLexicon<'a> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (self.lexicon.generation(), fuzzy).hash(&mut hasher);
        if fuzzy {
            (
                self.config.fuzzy_groups,
                self.config.fuzzy_penalty.to_bits(),
            )
                .hash(&mut hasher);
        }
        lexicon::MemoLexicon {
            inner,
            memo: &self.lookup_memo,
            salt: hasher.finish(),
        }
    }

    /// 设置光标前的文本（上一次上屏的内容）。
    pub fn set_context(&self, text: &str) {
        if let Ok(mut slot) = self.context.write() {
            slot.clear();
            slot.push_str(text);
        }
    }

    pub fn context(&self) -> String {
        self.context.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// 候选显示的字在词库里怎么写。开着繁体时候选是转换过的，学词、记热度和搭配、
    /// 忘词都得用转换前的写法：词库、热度、搭配都按它查，关掉繁体后学过的词也照样能用。
    /// 不是上一次 input() 的候选就原样返回。
    pub fn lexicon_form(&self, shown: &str) -> String {
        if self.config.traditional {
            if let Some(words) = self
                .last_words
                .read()
                .ok()
                .and_then(|m| m.get(shown).cloned())
            {
                if !words.is_empty() {
                    return words.concat();
                }
            }
        }
        shown.to_string()
    }

    /// 词库里的写法在候选里显示成什么：开着繁体就转一次。
    pub(crate) fn shown_text(&self, text: &str) -> String {
        match self.converter.read() {
            Ok(conv) if self.config.traditional && !conv.is_empty() => conv.convert(text),
            _ => text.to_string(),
        }
    }

    pub fn lexicon(&self) -> &Arc<UserLexicon> {
        &self.lexicon
    }

    pub fn base_lexicon(&self) -> &Arc<dyn Lexicon> {
        &self.base_lexicon
    }

    pub fn lm(&self) -> &Arc<dyn LanguageModel> {
        &self.lm
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// 输入拼音串，返回候选；解析失败时返回空列表。
    /// 基于上一词的候选预测（「预测词库」）。
    ///
    /// 返回 (词, 对数概率)，按概率降序。语言模型没接入预测能力时返回空。
    pub fn predict_next(&self, prev: &str, limit: usize) -> Vec<(String, f64)> {
        self.lm.top_next(prev, limit)
    }

    pub fn input(&self, raw: &str) -> Vec<Candidate> {
        self.input_at(raw, now_ms())
    }

    /// 带时间戳的输入：历史特征的时间衰减按 at_ms 计算，便于测试与回放。
    pub fn input_at(&self, raw: &str, at_ms: u64) -> Vec<Candidate> {
        self.try_input_at(raw, at_ms).unwrap_or_default()
    }

    /// 可返回错误的版本，供 CLI/FFI 使用。
    pub fn try_input(&self, raw: &str) -> Result<Vec<Candidate>> {
        self.try_input_at(raw, now_ms())
    }

    pub fn try_input_at(&self, raw: &str, at_ms: u64) -> Result<Vec<Candidate>> {
        let mut result = if self.config.shuangpin {
            self.shuangpin_candidates(raw, at_ms)?
        } else {
            self.pinyin_candidates(raw, None, true, at_ms)?
        };
        self.prepend_custom_codes(raw, at_ms, &mut result);
        self.add_long_completion(raw, &mut result);
        self.add_english_prefix(raw, at_ms, &mut result);
        if let Ok(mut last) = self.last_words.write() {
            last.clear();
            for c in &result {
                if !is_raw_candidate(c) {
                    let words: Vec<String> = c.words.iter().map(|w| w.text.clone()).collect();
                    if self.config.traditional {
                        last.insert(words.concat(), words.clone());
                    }
                    last.insert(c.text.clone(), words);
                }
            }
        }
        // 自定义词里的 \n、\t 到这里才换成真的换行、制表符（词库是一行一条的 TSV）
        for c in &mut result {
            if c.text.contains('\\') {
                c.text = unescape_custom(&c.text);
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;

mod candidate_order;
mod engine_candidates;
mod engine_learning;
mod engine_query;
mod engine_settings;
pub use candidate_order::unescape_custom;
use candidate_order::*;
mod candidate_shape;
use candidate_shape::*;
