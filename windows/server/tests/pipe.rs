//! 端到端：启动真的 FreeTypeServer.exe，经命名管道打字、导入快捷短语、退出。
#![cfg(windows)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use freetype_protocol::{decode, encode, pipe_name, DefaultMode, Frame, Key, Mode, Request, Response};

struct Server {
    root: PathBuf,
    pipe: String,
    child: std::process::Child,
    instance: String,
}

impl Server {
    fn start(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("freetype-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let resources = root.join("res");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(
            resources.join("lexicon.tsv"),
            "ni hao\t你好\t1000\nni\t你\t900\nhao\t好\t800\nba\t八\t900\nba\t巴\t1000\n",
        )
        .unwrap();
        let instance = format!("e2e-{tag}-{}", std::process::id());
        let sid = freetype_server::pipe::current_user_sid().unwrap();
        let pipe = format!("{}-{instance}", pipe_name(&sid));
        let child = Self::launch(&root, &instance);
        Self { root, pipe, child, instance }
    }

    fn launch(root: &Path, instance: &str) -> std::process::Child {
        Command::new(env!("CARGO_BIN_EXE_FreeTypeServer"))
            .env("FREETYPE_INSTANCE", instance)
            .env("FREETYPE_DATA_DIR", root.join("data"))
            .env("FREETYPE_RESOURCE_DIR", root.join("res"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .spawn()
            .unwrap()
    }

    fn restart(&mut self) {
        assert!(self.connect().call(&Request::Shutdown).ok);
        let started = Instant::now();
        while self.child.try_wait().unwrap().is_none() {
            assert!(started.elapsed() < Duration::from_secs(5), "服务进程没有退出");
            std::thread::sleep(Duration::from_millis(50));
        }
        self.child = Self::launch(&self.root, &self.instance);
    }

    fn connect(&self) -> Connection {
        let started = Instant::now();
        loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&self.pipe) {
                Ok(file) => {
                    return Connection {
                        reader: BufReader::new(file.try_clone().unwrap()),
                        writer: file,
                    }
                }
                Err(_) if started.elapsed() < Duration::from_secs(10) => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(e) => panic!("连不上服务进程：{e}"),
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Connection {
    reader: BufReader<std::fs::File>,
    writer: std::fs::File,
}

impl Connection {
    fn call(&mut self, request: &Request) -> Response {
        self.writer.write_all(&encode(request)).unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        decode(line.as_bytes()).unwrap()
    }

    fn key(&mut self, c: char) -> Frame {
        let key = Key {
            keysym: c as u32,
            modifiers: 0,
            test: false,
        };
        self.call(&Request::Key { key, before: None }).frame.unwrap()
    }
}

#[test]
fn typing_through_the_pipe() {
    let server = Server::start("typing");
    let mut conn = server.connect();
    let started = Instant::now();
    loop {
        let status = conn.call(&Request::Hello { session: true, app: "test.exe".into() }).status.unwrap();
        if status.ready {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "词库一直没载入：{:?}", status.error);
        std::thread::sleep(Duration::from_millis(50));
    }
    for c in "niha".chars() {
        assert!(conn.key(c).handled);
    }
    let frame = conn.key('o');
    assert_eq!(frame.candidates.first().map(String::as_str), Some("你好"));
    let frame = conn.key(' ');
    assert_eq!(frame.commit.as_deref(), Some("你好"));

    let file = server.root.join("qp.txt");
    std::fs::write(&file, "dz\t四川省\\n宜宾市\n").unwrap();
    let report = conn
        .call(&Request::ImportQuickPhrases {
            path: file.display().to_string(),
            replace: false,
        })
        .report
        .unwrap();
    assert_eq!(report.added, 1);
    conn.key('d');
    assert_eq!(conn.key('z').candidates[0], "四川省⏎宜宾市");
    assert_eq!(conn.key(' ').commit.as_deref(), Some("四川省\n宜宾市"));

    // 第二条连接是另一个输入框：互不影响
    let mut other = server.connect();
    other.call(&Request::Hello { session: true, app: "test.exe".into() });
    other.key('n');
    assert!(conn.key('h').preedit.starts_with('h'));

    assert!(conn.call(&Request::Shutdown).ok);
    drop(conn);
    drop(other);
    let mut server = server;
    let started = Instant::now();
    while server.child.try_wait().unwrap().is_none() {
        assert!(started.elapsed() < Duration::from_secs(5), "收到退出请求后没有退出");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(server.root.join("data").join("quick_phrases.txt").exists());
}

#[test]
fn global_mode_survives_process_restart_and_keycaps_round_trip() {
    let mut server = Server::start("global-keycap");
    let mut first = server.connect();
    let started = Instant::now();
    loop {
        let status = first.call(&Request::Hello { session: true, app: "first.exe".into() }).status.unwrap();
        if status.ready { break; }
        assert!(started.elapsed() < Duration::from_secs(20), "词库未载入：{:?}", status.error);
        std::thread::sleep(Duration::from_millis(50));
    }
    first.key('b');
    let frame = first.key('a');
    let index = frame.candidates.iter().position(|s| s == "8️⃣").expect("应保留上游数字表情");
    let selected = first.call(&Request::Select { index }).frame.unwrap();
    assert_eq!(selected.commit.as_deref(), Some("8️⃣"), "跨管道仍是完整的表情序列");
    let mut config = first.call(&Request::GetConfig).config.unwrap();
    config.default_mode = DefaultMode::Global;
    config.mode_scope = "app".into();
    assert!(first.call(&Request::SetConfig { config }).ok);
    assert_eq!(first.call(&Request::ToggleMode { shift: None }).frame.unwrap().mode, Mode::English);
    let mut second = server.connect();
    second.call(&Request::Hello { session: true, app: "second.exe".into() });
    assert!(!second.key('n').handled, "另一个应用立即沿用英文状态");
    drop(first);
    drop(second);
    server.restart();
    let mut restarted = server.connect();
    let status = restarted.call(&Request::Hello { session: true, app: "third.exe".into() }).status.unwrap();
    assert_eq!(status.mode, Mode::English);
    assert!(!restarted.key('n').handled, "新进程也沿用最后主动选择的英文状态");
}

#[test]
fn common_phrases_and_all_settings_backup_round_trip_through_real_pipe() {
    let mut server = Server::start("profile");
    let mut conn = server.connect();
    let started = Instant::now();
    loop {
        let status = conn.call(&Request::Hello { session: true, app: "profile.exe".into() }).status.unwrap();
        if status.ready { break; }
        assert!(started.elapsed() < Duration::from_secs(20), "词库未载入：{:?}", status.error);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(conn.call(&Request::SetCommonPhrases { phrases: vec!["打tag\n下一行".into()] }).ok);
    conn.key('v'); conn.key(' '); conn.key('2');
    assert_eq!(conn.key('1').commit.as_deref(), Some("打 tag\n下一行"));
    conn.call(&Request::AddUserWord { word: "阿特".into(), pinyin: "a te".into() });
    let path = server.root.join("all.json").display().to_string();
    assert!(conn.call(&Request::ExportSettings { path: path.clone() }).ok);
    assert!(conn.call(&Request::SetCommonPhrases { phrases: Vec::new() }).ok);
    assert!(conn.call(&Request::ClearLearned).ok);
    assert!(conn.call(&Request::InspectSettingsBackup { path: path.clone() }).ok);
    let restored = conn.call(&Request::ImportSettings { path });
    assert!(restored.ok, "{:?}", restored.error);
    assert_eq!(restored.common_phrases.unwrap(), vec!["打tag\n下一行"]);
    assert!(restored.user_words.unwrap().iter().any(|word| word.word == "阿特"));
    drop(conn);
    server.restart();
    let mut conn = server.connect();
    assert_eq!(conn.call(&Request::ListCommonPhrases).common_phrases.unwrap(), vec!["打tag\n下一行"]);
}
