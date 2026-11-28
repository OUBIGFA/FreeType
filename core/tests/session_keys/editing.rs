//! 编辑区：退格、Esc、大小写、光标与方向键。
use super::*;

#[test]
fn backspace_undoes_selection_first() {
    // key_harness：「长句先选开头的词」「退格先撤销选词（#49），剩余拼音不动；没有选词了才删字母」
    let mut session = fresh();
    type_keys(&mut session, "nihaozhongguo");
    select_prefix(&mut session, "你好", "ni hao", 5);
    assert_eq!(
        session.frame(true).preedit,
        "你好zhongguo",
        "编辑区 = 已选 + 剩余拼音"
    );
    assert_eq!(session.remaining(), "zhongguo");

    let frame = type_keys(&mut session, "<bs>");
    assert!(frame.handled);
    assert_eq!(frame.preedit, "nihaozhongguo", "退格先撤销选词");
    assert!(session.selections().is_empty());

    let frame = type_keys(&mut session, "<bs>");
    assert_eq!(frame.preedit, "nihaozhonggu", "没有选词了退格删字母");

    // 打回 o，再选一次开头的词：剩下的拼音接着选
    type_keys(&mut session, "o");
    select_prefix(&mut session, "你好", "ni hao", 5);
    assert_eq!(session.selected_text(), "你好");
    assert_eq!(session.remaining(), "zhongguo");
    let frame = session.reset(true);
    assert_eq!(frame.commit.as_deref(), Some("你好zhongguo"));
}

#[test]
fn backspace_pops_selections_in_order() {
    let mut session = fresh();
    type_keys(&mut session, "nihaozhongguoren");
    select_prefix(&mut session, "你好", "ni hao", 5);
    select_prefix(&mut session, "中国", "zhong guo", 8);
    assert_eq!(session.frame(true).preedit, "你好中国ren");
    assert_eq!(type_keys(&mut session, "<bs>").preedit, "你好zhongguoren");
    assert_eq!(type_keys(&mut session, "<bs>").preedit, "nihaozhongguoren");
    assert_eq!(type_keys(&mut session, "<bs>").preedit, "nihaozhongguore");
}

#[test]
fn backspace_to_empty_closes_composition() {
    // key_harness：「退格删光后编辑区关闭」
    let mut session = fresh();
    let frame = type_keys(&mut session, "ni<bs><bs>");
    assert!(frame.handled);
    assert!(frame.preedit.is_empty());
    assert!(!session.composing());
    // 编辑区空了，退格交给应用
    assert!(!type_keys(&mut session, "<bs>").handled);
}

#[test]
fn backspace_after_selecting_everything_but_one_letter() {
    // 选了句首词后把剩下的字母删光：撤销选词，回到原始输入
    let mut session = fresh();
    type_keys(&mut session, "nihaom");
    select_prefix(&mut session, "你好", "ni hao", 5);
    assert_eq!(type_keys(&mut session, "<bs>").preedit, "nihaom");
    assert_eq!(type_keys(&mut session, "<bs>").preedit, "nihao");
}

#[test]
fn escape_clears() {
    let mut session = fresh();
    type_keys(&mut session, "nihaozhongguo");
    select_prefix(&mut session, "你好", "ni hao", 5);
    let frame = type_keys(&mut session, "<esc>");
    assert!(frame.handled);
    assert!(frame.preedit.is_empty());
    assert!(!session.composing());
    assert!(session.selections().is_empty());
    // 空编辑区的 Esc 交给应用
    assert!(!type_keys(&mut session, "<esc>").handled);
}

#[test]
fn uppercase_kept_and_apostrophe() {
    // key_harness：「大写保留 Hello」、隔音符（xi'an）、开头的隔音符没有意义
    let mut session = fresh();
    assert_eq!(type_keys(&mut session, "Hello").preedit, "Hello");
    let mut session = fresh();
    assert_eq!(type_keys(&mut session, "xi'an").preedit, "xi'an");
    let mut session = fresh();
    assert!(!type_keys(&mut session, "'").handled);
    assert!(!session.composing());
}

#[test]
fn keys_without_candidates() {
    // 不带引擎（词库还没载入）：没有候选。空格、回车上屏原始输入，数字吃掉；
    // 标点、翻页键、方向键还交给外壳（外壳先上屏再出标点，或者吃掉）
    let mut session = fresh();
    type_keys(&mut session, "shi");
    for keys in [",", "<down>", "<pgdn>", "<tab>", "-"] {
        let frame = type_keys(&mut session, keys);
        assert!(!frame.handled, "{keys} 没有候选时归外壳");
        assert_eq!(frame.preedit, "shi");
    }
    let frame = type_keys(&mut session, "1");
    assert!(frame.handled, "数字吃掉");
    assert_eq!(frame.preedit, "shi");
    let frame = type_keys(&mut session, "<sp>");
    assert!(frame.handled);
    assert_eq!(frame.commit.as_deref(), Some("shi"));
    assert!(!session.composing());

    type_keys(&mut session, "shi");
    assert_eq!(
        type_keys(&mut session, "<cr>").commit.as_deref(),
        Some("shi")
    );
    // 编辑区空了，空格、回车、数字交给应用
    for keys in ["<sp>", "<cr>", "1"] {
        assert!(!type_keys(&mut session, keys).handled, "{keys}");
    }
}

#[test]
fn switching_input_method_commits_raw() {
    // key_harness：「切换输入法时打了一半的字母上屏」；失去焦点（不上屏）是 reset(false)
    let mut session = fresh();
    type_keys(&mut session, "nihao");
    assert_eq!(session.reset(true).commit.as_deref(), Some("nihao"));
    let mut session = fresh();
    type_keys(&mut session, "nihao");
    let frame = session.reset(false);
    assert_eq!(frame.commit, None);
    assert!(frame.preedit.is_empty());
}

#[test]
fn arrow_keys_in_each_layout() {
    // #129：「方向键」两种用法 × 横排竖排，四种组合下各个方向键做什么
    use ArrowKeys::{Candidates, Caret};
    let cases = [
        (true, Candidates, ["上一个", "下一个", "上一页", "展开"]),
        (false, Candidates, ["上一页", "下一页", "上一个", "下一个"]),
        // 移光标：上下键选词（跨页），左右键只管拼音里的光标，右键在末尾不动、不去挑候选
        (true, Caret, ["移光标", "不动", "上一个", "下一个"]),
        (false, Caret, ["移光标", "不动", "上一个", "下一个"]),
    ];
    for (horizontal, arrow_keys, want) in cases {
        let got = ["<left>", "<right>", "<up>", "<down>"]
            .map(|k| arrow_effect(horizontal, arrow_keys, k));
        assert_eq!(got, want, "横排 {horizontal}、{arrow_keys:?}");
    }
}

#[test]
fn caret_edits_pinyin() {
    // #129：左右键在拼音里移动光标，字母插在光标处，退格删光标前一个；到了两头吃掉，不去挑候选
    let mut session = caret_session();
    let frame = type_keys(&mut session, "nihao");
    assert_eq!(
        frame.preedit_cursor,
        frame.preedit.len(),
        "一开始光标在末尾"
    );
    let first = frame.cand_cursor;
    let frame = type_keys(&mut session, "<right>");
    assert!(
        frame.handled && frame.cand_cursor == first,
        "末尾的右键吃掉，候选不动"
    );

    let frame = type_keys(&mut session, "<left><left><left>");
    assert_eq!(
        &frame.preedit[..frame.preedit_cursor],
        "ni",
        "光标在 ni 后面：{}",
        frame.preedit
    );
    assert_eq!(frame.cand_cursor, first, "左键不动候选");
    type_keys(&mut session, "x");
    assert_eq!(session.input(), "nixhao", "字母插在光标处");
    let frame = type_keys(&mut session, "<bs>");
    assert_eq!(session.input(), "nihao", "退格删光标前一个");
    assert_eq!(&frame.preedit[..frame.preedit_cursor], "ni");

    // Home 到最前：退格没东西删，左键吃掉
    type_keys(&mut session, "<home>");
    assert!(session.key(&Key::press(keysym::BACKSPACE)).handled);
    assert_eq!(session.input(), "nihao", "光标在最前面时退格不删");
    let frame = type_keys(&mut session, "<left>");
    assert!(
        frame.handled && frame.preedit_cursor == 0 && frame.cand_cursor == first,
        "最前面的左键吃掉"
    );
    let frame = type_keys(&mut session, "<right>");
    assert_eq!(&frame.preedit[..frame.preedit_cursor], "n");
    // Delete 删光标后一个；末尾的 Delete 吃掉
    type_keys(&mut session, "<del>");
    assert_eq!(session.input(), "nhao");
    type_keys(&mut session, "i<end>");
    assert_eq!(session.input(), "nihao");
    assert!(session.key(&Key::press(keysym::DELETE)).handled);
    assert_eq!(session.input(), "nihao");

    // 下键选下一个，空格上屏它
    let second = texts(&mut session)[1].clone();
    let frame = type_keys(&mut session, "<down><sp>");
    assert_eq!(frame.commit, Some(second));
}

#[test]
fn caret_resets_after_choosing_a_word() {
    // 选了句首词：光标回到剩下的拼音末尾
    let mut session = caret_session();
    type_keys(&mut session, "nihaoshijie<left><left>");
    choose_text(&mut session, "你好");
    let frame = session.frame(true);
    assert_eq!(
        frame.preedit_cursor,
        frame.preedit.len(),
        "{}",
        frame.preedit
    );
    type_keys(&mut session, "a");
    assert_eq!(session.remaining(), "shijiea");
}

#[test]
fn caret_resets_when_switching_back_to_candidates() {
    let mut session = caret_session();
    type_keys(&mut session, "nihao<left><left>");
    session.configure(Config::default());
    let frame = session.frame(true);
    assert_eq!(
        frame.preedit_cursor,
        frame.preedit.len(),
        "换回挑候选：光标回到末尾"
    );
    type_keys(&mut session, "x");
    assert_eq!(session.input(), "nihaox");
}

#[test]
fn caret_shuangpin_shows_keys() {
    // 双拼显示成全拼（zhong'guo）时换不回光标位置：光标在中间时显示打的键
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(LEXICON).unwrap());
    let mut engine = Engine::new(lexicon, Arc::new(BigramLm::default()));
    engine.set_shuangpin(true);
    let engine: &'static Engine = Box::leak(Box::new(engine));
    let mut session = with(engine);
    session.configure(Config {
        arrow_keys: ArrowKeys::Caret,
        ..Config::default()
    });
    let frame = type_keys(&mut session, "vsgo");
    assert!(
        frame.preedit.contains("zhong"),
        "光标在末尾时照常显示全拼：{}",
        frame.preedit
    );
    let frame = type_keys(&mut session, "<left>");
    assert_eq!(frame.preedit, "vsgo");
    assert_eq!(frame.preedit_cursor, 3);
    assert_eq!(texts(&mut session)[0], "中国", "候选照旧按整个输入算");
}
