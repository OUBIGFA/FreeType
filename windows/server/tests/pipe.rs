//! 端到端：启动真的 FreeTypeServer.exe，经命名管道打字、导入快捷短语、退出。
#![cfg(windows)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use freetype_protocol::{decode, encode, pipe_name, Frame, Key, Request, Response};

struct Server {
    root: PathBuf,
    pipe: String,
    child: std::process::Child,
}

impl Server {
    fn start(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("freetype-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let resources = root.join("res");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(
            resources.join("lexicon.tsv"),
            "ni hao\t你好\t1000\nni\t你\t900\nhao\t好\t800\n",
        )
        .unwrap();
        let instance = format!("e2e-{tag}-{}", std::process::id());
        let sid = freetype_server::pipe::current_user_sid().unwrap();
        let pipe = format!("{}-{instance}", pipe_name(&sid));
        let child = Command::new(env!("CARGO_BIN_EXE_FreeTypeServer"))
            .env("FREETYPE_INSTANCE", &instance)
            .env("FREETYPE_DATA_DIR", root.join("data"))
            .env("FREETYPE_RESOURCE_DIR", &resources)
            .env("XDG_CACHE_HOME", root.join("cache"))
            .spawn()
            .unwrap();
        Self { root, pipe, child }
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
