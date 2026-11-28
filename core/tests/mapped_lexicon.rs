//! Regression coverage for the compiled dictionary through the real Lexicon interface.
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use wowtype_core::lexicon::MappedLexicon;
use wowtype_core::{BigramLm, Engine, Lexicon, MemoryLexicon, Syllable, SyllableQuery};

const WORDS: &str = "ni\t拟\t0.5\nni\t你\t500\nni\t尼\t100\nhao\t好\t400\n\
    ni hao\t你好\t300\nzhong guo\t中国\t200\nzhong guo ren\t中国人\t150\n\
    zong guo\t总过\t100\nzong guo\t中国\t50\nzheng guo\t整过\t80\n\
    shi chi\t诗痴\t70\nsi ci\t私词\t60\nhua r\t花儿\t40\nhua\t花\t30\n";

fn fixture() -> (PathBuf, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::var_os("FREETYPE_TEST_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../_temp/mapped-lexicon")
        });
    let root = root.join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("lexicon.tsv");
    std::fs::write(&source, WORDS).unwrap();
    (source, root.join("lexicon.ftd"))
}

fn full(text: &str) -> Vec<SyllableQuery> {
    text.split_whitespace()
        .map(|part| Syllable::from_text(part).unwrap().query(false))
        .collect()
}

#[test]
fn mapped_queries_and_candidates_match_the_tsv_engine() {
    let (source, index) = fixture();
    MappedLexicon::compile(&source, &index).unwrap();
    let mapped = MappedLexicon::open(&source, &index).unwrap();
    let memory = MemoryLexicon::from_tsv_path(&source).unwrap();
    assert_eq!(mapped.len(), 14);
    assert_eq!(mapped.total_freq(), memory.total_freq());
    let mut queries = vec![
        vec![],
        full("ni"),
        full("ni hao"),
        full("zhong guo"),
        full("zhong guo ren"),
        full("xi an"),
        full("hua"),
        full("si ci"),
    ];
    queries.push(vec![Syllable::from_text("hua").unwrap().query(true)]);
    for letters in ["ng", "ngr", "Zg", "sc", "SC", "q"] {
        queries.push(
            letters
                .bytes()
                .map(|letter| SyllableQuery::Jianpin { letter })
                .collect(),
        );
    }
    queries.push(vec![
        SyllableQuery::Jianpin { letter: b'z' },
        full("guo")[0],
    ]);
    for query in queries {
        assert_eq!(
            mapped.lookup(&query),
            memory.lookup(&query),
            "query {query:?}"
        );
        assert_eq!(
            mapped.lookup_common(&query),
            memory.lookup_common(&query),
            "common {query:?}"
        );
        assert_eq!(
            mapped.has_longer(&query),
            memory.has_longer(&query),
            "prefix {query:?}"
        );
        assert_eq!(
            mapped.has_longer_fuzzy(&query),
            memory.has_longer_fuzzy(&query),
            "fuzzy prefix {query:?}"
        );
        for groups in [
            0,
            wowtype_core::syllable::fuzzy::ZH,
            wowtype_core::syllable::fuzzy::ALL,
        ] {
            assert_eq!(
                mapped.lookup_fuzzy(&query, groups),
                memory.lookup_fuzzy(&query, groups),
                "fuzzy {query:?}"
            );
        }
    }
    assert_eq!(
        mapped
            .lookup_common(&full("ni"))
            .iter()
            .map(|entry| entry.word.as_str())
            .collect::<Vec<_>>(),
        vec!["你", "尼"]
    );
    let prior = mapped.prediction_index().unwrap();
    assert_eq!(prior.log_frequency("中国"), Some(200.0_f64.ln() as f32));
    assert_eq!(prior.log_frequency("拟"), None);
    assert_eq!(prior.log_frequency("missing"), None);
    let mut words = vec![];
    mapped.for_each_word(&mut |word, frequency| words.push((word.to_owned(), frequency)));
    assert_eq!(words.len(), mapped.len());
    assert_eq!(words[0], ("拟".into(), 0.5));
    let mapped = Engine::new(Arc::new(mapped), Arc::new(BigramLm::default()));
    let memory = Engine::new(Arc::new(memory), Arc::new(BigramLm::default()));
    for code in ["nihao", "zg", "zhongguoren", "SC", "huar"] {
        let candidates = |engine: &Engine| {
            engine
                .input(code)
                .into_iter()
                .map(|candidate| candidate.text)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            candidates(&mapped),
            candidates(&memory),
            "candidates {code}"
        );
    }
}

#[test]
fn compiled_index_is_reproducible_and_survives_atomic_replacement() {
    let (source, index) = fixture();
    MappedLexicon::compile(&source, &index).unwrap();
    let original = std::fs::read(&index).unwrap();
    let current = MappedLexicon::open(&source, &index).unwrap();
    MappedLexicon::compile(&source, &index).unwrap();
    assert_eq!(std::fs::read(&index).unwrap(), original);
    std::fs::write(&source, "ni\t妮\t10\n").unwrap();
    MappedLexicon::compile(&source, &index).unwrap();
    let next = MappedLexicon::open(&source, &index).unwrap();
    assert_eq!(current.lookup_common(&full("ni"))[0].word, "你");
    assert_eq!(next.lookup(&full("ni"))[0].word, "妮");
}

#[test]
fn mapped_prediction_matches_memory_for_multiple_readings_and_learned_pairs() {
    let (source, index) = fixture();
    MappedLexicon::compile(&source, &index).unwrap();
    let lm = Arc::new(
        BigramLm::from_bigram_tsv_str("你\t好\t100\n你\t中国\t80\n好\t你好\t90\n").unwrap(),
    );
    let memory = Engine::new(
        Arc::new(MemoryLexicon::from_tsv_path(&source).unwrap()),
        lm.clone(),
    );
    let mapped = Engine::new(Arc::new(MappedLexicon::open(&source, &index).unwrap()), lm);
    for engine in [&memory, &mapped] {
        engine.prepare_prediction();
        engine.set_context("");
        engine.record_selection_at("你", 1000);
    }
    let expected = memory.predict_after_at("你", 10, 1000);
    assert!(!expected.is_empty());
    assert_eq!(mapped.predict_after_at("你", 10, 1000), expected);
    for engine in [&memory, &mapped] {
        engine.set_context("你");
        engine.record_selection_at("中国", 1100);
        engine.set_context("");
        engine.record_selection_at("你", 1200);
    }
    let expected = memory.predict_after_at("你", 10, 1200);
    assert!(expected.iter().any(|prediction| prediction.word == "中国"));
    assert_eq!(mapped.predict_after_at("你", 10, 1200), expected);
}

#[test]
fn damaged_or_stale_indexes_are_rejected() {
    let (source, index) = fixture();
    MappedLexicon::compile(&source, &index).unwrap();
    let original = std::fs::read(&index).unwrap();
    for length in [0, 31, original.len() - 1] {
        std::fs::write(&index, &original[..length]).unwrap();
        assert!(MappedLexicon::open(&source, &index).is_err());
    }
    let mut damaged = original.clone();
    damaged[40] ^= 1;
    std::fs::write(&index, damaged).unwrap();
    assert!(MappedLexicon::open(&source, &index).is_err());
    std::fs::write(&index, original).unwrap();
    std::fs::write(&source, WORDS.replace("你好", "拟好")).unwrap();
    assert!(MappedLexicon::open(&source, &index).is_err());
}

#[test]
fn compiler_preserves_source_if_target_is_the_same_file() {
    let (source, _) = fixture();
    assert!(MappedLexicon::compile(&source, &source).is_err());
    assert_eq!(std::fs::read_to_string(&source).unwrap(), WORDS);
}

#[test]
fn deployed_index_works_without_source_and_detects_replacement_damage() {
    let (source, index) = fixture();
    let record = index.with_file_name("standalone.verified");
    MappedLexicon::compile(&source, &index).unwrap();
    // Shipping only the index must preserve candidates and predictions, not bypass integrity checks.
    std::fs::rename(&source, source.with_extension("source-only")).unwrap();
    let loaded = MappedLexicon::open_verified(&source, &index, &record);
    assert!(
        loaded.is_ok(),
        "a deployed index must not require the build-time TSV: {:?}",
        loaded.as_ref().err()
    );
    let loaded = loaded.unwrap();
    assert_eq!(loaded.lookup(&full("ni hao"))[0].word, "你好");
    assert_eq!(
        loaded.prediction_index().unwrap().log_frequency("中国"),
        Some(200_f64.ln() as f32)
    );
    drop(loaded);
    std::fs::write(&source, "old installation TSV").unwrap();
    assert_eq!(
        MappedLexicon::open_deployed(&index, &record)
            .unwrap()
            .lookup(&full("ni hao"))[0]
            .word,
        "你好"
    );
    assert!(
        MappedLexicon::open_verified(&source, &index, &record).is_err(),
        "explicit source checks stay strict"
    );
    let mut bytes = std::fs::read(&index).unwrap();
    bytes[40] ^= 1;
    std::fs::write(&index, bytes).unwrap();
    assert!(MappedLexicon::open_deployed(&index, &record).is_err());
}

#[test]
fn legacy_deployed_index_keeps_candidates_and_float_priors() {
    let (source, index) = fixture();
    // Captured with the pre-optimization compiler (01927f8); the hex fixture is a fixed old-format artifact.
    let bytes: Vec<u8> = include_str!("fixtures/lexicon-ftd1.hex")
        .split_whitespace()
        .flat_map(|line| {
            line.as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        })
        .collect();
    std::fs::write(
        &source,
        "ni\t你\t500\nni\t拟\t0.5\nni hao\t你好\t300\nzhong guo\t中国\t200\n",
    )
    .unwrap();
    std::fs::write(&index, bytes).unwrap();
    let mapped = MappedLexicon::open(&source, &index).unwrap();
    assert_eq!(mapped.lookup(&full("ni hao"))[0].word, "你好");
    assert_eq!(mapped.lookup_common(&full("ni")).len(), 1);
    assert_eq!(
        mapped.prediction_index().unwrap().log_frequency("中国"),
        Some(200_f64.ln() as f32)
    );
    assert_eq!(mapped.prediction_index().unwrap().log_frequency("拟"), None);
}

#[test]
fn compact_records_keep_multibyte_lengths_and_fractional_frequencies() {
    let (source, index) = fixture();
    // Both UTF-8 length and syllable count cross a one-byte integer boundary.
    let reading = vec!["ni"; 130].join(" ");
    let word = "你".repeat(130);
    std::fs::write(&source, format!("{WORDS}{reading}\t{word}\t3.25\n")).unwrap();
    MappedLexicon::compile(&source, &index).unwrap();
    let mapped = MappedLexicon::open(&source, &index).unwrap();
    let expected = MemoryLexicon::from_tsv_path(&source).unwrap();
    assert_eq!(
        mapped.lookup(&full(&reading)),
        expected.lookup(&full(&reading))
    );
    let mut visited = Vec::new();
    mapped.for_each_word(&mut |word, freq| visited.push((word.to_owned(), freq)));
    assert_eq!(visited.last(), Some(&(word, 3.25)));
    assert_eq!(visited.len(), 15);
}

#[cfg(windows)]
#[test]
fn open_mapping_denies_in_place_writers() {
    let (source, index) = fixture();
    MappedLexicon::compile(&source, &index).unwrap();
    let mapping = MappedLexicon::open(&source, &index).unwrap();
    assert!(std::fs::OpenOptions::new()
        .write(true)
        .open(&index)
        .is_err());
    assert_eq!(mapping.lookup(&full("ni hao"))[0].word, "你好");
}

#[test]
fn verified_record_skips_full_checksums_only_for_unchanged_files() {
    let (source, index) = fixture();
    let record = index.with_file_name("lexicon.verified");
    MappedLexicon::compile(&source, &index).unwrap();
    // The first open performs the full check and remembers both file identities.
    MappedLexicon::open_verified(&source, &index, &record).unwrap();
    assert!(record.is_file());
    // Same size and modification time: startup trusts the record and skips the
    // full-file CRC. Damage the trailing checksum but keep the identity unchanged.
    let modified = std::fs::metadata(&index).unwrap().modified().unwrap();
    let mut bytes = std::fs::read(&index).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&index, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&index)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let mapped = MappedLexicon::open_verified(&source, &index, &record).unwrap();
    assert_eq!(mapped.lookup(&full("ni hao"))[0].word, "你好");
    drop(mapped);
    assert!(
        MappedLexicon::open(&source, &index).is_err(),
        "plain open still verifies everything"
    );
    // Any replacement changes the identity, so the full check runs again.
    std::fs::write(&index, &bytes).unwrap();
    assert!(MappedLexicon::open_verified(&source, &index, &record).is_err());
    MappedLexicon::compile(&source, &index).unwrap();
    MappedLexicon::open_verified(&source, &index, &record).unwrap();
    std::fs::write(&source, WORDS.replace("你好", "拟好")).unwrap();
    assert!(MappedLexicon::open_verified(&source, &index, &record).is_err());
}

#[test]
fn fuzzy_pruning_keeps_every_longer_fuzzy_word() {
    use wowtype_core::lexicon::FuzzyLexicon;
    use wowtype_core::syllable::fuzzy::ALL;
    let (source, index) = fixture();
    MappedLexicon::compile(&source, &index).unwrap();
    let mapped = MappedLexicon::open(&source, &index).unwrap();
    let memory = MemoryLexicon::from_tsv_path(&source).unwrap();
    for inner in [&mapped as &dyn Lexicon, &memory] {
        let view = FuzzyLexicon::new(inner, ALL, 2.5);
        // 只有 zhong guo / zong guo 两种读音：zeng 经 eng/ong 不相关，zhong 精确就有更长的词。
        assert!(view.has_longer(&full("zhong")));
        // 精确的 zong 后面也有词；cong 只能靠 c/ch、ong 模糊到 zhong 以外的读音，不剪错就行。
        assert!(view.has_longer(&full("zong")));
        // zheng guo 按 eng/ong 模糊到 zhong guo ren，后面还有词；ni hao 精确、模糊都没有更长的词。
        assert!(view.has_longer(&full("zheng guo")));
        assert!(!view.has_longer(&full("ni hao")));
        // 不剪错：凡是模糊查询能查到的两音节词，它的第一个音节都必须判为「有更长的词」。
        for first in [
            "zhong", "zong", "zheng", "zeng", "si", "shi", "ci", "chi", "ni", "li", "hua",
        ] {
            for second in ["guo", "ci", "chi", "hao", "r"] {
                let Some(second) = Syllable::from_text(second) else {
                    continue;
                };
                let query = vec![full(first)[0], second.query(false)];
                if !view.lookup_common(&query).is_empty() {
                    assert!(
                        view.has_longer(&query[..1]),
                        "pruned {first} before {query:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn lookup_memo_follows_learning_and_fuzzy_settings() {
    let (source, index) = fixture();
    MappedLexicon::compile(&source, &index).unwrap();
    let lm = Arc::new(BigramLm::default());
    let mut engine = Engine::new(Arc::new(MappedLexicon::open(&source, &index).unwrap()), lm);
    let first = |engine: &Engine, code: &str| {
        engine
            .input(code)
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
    };
    assert!(!first(&engine, "nihao").contains(&"泥号".to_string()));
    // 缓存里已有 ni hao 的查表结果；学到的新词必须马上出现。
    engine.add_user_word("ni hao", "泥号", 1e9).unwrap();
    assert_eq!(first(&engine, "nihao")[0], "泥号");
    assert!(engine.lexicon().remove("泥号"));
    assert!(!first(&engine, "nihao").contains(&"泥号".to_string()));
    // 模糊音开关和分组变化也不能沿用旧结果。
    assert!(!first(&engine, "zongguo").contains(&"整过".to_string()));
    engine.set_fuzzy(true);
    engine.set_fuzzy_groups(wowtype_core::syllable::fuzzy::ALL);
    let fuzzy = first(&engine, "zongguo");
    assert!(fuzzy.contains(&"中国".to_string()), "{fuzzy:?}");
    engine.set_fuzzy_groups(0);
    assert_eq!(first(&engine, "zongguo"), {
        let mut plain = Engine::new(
            Arc::new(MappedLexicon::open(&source, &index).unwrap()),
            Arc::new(BigramLm::default()),
        );
        plain.set_fuzzy(true);
        plain.set_fuzzy_groups(0);
        first(&plain, "zongguo")
    });
}
