//! 后台载入引擎：默认词库、更新包词库、自定义主词库、模型与学习记录。
use super::*;

pub(super) fn default_lexicon(
    resources: &Path,
) -> Result<(Arc<dyn wowtype_core::Lexicon>, PathBuf), String> {
    let index = resources.join("lexicon.ftd");
    if index
        .try_exists()
        .map_err(|error| format!("无法检查默认词库索引（{}）：{error}", index.display()))?
    {
        let record = crate::local_dir().join("cache").join("lexicon.verified");
        let mapped = wowtype_core::lexicon::MappedLexicon::open_deployed(&index, &record)
            .map_err(|error| format!("默认词库索引载入失败（{}）：{error:#}", index.display()))?;
        Ok((Arc::new(mapped), index))
    } else {
        let path = resources.join("lexicon.tsv");
        let lexicon = wowtype_core::MemoryLexicon::from_tsv_path_cached(&path)
            .map_err(|error| format!("默认词库载入失败（{}）：{error:#}", path.display()))?;
        Ok((Arc::new(lexicon), path))
    }
}

/// 载入引擎（慢：几秒），在后台线程里调。`lexicon` 是设置里的词库路径（空串用默认的）。
pub fn load_engine(
    resources: &Path,
    store_dir: &Path,
    configured: &str,
) -> Result<LoadedEngine, String> {
    load_engine_from(resources, resources, store_dir, configured)
}

/// 同 [`load_engine`]，但默认主词库和英文词表从 `dictionary` 读取（词库更新包）；
/// 模型、简繁映射和查词数据仍从随安装包的 `resources` 读取。
pub fn load_engine_from(
    resources: &Path,
    dictionary: &Path,
    store_dir: &Path,
    configured: &str,
) -> Result<LoadedEngine, String> {
    // 开机时各项读盘互相争抢，分段用时写进日志，便于判断哪一步慢。
    let mut stages = Vec::new();
    let mut started = std::time::Instant::now();
    let mut stage = |name: &str| {
        stages.push(format!("{name} {} ms", started.elapsed().as_millis()));
        started = std::time::Instant::now();
    };
    let custom = Path::new(configured);
    let mut warning = None;
    let mut invalid_custom_format = false;
    let (lexicon, path): (Arc<dyn wowtype_core::Lexicon>, PathBuf) = if configured.is_empty()
        || custom == resources.join("lexicon.tsv")
        || custom == resources.join("lexicon.ftd")
        || custom == dictionary.join("lexicon.ftd")
    {
        default_lexicon(dictionary)?
    } else {
        match wowtype_core::MemoryLexicon::from_tsv_path_cached(custom) {
            Ok(lexicon) => (Arc::new(lexicon), custom.to_owned()),
            Err(error) => {
                invalid_custom_format = error
                    .downcast_ref::<std::io::Error>()
                    .is_none_or(|cause| cause.kind() == std::io::ErrorKind::InvalidData);
                let reason = format!("自定义词库载入失败（{configured}）：{error:#}");
                let default =
                    default_lexicon(dictionary).map_err(|error| format!("{reason}；{error}"))?;
                warning = Some(format!("已使用默认词库；{reason}"));
                default
            }
        }
    };
    stage("词库");
    let lm: std::sync::Arc<dyn wowtype_core::LanguageModel> =
        match wowtype_core::lm::load_lm(resources.join("words.lm")) {
            Ok(lm) => lm,
            Err(_) => std::sync::Arc::new(wowtype_core::BigramLm::default()),
        };
    stage("语言模型");
    let engine = Engine::new(lexicon, lm);
    // 模糊音和默认开启的纠错兜底都使用此索引。必须在后台载入阶段建好，
    // 否则 ready 后的首次按键仍会遍历整份词库，可能超过 TSF 的 1.5 秒等待上限。
    engine.prepare_fuzzy();
    // 文本词库的联想词频表也要遍历全部词条。无论当前开关是否开启，都在
    // 后台准备，避免交接引擎或稍后开启联想时阻塞输入；编译词库直接复用索引。
    engine.prepare_prediction();
    let _ = engine.load_english(dictionary.join("english.txt"));
    let _ = engine.load_conversion(resources.join("s2t.tsv"));
    if resources.join("INPUT-TOOLS.json").is_file() {
        engine
            .load_input_tools(resources)
            .map_err(|error| format!("英文与拆字词典载入失败：{error:#}"))?;
    }
    stage("索引与字表");
    let store = Store::new(store_dir.to_path_buf());
    for (path, history) in [
        (store.user_dict_path(), false),
        (store.history_path(), true),
    ] {
        if !path.exists() {
            continue;
        }
        let loaded = if history {
            engine.load_history(&path)
        } else {
            engine.load_user_dict(&path)
        };
        if let Err(error) = loaded {
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() != std::io::ErrorKind::InvalidData)
            {
                return Err(format!("学习文件读取失败：{error:#}"));
            }
            let bad = path.with_extension(format!(
                "tsv.{}-{}.bad",
                std::process::id(),
                wowtype_core::now_ms()
            ));
            std::fs::rename(&path, &bad)
                .map_err(|failure| format!("损坏学习文件无法留底：{failure}"))?;
            crate::log("检测到损坏的学习文件，已保留 .bad 副本");
        }
    }
    stage("学习记录");
    // 用户导入的模型优先；不可用时仍可使用随安装包发布的固定模型。
    let external = store.dir().join("wetype").join("nlrgpt2_nnlm.bin");
    for model in [external, resources.join("nnlm.bin")] {
        if !model.is_file() {
            continue;
        }
        match engine.load_nnlm(&model) {
            Ok(_) => break,
            Err(error) => crate::log(&format!("整句增强模型载入失败：{error:#}")),
        }
    }
    stage("整句模型");
    crate::log(&format!("载入分段：{}", stages.join("，")));
    Ok(LoadedEngine {
        engine,
        path,
        warning,
        invalid_custom_format,
    })
}
