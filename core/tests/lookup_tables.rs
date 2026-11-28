use wowtype_core::lookup_table::LookupTable;

fn fixture() -> Vec<u8> {
    let rows = [
        "a\t5\ta\teɪ\t一个\n",
        "alot\t90\ta lot\t\t许多\n",
        "apple\t100\tapple\tæpl\t苹果\n",
        "nasa\t80\tNASA\tnæsə\t美国航空航天局\n",
    ];
    let mut body = Vec::new();
    let mut offset = 16 + (rows.len() + 1) * 8;
    for row in rows {
        body.extend_from_slice(&(offset as u64).to_le_bytes());
        offset += row.len();
    }
    body.extend_from_slice(&(offset as u64).to_le_bytes());
    for row in rows {
        body.extend_from_slice(row.as_bytes());
    }
    let mut bytes = b"FTLOOK01".to_vec();
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
    bytes.extend(body);
    bytes
}

#[test]
fn mapped_lookup_keeps_exact_word_first_and_preserves_phrase_metadata() {
    let table = LookupTable::from_bytes(fixture()).unwrap();
    let words = table.search("a", 10);
    assert_eq!(
        words.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
        ["a", "apple", "a lot"]
    );
    assert_eq!(words[1].reading, "æpl");
    assert_eq!(words[1].gloss, "苹果");
    assert_eq!(table.search("alot", 10)[0].text, "a lot");
    assert_eq!(table.search("nasa", 10)[0].text, "NASA");
    assert!(table.search("unknown", 10).is_empty());
}

#[test]
fn damaged_tables_are_rejected_before_queries() {
    let mut bytes = fixture();
    bytes[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    let crc = crc32fast::hash(&bytes[16..]);
    bytes[12..16].copy_from_slice(&crc.to_le_bytes());
    assert!(
        LookupTable::from_bytes(bytes).is_err(),
        "校验值正确也必须拒绝越界索引"
    );
    let mut bytes = fixture();
    *bytes.last_mut().unwrap() ^= 1;
    assert!(
        LookupTable::from_bytes(bytes).is_err(),
        "内容损坏不能静默加载"
    );
}

#[test]
fn packaged_english_and_radical_tables_cover_real_queries() {
    let Some(root) = std::env::var_os("FREETYPE_TEST_INPUT_TOOLS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let english = LookupTable::open(root.join("english-lookup.ftd")).unwrap();
    assert!(english.len() > 500_000);
    assert!(english
        .search("inspiteof", 10)
        .iter()
        .any(|e| e.text.eq_ignore_ascii_case("in spite of") && !e.gloss.is_empty()));
    let radical = LookupTable::open(root.join("radical-lookup.ftd")).unwrap();
    assert!(radical
        .search("ririri", 10)
        .iter()
        .any(|e| e.text == "晶" && e.reading == "jīng"));
}
