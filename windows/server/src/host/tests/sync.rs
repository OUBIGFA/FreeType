//! 同步快照、后台合并与同步记录回放。
use super::*;

#[test]
fn sync_work_runs_beside_typing_and_merges_apply_only_if_local_data_is_unchanged() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let mut a = host("sync-beside-a");
    let mut b = host("sync-beside-b");
    let (af, bf) = (a.data_dir().join("a.json"), b.data_dir().join("b.json"));
    let path = |path: &std::path::Path| path.display().to_string();
    assert!(
        a.handle(
            1,
            Request::SetCommonPhrases {
                phrases: vec!["来自另一台".into()]
            }
        )
        .ok
    );
    assert!(
        a.handle(1, Request::ExportSyncSettings { path: path(&af) })
            .ok
    );
    let (apply_tx, apply_rx) = channel();
    b.set_apply_channel(apply_tx);

    // 占住后台线程，相当于开机时慢到数秒的同步计算：按键照常出候选，同步应答在后面排队。
    let (release, blocked) = channel::<()>();
    b.tasks.submit(
        Box::new(move || {
            let _ = blocked.recv();
            Step::Done
        }),
        None,
    );
    let (reply, export) = channel();
    b.dispatch(1, Request::ExportSyncSettings { path: path(&bf) }, reply);
    assert_eq!(
        typing(&mut b, "nihao")
            .candidates
            .first()
            .map(String::as_str),
        Some("你好")
    );
    assert!(export.try_recv().is_err(), "同步导出不应在引擎线程里完成");
    release.send(()).unwrap();
    assert!(export.recv_timeout(Duration::from_secs(10)).unwrap().ok);
    assert!(b.handle(1, Request::Reset { commit: false }).ok);

    // 合并在后台比较，回到引擎线程应用后才应答。
    let (reply, merged) = channel();
    b.dispatch(
        1,
        Request::MergeSyncSettings {
            path: path(&af),
            expected_path: path(&bf),
        },
        reply,
    );
    let apply = apply_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(merged.try_recv().is_err());
    apply(&mut b);
    let response = merged.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(
        b.handle(1, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["来自另一台"]
    );

    // 后台比较期间本机又改了：放弃这次合并，保留刚做的修改，由同步重新比较。
    assert!(
        a.handle(
            1,
            Request::SetCommonPhrases {
                phrases: vec!["另一台又改了".into()]
            }
        )
        .ok
    );
    assert!(
        a.handle(1, Request::ExportSyncSettings { path: path(&af) })
            .ok
    );
    let (reply, export) = channel();
    b.dispatch(1, Request::ExportSyncSettings { path: path(&bf) }, reply);
    assert!(export.recv_timeout(Duration::from_secs(10)).unwrap().ok);
    let (reply, merged) = channel();
    b.dispatch(
        1,
        Request::MergeSyncSettings {
            path: path(&af),
            expected_path: path(&bf),
        },
        reply,
    );
    let apply = apply_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        b.handle(
            1,
            Request::SetCommonPhrases {
                phrases: vec!["本机刚改的".into()]
            }
        )
        .ok
    );
    apply(&mut b);
    let response = merged.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!response.ok);
    assert_eq!(
        b.handle(1, Request::ListCommonPhrases)
            .common_phrases
            .unwrap(),
        ["本机刚改的"]
    );

    // 手动重置同步记录排在后台登记之后，回到引擎线程执行，不会互相等待。
    let (reply, reset) = channel();
    b.dispatch(1, Request::ResetSyncRecords, reply);
    apply_rx.recv_timeout(Duration::from_secs(10)).unwrap()(&mut b);
    assert!(reset.recv_timeout(Duration::from_secs(10)).unwrap().ok);
    cleanup(&a);
    cleanup(&b);
}

#[test]
fn merge_settings_adds_remote_data_without_dropping_local_data() {
    let mut source = learning_host("merge-source");
    configure(&mut source, |c| {
        c.page_size = 9;
        c.predict_next = true;
    });
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
    assert!(source.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["远端常用语", "两边都有"]}))).ok);
    std::fs::create_dir_all(source.data_dir().join("dicts")).unwrap();
    std::fs::write(
        source.data_dir().join("dicts/人名.txt"),
        "阿特\ta te\t800\n",
    )
    .unwrap();
    std::fs::write(
        source.data_dir().join("dicts/共有.txt"),
        "远端\tyuan duan\t800\n",
    )
    .unwrap();
    source.recent_emoji = vec!["😊".into()];
    for _ in 0..3 {
        source
            .engine
            .as_ref()
            .unwrap()
            .record_selection_with_pinyin("阿特", Some("a te"));
    }
    let snapshot = source.data_dir().join("snapshot.json");
    assert!(
        source
            .handle(1, learning_request("export_settings", &snapshot))
            .ok
    );

    let mut target = learning_host("merge-target");
    configure(&mut target, |c| {
        c.font_size = 24;
        c.horizontal = false;
    });
    target.mode = Mode::English;
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
    assert!(target.handle(1, settings_request(serde_json::json!({"op": "set_common_phrases", "phrases": ["本机常用语", "两边都有"]}))).ok);
    std::fs::create_dir_all(target.data_dir().join("dicts")).unwrap();
    std::fs::write(
        target.data_dir().join("dicts/共有.txt"),
        "本机\tben ji\t100\n",
    )
    .unwrap();
    target.recent_emoji = vec!["🎉".into()];
    target
        .engine
        .as_ref()
        .unwrap()
        .record_selection_with_pinyin("旧词", Some("jiu ci"));

    let result = target.handle(1, learning_request("merge_settings", &snapshot));
    assert!(result.ok, "{result:?}");
    // 设置：采纳远端改过的项，本机改过而远端没改的留着
    assert_eq!(target.config.page_size, 9);
    assert!(target.config.predict_next);
    assert_eq!(target.config.font_size, 24);
    assert!(!target.config.horizontal);
    assert_eq!(target.mode, Mode::English, "中英文状态是各设备自己的");
    let codes: Vec<String> = target
        .store
        .list_quick_phrases()
        .into_iter()
        .map(|p| p.code)
        .collect();
    assert_eq!(codes, ["dz", "yx"], "列表按编码排");
    assert_eq!(
        target.store.load_common_phrases().unwrap(),
        ["本机常用语", "两边都有", "远端常用语"]
    );
    let mut names: Vec<String> = target
        .store
        .list_dictionaries()
        .into_iter()
        .map(|d| d.name)
        .collect();
    names.sort();
    assert_eq!(names, ["人名", "共有"]);
    assert_eq!(
        std::fs::read_to_string(target.data_dir().join("dicts/共有.txt")).unwrap(),
        "本机\tben ji\t100\n",
        "同名词库保留本机的"
    );
    assert_eq!(target.recent_emoji, ["🎉", "😊"]);
    let engine = target.engine.as_ref().unwrap();
    assert_eq!(
        (engine.history_count("阿特"), engine.history_count("旧词")),
        (3, 1)
    );
    assert!(
        engine.input("ate").iter().any(|c| c.text == "阿特"),
        "并进来的词马上能用"
    );
    // 再合并一次不重复、不累加
    let again = target.handle(1, learning_request("merge_settings", &snapshot));
    assert!(again.ok);
    assert_eq!(target.store.list_quick_phrases().len(), 2);
    assert_eq!(target.store.load_common_phrases().unwrap().len(), 3);
    assert_eq!(target.engine.as_ref().unwrap().history_count("阿特"), 3);
    // 坏文件不动数据
    let bad = target.data_dir().join("bad.json");
    std::fs::write(&bad, "{}").unwrap();
    assert!(
        !target
            .handle(1, learning_request("merge_settings", &bad))
            .ok
    );
    assert_eq!(target.config.page_size, 9);
    cleanup(&source);
    cleanup(&target);
}

#[test]
fn boundary_and_pending_learning_sync_replay_does_not_multiply_or_revive_deletions() {
    let mut a = host("sentence-sync-a");
    let mut b = host("sentence-sync-b");
    a.engine.as_ref().unwrap().remember_boundary(
        "ni hao shi jie",
        vec![
            ("你".into(), "ni".into()),
            ("好世界".into(), "hao shi jie".into()),
        ],
    );
    a.engine
        .as_ref()
        .unwrap()
        .record_merged_word("你好世界", "ni hao shi jie");
    let file = a.data_dir().join("boundary-sync.json");
    let path = file.display().to_string();
    assert!(
        a.handle(1, Request::ExportSyncSettings { path: path.clone() })
            .ok
    );
    assert!(
        b.handle(1, Request::MergeSettings { path: path.clone() })
            .ok
    );
    assert!(
        b.handle(1, Request::MergeSettings { path: path.clone() })
            .ok
    );
    assert_eq!(b.engine.as_ref().unwrap().sentence_boundaries().len(), 1);
    assert!(!b
        .engine
        .as_ref()
        .unwrap()
        .user_words()
        .iter()
        .any(|(_, w, _)| w == "你好世界"));
    b.engine
        .as_ref()
        .unwrap()
        .record_merged_word("你好世界", "ni hao shi jie");
    assert!(b
        .engine
        .as_ref()
        .unwrap()
        .user_words()
        .iter()
        .any(|(_, w, _)| w == "你好世界"));
    cleanup(&a);
    cleanup(&b);
}
