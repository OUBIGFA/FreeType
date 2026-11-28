// Modified by FreeType; compare with WowType 5656e136 for the original implementation.
//! 输入会话的按键测试：从 fcitx5-addon/test/key_harness.cpp 搬过来的用例，
//! 用纯 Rust 驱动 [`wowtype_core::session::Session`]，macOS 的 CI 也跑。
//!
//! 只搬已经挪进会话的行为：编辑区、已选词栈、退格、Esc，候选列表（引擎候选、计算器、
//! emoji 推荐、数字按键 emoji、日期时间）、编辑区分音节，选词、整句上屏和学习，
//! 空格、回车、数字、分号引号选词、翻页、移动候选和展开面板。
//! 标点（先上屏再出标点）、工具栏、emoji 面板、互译还在 fcitx5 外壳里。
//! 不带引擎的用例没有候选，用 `push_selection` 造已选词。
//! 以后每挪一块，就把 key_harness 里对应的用例搬过来。
//!
//! 前面的用例不带引擎（词库在后台载入好之前就是这样），编辑区是原始输入；
//! 后面的用例用一个小词库建引擎，看候选和分音节。

use std::sync::Arc;

use wowtype_core::datetime::LocalTime;
use wowtype_core::lm::BigramLm;
use wowtype_core::session::{
    keysym, modifier, ArrowKeys, CandidateKind, Config, Frame, Key, Panel, Selection, Session,
};
use wowtype_core::{Converter, Engine, Lexicon, MemoryLexicon};

/// 按一串键，和 key_harness 的 type() 一样：<sp> <bs> <cr> <esc> 等是特殊键，其余按字符。
fn type_keys(session: &mut Session, keys: &str) -> Frame {
    let mut frame = session.frame(false);
    let mut rest = keys;
    while let Some(c) = rest.chars().next() {
        let key = if c == '<' {
            let end = rest.find('>').expect("特殊键没有 >");
            let name = &rest[1..end];
            rest = &rest[end + 1..];
            Key::press(match name {
                "sp" => keysym::SPACE,
                "bs" => keysym::BACKSPACE,
                "cr" => keysym::RETURN,
                "esc" => keysym::ESCAPE,
                "down" => keysym::DOWN,
                "up" => keysym::UP,
                "left" => keysym::LEFT,
                "right" => keysym::RIGHT,
                "pgdn" => keysym::PAGE_DOWN,
                "tab" => keysym::TAB,
                "home" => keysym::HOME,
                "end" => keysym::END,
                "del" => keysym::DELETE,
                other => panic!("不认识的特殊键 <{other}>"),
            })
        } else {
            rest = &rest[c.len_utf8()..];
            Key::char(c)
        };
        frame = session.key(&key);
        // 外壳每个键都送按下和松开，松开不该改变编辑区
        let release = Key {
            release: true,
            ..key
        };
        let after = session.key(&release);
        assert!(!after.handled, "松开不该被会话处理");
        assert_eq!(after.preedit, frame.preedit, "松开改了编辑区");
    }
    frame
}

/// 每个用例前清空（key_harness 的 fresh）。
fn fresh() -> Session<'static> {
    let mut session = Session::new();
    type_keys(&mut session, "<esc>");
    session
}

/// 选句首词（候选还在外壳：这里直接按外壳会传的参数压栈）。
fn select_prefix(session: &mut Session, text: &str, pinyin: &str, bytes: usize) {
    session.push_selection(Selection {
        text: text.into(),
        pinyin: pinyin.into(),
        bytes,
        ..Default::default()
    });
}

// ---- 带引擎：候选和分音节 ----

const LEXICON: &str = "\
ni hao\t你好\t9000
zhong guo\t中国\t9000
heng xiang\t横向\t5000
kai xin\t开心\t9000
kai xin\t凯欣\t300
kai xin\t开新\t200
ba\t吧\t9000
ba\t八\t8000
ba\t把\t7000
ri qi\t日期\t9000
shi jian\t时间\t9000
xing qi\t星期\t9000
shi\t是\t9000
shi\t十\t8000
";

fn engine() -> &'static Engine {
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(LEXICON).unwrap());
    Box::leak(Box::new(Engine::new(
        lexicon,
        Arc::new(BigramLm::default()),
    )))
}

/// 2026-11-11 10:20:30，星期三。
fn fixed_time() -> Option<LocalTime> {
    Some(LocalTime {
        year: 2026,
        month: 11,
        day: 11,
        hour: 10,
        minute: 20,
        second: 30,
        weekday: 3,
    })
}

fn with(engine: &'static Engine) -> Session<'static> {
    let mut session = Session::with_engine(engine);
    session.set_clock(fixed_time);
    session
}

/// 选文字是 text 的那个候选（外壳按数字键、点鼠标都是这样）。
fn choose_text(session: &mut Session, text: &str) -> Frame {
    let index = session
        .candidates()
        .iter()
        .position(|c| c.text == text)
        .unwrap_or_else(|| panic!("没有候选 {text}"));
    session.choose(index)
}

fn texts(session: &mut Session) -> Vec<String> {
    session
        .candidates()
        .iter()
        .map(|c| c.text.clone())
        .collect()
}

// ---- 选词、整句上屏、学习 ----

fn with_engine_ref(engine: &Engine) -> Session<'_> {
    let mut session = Session::with_engine(engine);
    session.set_clock(fixed_time);
    session
}

// ---- 空格、回车、数字、选词键、翻页、展开面板 ----

/// 一个音有很多候选，翻页和展开面板用。
const MANY: &str = "\
shi\t是\t9000
shi\t十\t8900
shi\t事\t8800
shi\t时\t8700
shi\t市\t8600
shi\t使\t8500
shi\t世\t8400
shi\t师\t8300
shi\t式\t8200
shi\t试\t8100
shi\t实\t8000
shi\t室\t7900
shi\t视\t7800
shi\t石\t7700
shi\t湿\t7600
shi\t诗\t7500
shi\t史\t7400
shi\t示\t7300
shi\t士\t7200
shi\t失\t7100
ni hao\t你好\t9000
ni\t你\t8000
hao\t好\t8000
";

fn many_engine() -> &'static Engine {
    let lexicon: Arc<dyn Lexicon> = Arc::new(MemoryLexicon::from_tsv_str(MANY).unwrap());
    let mut engine = Engine::new(lexicon, Arc::new(BigramLm::default()));
    engine.set_max_candidates(300);
    Box::leak(Box::new(engine))
}

/// 一页 5 个、不推荐 emoji（候选顺序固定）的会话，打好 shi。
fn paged(config: Config) -> (Session<'static>, Vec<String>) {
    let mut session = with(many_engine());
    session.configure(Config {
        page_size: 5,
        emoji_suggest: false,
        ..config
    });
    type_keys(&mut session, "shi");
    let all = texts(&mut session);
    assert!(all.len() >= 20, "{all:?}");
    (session, all)
}

fn default_paged() -> (Session<'static>, Vec<String>) {
    paged(Config::default())
}

fn shown_texts(session: &mut Session) -> Vec<String> {
    session.shown().iter().map(|c| c.text.clone()).collect()
}

fn forget_engine() -> &'static Engine {
    let lexicon: Arc<dyn Lexicon> = Arc::new(
        MemoryLexicon::from_tsv_str(concat!(
            "shuang\t双\t9000\n",
            "shuang\t霜\t500\n",
            "pin\t拼\t9000\n",
            "pin\t频\t500\n",
            "ge\t个\t9000\n",
            "ge\t阁\t500\n",
        ))
        .unwrap(),
    );
    Box::leak(Box::new(Engine::new(
        lexicon,
        Arc::new(BigramLm::default()),
    )))
}

fn ctrl(keysym: u32) -> Key {
    Key {
        keysym,
        modifiers: modifier::CTRL,
        ..Key::default()
    }
}

/// 一页 5 个、翻到第 2 页（光标在第 6 个上）以后按一个方向键，看它做了什么。
fn arrow_effect(horizontal: bool, arrow_keys: ArrowKeys, keys: &str) -> &'static str {
    let (mut session, _) = paged(Config {
        horizontal,
        arrow_keys,
        ..Config::default()
    });
    type_keys(&mut session, "<pgdn>");
    let frame = type_keys(&mut session, keys);
    assert!(frame.handled, "{keys} 吃掉");
    if frame.panel == Panel::Expanded {
        return "展开";
    }
    if frame.preedit_cursor != frame.preedit.len() {
        return "移光标";
    }
    match frame.cand_start + frame.cand_cursor.unwrap_or(0) {
        0 => "上一页",
        4 => "上一个",
        5 => "不动",
        6 => "下一个",
        10 => "下一页",
        other => panic!("{keys}：光标到了第 {other} 个"),
    }
}

fn caret_session() -> Session<'static> {
    let mut session = with(engine());
    session.configure(Config {
        arrow_keys: ArrowKeys::Caret,
        ..Config::default()
    });
    session
}

mod candidates;
mod editing;
mod learning;
mod selection;
