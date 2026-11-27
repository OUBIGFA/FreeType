//! Regression inputs from upstream reports, exercised through the real query and learning paths.
use std::sync::Arc;
use wowtype_core::shuangpin::Scheme;
use wowtype_core::syllable::fuzzy;
use wowtype_core::{BigramLm, Engine, MemoryLexicon};

fn engine(words: &str) -> Engine {
    Engine::new(
        Arc::new(MemoryLexicon::from_tsv_str(words).unwrap()),
        Arc::new(BigramLm::default()),
    )
}

#[test]
fn invalid_shuangpin_syllables_use_only_enabled_fuzzy_groups() {
    let mut e = engine("ding\t定\t50000\nting\t听\t50000\nzhong\t中\t40000\n");
    e.set_shuangpin(true);
    e.set_shuangpin_scheme(Scheme::Xiaohe);
    e.set_fuzzy(true);
    e.set_fuzzy_groups(fuzzy::IN);
    for (keys, word) in [
        ("db", "定"),
        ("tb", "听"),
        ("dk", "定"),
        ("tk", "听"),
        ("vs", "中"),
    ] {
        assert_eq!(e.input(keys)[0].text, word, "{keys}");
    }
    e.set_fuzzy_groups(fuzzy::AN);
    assert!(!e.input("db").iter().any(|c| c.text == "定"));
    e.set_fuzzy(false);
    assert!(!e.input("tb").iter().any(|c| c.text == "听"));
}

#[test]
fn all_shuangpin_layouts_decode_missing_in_syllables_without_changing_valid_pairs() {
    for (scheme, key) in [
        (Scheme::Ziranma, 'n'),
        (Scheme::Xiaohe, 'b'),
        (Scheme::Microsoft, 'n'),
        (Scheme::Sogou, 'n'),
        (Scheme::Abc, 'c'),
        (Scheme::Ziguang, 'y'),
        (Scheme::PinyinJiajia, 'l'),
    ] {
        let mut e = engine("ding\t定\t10000\nting\t听\t10000\n");
        e.set_shuangpin(true);
        e.set_shuangpin_scheme(scheme);
        e.set_fuzzy(true);
        e.set_fuzzy_groups(fuzzy::IN);
        for (initial, word) in [('d', "定"), ('t', "听")] {
            let input = format!("{initial}{key}");
            assert_eq!(e.input(&input)[0].text, word, "{scheme:?} {input}");
        }
    }
}

#[test]
fn an_unknown_erhua_completion_does_not_hide_a_known_mixed_word() {
    let words = "shu ru fa\t输入法\t50000\nshu\t书\t20000\nfa\t发\t20000\nshu e\t数额\t10000\nhua r\t花儿\t10000\nhao\t好\t20000\nhua rong fu\t华荣府\t10000\n";
    for typo in [false, true] {
        let mut e = engine(words);
        e.set_typo_correction(typo);
        assert_eq!(e.input("shurf")[0].text, "输入法", "typo={typo}");
        assert!(
            e.input("huarhao").iter().any(|c| c.text == "花儿好"),
            "valid medial erhua must remain"
        );
        assert!(e.input("huar").iter().any(|c| c.text == "花儿"));
        assert_eq!(e.input("shurufa")[0].text, "输入法");
    }
}

#[test]
fn known_erhua_completion_preserves_whole_mixed_words_and_legitimate_erhua() {
    let mut e = engine("shu ru fa\t输入法\t50000\nshu r\t数儿\t10000\nfa\t发\t20000\nhua r\t花儿\t10000\nhua rong fu\t华荣府\t50000\nhao\t好\t20000\n");
    for typo in [false, true] {
        e.set_typo_correction(typo);
        for (input, whole, erhua) in [("shurf","输入法","数儿发"),("huarf","华荣府","花儿发")] {
            let found=e.input(input);
            assert_eq!(found[0].text,whole,"{input}, typo={typo}: {found:?}");
            assert!(found.iter().any(|c|c.text==erhua),"legitimate erhua + abbreviation remains reachable");
        }
        assert_eq!(e.input("huarhao")[0].text,"花儿好");
        assert_eq!(e.input("huar")[0].text,"花儿");
    }
}

#[test]
fn invalid_full_fuzzy_spellings_compete_without_typo_correction() {
    let mut e = engine("shuang\t双\t100000\nzhuang\t装\t100000\nchuang\t床\t100000\nsuan ge\t算个\t10000\nzuan ge\t赚个\t10000\ncuan ge\t窜个\t10000\nshuang zi\t双子\t200000\n");
    e.set_fuzzy(true);
    e.set_fuzzy_groups(fuzzy::ZH | fuzzy::CH | fuzzy::SH);
    for typo in [false, true] {
        e.set_typo_correction(typo);
        for (input, word) in [
            ("suang", "双"),
            ("zuang", "装"),
            ("cuang", "床"),
            ("suangzi", "双子"),
        ] {
            let found = e.input(input);
            assert_eq!(found[0].text, word, "{input}, typo={typo}: {found:?}");
            assert_eq!(
                found[0].features.typo_count, 0.0,
                "enabled fuzzy reading is not a typo"
            );
        }
    }
    e.set_typo_correction(false);
    e.set_fuzzy_groups(fuzzy::NL);
    assert!(!e.input("suang").iter().any(|c| c.text == "双"));
}

#[test]
fn fuzzy_alias_does_not_chain_two_initial_or_final_changes() {
    let mut e = engine("lin\t林\t1000\nning\t宁\t100000\nleng\t冷\t1000\nlong\t龙\t100000\n");
    e.set_fuzzy(true);
    e.set_typo_correction(false);
    e.set_fuzzy_groups(fuzzy::RL | fuzzy::NL | fuzzy::IN | fuzzy::EN | fuzzy::ENG_ONG);
    let rin = e.input("rin");
    assert!(rin.iter().any(|c| c.text == "林"));
    assert!(
        !rin.iter().any(|c| c.text == "宁"),
        "r/l plus n/l must not become r/n"
    );
    let len = e.input("len");
    assert!(len.iter().any(|c| c.text == "冷"));
    assert!(
        !len.iter().any(|c| c.text == "龙" && c.consumed == 3),
        "en/eng plus eng/ong must not become en/ong: {len:?}"
    );
}
