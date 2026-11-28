//! 中英文状态、按应用预设、Caps Lock、焦点提示、状态栏与品牌图标。
use super::*;

#[test]
fn status_reports_whole_seconds_since_actual_input_only() {
    let mut host = host("input-idle");
    let idle = |host: &mut Host| {
        host.handle(1, Request::Status)
            .status
            .unwrap()
            .input_idle_seconds
    };
    assert_eq!(idle(&mut host), None, "启动后没打字时不能谎报刚有输入");
    host.handle(
        1,
        Request::Key {
            key: Key {
                keysym: 'n' as u32,
                modifiers: 0,
                test: true,
            },
            before: None,
        },
    );
    assert_eq!(idle(&mut host), None, "按键预判不是实际输入");
    typing(&mut host, "ni");
    assert_eq!(idle(&mut host), Some(0));
    cleanup(&host);
}

#[test]
fn vim_escape_switches_only_on_real_unmodified_idle_press_for_configured_app() {
    let mut host = host("vim-escape");
    let mut config = serde_json::to_value(host.config()).unwrap();
    config["vim_mode_apps"] = serde_json::json!(["NOTEPAD.EXE"]);
    assert!(
        host.handle(
            1,
            Request::SetConfig {
                config: serde_json::from_value(config).unwrap()
            }
        )
        .ok
    );
    typing(&mut host, "ni");
    let cancelled = key_with(&mut host, 1, keysym::ESCAPE, 0);
    assert!(cancelled.handled && cancelled.preedit.is_empty());
    assert_eq!(cancelled.mode, Mode::Chinese);
    // Bit 5 is the Windows physical-repeat hint; holding the cancellation Esc must not switch.
    assert_eq!(
        key_with(&mut host, 1, keysym::ESCAPE, 1 << 5).mode,
        Mode::Chinese
    );
    for mods in [
        modifier::SHIFT,
        modifier::CTRL,
        modifier::ALT,
        modifier::SUPER,
    ] {
        assert_eq!(
            key_with(&mut host, 1, keysym::ESCAPE, mods).mode,
            Mode::Chinese
        );
    }
    let preview = host
        .handle(
            1,
            Request::Key {
                key: Key {
                    keysym: keysym::ESCAPE,
                    modifiers: 0,
                    test: true,
                },
                before: None,
            },
        )
        .frame
        .unwrap();
    assert!(preview.handled, "预判需保留实际处理机会");
    assert_eq!(
        host.handle(1, Request::Status).status.unwrap().mode,
        Mode::Chinese,
        "预判不切换模式"
    );
    let actual = key_with(&mut host, 1, keysym::ESCAPE, 0);
    assert!(!actual.handled, "Esc 交给编辑器退出 insert 模式");
    assert_eq!(actual.mode, Mode::English);
    let restarted = Host::new(Store::new(host.data_dir().to_path_buf()));
    assert_eq!(
        serde_json::to_value(restarted.config()).unwrap()["vim_mode_apps"],
        serde_json::json!(["notepad.exe"])
    );
    cleanup(&host);
}

#[test]
fn brand_icon_changes_persist_without_interrupting_composition() {
    let mut host = host("brand-icon");
    typing(&mut host, "ni");
    configure(&mut host, |config| config.brand_icon = "white".into());
    let frame = typing(&mut host, "hao");
    assert_eq!(frame.candidates.first().map(String::as_str), Some("你好"));
    let restarted = Host::new(Store::new(host.data_dir().to_path_buf()));
    assert_eq!(restarted.config().brand_icon, "white");
    cleanup(&host);
}

#[test]
fn language_bar_reads_only_changed_brand_images_without_an_input_session() {
    let mut host = host("language-bar-brand");
    let request = |known: Option<u64>| {
        serde_json::from_value(serde_json::json!({
            "op": "get_brand_icon", "known_revision": known
        }))
        .expect("语言栏必须能单独读取用于显示的品牌图像")
    };
    let first = serde_json::to_value(host.handle(99, request(None))).unwrap();
    assert_eq!(first["brand_icon"]["appearance"], "default");
    assert!(first.get("config").is_none(), "图标读取不能暴露其他设置");
    assert!(!host.clients.contains_key(&99), "读取图像不创建输入会话");
    let revision = first["brand_icon"]["revision"].as_u64().unwrap();
    let unchanged = serde_json::to_value(host.handle(99, request(Some(revision)))).unwrap();
    assert!(
        unchanged.get("brand_icon").is_none(),
        "未变图标不能反复传输或解码"
    );
    configure(&mut host, |config| config.font_size = 20);
    assert!(
        serde_json::to_value(host.handle(99, request(Some(revision))))
            .unwrap()
            .get("brand_icon")
            .is_none()
    );
    configure(&mut host, |config| config.brand_icon = "white".into());
    let white = serde_json::to_value(host.handle(99, request(Some(revision)))).unwrap();
    assert_eq!(white["brand_icon"]["appearance"], "white");
    assert_ne!(white["brand_icon"]["revision"].as_u64().unwrap(), revision);
    configure(&mut host, |config| {
        config.brand_icon = "custom".into();
        config.custom_brand_icon = "saved-png".into();
    });
    let custom = serde_json::to_value(host.handle(99, request(None))).unwrap();
    assert_eq!(custom["brand_icon"]["custom_png"], "saved-png");
    cleanup(&host);
}

#[test]
fn language_bar_is_notified_only_when_the_displayed_brand_changes() {
    struct Display(std::rc::Rc<std::cell::Cell<u32>>);
    impl Desktop for Display {
        fn update(&self, _: &BarState) {}
        fn open_settings(&self) {}
        fn brand_icon_changed(&self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut host = host("brand-notify");
    host.set_desktop(Box::new(Display(notifications.clone())));
    configure(&mut host, |config| config.font_size = 20);
    configure(&mut host, |config| {
        config.custom_brand_icon = "saved-png".into()
    });
    assert_eq!(
        notifications.get(),
        0,
        "普通设置和未选中的自定义图像不应刷新语言栏"
    );
    configure(&mut host, |config| config.brand_icon = "white".into());
    assert_eq!(notifications.get(), 1);
    configure(&mut host, |config| config.brand_icon = "white".into());
    assert_eq!(notifications.get(), 1, "同一外观不重复通知");
    configure(&mut host, |config| config.brand_icon = "custom".into());
    assert_eq!(notifications.get(), 2);
    configure(&mut host, |config| {
        config.custom_brand_icon = "new-png".into()
    });
    assert_eq!(notifications.get(), 3, "重新导入自定义图标也应立即刷新");
    cleanup(&host);
}

#[test]
fn english_mode_passes_keys_and_toggle_commits_letters() {
    let mut host = host("mode");
    typing(&mut host, "nih");
    let frame = host
        .handle(1, Request::ToggleMode { shift: None })
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint.as_deref(), Some("英"));
    assert_eq!(frame.commit.as_deref(), Some("nih"));
    assert!(!press(&mut host, 'a' as u32).handled);
    let frame = host
        .handle(1, Request::ToggleMode { shift: None })
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::Chinese);
    assert!(press(&mut host, 'a' as u32).handled);
    cleanup(&host);
}

#[test]
fn shift_side_follows_setting_and_hint_can_be_off() {
    let mut host = host("shift-side");
    configure(&mut host, |c| {
        c.shift_toggle = "left".into();
        c.hint_on_toggle = false;
    });
    let frame = host
        .handle(
            1,
            Request::ToggleMode {
                shift: Some("right".into()),
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::Chinese, "右 Shift 不切");
    let frame = host
        .handle(
            1,
            Request::ToggleMode {
                shift: Some("left".into()),
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint, None);
    cleanup(&host);
}

#[test]
fn per_app_mode_scope() {
    let mut host = host("app-scope");
    configure(&mut host, |c| c.mode_scope = "app".into());
    host.handle(
        2,
        Request::Hello {
            session: true,
            app: "Code.exe".into(),
        },
    );
    host.handle(1, Request::ToggleMode { shift: None });
    assert!(!press(&mut host, 'a' as u32).handled, "记事本切成英文");
    assert!(
        key_with(&mut host, 2, 'a' as u32, 0).handled,
        "另一个应用还是中文"
    );
    configure(&mut host, |c| c.default_mode = DefaultMode::English);
    host.handle(
        3,
        Request::Hello {
            session: true,
            app: "new.exe".into(),
        },
    );
    assert!(
        !key_with(&mut host, 3, 'a' as u32, 0).handled,
        "新应用用默认的英文"
    );
    cleanup(&host);
}

#[test]
fn global_default_keeps_manual_mode_across_apps_settings_and_restart() {
    let mut host = host("global-default");
    // 配置经由真实 JSON 协议读取，旧的中文、英文写法继续兼容。
    let parsed = serde_json::from_str::<Config>(r#"{"default_mode":"global","mode_scope":"app"}"#);
    assert!(parsed.is_ok(), "默认状态必须接受全局选项：{parsed:?}");
    let response = host.handle(
        1,
        Request::SetConfig {
            config: parsed.unwrap(),
        },
    );
    assert!(response.ok, "{response:?}");
    let frame = host
        .handle(1, Request::ToggleMode { shift: None })
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    host.handle(
        1,
        Request::Focus {
            active: false,
            before: None,
        },
    );
    host.handle(
        2,
        Request::Hello {
            session: true,
            app: "browser.exe".into(),
        },
    );
    let focused = host
        .handle(
            2,
            Request::Focus {
                active: true,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(
        focused.mode,
        Mode::English,
        "全局模式不受旧的按应用设置影响"
    );
    assert!(!key_with(&mut host, 2, 'a' as u32, 0).handled);
    configure(&mut host, |c| c.font_size = 22);
    assert_eq!(
        host.handle(2, Request::Status).status.unwrap().mode,
        Mode::English
    );

    let dir = host.data_dir();
    drop(host);
    let mut restarted = Host::new(Store::new(dir));
    restarted.connect(3, "new.exe".into());
    assert_eq!(
        restarted.handle(3, Request::Status).status.unwrap().mode,
        Mode::English
    );
    let frame = restarted
        .handle(3, Request::ToggleMode { shift: None })
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::Chinese);
    let dir = restarted.data_dir();
    drop(restarted);
    let mut restarted = Host::new(Store::new(dir));
    assert_eq!(
        restarted.handle(3, Request::Status).status.unwrap().mode,
        Mode::Chinese
    );
    cleanup(&restarted);
}

#[test]
fn selecting_global_default_keeps_the_active_apps_current_mode() {
    let mut host = host("enable-global");
    configure(&mut host, |c| c.mode_scope = "app".into());
    host.handle(1, Request::ToggleMode { shift: None });
    host.handle(
        1,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    let mut json = serde_json::to_value(host.config()).unwrap();
    json["default_mode"] = serde_json::json!("global");
    let config = serde_json::from_value::<Config>(json);
    assert!(config.is_ok(), "默认状态必须接受全局选项");
    assert!(
        host.handle(
            1,
            Request::SetConfig {
                config: config.unwrap()
            }
        )
        .ok
    );
    assert_eq!(
        host.handle(1, Request::Status).status.unwrap().mode,
        Mode::English
    );
    let dir = host.data_dir();
    drop(host);
    let mut restarted = Host::new(Store::new(dir));
    assert_eq!(
        restarted.handle(1, Request::Status).status.unwrap().mode,
        Mode::English
    );
    cleanup(&restarted);
}

#[test]
fn selecting_global_in_settings_after_focus_leaves_keeps_last_manual_mode() {
    let mut host = host("global-after-blur");
    configure(&mut host, |c| c.mode_scope = "app".into());
    host.handle(
        1,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    host.handle(1, Request::ToggleMode { shift: None });
    // 打开设置窗口会让原来的文本框失焦，设置连接本身没有输入会话。
    host.handle(
        1,
        Request::Focus {
            active: false,
            before: None,
        },
    );
    let mut config = host.config().clone();
    config.default_mode = DefaultMode::Global;
    assert!(host.handle(90, Request::SetConfig { config }).ok);
    assert_eq!(
        host.handle(90, Request::Status).status.unwrap().mode,
        Mode::English,
        "从设置启用全局不能因原应用已失焦而丢掉最后手动选择的英文"
    );
    cleanup(&host);
}

#[test]
fn global_mode_toggle_invalidates_other_clients_key_predictions() {
    let mut host = host("global-prediction");
    host.connect(2, "browser.exe".into());
    let key = Key {
        keysym: 'n' as u32,
        test: true,
        ..Key::default()
    };
    assert!(
        host.handle(1, Request::Key { key, before: None })
            .frame
            .unwrap()
            .handled
    );
    host.handle(2, Request::ToggleMode { shift: None });
    let frame = press(&mut host, 'n' as u32);
    assert_eq!(frame.mode, Mode::English);
    assert!(
        !frame.handled && frame.preedit.is_empty(),
        "切换全局状态后不能接纳旧中文状态的按键预判"
    );
    cleanup(&host);
}

#[test]
fn caps_lock_letters_go_to_app() {
    let mut host = host("caps");
    assert!(!key_with(&mut host, 1, 'A' as u32, modifier::CAPS).handled);
    cleanup(&host);
}

#[test]
fn caps_lock_types_english_and_restores_the_remembered_mode() {
    let mut host = host("caps-english");
    let on = host
        .handle(1, Request::CapsLock { on: true })
        .frame
        .unwrap();
    assert_eq!(on.mode, Mode::English, "大写锁定开着：显示和输入都是英文");
    assert_eq!(on.hint.as_deref(), Some("A"));
    assert_eq!(
        host.handle(1, Request::Status).status.unwrap().mode,
        Mode::English
    );
    for sym in [',' as u32, '1' as u32, 'A' as u32] {
        let frame = key_with(&mut host, 1, sym, modifier::CAPS);
        assert!(
            !frame.handled && frame.commit.is_none(),
            "标点、数字、字母都原样交给应用"
        );
    }
    let off = host
        .handle(1, Request::CapsLock { on: false })
        .frame
        .unwrap();
    assert_eq!(off.mode, Mode::Chinese, "关掉后回到原来的中文");
    assert_eq!(off.hint.as_deref(), Some("中"));
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some("，"));
    // 重复报同一个状态（输入框得到焦点时也报）不提示
    assert_eq!(
        host.handle(1, Request::CapsLock { on: false })
            .frame
            .unwrap()
            .hint,
        None
    );
    cleanup(&host);
}

#[test]
fn caps_lock_commits_half_typed_letters() {
    let mut host = host("caps-composing");
    assert_eq!(typing(&mut host, "ni").preedit, "ni");
    let frame = host
        .handle(1, Request::CapsLock { on: true })
        .frame
        .unwrap();
    assert_eq!(
        frame.commit.as_deref(),
        Some("ni"),
        "打到一半的字母原样上屏"
    );
    assert!(frame.preedit.is_empty() && frame.candidates.is_empty());
    cleanup(&host);
}

#[test]
fn caps_lock_state_follows_key_modifiers_without_a_notification() {
    // 在没有 FreeType 的窗口里开关了大写锁定：服务进程从下一个键带的状态里跟上
    let mut host = host("caps-keys");
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    host.set_desktop(Box::new(Recorder(log.clone())));
    assert!(!key_with(&mut host, 1, ',' as u32, modifier::CAPS).handled);
    assert_eq!(
        log.borrow().last().unwrap().mode,
        Mode::English,
        "状态栏跟着变"
    );
    assert_eq!(
        press(&mut host, ',' as u32).commit.as_deref(),
        Some("，"),
        "下一个键没带大写锁定：回到中文"
    );
    assert_eq!(log.borrow().last().unwrap().mode, Mode::Chinese);
    cleanup(&host);
}

#[test]
fn caps_lock_keeps_the_remembered_mode_from_being_toggled() {
    let mut host = host("caps-toggle");
    host.handle(1, Request::CapsLock { on: true });
    let frame = host
        .handle(
            1,
            Request::ToggleMode {
                shift: Some("left".into()),
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint.as_deref(), Some("A"), "提示还在大写锁定");
    host.handle(1, Request::CapsLock { on: false });
    assert!(press(&mut host, 'n' as u32).handled, "记住的中文没有被切走");
    cleanup(&host);
}

#[test]
fn app_preset_applies_once_when_switching_into_the_app() {
    let mut host = host("preset");
    configure(&mut host, |c| {
        c.app_presets = true;
        // 规则里的大小写不影响匹配
        c.app_preset_rules = vec![
            preset("Code.EXE", Mode::English),
            preset("wechat.exe", Mode::Chinese),
        ];
    });
    host.foreground("notepad.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "没有规则的程序不动");
    host.connect(2, "code.exe".into());
    host.foreground("code.exe");
    assert_eq!(
        status_mode(&mut host),
        Mode::English,
        "切进规则里的程序：切一次"
    );
    // 在这个程序里手动切回中文，再换它自己的窗口（查找框、对话框）：不重置
    host.handle(2, Request::ToggleMode { shift: None });
    host.foreground("code.exe");
    assert_eq!(
        status_mode(&mut host),
        Mode::Chinese,
        "同一个程序里不再干预"
    );
    host.foreground("explorer.exe");
    assert_eq!(
        status_mode(&mut host),
        Mode::Chinese,
        "没有规则的程序恢复原状态"
    );
    host.foreground("code.exe");
    assert_eq!(
        status_mode(&mut host),
        Mode::English,
        "离开后再切回来：重新套用一次"
    );
    host.foreground("wechat.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese, "规则也可以指定中文");
    cleanup(&host);
}

#[test]
fn app_preset_does_nothing_when_switched_off_or_already_in_that_mode() {
    let mut host = host("preset-off");
    configure(&mut host, |c| {
        c.app_preset_rules = vec![preset("code.exe", Mode::English)]
    });
    host.foreground("code.exe");
    assert_eq!(
        status_mode(&mut host),
        Mode::Chinese,
        "总开关关着：规则不生效"
    );
    // 打开开关只管之后的切入，不追溯现在所在的程序
    configure(&mut host, |c| c.app_presets = true);
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    host.foreground("notepad.exe");
    host.handle(1, Request::ToggleMode { shift: None });
    host.foreground("code.exe");
    let frame = host
        .handle(
            1,
            Request::Focus {
                active: true,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(frame.hint, None, "已经是英文：不切换，也不提示");
    cleanup(&host);
}

#[test]
fn app_preset_is_applied_by_focus_and_hints_once() {
    let mut host = host("preset-focus");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    let foreground = std::rc::Rc::new(std::cell::RefCell::new(None));
    host.set_desktop(Box::new(Foreground(foreground.clone())));
    host.handle(
        2,
        Request::Hello {
            session: true,
            app: "Code.exe".into(),
        },
    );
    // 前台事件还没到，输入框先得到了焦点：焦点这一步自己查前台程序
    *foreground.borrow_mut() = Some("code.exe".to_string());
    let frame = host
        .handle(
            2,
            Request::Focus {
                active: true,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::English);
    assert_eq!(
        frame.hint.as_deref(),
        Some("英"),
        "预设切换了状态：提示一次"
    );
    let frame = host
        .handle(
            2,
            Request::Focus {
                active: true,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.hint, None, "同一个程序里换输入框：不再提示");
    assert!(!key_with(&mut host, 2, 'a' as u32, 0).handled);
    // FreeType 自己的窗口（桌面报 None）不算离开
    *foreground.borrow_mut() = None;
    host.handle(2, Request::ToggleMode { shift: None });
    let frame = host
        .handle(
            2,
            Request::Focus {
                active: true,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.mode, Mode::Chinese, "手动切换的结果留着");
    let apps = host.handle(9, Request::ListApps).apps.unwrap();
    assert_eq!(apps[0].app, "code.exe");
    cleanup(&host);
}

#[test]
fn app_preset_sets_the_apps_own_mode_when_modes_are_per_app() {
    let mut host = host("preset-app-scope");
    configure(&mut host, |c| {
        c.mode_scope = "app".into();
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.handle(
        2,
        Request::Hello {
            session: true,
            app: "code.exe".into(),
        },
    );
    host.foreground("code.exe");
    assert!(
        !key_with(&mut host, 2, 'a' as u32, 0).handled,
        "规则里的程序切成英文"
    );
    assert!(
        press(&mut host, 'n' as u32).handled,
        "记事本各记各的，还是中文"
    );
    cleanup(&host);
}

#[test]
fn app_preset_restores_normal_mode_after_leaving() {
    for (name, default_mode, scope) in [
        ("shared", DefaultMode::Chinese, "global"),
        ("remembered", DefaultMode::Global, "app"),
        ("per-app", DefaultMode::Chinese, "app"),
    ] {
        let mut host = host(&format!("preset-restore-{name}"));
        configure(&mut host, |c| {
            c.default_mode = default_mode;
            c.mode_scope = scope.into();
            c.app_presets = true;
            c.app_preset_rules = vec![preset("code.exe", Mode::English)];
        });
        host.connect(2, "code.exe".into());
        host.foreground("notepad.exe");
        host.handle(
            1,
            Request::Focus {
                active: true,
                before: None,
            },
        );
        for _ in 0..3 {
            host.foreground("code.exe");
            assert_eq!(
                host.handle(
                    2,
                    Request::Focus {
                        active: true,
                        before: None
                    }
                )
                .frame
                .unwrap()
                .mode,
                Mode::English
            );
            assert!(!key_with(&mut host, 2, 'n' as u32, 0).handled);
            host.foreground("notepad.exe");
            // 前台通知先于旧输入框失焦到达时，状态也必须已经恢复。
            assert_eq!(
                status_mode(&mut host),
                Mode::Chinese,
                "{name}: 规则不能污染原状态"
            );
            host.handle(
                1,
                Request::Focus {
                    active: true,
                    before: None,
                },
            );
            host.handle(
                2,
                Request::Focus {
                    active: false,
                    before: None,
                },
            );
            assert!(press(&mut host, 'n' as u32).handled);
            press(&mut host, keysym::ESCAPE);
        }
        cleanup(&host);
    }
}

#[test]
fn app_preset_same_mode_still_isolates_manual_changes_and_chained_rules() {
    let mut host = host("preset-same-mode");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![
            preset("code.exe", Mode::English),
            preset("chat.exe", Mode::Chinese),
        ];
    });
    host.connect(2, "code.exe".into());
    host.connect(3, "chat.exe".into());
    host.foreground("notepad.exe");
    host.handle(1, Request::ToggleMode { shift: None });
    host.foreground("code.exe");
    assert_eq!(
        host.handle(2, Request::ToggleMode { shift: None })
            .frame
            .unwrap()
            .mode,
        Mode::Chinese
    );
    host.foreground("code.exe");
    assert_eq!(
        key_with(&mut host, 2, 'n' as u32, 0).mode,
        Mode::Chinese,
        "软件内部手动切换必须保留"
    );
    host.foreground("chat.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    host.foreground("notepad.exe");
    assert_eq!(
        status_mode(&mut host),
        Mode::English,
        "连续经过多条规则后仍恢复原英文"
    );
    assert!(!press(&mut host, 'n' as u32).handled);
    cleanup(&host);
}

#[test]
fn app_preset_does_not_persist_temporary_mode_when_saving_settings_or_restarting() {
    let mut host = host("preset-persistence");
    configure(&mut host, |c| {
        c.default_mode = DefaultMode::Global;
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    host.foreground("code.exe");
    host.handle(
        2,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    host.handle(2, Request::ToggleMode { shift: None });
    host.handle(2, Request::ToggleMode { shift: None });
    configure(&mut host, |c| c.font_size = 22);
    assert_eq!(
        status_mode(&mut host),
        Mode::English,
        "保存无关设置不能重置软件内状态"
    );
    let mut restarted = Host::new(Store::new(host.data_dir()));
    assert_eq!(
        status_mode(&mut restarted),
        Mode::Chinese,
        "临时规则及软件内切换不能写成全局记忆"
    );
    host.foreground("notepad.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    cleanup(&host);
}

#[test]
fn app_preset_rejects_delayed_foreground_and_background_focus_notifications() {
    let mut host = host("preset-stale-events");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    let foreground = std::rc::Rc::new(std::cell::RefCell::new(Some("code.exe".into())));
    host.set_desktop(Box::new(Foreground(foreground.clone())));
    host.handle(
        2,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    host.handle(2, Request::ToggleMode { shift: None });
    // 输入框已查询到 Code 是前台，旧的 Notepad 通知才从另一个队列送达。
    host.foreground("notepad.exe");
    host.foreground("code.exe");
    assert_eq!(
        status_mode(&mut host),
        Mode::Chinese,
        "延迟通知不能重新套用英文规则"
    );
    *foreground.borrow_mut() = Some("notepad.exe".into());
    host.handle(
        1,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    host.handle(
        2,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    host.foreground("code.exe");
    assert_eq!(status_mode(&mut host), Mode::Chinese);
    // 状态栏按钮必须操作当前软件，不能被后台输入框抢走。
    host.handle(99, Request::ToggleMode { shift: None });
    assert!(!press(&mut host, 'n' as u32).handled);
    cleanup(&host);
}

#[test]
fn app_preset_disabling_or_removing_rule_restores_base_mode() {
    for remove in [false, true] {
        let mut host = host(&format!("preset-disable-{remove}"));
        configure(&mut host, |c| {
            c.default_mode = DefaultMode::Global;
            c.app_presets = true;
            c.app_preset_rules = vec![preset("code.exe", Mode::English)];
        });
        host.connect(2, "code.exe".into());
        host.foreground("code.exe");
        host.handle(
            2,
            Request::Focus {
                active: true,
                before: None,
            },
        );
        configure(&mut host, |c| {
            if remove {
                c.app_preset_rules.clear();
            } else {
                c.app_presets = false;
            }
        });
        assert_eq!(
            status_mode(&mut host),
            Mode::Chinese,
            "关闭或移除规则后恢复原状态"
        );
        cleanup(&host);
    }
}

#[test]
fn app_preset_first_key_observes_foreground_before_delayed_events() {
    let mut host = host("preset-first-key");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    let foreground = std::rc::Rc::new(std::cell::RefCell::new(Some("code.exe".into())));
    host.set_desktop(Box::new(Foreground(foreground.clone())));
    assert!(
        !key_with(&mut host, 2, 'n' as u32, 0).handled,
        "首键先于焦点与桌面事件到达仍用预设"
    );
    host.handle(2, Request::ToggleMode { shift: None });
    let preview = Key {
        keysym: 'n' as u32,
        test: true,
        ..Key::default()
    };
    assert!(
        host.handle(
            2,
            Request::Key {
                key: preview,
                before: None
            }
        )
        .frame
        .unwrap()
        .handled
    );
    *foreground.borrow_mut() = Some("notepad.exe".into());
    host.handle(
        1,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    *foreground.borrow_mut() = Some("code.exe".into());
    let frame = key_with(&mut host, 2, 'n' as u32, 0);
    assert_eq!(frame.mode, Mode::English);
    assert!(
        !frame.handled && frame.preedit.is_empty(),
        "重入后旧中文按键预判必须失效"
    );
    cleanup(&host);
}

#[test]
fn app_preset_keeps_caps_lock_separate_from_restored_mode() {
    let mut host = host("preset-caps");
    configure(&mut host, |c| {
        c.app_presets = true;
        c.app_preset_rules = vec![preset("code.exe", Mode::English)];
    });
    host.connect(2, "code.exe".into());
    host.foreground("code.exe");
    host.handle(2, Request::CapsLock { on: true });
    host.foreground("notepad.exe");
    assert_eq!(status_mode(&mut host), Mode::English);
    host.handle(1, Request::CapsLock { on: false });
    assert_eq!(
        status_mode(&mut host),
        Mode::Chinese,
        "关闭大写锁定后恢复规则之前的中文"
    );
    cleanup(&host);
}

#[test]
fn hotkeys_toggle_punct_and_traditional_only_when_set() {
    let mut host = host("hotkeys");
    let frame = key_with(&mut host, 1, '.' as u32, modifier::CTRL);
    assert!(!frame.handled, "默认没设快捷键，交给应用");
    configure(&mut host, |c| {
        c.hotkey_punct = "ctrl+period".into();
        c.hotkey_traditional = "ctrl+shift+f".into();
    });
    let frame = key_with(&mut host, 1, '.' as u32, modifier::CTRL);
    assert!(frame.handled);
    assert_eq!(frame.hint.as_deref(), Some("英文标点"));
    assert!(!frame.chinese_punct);
    assert_eq!(press(&mut host, ',' as u32).commit.as_deref(), Some(","));
    let frame = key_with(&mut host, 1, 'F' as u32, modifier::CTRL | modifier::SHIFT);
    assert_eq!(frame.hint.as_deref(), Some("繁體"));
    assert!(
        host.handle(1, Request::GetConfig)
            .config
            .unwrap()
            .traditional,
        "存进了设置"
    );
    cleanup(&host);
}

#[test]
fn ctrl_space_from_system_toggle_switches_mode_only_when_set() {
    // TSF 在系统吃掉 Ctrl+空格 时补发的就是这个请求：没有上文、没有候选窗代次
    let mut host = host("ctrl-space");
    let ctrl_space = |host: &mut Host| {
        let key = Key {
            keysym: 0x20,
            modifiers: modifier::CTRL,
            test: false,
        };
        host.handle(
            1,
            Request::PhysicalKey {
                key,
                before: None,
                scan: 0,
                shown_revision: None,
            },
        )
        .frame
        .unwrap()
    };
    let start = host.handle(1, Request::Status).status.unwrap().mode;
    assert!(!ctrl_space(&mut host).handled, "默认没设快捷键，不切换");
    assert_eq!(host.handle(1, Request::Status).status.unwrap().mode, start);
    configure(&mut host, |c| c.hotkey_mode = "ctrl+space".into());
    let first = ctrl_space(&mut host);
    assert!(first.handled);
    assert_ne!(first.mode, start);
    assert_eq!(ctrl_space(&mut host).mode, start, "再按一次切回来");
    cleanup(&host);
}

#[test]
fn focus_reports_mode_hint_when_enabled() {
    let mut host = host("focus");
    let frame = host
        .handle(
            1,
            Request::Focus {
                active: true,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.hint, None);
    configure(&mut host, |c| c.hint_on_focus = true);
    let frame = host
        .handle(
            1,
            Request::Focus {
                active: true,
                before: None,
            },
        )
        .frame
        .unwrap();
    assert_eq!(frame.hint.as_deref(), Some("中"));
    cleanup(&host);
}

#[test]
fn status_bar_follows_focus_and_mode() {
    let mut host = host("bar");
    configure(&mut host, |c| c.status_bar = true);
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    host.set_desktop(Box::new(Recorder(log.clone())));
    assert!(!log.borrow().last().unwrap().visible, "没有输入框时不显示");
    host.handle(
        1,
        Request::Focus {
            active: true,
            before: None,
        },
    );
    assert!(log.borrow().last().unwrap().visible);
    host.handle(1, Request::ToggleMode { shift: None });
    assert_eq!(log.borrow().last().unwrap().mode, Mode::English);
    host.handle(
        1,
        Request::Focus {
            active: false,
            before: None,
        },
    );
    assert!(!log.borrow().last().unwrap().visible);
    cleanup(&host);
}
