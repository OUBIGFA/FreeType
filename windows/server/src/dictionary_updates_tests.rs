use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_temp/dictionary-update-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 写一份更新包：文件内容任意，清单记下真实大小和摘要。
fn package(root: &Path, version: &str, lexicon: &[u8]) -> PathBuf {
    let dir = root.join(version);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lexicon.ftd"), lexicon).unwrap();
    std::fs::write(dir.join("english.txt"), "github\n").unwrap();
    write_manifest(&dir, version, "FTD2");
    dir
}

fn write_manifest(dir: &Path, version: &str, format: &str) {
    let files: serde_json::Map<String, serde_json::Value> = FILES
        .iter()
        .map(|name| {
            let path = dir.join(name);
            (
                name.to_string(),
                serde_json::json!({
                    "bytes": std::fs::metadata(&path).unwrap().len(),
                    "sha256": file_sha256(&path).unwrap(),
                }),
            )
        })
        .collect();
    let manifest = serde_json::json!({
        "format": 1, "channel": "freetype-data", "version": version,
        "lexiconFormat": format, "files": files,
    });
    std::fs::write(dir.join(MANIFEST), manifest.to_string()).unwrap();
}

fn set_state(root: &Path, state: State) {
    write_state(root, &state).unwrap();
}

/// 载入函数的替身：记下尝试的目录，`bad` 中的目录载入失败。
fn loader<'a>(
    tried: &'a std::cell::RefCell<Vec<PathBuf>>,
    bad: &'a [PathBuf],
) -> impl FnMut(&Path) -> Result<PathBuf, String> + 'a {
    move |dir| {
        tried.borrow_mut().push(dir.to_path_buf());
        if bad.iter().any(|b| b == dir) {
            Err("载入失败".into())
        } else {
            Ok(dir.to_path_buf())
        }
    }
}

#[test]
fn versions_sort_by_date_and_reject_paths() {
    assert!(parse_version("2026.11.29.2") > parse_version("2026.11.29.1"));
    assert!(parse_version("2026.12.01.1") > parse_version("2026.11.30.9"));
    assert_eq!(parse_version("2026-11-14.1"), Some((2026, 11, 14, 1)));
    for bad in [
        "..",
        "2026.11.29",
        "2026.11.29.1/..",
        "bundled",
        "2026.11.29.x",
    ] {
        assert!(!valid_version(bad), "{bad}");
    }
    assert!(
        !valid_version("2026-11-14.1"),
        "安装包的数据版本不是更新包目录名"
    );
}

#[test]
fn pending_package_becomes_current_and_keeps_the_old_one_as_previous() {
    let root = scratch("activate");
    let bundled = scratch("activate-bundled");
    let new = package(&root, "2026.11.29.1", b"new");
    package(&root, "2026.11.22.1", b"old");
    set_state(
        &root,
        State {
            current: "2026.11.22.1".into(),
            pending: "2026.11.29.1".into(),
            pending_automatic: true,
            ..State::default()
        },
    );
    let tried = Default::default();
    assert_eq!(activate(&root, &bundled, loader(&tried, &[])).unwrap(), new);
    let state = read_state(&root);
    assert_eq!(
        state,
        State {
            current: "2026.11.29.1".into(),
            previous: "2026.11.22.1".into(),
            ..State::default()
        }
    );
}

#[test]
fn a_damaged_pending_package_is_skipped_and_reported() {
    let root = scratch("damaged");
    let bundled = scratch("damaged-bundled");
    let current = package(&root, "2026.11.22.1", b"old");
    let pending = package(&root, "2026.11.29.1", b"new");
    // 大小不变、内容被改：只有逐个核对摘要才能发现。
    std::fs::write(pending.join("lexicon.ftd"), b"bad").unwrap();
    set_state(
        &root,
        State {
            current: "2026.11.22.1".into(),
            pending: "2026.11.29.1".into(),
            ..State::default()
        },
    );
    let tried = Default::default();
    assert_eq!(
        activate(&root, &bundled, loader(&tried, &[])).unwrap(),
        current
    );
    assert_eq!(
        tried.borrow().as_slice(),
        [current.clone()],
        "损坏的包不进入载入"
    );
    let state = read_state(&root);
    assert_eq!(state.current, "2026.11.22.1");
    assert!(state.pending.is_empty());
    assert_eq!(state.failed_version, "2026.11.29.1");
    assert!(
        state.failed_reason.contains("校验失败"),
        "{}",
        state.failed_reason
    );
    assert!(!pending.exists(), "未被引用的失败包启动后清理");
}

#[test]
fn a_current_package_that_stops_loading_falls_back_to_the_previous_one() {
    let root = scratch("fallback");
    let bundled = scratch("fallback-bundled");
    let current = package(&root, "2026.11.29.1", b"new");
    let previous = package(&root, "2026.11.22.1", b"old");
    set_state(
        &root,
        State {
            current: "2026.11.29.1".into(),
            previous: "2026.11.22.1".into(),
            ..State::default()
        },
    );
    let tried = Default::default();
    let bad = [current];
    assert_eq!(
        activate(&root, &bundled, loader(&tried, &bad)).unwrap(),
        previous
    );
    let state = read_state(&root);
    assert_eq!(state.current, "2026.11.22.1");
    assert!(state.previous.is_empty());
    assert_eq!(state.failed_version, "2026.11.29.1");
}

#[test]
fn packages_that_all_fail_fall_back_to_the_bundled_dictionary() {
    let root = scratch("bundled");
    let bundled = scratch("bundled-dir");
    let current = package(&root, "2026.11.29.1", b"new");
    set_state(
        &root,
        State {
            current: "2026.11.29.1".into(),
            ..State::default()
        },
    );
    let tried = Default::default();
    let bad = [current];
    assert_eq!(
        activate(&root, &bundled, loader(&tried, &bad)).unwrap(),
        bundled
    );
    let state = read_state(&root);
    assert!(state.current.is_empty());
    assert_eq!(state.failed_version, "2026.11.29.1");
}

#[test]
fn a_failure_unrelated_to_packages_keeps_the_update_record() {
    let root = scratch("unrelated");
    let bundled = scratch("unrelated-bundled");
    let current = package(&root, "2026.11.29.1", b"new");
    let before = State {
        current: "2026.11.29.1".into(),
        ..State::default()
    };
    set_state(&root, before.clone());
    let tried = Default::default();
    let bad = [current, bundled.clone()];
    assert!(activate(&root, &bundled, loader(&tried, &bad)).is_err());
    assert_eq!(
        read_state(&root),
        before,
        "例如学习记录读不了时，不能冤枉更新包"
    );
}

#[test]
fn restoring_goes_back_one_step_and_then_to_the_bundled_dictionary() {
    let root = scratch("restore");
    let bundled = scratch("restore-bundled");
    package(&root, "2026.11.29.1", b"new");
    let old = package(&root, "2026.11.22.1", b"old");
    set_state(
        &root,
        State {
            current: "2026.11.29.1".into(),
            previous: "2026.11.22.1".into(),
            ..State::default()
        },
    );
    restore_previous(&root).unwrap();
    assert_eq!(read_state(&root).pending, "2026.11.22.1");
    let tried = Default::default();
    assert_eq!(activate(&root, &bundled, loader(&tried, &[])).unwrap(), old);
    assert_eq!(read_state(&root).previous, "2026.11.29.1");

    set_state(
        &root,
        State {
            current: "2026.11.22.1".into(),
            ..State::default()
        },
    );
    restore_previous(&root).unwrap();
    assert_eq!(read_state(&root).pending, BUNDLED);
    let tried = Default::default();
    assert_eq!(
        activate(&root, &bundled, loader(&tried, &[])).unwrap(),
        bundled
    );
    let state = read_state(&root);
    assert!(state.current.is_empty());
    assert_eq!(state.previous, "2026.11.22.1");
    assert!(restore_previous(&scratch("restore-none")).is_err());
}

#[test]
fn staging_checks_the_package_and_respects_failed_automatic_versions() {
    let root = scratch("stage");
    assert!(stage(&root, "../x", false).is_err());
    assert!(stage(&root, "2026.11.29.1", false).is_err(), "目录不存在");
    let dir = package(&root, "2026.11.29.1", b"new");
    std::fs::write(dir.join("english.txt"), "changed size\n").unwrap();
    assert!(stage(&root, "2026.11.29.1", false)
        .unwrap_err()
        .contains("大小"));
    write_manifest(&dir, "2026.11.29.1", "FTD9");
    assert!(stage(&root, "2026.11.29.1", false)
        .unwrap_err()
        .contains("需要更新 FreeType"));
    write_manifest(&dir, "2026.11.29.1", "FTD2");
    set_state(
        &root,
        State {
            failed_version: "2026.11.29.1".into(),
            ..State::default()
        },
    );
    assert!(stage(&root, "2026.11.29.1", true).is_err());
    stage(&root, "2026.11.29.1", false).unwrap();
    let state = read_state(&root);
    assert_eq!(state.pending, "2026.11.29.1");
    assert!(!state.pending_automatic);
}

#[test]
fn disabling_automatic_updates_cancels_only_automatic_staging() {
    let root = scratch("cancel");
    set_state(
        &root,
        State {
            pending: "2026.11.29.1".into(),
            ..State::default()
        },
    );
    cancel_pending(&root, true).unwrap();
    assert_eq!(read_state(&root).pending, "2026.11.29.1", "手动安排保留");
    set_state(
        &root,
        State {
            pending: "2026.11.29.1".into(),
            pending_automatic: true,
            ..State::default()
        },
    );
    cancel_pending(&root, true).unwrap();
    assert!(read_state(&root).pending.is_empty());
}

#[test]
fn cleanup_keeps_referenced_versions_and_downloads_in_progress() {
    let root = scratch("cleanup");
    let bundled = scratch("cleanup-bundled");
    package(&root, "2026.11.29.1", b"new");
    let stale = package(&root, "2026.11.15.1", b"stale");
    let partial = root.join("2026.12.06.1.partial");
    std::fs::create_dir_all(&partial).unwrap();
    set_state(
        &root,
        State {
            current: "2026.11.29.1".into(),
            ..State::default()
        },
    );
    let tried = Default::default();
    activate(&root, &bundled, loader(&tried, &[])).unwrap();
    assert!(!stale.exists());
    assert!(partial.exists());
    assert!(root.join("2026.11.29.1").exists());
}

/// 真实编译的小词库走完整的发布前检查。
#[test]
fn built_package_check_loads_the_engine_and_verifies_representative_candidates() {
    let root = scratch("built");
    let dir = root.join("2026.11.29.1");
    std::fs::create_dir_all(&dir).unwrap();
    let tsv = root.join("lexicon.tsv");
    std::fs::write(
        &tsv,
        "ni hao\t你好\t9000\nzhong guo\t中国\t9000\nshu ru fa\t输入法\t9000\nwo men\t我们\t9000\n",
    )
    .unwrap();
    wowtype_core::lexicon::MappedLexicon::compile(&tsv, &dir.join("lexicon.ftd")).unwrap();
    std::fs::write(dir.join("english.txt"), "github\n").unwrap();
    write_manifest(&dir, "2026.11.29.1", "FTD2");
    assert_eq!(check_built_package(&dir).unwrap(), "2026.11.29.1");
    std::fs::write(&tsv, "ni hao\t你好\t9000\n").unwrap();
    wowtype_core::lexicon::MappedLexicon::compile(&tsv, &dir.join("lexicon.ftd")).unwrap();
    write_manifest(&dir, "2026.11.29.1", "FTD2");
    assert!(check_built_package(&dir).unwrap_err().contains("中国"));
}
