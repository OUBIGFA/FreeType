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
        /// 后台任务线程算好、需要在引擎线程应用的结果（同步合并）。
        Apply(freetype_server::tasks::Apply),
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

    /// 设置或同步换了主词库：在后台重新载入（旧的晚到了不要）。
    fn reload_if_changed(host: &mut Host, generation: &mut u64, events: &mpsc::Sender<Event>) {
        if let Some(lexicon) = host.take_lexicon_change() {
            *generation += 1;
            host.set_lexicon_in_use(freetype_server::host::lexicon_path(&resource_dir(), &lexicon).display().to_string());
            spawn_loader(events.clone(), *generation, lexicon);
        }
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
            Some("--compile-lexicon") => {
                let result = match args.as_slice() {
                    [_, source, target] => wowtype_core::lexicon::MappedLexicon::compile(
                        std::path::Path::new(source), std::path::Path::new(target)).map_err(|e| format!("{e:#}")),
                    _ => Err("需要源词库和输出索引两个路径".into()),
                };
                if let Err(error) = result { eprintln!("{error}"); return 1; }
                return 0;
            }
            Some("--serve") => return serve(),
            Some("--set-brand-icon") => {
                let result = match args.as_slice() {
                    [_, path] => freetype_server::brand_icon::apply(std::path::Path::new(path)),
                    _ => Err("需要品牌 ICO 图像路径".into()),
                };
                if let Err(error) = &result { eprintln!("{error}"); }
                return report(result, "更新系统品牌图标");
            }
            Some("--set-brand-icon-elevated") => {
                use std::io::Write;
                let result = match args.as_slice() {
                    [_, path, result_file] => {
                        let result = freetype_server::brand_icon::install(std::path::Path::new(path));
                        let response = match &result {
                            Ok(()) => freetype_protocol::Response::ok(),
                            Err(error) => freetype_protocol::Response::error(error.clone()),
                        };
                        let recorded = serde_json::to_vec(&response).map_err(|error| error.to_string())
                            .and_then(|data| std::fs::OpenOptions::new().write(true).create_new(true).open(result_file)
                                .and_then(|mut file| file.write_all(&data)).map_err(|error| error.to_string()));
                        result.and(recorded)
                    }
                    _ => Err("需要品牌 ICO 图像和结果文件路径".into()),
                };
                return report(result, "更新系统品牌图标");
            }
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
            Some("--start-and-check") => return report(setup::start_and_check(), "启动并核对新版服务"),
            Some("--stop") => {
                let result = pipe_path().and_then(|name| {
                    match pipe::request(&name, &Request::Shutdown) {
                        Ok(reply) if reply.ok => Ok(()),
                        Ok(reply) => Err(reply.error.unwrap_or_else(|| "服务未完成保存".into())),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(error.to_string()),
                    }
                });
                if result.is_ok() { stop_watchdog(); }
                return report(result, "停止服务");
            }
            Some(other) if other.starts_with("--") => {
                log(&format!("不认识的参数 {other}"));
                return 2;
            }
            _ => {}
        }
        if instance_suffix().is_empty() { supervise() } else { serve() }
    }

    fn stop_watchdog() {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
        let name = windows::core::HSTRING::from(format!(r"Local\FreeType.StopWatchdog{}", instance_suffix()));
        unsafe {
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, &name) {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
        }
    }

    fn supervise() -> i32 {
        use std::os::windows::process::CommandExt;
        use std::time::Instant;
        if !pipe::single_watchdog() { return 0; }
        let exe = match std::env::current_exe() { Ok(exe) => exe, Err(error) => return report(Err(error.to_string()), "定位服务") };
        use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
        let name = windows::core::HSTRING::from(format!(r"Local\FreeType.StopWatchdog{}", instance_suffix()));
        let stop = match unsafe { CreateEventW(None, true, false, &name) } {
            Ok(event) => event,
            Err(error) => return report(Err(error.to_string()), "创建服务停机事件"),
        };
        let mut delay = 1u64;
        loop {
            if unsafe { WaitForSingleObject(stop, 0) } == WAIT_OBJECT_0 { break; }
            let started = Instant::now();
            let status = std::process::Command::new(&exe).arg("--serve").creation_flags(0x08000000).status();
            match status {
                Ok(exit) if exit.success() => break, // 正常退出/升级后不要再拉起。
                Ok(exit) => log(&format!("服务意外退出：{exit}")),
                Err(error) => log(&format!("无法启动服务：{error}")),
            }
            if started.elapsed() >= Duration::from_secs(120) { delay = 1; }
            if unsafe { WaitForSingleObject(stop, (delay * 1000) as u32) } == WAIT_OBJECT_0 { break; }
            delay = (delay * 2).min(60);
        }
        unsafe { let _ = CloseHandle(stop); }
        0
    }

    fn serve() -> i32 {
        if !pipe::single_instance() {
            return 0;
        }
        freetype_server::prioritize_input();
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
        let (apply_tx, apply_rx) = mpsc::channel::<freetype_server::tasks::Apply>();
        host.set_apply_channel(apply_tx);
        let forward = events.clone();
        std::thread::spawn(move || {
            while let Ok(apply) = apply_rx.recv() {
                if forward.send(Event::Apply(apply)).is_err() {
                    break;
                }
            }
        });
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
        let mut next_save = std::time::Instant::now() + SAVE_INTERVAL;
        loop {
            // 定时保存以经过的时间为准，不能等消息队列连续空闲 30 秒。
            // 持续输入和托盘轮询都会产生消息，否则学习记录可能整天不落盘。
            if std::time::Instant::now() >= next_save {
                host.save_if_dirty();
                next_save = std::time::Instant::now() + SAVE_INTERVAL;
            }
            clipboard_on.store(host.config().clipboard_history, Ordering::Relaxed);
            match inbox.recv_timeout(next_save.saturating_duration_since(std::time::Instant::now())) {
                Ok(Event::Bar(event)) => {
                    let request = match event {
                        BarEvent::Foreground(app) => {
                            host.foreground(&app);
                            continue;
                        }
                        BarEvent::ToggleMode => Request::ToggleMode { shift: None },
                        BarEvent::TogglePunct => Request::Command { command: Command::TogglePunct },
                        BarEvent::ToggleTraditional => Request::Command {
                            command: Command::ToggleTraditional,
                        },
                        BarEvent::EmojiPanel => {
                            desktop::open_emoji_panel();
                            continue;
                        }
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
                Ok(Event::Apply(apply)) => {
                    apply(&mut host);
                    reload_if_changed(&mut host, &mut generation, &events);
                }
                Ok(Event::Pipe(Message::Request(id, request, reply))) => {
                    if request != Request::Shutdown {
                        // 同步的重计算转到后台任务线程，其余请求照常在这里处理。
                        host.dispatch(id, request, reply);
                        reload_if_changed(&mut host, &mut generation, &events);
                        continue;
                    }
                    let response = host.handle(id, request);
                    let should_exit = response.ok;
                    let _ = reply.send(response);
                    if should_exit {
                        host.save_if_dirty();
                        log("收到退出请求");
                        // 让管道线程把应答写出去
                        std::thread::sleep(Duration::from_millis(100));
                        // 先丢弃还没应用的后台结果，后台线程不再等主循环，宿主析构时能顺利收尾。
                        drop(inbox);
                        return 0;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {},
                Err(mpsc::RecvTimeoutError::Disconnected) => return 1,
            }
        }
    }
}
