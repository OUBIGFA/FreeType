use super::*;
use crate::lexicon::MemoryLexicon;
use crate::lm::BigramLm;

mod fuzzy;
mod input;
mod learning;
mod ranking;

fn engine(tsv: &str) -> Engine {
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(tsv).unwrap());
    let lm: Arc<dyn LanguageModel> = Arc::new(BigramLm::default());
    Engine::new(lexicon, lm)
}

fn default_engine() -> Engine {
    engine(concat!(
        "xian\t先\t1000\n",
        "xi an\t西安\t500\n",
        "xi\t西\t400\n",
        "an\t安\t300\n",
        "zhong guo\t中国\t800\n",
        "ni hao\t你好\t100000\n",
        "hua r\t花儿\t100\n",
    ))
}

/// yuan 有很多常用单字，「预案」(yu'an) 本来排到好几页后面。
fn split_reading_engine() -> Engine {
    let mut tsv = String::from("yu an\t预案\t110000\nji ang\t激昂\t2000\n");
    for (i, c) in "原愿远元袁员圆缘源怨".chars().enumerate() {
        tsv.push_str(&format!("yuan\t{c}\t{}\n", 1_400_000 - i * 50_000));
    }
    for (i, c) in "将江讲奖降姜浆疆蒋酱".chars().enumerate() {
        tsv.push_str(&format!("jiang\t{c}\t{}\n", 2_600_000 - i * 50_000));
    }
    for (i, c) in "与于语雨鱼余".chars().enumerate() {
        tsv.push_str(&format!("yu\t{c}\t{}\n", 900_000 - i * 50_000));
    }
    tsv.push_str("an\t安\t500000\nji\t及\t900000\nang\t昂\t10000\n");
    let mut e = engine(&tsv);
    e.set_max_candidates(100);
    e
}

/// #93：精确读音本身有词时（zong 有「总」，dan si 有「单丝」），模糊音的词也要列出来、
/// 按频次和精确读音的词排在一起。
fn fuzzy_rank_engine() -> Engine {
    engine(concat!(
        "zong\t总\t3000\n",
        "zong\t宗\t800\n",
        "zhong\t中\t20000\n",
        "si\t四\t3000\n",
        "si\t似\t2000\n",
        "shi\t是\t50000\n",
        "sou\t搜\t1000\n",
        "shou\t手\t8000\n",
        "dan\t单\t3000\n",
        "dan\t但\t5000\n",
        "dan si\t单丝\t5\n",
        "dan shi\t但是\t30000\n",
        "cu\t粗\t1000\n",
        "chu\t出\t8000\n",
        "lai\t来\t9000\n",
        "chu lai\t出来\t20000\n",
        "ji\t机\t5000\n",
        "sou ji\t搜集\t800\n",
        "shou ji\t手机\t20000\n",
    ))
}

fn position(e: &Engine, input: &str, word: &str) -> Option<usize> {
    e.input(input).iter().position(|c| c.text == word)
}

/// 模拟词级 LM 的句首（`<s>`）分：只给「何」一个很大的提升。
struct SentenceStartBoost;

impl LanguageModel for SentenceStartBoost {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        if prev.is_none() && cur == "何" {
            5.0
        } else {
            0.0
        }
    }
}

/// 模拟词级 LM 的分词缺口：「我要」是一个词，(我, 要) 从没计过数（回退分 −2.5），
/// 偶然见过几次的 (我, 腰) 反而有 +1.5。
struct MergedPairLm;

impl LanguageModel for MergedPairLm {
    fn score(&self, prev: Option<&str>, cur: &str) -> f64 {
        match (prev, cur) {
            (Some("我"), "腰") => 1.5,
            (Some("我"), _) => -2.5,
            _ => 0.0,
        }
    }

    fn knows_word(&self, word: &str) -> Option<bool> {
        Some(matches!(word, "我" | "要" | "腰" | "我要" | "今天"))
    }
}
