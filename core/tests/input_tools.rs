use wowtype_core::{input_tools::InputTools, shuangpin::Scheme};

#[test]
fn equally_ranked_spelling_suggestions_keep_a_stable_order_and_membership() {
    let Some(root) = std::env::var_os("FREETYPE_TEST_INPUT_TOOLS") else { return; };
    let tools = InputTools::open(root).unwrap();
    let first = tools.english("abnd", 20, true);
    assert!(first.iter().any(|candidate| candidate.corrected));
    for _ in 0..20 {
        assert_eq!(tools.english("abnd", 20, true), first, "同一输入的同分候选不能换序或换词");
    }
}

#[test]
fn english_mode_completes_phrases_corrects_spelling_and_follows_capitalization() {
    let Some(root) = std::env::var_os("FREETYPE_TEST_INPUT_TOOLS") else { return; };
    let tools = InputTools::open(root).unwrap();
    assert!(tools.english("Tes", 20, true).iter().any(|e| e.text == "Test"));
    assert!(tools.english("NASA", 20, true).iter().any(|e| e.text == "NASA"));
    assert!(tools.english("inspiteof", 20, true).iter().any(|e| e.text == "in spite of" && !e.gloss.is_empty()));
    let corrected = tools.english("conectivity", 20, true);
    assert!(corrected.iter().any(|e| e.text.eq_ignore_ascii_case("connectivity") && e.corrected));
    assert!(tools.english("conectivity", 20, false).is_empty());
    assert!(tools.english("kerrage", 20, true).iter().any(|e| e.text.eq_ignore_ascii_case("courage") && e.corrected));
}

#[test]
fn personal_english_names_return_with_their_learned_case() {
    use std::sync::Arc;
    let Some(root) = std::env::var_os("FREETYPE_TEST_INPUT_TOOLS") else { return; };
    let engine = wowtype_core::Engine::new(Arc::new(wowtype_core::MemoryLexicon::new()), Arc::new(wowtype_core::BigramLm::default()));
    engine.load_input_tools(root).unwrap();
    for _ in 0..3 { engine.record_selection_with_pinyin("FreeTypeTester", Some("freetypetester")); }
    assert_eq!(engine.english_tool_candidates("freetypetester", 20, true)[0].text, "FreeTypeTester");
}

#[test]
fn radical_mode_searches_complete_components_and_shuangpin_with_readings() {
    let Some(root) = std::env::var_os("FREETYPE_TEST_INPUT_TOOLS") else { return; };
    let tools = InputTools::open(root).unwrap();
    assert!(tools.radical("ririri", None, 50).iter().any(|e| e.text == "晶" && e.reading == "jīng"));
    assert!(tools.radical("ririri", None, 50).iter().filter(|e| e.text == "晶").count() == 1);
    let scheme = Scheme::from_name("xiaohe").unwrap();
    assert!(tools.radical("kou", Some(scheme), 100).iter().any(|e| e.text == "口"));
    assert!(tools.radical("kz", Some(scheme), 100).iter().any(|e| e.text == "口"));
}
