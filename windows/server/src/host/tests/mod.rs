//! 服务端会话测试。共用的构造与按键辅助函数在本文件，用例按主题放在子模块，
//! 便于并行修改不同功能时互不冲突。
use super::*;
use freetype_protocol::{digit_punct, keysym, Panel};

mod composing;
mod data;
mod modes;
mod prediction;
mod sentence;

fn host(name: &str) -> Host {
    let dir = std::env::temp_dir().join(format!("freetype-host-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut host = Host::new(Store::new(dir));
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
        "ni hao\t你好\t1000\nni\t你\t900\nhao\t好\t800\nshi jie\t世界\t700\nyun\t韵\t10\npin\t拼\t900\npin\t频\t500\n",
    )
    .unwrap();
    let engine = Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(wowtype_core::BigramLm::default()),
    );
    let _ = engine.load_english_str("GitHub\nhello\n");
    host.install_engine(Ok(engine));
    host.handle(
        1,
        Request::Hello {
            session: true,
            app: "notepad.exe".into(),
        },
    );
    host
}

fn configure(host: &mut Host, change: impl FnOnce(&mut Config)) {
    let mut config = host.handle(1, Request::GetConfig).config.unwrap();
    change(&mut config);
    assert!(host.handle(1, Request::SetConfig { config }).ok);
}

fn key_with(host: &mut Host, id: ClientId, sym: u32, modifiers: u32) -> Frame {
    let key = Key {
        keysym: sym,
        modifiers,
        test: false,
    };
    host.handle(id, Request::Key { key, before: None })
        .frame
        .unwrap()
}

fn press(host: &mut Host, sym: u32) -> Frame {
    key_with(host, 1, sym, 0)
}

fn typing(host: &mut Host, text: &str) -> Frame {
    let mut frame = Frame::default();
    for c in text.chars() {
        frame = press(host, c as u32);
    }
    frame
}

fn cleanup(host: &Host) {
    let _ = std::fs::remove_dir_all(host.data_dir());
}

fn learning_request(operation: &str, path: &Path) -> Request {
    serde_json::from_value(serde_json::json!({
        "op": operation,
        "path": path.to_string_lossy(),
    }))
    .expect("学习备份请求应被协议支持")
}

fn learning_host(name: &str) -> Host {
    let host = host(name);
    std::fs::create_dir_all(host.data_dir()).unwrap();
    host
}

/// 桌面：记状态栏，前台程序和开着的窗口由测试指定。
struct Foreground(std::rc::Rc<std::cell::RefCell<Option<String>>>);

impl Desktop for Foreground {
    fn update(&self, _state: &BarState) {}
    fn open_settings(&self) {}
    fn foreground_app(&self) -> Option<String> {
        self.0.borrow().clone()
    }
    fn open_apps(&self) -> Vec<freetype_protocol::AppWindow> {
        vec![freetype_protocol::AppWindow {
            app: "code.exe".into(),
            title: "main.rs".into(),
        }]
    }
}

fn preset(app: &str, mode: Mode) -> freetype_protocol::AppPreset {
    freetype_protocol::AppPreset {
        app: app.into(),
        mode,
    }
}

fn status_mode(host: &mut Host) -> Mode {
    host.handle(1, Request::Status).status.unwrap().mode
}

fn settings_request(value: serde_json::Value) -> Request {
    serde_json::from_value(value).expect("设置数据请求应被协议支持")
}

/// 只导出其中几类数据的请求。
fn export_parts_request(path: &Path, parts: &[&str]) -> Request {
    settings_request(
        serde_json::json!({"op": "export_settings", "path": path.to_string_lossy(), "parts": parts}),
    )
}

/// 每一类数据都有内容的一台「旧电脑」。
fn full_profile_host(name: &str) -> Host {
    let mut source = learning_host(name);
    let main_lexicon = source.data_dir().join("source-lexicon.tsv");
    std::fs::write(&main_lexicon, "ni hao\t你好\t1000\na te\t阿特\t900\n").unwrap();
    configure(&mut source, |c| {
        c.page_size = 9;
        c.lexicon = main_lexicon.to_string_lossy().into();
        c.default_mode = DefaultMode::Global;
    });
    source.mode = Mode::English;
    assert!(
        source
            .handle(
                1,
                Request::SetQuickPhrases {
                    phrases: vec![freetype_protocol::QuickPhrase {
                        code: "dz".into(),
                        text: "远端地址".into(),
                        weight: 20
                    }],
                }
            )
            .ok
    );
    assert!(
        source
            .handle(
                1,
                settings_request(
                    serde_json::json!({"op": "set_common_phrases", "phrases": ["远端常用语"]})
                )
            )
            .ok
    );
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(
        source.data_dir().join("dicts/人名.txt"),
        "阿特\ta te\t800\n",
    )
    .unwrap();
    source.recent_emoji = vec!["😊".into()];
    std::fs::write(source.data_dir().join("statusbar.txt"), "100,200\n").unwrap();
    for _ in 0..3 {
        source
            .engine
            .as_ref()
            .unwrap()
            .record_selection_with_pinyin("私密词", Some("si mi ci"));
    }
    source
}

/// 每一类数据都和「旧电脑」不一样的一台「本机」。
fn local_profile_host(name: &str) -> Host {
    let mut target = learning_host(name);
    let main_lexicon = target.data_dir().join("local-lexicon.tsv");
    std::fs::write(&main_lexicon, "ben ji\t本机\t1000\n").unwrap();
    configure(&mut target, |c| {
        c.page_size = 8;
        c.lexicon = main_lexicon.to_string_lossy().into();
    });
    assert!(
        target
            .handle(
                1,
                Request::SetQuickPhrases {
                    phrases: vec![freetype_protocol::QuickPhrase {
                        code: "yx".into(),
                        text: "本机邮箱".into(),
                        weight: 10
                    }],
                }
            )
            .ok
    );
    assert!(
        target
            .handle(
                1,
                settings_request(
                    serde_json::json!({"op": "set_common_phrases", "phrases": ["本机常用语"]})
                )
            )
            .ok
    );
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    std::fs::write(
        target.data_dir().join("dicts/旧词库.txt"),
        "旧词\tjiu ci\t100\n",
    )
    .unwrap();
    target.recent_emoji = vec!["🎉".into()];
    std::fs::write(target.data_dir().join("statusbar.txt"), "1,2\n").unwrap();
    target
        .engine
        .as_ref()
        .unwrap()
        .record_selection_with_pinyin("旧词", Some("jiu ci"));
    target
}

fn backup_keys(path: &Path) -> Vec<String> {
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    data.as_object().unwrap().keys().cloned().collect()
}

struct Recorder(std::rc::Rc<std::cell::RefCell<Vec<BarState>>>);

impl Desktop for Recorder {
    fn update(&self, state: &BarState) {
        self.0.borrow_mut().push(state.clone());
    }
    fn open_settings(&self) {}
}

/// 上屏联想的测试宿主：「今天」后面模型里有「下雨」「天气」。
fn predict_host(name: &str) -> Host {
    let mut host = host(name);
    let lexicon = wowtype_core::MemoryLexicon::from_tsv_str(
        "jin tian\t今天\t500000\nxia yu\t下雨\t20000\ntian qi\t天气\t60000\nkai hui\t开会\t30000\nbu cuo\t不错\t40000\n",
    )
    .unwrap();
    let lm = wowtype_core::BigramLm::from_bigram_tsv_str(
        "今天\t下雨\t300\n今天\t天气\t200\n天气\t不错\t300\n开会\t今天\t5\n不错\t今天\t5\n下雨\t今天\t5\n",
    )
    .unwrap();
    host.install_engine(Ok(Engine::new(
        std::sync::Arc::new(lexicon),
        std::sync::Arc::new(lm),
    )));
    host
}

fn commit_today(host: &mut Host) -> Frame {
    typing(host, "jintian");
    press(host, 0x20)
}
