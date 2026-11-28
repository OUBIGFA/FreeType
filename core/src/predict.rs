// Added by FreeType; not part of WowType 5656e136.
//! 上屏联想：一个词上屏以后，猜下一个词。
//!
//! 数据全用现成的：词级二元模型给「这两个词连着出现比各自出现常见多少」（提升度，
//! [`LanguageModel::next_lifts`]），词库给词频，用户学到的搭配（[`crate::learn::UserPairs`]）给习惯。
//!
//! 排序分 = [`LIFT_WEIGHT`] × 提升度 + ln 词频 + [`HABIT_WEIGHT`] × ln(1 + 搭配热度)。
//!
//! - 只按提升度排全是冷僻词（实测「今天」后面排出「腊月初九」「挨板子」），因为越少见的词
//!   提升度越容易顶到上限；
//! - 提升度 + ln 词频就是条件概率 P(下一个词 | 上一个词)，但前几名总是「的、了、是」；
//! - 所以提升度的权重给到 1.5：实测「明天」→ 早上、就、考试，「人工」→ 客服、窗口、操作。
//!
//! 耗时（随包词库和模型，实测）：一般不到 1 毫秒；后面搭配极多的词（「我」「是」「在」）约 4 毫秒，
//! 「的」（八万多个搭配）约 18 毫秒。只在上屏时算一次，不是每个键都算。
//!
//! 模型给的词必须是词库里的纯汉字词（词库已经滤过屏蔽词，模型的词表没有）；
//! 用户自己连着用过的词不受这个限制。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::{autospace, now_ms, Engine};

/// 提升度的权重，见模块说明。
pub const LIFT_WEIGHT: f64 = 1.5;
/// 用户搭配的权重：用过一次加 4.2、两次 6.6、三次 8.3（热度按 30 天半衰期衰减）。
/// 模型里靠前的词之间通常差 1–3 分，所以连着用过两三次的搭配会排到最前面，
/// 只碰巧连着打过一次的不会压过明显更常见的词。
pub const HABIT_WEIGHT: f64 = 6.0;
/// 用户用过、但词库里没有词频的词（自造词、英文）按这个词频算。
const UNKNOWN_FREQ: f64 = 1000.0;

/// 一个联想词。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prediction {
    /// 上屏、显示的字（开着繁体时是转换过的）。
    pub text: String,
    /// 词库里的写法，选中以后交给 [`Engine::record_prediction`]。
    pub word: String,
}

fn word_key(word: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    word.hash(&mut hasher);
    hasher.finish()
}

impl crate::lexicon::PredictionIndex for HashMap<u64, f32> {
    fn log_frequency(&self, word: &str) -> Option<f32> {
        self.get(&word_key(word)).copied()
    }
}

impl Engine {
    /// 词 → ln 词频。静态词库直接使用编译索引；内存词库只在第一次联想时建表。
    ///
    /// 只存词的哈希：存整个词要多占二三十 MB；两个词哈希相同只会让其中一个的词频不准。
    fn word_prior(&self) -> &dyn crate::lexicon::PredictionIndex {
        if let Some(prior) = self.base_lexicon.prediction_index() {
            return prior;
        }
        self.word_prior.get_or_init(|| {
            let mut prior: HashMap<u64, f32> = HashMap::with_capacity(self.base_lexicon.len());
            self.base_lexicon.for_each_word(&mut |word, freq| {
                // 词频不到 1 的是大字表补进来的生僻字，不拿来联想
                if freq < 1.0 {
                    return;
                }
                let value = freq.ln() as f32;
                let slot = prior.entry(word_key(word)).or_insert(value);
                // 多音字、多个读音的词：取最常用的那个读音
                *slot = slot.max(value);
            });
            prior
        })
    }

    /// 提前建好联想要用的词频表，免得第一次联想时卡一下（设置里开着上屏联想时外壳调）。
    pub fn prepare_prediction(&self) {
        self.word_prior();
    }

    /// 上屏联想。`context` 是光标前的字（刚上屏的字已经在里面）；它不以上一次选的词结尾
    /// （换了输入框、删了字、刚上屏的是标点）就不联想，返回空。最多 `limit` 个，最可能的在前。
    pub fn predict_after(&self, context: &str, limit: usize) -> Vec<Prediction> {
        self.predict_after_at(context, limit, now_ms())
    }

    /// 带时间戳的版本（搭配热度的衰减按 `at_ms` 算，测试用）。
    pub fn predict_after_at(&self, context: &str, limit: usize, at_ms: u64) -> Vec<Prediction> {
        let Some(prev) = self.start_word_in(context) else {
            return Vec::new();
        };
        if limit == 0 {
            return Vec::new();
        }
        let prior = self.word_prior();
        let Ok(pairs) = self.pairs.read() else {
            return Vec::new();
        };
        // 用户在 prev 后面用过的词：(加分, 模型里见过没有)
        let mut habits: HashMap<&str, (f64, bool)> = pairs
            .after(&prev)
            .map(|(word, entry)| {
                let heat = crate::learn::decayed(entry, at_ms);
                (word, (HABIT_WEIGHT * (1.0 + heat).ln(), false))
            })
            .collect();

        let model = self.nnlm.read().ok().and_then(|slot| slot.clone());
        let prefix = model.as_ref().map(|m| m.context_tokens(context));
        let mut first_scores = HashMap::new();
        // Only the best first-character continuations need full inference. Keep personal pairs separately.
        let keep = if model.is_some() {
            limit.max(12)
        } else {
            limit
        } + habits.len();
        let mut top: Vec<(f64, String)> = Vec::new();
        let mut floor = f64::NEG_INFINITY;
        self.lm.next_lifts(&prev, &mut |word, lift| {
            let habit = habits.get_mut(word).map(|slot| {
                slot.1 = true;
                slot.0
            });
            // 模型的词要比随便出现更常见；先看这个，省掉大多数词的词频查找
            if habit.is_none() && lift <= 0.0 {
                return;
            }
            let known = prior.log_frequency(word).map(f64::from);
            let ln_freq = match (habit, known) {
                (_, Some(known)) => known,
                (Some(_), None) => UNKNOWN_FREQ.ln(),
                (None, None) => return,
            };
            // 模型的词：不是原词重复、纯汉字
            if habit.is_none() && (word == prev || !word.chars().all(autospace::is_han)) {
                return;
            }
            let score = match (&model, &prefix) {
                (Some(model), Some(prefix)) if habit.is_none() => {
                    let Some(first) = word.chars().next() else {
                        return;
                    };
                    if !model.knows(first) {
                        return;
                    }
                    let score = *first_scores
                        .entry(first)
                        .or_insert_with(|| model.next_char_logprob(prefix, first))
                        as f64;
                    if score < -(400.0f64).ln() {
                        return;
                    }
                    score
                }
                (Some(_), _) => 100.0 + habit.unwrap_or_default(),
                _ => LIFT_WEIGHT * lift + ln_freq + habit.unwrap_or(0.0),
            };
            if score < floor {
                return;
            }
            top.push((score, word.to_string()));
            if top.len() >= keep * 2 {
                top.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                top.truncate(keep);
                floor = top[keep - 1].0;
            }
        });
        // 模型里没有的搭配：只有用户的习惯
        for (word, (bonus, seen)) in &habits {
            if !seen {
                let ln_freq = prior
                    .log_frequency(word)
                    .map_or(UNKNOWN_FREQ.ln(), f64::from);
                top.push((
                    if model.is_some() {
                        100.0 + bonus
                    } else {
                        ln_freq + bonus
                    },
                    (*word).to_string(),
                ));
            }
        }
        if let (Some(model), Some(prefix)) = (&model, &prefix) {
            for (score, word) in &mut top {
                if !habits.contains_key(word.as_str()) {
                    *score = model.score(prefix, word) / (word.chars().count() as f64).sqrt();
                }
            }
        }
        drop(pairs);
        top.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        top.truncate(limit);
        top.into_iter()
            .map(|(_, word)| Prediction {
                text: self.shown_text(&word),
                word,
            })
            .collect()
    }

    /// 用户选了一个联想词：记下「上一个词 → 它」的搭配，它成为新的上一个词（接着联想）。
    ///
    /// 调用前先 [`Engine::set_context`] 成选之前光标前的字。不走
    /// [`Engine::record_selection`]：那边按上一次拼音候选的分词拆词，联想词不是拼音打出来的。
    pub fn record_prediction(&self, word: &str) {
        self.record_prediction_at(word, now_ms());
    }

    pub fn record_prediction_at(&self, word: &str, at_ms: u64) {
        if word.is_empty() {
            return;
        }
        let start = self.start_word();
        let mut need_prune = false;
        if let (Some(prev), Ok(mut pairs)) = (start, self.pairs.write()) {
            if let Some(change) = pairs.bump(&prev, word, at_ms) {
                self.note_learning(change);
            }
            need_prune = pairs.len() > crate::learn::MAX_USER_PAIRS;
        }
        if need_prune {
            if let (Ok(mut history), Ok(mut pairs)) = (self.history.write(), self.pairs.write()) {
                crate::learn::prune_learning(&mut history, &mut pairs);
            }
        }
        self.record_last_word(word);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::lexicon::MemoryLexicon;
    use crate::lm::BigramLm;
    use crate::Engine;

    /// 「今天」后面：下雨（少见但搭配强）、去（常见但搭配弱）、的（极常见、没有搭配）、冷僻词。
    fn engine() -> Engine {
        let lexicon = MemoryLexicon::from_tsv_str(
            "jin tian\t今天\t500000\nxia yu\t下雨\t20000\nde\t的\t9000000\nqu\t去\t200000\n\
             tian qi\t天气\t60000\nkai hui\t开会\t30000\n",
        )
        .unwrap();
        let lm = BigramLm::from_bigram_tsv_str(
            "今天\t下雨\t300\n今天\t的\t500\n今天\t去\t400\n今天\t腊月初九\t5\n今天\t今天\t50\n\
             下雨\t的\t100\n的\t今天\t50\n的\t去\t4000\n的\t天气\t2000\n的\t开会\t500\n\
             去\t开会\t100\n天气\t的\t10\n",
        )
        .unwrap();
        Engine::new(Arc::new(lexicon), Arc::new(lm))
    }

    fn texts(engine: &Engine, context: &str) -> Vec<String> {
        engine
            .predict_after_at(context, 5, 1000)
            .into_iter()
            .map(|p| p.text)
            .collect()
    }

    /// 像用户刚用拼音上屏了「今天」。
    fn commit_today(engine: &Engine) {
        engine.set_context("");
        engine.record_selection_at("今天", 1000);
    }

    #[test]
    fn predicts_lexicon_words_that_follow_the_last_commit() {
        let engine = engine();
        commit_today(&engine);
        let got = texts(&engine, "今天");
        assert_eq!(got.first().map(String::as_str), Some("下雨"), "{got:?}");
        assert!(got.contains(&"去".to_string()), "{got:?}");
        // 「的」哪儿都常见，在「今天」后面并不比平时多：不出
        assert!(!got.contains(&"的".to_string()), "{got:?}");
        // 词库里没有的词、原词重复不出
        assert!(
            !got.contains(&"腊月初九".to_string()) && !got.contains(&"今天".to_string()),
            "{got:?}"
        );
    }

    #[test]
    fn no_prediction_when_the_text_before_the_caret_moved_on() {
        let engine = engine();
        assert!(texts(&engine, "今天").is_empty(), "还没上屏过词");
        commit_today(&engine);
        assert!(texts(&engine, "今天，").is_empty(), "后面跟了标点");
        assert!(texts(&engine, "").is_empty(), "换了输入框");
        assert!(engine.predict_after_at("今天", 0, 1000).is_empty());
    }

    #[test]
    fn chosen_predictions_are_learned_and_chain() {
        let engine = engine();
        commit_today(&engine);
        // 模型里「今天」后面没有「开会」：一开始不出
        assert!(!texts(&engine, "今天").contains(&"开会".to_string()));
        for _ in 0..2 {
            commit_today(&engine);
            engine.set_context("今天");
            engine.record_prediction_at("开会", 1000);
        }
        assert_eq!(engine.pair_count(), 1);
        // 接着联想：上一个词变成了「开会」
        assert!(
            texts(&engine, "今天开会").is_empty(),
            "模型和习惯里「开会」后面都没有词"
        );
        commit_today(&engine);
        assert_eq!(texts(&engine, "今天")[0], "开会", "用过两次的搭配排最前");
        // 很久以后热度退了，回到模型的排序
        let later = 1000 + 20 * crate::learn::HALF_LIFE_MS as u64;
        let got = engine.predict_after_at("今天", 5, later);
        assert_eq!(got[0].text, "下雨", "{got:?}");
    }

    #[test]
    fn forgetting_a_word_drops_its_predictions() {
        let engine = engine();
        commit_today(&engine);
        engine.set_context("今天");
        engine.record_prediction_at("开会", 1000);
        assert!(engine.forget_word("开会"));
        commit_today(&engine);
        assert!(!texts(&engine, "今天").contains(&"开会".to_string()));
    }
}
