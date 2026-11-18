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
