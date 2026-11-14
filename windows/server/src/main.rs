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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use freetype_protocol::{pipe_name, Command, Request};
    use freetype_server::desktop::{self, BarEvent};
    use freetype_server::host::{load_engine, Host};
    use freetype_server::pipe::{self, Message};
    use freetype_server::store::Store;
    use freetype_server::{data_dir, instance_suffix, local_dir, log, resource_dir, setup};

    /// 多久存一次学到的词。
    const SAVE_INTERVAL: Duration = Duration::from_secs(30);

    enum Event {
        Pipe(Message),
        /// 第几次载入（换了词库会再载入；旧的晚到了不要）、结果。
        Loaded(u64, Result<wowtype_core::Engine, String>),
        Bar(BarEvent),
    }

    /// 在后台载入引擎。
    fn spawn_loader(events: mpsc::Sender<Event>, generation: u64, lexicon: String) {
        let resources = resource_dir();
        let store_dir = data_dir();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = load_engine(&resources, &store_dir, &lexicon);
            let path = freetype_server::host::lexicon_path(&resources, &lexicon);
            match &result {
                Ok(_) => log(&format!("词库载入完成（{}），用时 {} ms", path.display(), started.elapsed().as_millis())),
                Err(e) => log(e),
            }
            let _ = events.send(Event::Loaded(generation, result));
        });
    }

    /// 状态栏、剪贴板这些不属于哪个输入框的请求用的连接号。
    const DESKTOP: u64 = 0;

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
        let marker = args.iter().position(|arg| arg == "--marker")
            .and_then(|index| args.get(index + 1)).map(PathBuf::from);
        match args.first().map(String::as_str) {
            Some("--register-tsf") => {
                let bin = args
                    .get(1)
                    .filter(|a| !a.starts_with("--"))
                    .map(PathBuf::from)
                    .or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from));
                let result = match bin {
                    Some(bin) => setup::register_tsf(&bin),
                    None => Err("找不到输入法组件目录".to_string()),
                };
                return report(setup::record_setup_result(result, marker.as_deref()), "注册输入法");
            }
            Some("--unregister-tsf") => {
                return report(setup::record_setup_result(setup::unregister_tsf(), marker.as_deref()), "注销输入法");
            }
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
        // panic 的原因写进日志（没有控制台，不然看不到）
        std::panic::set_hook(Box::new(|info| log(&format!("内部错误：{info}"))));
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
        let (events, inbox) = mpsc::channel::<Event>();

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
        let mut generation = 0u64;
        let lexicon = host.config().lexicon.clone();
        host.set_lexicon_in_use(freetype_server::host::lexicon_path(&resource_dir(), &lexicon).display().to_string());
        spawn_loader(events.clone(), generation, lexicon);
        // 状态栏和剪贴板（测试用的实例不要桌面上的东西）
        let clipboard_on = Arc::new(AtomicBool::new(host.config().clipboard_history));
        if instance_suffix().is_empty() {
            let (bar_tx, bar_rx) = mpsc::channel::<BarEvent>();
            let forward = events.clone();
            std::thread::spawn(move || {
                while let Ok(event) = bar_rx.recv() {
                    if forward.send(Event::Bar(event)).is_err() {
                        break;
                    }
                }
            });
            host.set_desktop(Box::new(desktop::start(bar_tx, clipboard_on.clone())));
        }
        loop {
            clipboard_on.store(host.config().clipboard_history, Ordering::Relaxed);
            match inbox.recv_timeout(SAVE_INTERVAL) {
                Ok(Event::Bar(event)) => {
                    let request = match event {
                        BarEvent::ToggleMode => Request::ToggleMode { shift: None },
                        BarEvent::TogglePunct => Request::Command { command: Command::TogglePunct },
                        BarEvent::ToggleTraditional => Request::Command {
                            command: Command::ToggleTraditional,
                        },
                        BarEvent::OpenSettings => Request::Command { command: Command::OpenSettings },
                        BarEvent::HideBar => Request::Command {
                            command: Command::ToggleStatusBar,
                        },
                    };
                    host.handle(DESKTOP, request);
                }
                Ok(Event::Loaded(gen, result)) => {
                    if gen == generation {
                        host.install_engine(result);
                    }
                }
                Ok(Event::Pipe(Message::Connected(_))) => {}
                Ok(Event::Pipe(Message::Disconnected(id))) => host.disconnect(id),
                Ok(Event::Pipe(Message::Request(id, request, reply))) => {
                    let shutdown = request == Request::Shutdown;
                    let response = host.handle(id, request);
                    let _ = reply.send(response);
                    if let Some(lexicon) = host.take_lexicon_change() {
                        generation += 1;
                        host.set_lexicon_in_use(
                            freetype_server::host::lexicon_path(&resource_dir(), &lexicon).display().to_string(),
                        );
                        spawn_loader(events.clone(), generation, lexicon);
                    }
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
