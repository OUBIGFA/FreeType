//! FreeTypeServer.exe
//!
//! 不带参数：后台常驻（没有窗口），载入引擎，在 `\\.\pipe\FreeType-<SID>` 上服务。
//! 安装器用的命令见 `setup.rs`；`--stop` 让正在跑的服务进程存盘后退出。
#![cfg_attr(not(test), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    std::process::exit(app::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("FreeTypeServer 只能在 Windows 上运行");
    std::process::exit(1);
}

#[cfg(windows)]
mod app {
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use freetype_protocol::{pipe_name, Request};
    use freetype_server::host::{load_engine, Host};
    use freetype_server::pipe::{self, Message};
    use freetype_server::store::Store;
    use freetype_server::{data_dir, instance_suffix, local_dir, log, resource_dir, setup};

    /// 多久存一次学到的词。
    const SAVE_INTERVAL: Duration = Duration::from_secs(30);

    enum Event {
        Pipe(Message),
        Loaded(Result<wowtype_core::Engine, String>),
    }

    fn report(result: Result<(), String>, what: &str) -> i32 {
        match result {
            Ok(()) => 0,
            Err(e) => {
                log(&format!("{what}失败：{e}"));
                1
            }
        }
    }

    fn pipe_path() -> Result<String, String> {
        pipe::current_user_sid()
            .map(|sid| pipe_name(&sid) + &instance_suffix())
            .map_err(|e| format!("读不到当前用户：{e}"))
    }

    pub fn run() -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("--register-tsf") => {
                let bin = args
                    .get(1)
                    .filter(|a| !a.starts_with("--"))
                    .map(PathBuf::from)
                    .or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from));
                let marker = args
                    .iter()
                    .position(|a| a == "--marker")
                    .and_then(|i| args.get(i + 1))
                    .map(PathBuf::from);
                let code = match bin {
                    Some(bin) => report(setup::register_tsf(&bin), "注册输入法"),
                    None => 1,
                };
                // 安装器靠这个文件知道注册成功了（提权运行拿不到退出码）
                if code == 0 {
                    if let Some(marker) = marker {
                        let _ = std::fs::write(marker, "ok");
                    }
                }
                return code;
            }
            Some("--unregister-tsf") => return report(setup::unregister_tsf(), "注销输入法"),
            Some("--enable") => return report(setup::enable_for_user(true), "加入输入法列表"),
            Some("--disable") => return report(setup::enable_for_user(false), "移出输入法列表"),
            Some("--stop") => {
                let result = pipe_path().and_then(|name| {
                    pipe::request(&name, &Request::Shutdown)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
                // 没在跑也算成功：安装器在升级前调用，不该因此失败
                let _ = result;
                return 0;
            }
            Some(other) if other.starts_with("--") => {
                log(&format!("不认识的参数 {other}"));
                return 2;
            }
            _ => {}
        }
        serve()
    }

    fn serve() -> i32 {
        if !pipe::single_instance() {
            return 0;
        }
        // 词库快照写不进安装目录时放到 %LOCALAPPDATA%\FreeType\cache
        if std::env::var_os("XDG_CACHE_HOME").is_none() {
            std::env::set_var("XDG_CACHE_HOME", local_dir().join("cache"));
        }
        let name = match pipe_path() {
            Ok(name) => name,
            Err(e) => {
                log(&e);
                return 1;
            }
        };
        let data = data_dir();
        let resources = resource_dir();
        let (events, inbox) = mpsc::channel::<Event>();

        let loader = events.clone();
        let store_dir = data.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = load_engine(&resources, &store_dir);
            match &result {
                Ok(_) => log(&format!("词库载入完成，用时 {} ms", started.elapsed().as_millis())),
                Err(e) => log(e),
            }
            let _ = loader.send(Event::Loaded(result));
        });

        let (pipe_tx, pipe_rx) = mpsc::channel::<Message>();
        let forward = events.clone();
        std::thread::spawn(move || {
            while let Ok(message) = pipe_rx.recv() {
                if forward.send(Event::Pipe(message)).is_err() {
                    break;
                }
            }
        });
        let listen_name = name.clone();
        std::thread::spawn(move || {
            let sid = pipe::current_user_sid().unwrap_or_default();
            if let Err(e) = pipe::listen(&listen_name, &sid, pipe_tx) {
                log(&format!("建不了管道：{e}"));
                std::process::exit(1);
            }
        });

        let mut host = Host::new(Store::new(data));
        loop {
            match inbox.recv_timeout(SAVE_INTERVAL) {
                Ok(Event::Loaded(result)) => host.install_engine(result),
                Ok(Event::Pipe(Message::Connected(_))) => {}
                Ok(Event::Pipe(Message::Disconnected(id))) => host.disconnect(id),
                Ok(Event::Pipe(Message::Request(id, request, reply))) => {
                    let shutdown = request == Request::Shutdown;
                    let response = host.handle(id, request);
                    let _ = reply.send(response);
                    if shutdown {
                        host.save_if_dirty();
                        log("收到退出请求");
                        // 让管道线程把应答写出去
                        std::thread::sleep(Duration::from_millis(100));
                        return 0;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => host.save_if_dirty(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return 1,
            }
        }
    }
}
