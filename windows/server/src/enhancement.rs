//! Optional GPT-2 worker. A single replaceable request prevents slow inference
//! from building a queue behind typing. Native weights live only on this worker.
use freetype_protocol::EnhancementStatus;
use std::{
    collections::{HashMap, VecDeque},
    ffi::{c_void, CStr, CString},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
};
use wowtype_core::enhancement::{Answer, Backend, Query};

const SHA: &str = "b72e0c8728e05454677373c700aba7860c8595c589d73b39683e3de2671708e3";
type Load = unsafe extern "C" fn(*const u16, i32) -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type Token = unsafe extern "C" fn(*mut c_void, i32) -> *const i8;
type Add = unsafe extern "C" fn(*mut c_void, *const i32, *const i32, i32) -> i32;
type Clear = unsafe extern "C" fn(*mut c_void, i32) -> i32;
type Logp = unsafe extern "C" fn(*mut c_void, i32, i32) -> f32;
type Heads = unsafe extern "C" fn(*mut c_void, *const i32, i32, i32, *mut f32) -> i32;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(path: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const i8) -> *mut c_void;
    fn FreeLibrary(h: *mut c_void) -> i32;
}
struct Native {
    lib: *mut c_void,
    model: *mut c_void,
    free: Free,
    idle: Free,
    add: Add,
    clear: Clear,
    logp: Logp,
    heads: Heads,
    tokens: HashMap<char, i32>,
    bos: i32,
    nodes: HashMap<Vec<i32>, i32>,
    count: i32,
    context: Vec<i32>,
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            (self.free)(self.model);
            FreeLibrary(self.lib);
        }
    }
}
impl Native {
    fn load(path: &Path) -> Result<Self, String> {
        if !std::is_x86_feature_detected!("avx2")
            || !std::is_x86_feature_detected!("fma")
            || !std::is_x86_feature_detected!("f16c")
        {
            return Err("增强模型需要支持 AVX2、FMA 和 F16C 的处理器".into());
        }
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if bytes.len() != 116198144 {
            return Err("增强模型大小不正确；请重新下载".into());
        }
        let mut hash = [0u8; 32];
        unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, &bytes, &mut hash).ok() }
            .map_err(|e| e.to_string())?;
        if hash.iter().map(|x| format!("{x:02x}")).collect::<String>() != SHA {
            return Err("增强模型校验失败；请重新下载".into());
        }
        drop(bytes);
        let dll = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("无法定位增强模型运行库")?
            .join("FreeTypeEnhancement.dll");
        #[cfg(test)]
        let dll = std::env::var_os("FREETYPE_TEST_ENHANCEMENT_DLL")
            .map(PathBuf::from)
            .unwrap_or(dll);
        let wide: Vec<_> = dll.as_os_str().encode_wide().chain(Some(0)).collect();
        let lib = unsafe { LoadLibraryW(wide.as_ptr()) };
        if lib.is_null() {
            return Err("无法加载增强模型运行库；需要支持 AVX2 的处理器".into());
        }
        unsafe fn symbol<T: Copy>(lib: *mut c_void, name: &str) -> Result<T, String> {
            let name = CString::new(name).unwrap();
            let p = unsafe { GetProcAddress(lib, name.as_ptr()) };
            if p.is_null() {
                return Err("增强模型运行库版本不匹配".into());
            }
            Ok(unsafe { std::mem::transmute_copy(&p) })
        }
        let loaded = (|| unsafe {
            let load: Load = symbol(lib, "ft_model_load")?;
            let free: Free = symbol(lib, "ft_model_free")?;
            let idle: Free = symbol(lib, "ft_model_idle")?;
            let token: Token = symbol(lib, "ft_model_token")?;
            let add: Add = symbol(lib, "ft_model_add")?;
            let clear: Clear = symbol(lib, "ft_model_clear")?;
            let logp: Logp = symbol(lib, "ft_model_logp")?;
            let heads: Heads = symbol(lib, "ft_model_heads")?;
            let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let model = load(wide.as_ptr(), 2);
            if model.is_null() {
                return Err("增强模型无法载入".into());
            }
            let mut tokens = HashMap::new();
            let mut bos = 101;
            for i in 0..21128 {
                let p = token(model, i);
                if p.is_null() {
                    continue;
                }
                let text = CStr::from_ptr(p).to_string_lossy();
                if text == "[CLS]" {
                    bos = i;
                }
                let mut chars = text.chars();
                if let (Some(ch), None) = (chars.next(), chars.next()) {
                    tokens.insert(ch, i);
                }
            }
            Ok(Self {
                lib,
                model,
                free,
                idle,
                add,
                clear,
                logp,
                heads,
                tokens,
                bos,
                nodes: HashMap::new(),
                count: 0,
                context: Vec::new(),
            })
        })();
        if loaded.is_err() {
            unsafe {
                FreeLibrary(lib);
            }
        }
        loaded
    }
    fn reset(&mut self) -> Result<(), String> {
        if unsafe { (self.clear)(self.model, 0) } == 0 {
            return Err("模型缓存重置失败".into());
        }
        self.nodes.clear();
        self.count = 0;
        Ok(())
    }
    fn ensure(&mut self, sequences: &[Vec<i32>]) -> Result<(), String> {
        let max = sequences.iter().map(Vec::len).max().unwrap_or(0);
        for depth in 1..=max {
            let mut missing = Vec::new();
            for s in sequences.iter().filter(|s| s.len() >= depth) {
                let p = s[..depth].to_vec();
                if !self.nodes.contains_key(&p) && !missing.contains(&p) {
                    missing.push(p);
                }
            }
            for batch in missing.chunks(128) {
                if self.count + batch.len() as i32 > 2048 {
                    return Err("模型候选缓存已满".into());
                }
                let tokens: Vec<_> = batch.iter().map(|s| *s.last().unwrap()).collect();
                let parents: Vec<_> = batch
                    .iter()
                    .map(|s| {
                        if s.len() == 1 {
                            -1
                        } else {
                            self.nodes[&s[..s.len() - 1]]
                        }
                    })
                    .collect();
                if unsafe {
                    (self.add)(
                        self.model,
                        tokens.as_ptr(),
                        parents.as_ptr(),
                        tokens.len() as i32,
                    )
                } == 0
                {
                    return Err("模型计算失败".into());
                }
                for s in batch {
                    self.nodes.insert(s.clone(), self.count);
                    self.count += 1;
                }
            }
        }
        Ok(())
    }
    fn score(&self, sequence: &[i32], prefix: usize) -> f32 {
        (prefix..sequence.len())
            .map(|i| unsafe { (self.logp)(self.model, self.nodes[&sequence[..i]], sequence[i]) })
            .sum()
    }
    fn run(
        &mut self,
        q: &Query,
        revision: u64,
        latest: &AtomicU64,
    ) -> Result<Option<Answer>, String> {
        let result = self.run_active(q, revision, latest);
        unsafe {
            (self.idle)(self.model);
        }
        result
    }
    fn run_active(
        &mut self,
        q: &Query,
        revision: u64,
        latest: &AtomicU64,
    ) -> Result<Option<Answer>, String> {
        let mut queries = vec![q.clone()];
        for (pinyin, letters) in &q.variants {
            let mut variant = q.clone();
            variant.pinyin = pinyin.clone();
            variant.letters = letters.clone();
            variant.candidates.clear();
            variant.variants.clear();
            queries.push(variant);
        }
        let mut best: Option<(Answer, f32)> = None;
        for query in queries {
            if latest.load(Ordering::Relaxed) != revision {
                return Ok(None);
            }
            if let Some(answer) = self.run_one(&query, revision, latest)? {
                let mut prefix = vec![self.bos];
                prefix.extend(
                    query
                        .context
                        .chars()
                        .rev()
                        .take(30)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .filter_map(|c| self.tokens.get(&c).copied()),
                );
                let n = prefix.len();
                prefix.extend(
                    answer
                        .text
                        .chars()
                        .filter_map(|c| self.tokens.get(&c).copied()),
                );
                self.ensure(&[prefix.clone()])?;
                let score = self.score(&prefix, n);
                if best.as_ref().is_none_or(|(_, v)| score > *v) {
                    best = Some((answer, score));
                }
            }
        }
        Ok(best.map(|(a, _)| a))
    }
    fn run_one(
        &mut self,
        q: &Query,
        revision: u64,
        latest: &AtomicU64,
    ) -> Result<Option<Answer>, String> {
        let mut prefix = vec![self.bos];
        prefix.extend(
            q.context
                .chars()
                .rev()
                .take(30)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .filter_map(|c| self.tokens.get(&c).copied()),
        );
        if self.context != prefix || self.count + q.letters.len() as i32 * 28 > 2000 {
            self.reset()?;
            self.context = prefix.clone();
        }
        self.ensure(&[prefix.clone()])?;
        let mut beam = vec![(String::new(), prefix.clone(), 0f32)];
        // Candidate generation is constrained by the existing dictionary's real
        // readings; it never emits unrelated free-form language-model text.
        for choices in &q.letters {
            if latest.load(Ordering::Relaxed) != revision {
                return Ok(None);
            }
            let mut next = Vec::new();
            for (text, seq, score) in &beam {
                let node = self.nodes[seq];
                for ch in choices {
                    if let Some(&id) = self.tokens.get(ch) {
                        let mut s = seq.clone();
                        s.push(id);
                        next.push((
                            format!("{text}{ch}"),
                            s,
                            *score + unsafe { (self.logp)(self.model, node, id) },
                        ));
                    }
                }
            }
            next.sort_by(|a, b| b.2.total_cmp(&a.2));
            next.truncate((1600 / q.letters.len().max(1)).min(20));
            if next.is_empty() {
                return Ok(None);
            }
            self.ensure(&next.iter().map(|x| x.1.clone()).collect::<Vec<_>>())?;
            beam = next;
        }
        let mut all: Vec<_> = beam.into_iter().map(|(t, s, _)| (t, s)).collect();
        for text in &q.candidates {
            let ids: Option<Vec<_>> = text.chars().map(|c| self.tokens.get(&c).copied()).collect();
            if let Some(ids) = ids {
                let mut seq = prefix.clone();
                seq.extend(ids);
                if !all.iter().any(|(t, _)| t == text) {
                    all.push((text.clone(), seq));
                }
            }
        }
        self.ensure(&all.iter().map(|x| x.1.clone()).collect::<Vec<_>>())?;
        let scores: Vec<_> = all
            .iter()
            .map(|(_, seq)| self.score(seq, prefix.len()))
            .collect();
        let Some(best) = (0..all.len()).max_by(|&a, &b| scores[a].total_cmp(&scores[b])) else {
            return Ok(None);
        };
        let (mut text, mut seq) = all[best].clone();
        // Keep established word blocks when the competing sentence distribution
        // still supports them. This is confidence over alternatives, not a claim
        // that any displayed word is certainly correct.
        if q.candidates
            .first()
            .is_some_and(|s| q.blocks.concat() == *s)
        {
            let mut chars: Vec<_> = text.chars().collect();
            let max = scores[best];
            let weights: Vec<_> = scores.iter().map(|s| (*s - max).exp()).collect();
            let total: f32 = weights.iter().sum();
            let mut at = 0;
            for word in &q.blocks {
                let n = word.chars().count();
                if at + n > chars.len() {
                    break;
                }
                let mass: f32 = all
                    .iter()
                    .zip(&weights)
                    .filter(|((candidate, _), _)| {
                        candidate.chars().skip(at).take(n).collect::<String>() == *word
                    })
                    .map(|(_, w)| *w)
                    .sum();
                if mass >= 0.2 * total {
                    for (slot, ch) in chars[at..at + n].iter_mut().zip(word.chars()) {
                        *slot = ch;
                    }
                }
                at += n;
            }
            text = chars.iter().collect();
            seq = prefix.clone();
            seq.extend(chars.iter().map(|c| self.tokens[c]));
            self.ensure(&[seq.clone()])?;
        }
        let chars: Vec<_> = text.chars().collect();
        let syllables: Vec<_> = q.pinyin.split_whitespace().collect();
        let nodes: Vec<_> = (prefix.len() + 1..=seq.len())
            .map(|n| self.nodes[&seq[..n]])
            .collect();
        let mut segments = vec![0f32; nodes.len() * 2];
        let mut entities = vec![0f32; nodes.len() * 5];
        let heads_ok = unsafe {
            (self.heads)(
                self.model,
                nodes.as_ptr(),
                nodes.len() as i32,
                1,
                segments.as_mut_ptr(),
            ) != 0
                && (self.heads)(
                    self.model,
                    nodes.as_ptr(),
                    nodes.len() as i32,
                    0,
                    entities.as_mut_ptr(),
                ) != 0
        };
        let mut words: Vec<(String, String)> = Vec::new();
        let mut start = 0;
        for i in 1..=chars.len() {
            let inside_entity =
                i < chars.len() && (entities[i * 5 + 2] >= 0.8 || entities[i * 5 + 4] >= 0.8);
            if i == chars.len() || (heads_ok && segments[i * 2 + 1] >= 0.5 && !inside_entity) {
                words.push((
                    chars[start..i].iter().collect(),
                    syllables[start..i].join(" "),
                ));
                start = i;
            }
        }
        let peak = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let weights: Vec<_> = scores.iter().map(|s| (*s - peak).exp()).collect();
        let total: f32 = weights.iter().sum();
        let mut uncertain = vec![false; chars.len()];
        let mut at = 0;
        for (word, _) in &words {
            let n = word.chars().count();
            let mass: f32 = all
                .iter()
                .zip(&weights)
                .filter(|((candidate, _), _)| {
                    candidate.chars().skip(at).take(n).collect::<String>() == *word
                })
                .map(|(_, w)| *w)
                .sum();
            if mass < total * 0.075 {
                uncertain[at..at + n].fill(true);
            }
            at += n;
        }
        Ok(Some(Answer {
            key: q.key.clone(),
            text,
            pinyin: q.pinyin.clone(),
            words,
            uncertain,
        }))
    }
}

#[derive(Default)]
struct Control {
    enabled: bool,
    model_generation: u64,
    query: Option<Query>,
    running: Option<String>,
    shutdown: bool,
    answers: VecDeque<Answer>,
    status: EnhancementStatus,
}
struct Shared {
    control: Mutex<Control>,
    wake: Condvar,
    latest: AtomicU64,
}
pub struct Worker {
    shared: Arc<Shared>,
}
impl Worker {
    pub fn new(dir: PathBuf) -> Arc<Self> {
        let shared = Arc::new(Shared {
            control: Mutex::new(Control::default()),
            wake: Condvar::new(),
            latest: AtomicU64::new(0),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("freetype-enhancement".into())
            .spawn(move || run_worker(s, dir))
            .expect("enhancement worker");
        Arc::new(Self { shared })
    }
    pub fn enable(&self, on: bool) {
        let mut c = self.shared.control.lock().unwrap();
        c.model_generation = c.model_generation.wrapping_add(1);
        c.enabled = on;
        c.query = None;
        c.answers.clear();
        c.status.enabled = on;
        c.status.state = if on { "loading" } else { "stopping" }.into();
        self.shared.latest.fetch_add(1, Ordering::Relaxed);
        self.shared.wake.notify_one();
    }
    pub fn status(&self) -> EnhancementStatus {
        self.shared.control.lock().unwrap().status.clone()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let mut c = self.shared.control.lock().unwrap();
        c.shutdown = true;
        self.shared.latest.fetch_add(1, Ordering::Relaxed);
        self.shared.wake.notify_one();
    }
}
impl Backend for Worker {
    fn available(&self) -> bool {
        let c = self.shared.control.lock().unwrap();
        c.enabled && c.status.state != "error"
    }
    fn submit(&self, q: Query) {
        let mut c = self.shared.control.lock().unwrap();
        if !c.enabled
            || c.status.state == "error"
            || c.answers.iter().any(|a| a.key == q.key)
            || c.query.as_ref().is_some_and(|v| v.key == q.key)
            || c.running.as_ref() == Some(&q.key)
        {
            return;
        }
        c.query = Some(q);
        self.shared.latest.fetch_add(1, Ordering::Relaxed);
        self.shared.wake.notify_one();
    }
    fn answer(&self, key: &str) -> Option<Answer> {
        self.shared
            .control
            .lock()
            .unwrap()
            .answers
            .iter()
            .find(|a| a.key == key)
            .cloned()
    }
}
fn run_worker(s: Arc<Shared>, dir: PathBuf) {
    crate::background_thread();
    let mut model = None;
    let mut attempted = false;
    let mut generation = 0;
    loop {
        let mut c = s.control.lock().unwrap();
        if c.shutdown {
            return;
        }
        if generation != c.model_generation {
            generation = c.model_generation;
            let old = model.take();
            drop(c);
            drop(old);
            attempted = false;
            c = s.control.lock().unwrap();
            if generation != c.model_generation {
                continue;
            }
        }
        // A loading request can be cancelled while its file is read. Recheck
        // the generation after dropping the native allocation, never load again
        // under the superseded enabled state.
        if generation != c.model_generation {
            continue;
        }
        if !c.enabled {
            let old = model.take();
            drop(c);
            drop(old);
            attempted = false;
            c = s.control.lock().unwrap();
            if !c.enabled {
                c.status = EnhancementStatus {
                    state: "off".into(),
                    ..Default::default()
                };
            }
        }
        if c.enabled && !attempted {
            attempted = true;
            drop(c);
            let result = Native::load(&dir.join("enhancement/nova-v1.gguf"));
            c = s.control.lock().unwrap();
            if generation != c.model_generation || !c.enabled {
                drop(c);
                drop(result);
                continue;
            }
            match result {
                Ok(m) => {
                    model = Some(m);
                    c.status.state = "ready".into();
                    c.status.error.clear();
                }
                Err(e) => {
                    c.status.state = "error".into();
                    c.status.error = e;
                }
            }
        }
        if c.enabled && c.status.state == "ready" {
            if let (Some(m), Some(q)) = (model.as_mut(), c.query.take()) {
                let revision = s.latest.load(Ordering::Relaxed);
                c.running = Some(q.key.clone());
                drop(c);
                let result = m.run(&q, revision, &s.latest);
                c = s.control.lock().unwrap();
                c.running = None;
                match result {
                    Ok(answer) if c.enabled && s.latest.load(Ordering::Relaxed) == revision => {
                        c.status.error.clear();
                        // A spelling containing only out-of-vocabulary characters
                        // stays with the base result and must not poll forever.
                        let answer = answer.or_else(|| {
                            q.candidates.first().map(|text| Answer {
                                uncertain: Vec::new(),
                                key: q.key.clone(),
                                text: text.clone(),
                                pinyin: q.pinyin.clone(),
                                words: vec![(text.clone(), q.pinyin.clone())],
                            })
                        });
                        if let Some(a) = answer {
                            c.answers.push_back(a);
                            while c.answers.len() > 8 {
                                c.answers.pop_front();
                            }
                        }
                    }
                    Err(e) => {
                        c.status.error = e;
                        c.status.state = "error".into();
                        c.query = None;
                    }
                    _ => {}
                }
                continue;
            }
        }
        c = s.wake.wait(c).unwrap();
        drop(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_model_generates_and_scores_constrained_sentence() {
        let Some(path) = std::env::var_os("FREETYPE_TEST_NOVA") else {
            return;
        };
        let mut native = Native::load(Path::new(&path)).unwrap();
        let q = Query {
            key: "test".into(),
            context: String::new(),
            pinyin: "zhe ge jie kou fan hui tai man le".into(),
            candidates: vec!["这个借口返回太慢了".into()],
            letters: [
                "这者",
                "个各",
                "借接姐",
                "口扣",
                "返反",
                "回会",
                "太台",
                "慢满",
                "了乐",
            ]
            .iter()
            .map(|s| s.chars().collect())
            .collect(),
            variants: Vec::new(),
            blocks: Vec::new(),
        };
        let result = native.run(&q, 1, &AtomicU64::new(1)).unwrap().unwrap();
        assert_eq!(result.text, "这个接口返回太慢了");
        assert_eq!(
            result
                .words
                .iter()
                .map(|(w, _)| w.as_str())
                .collect::<String>(),
            result.text
        );
    }
    #[test]
    fn actual_dictionary_supplies_model_generation_candidates() {
        let Some(path) = std::env::var_os("FREETYPE_TEST_NOVA") else {
            return;
        };
        let Some(resources) = std::env::var_os("FREETYPE_TEST_REAL_RESOURCES") else {
            return;
        };
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_temp/enhancement-tests")
            .join(format!("freetype-model-real-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let e = crate::host::load_engine(Path::new(&resources), &directory, "")
            .unwrap()
            .engine;
        struct Idle;
        impl Backend for Idle {
            fn submit(&self, _: Query) {}
            fn answer(&self, _: &str) -> Option<Answer> {
                None
            }
        }
        e.set_enhancement(Some(Arc::new(Idle)));
        let choices = e.input("zhegejiekoufanhuitaimanle");
        let top = &choices[0];
        let q = e
            .enhancement_query(
                "zhegejiekoufanhuitaimanle",
                &top.pinyin,
                choices
                    .iter()
                    .filter(|c| c.pinyin == top.pinyin)
                    .map(|c| c.text.clone())
                    .collect(),
            )
            .unwrap();
        let mut native = Native::load(Path::new(&path)).unwrap();
        let result = native.run(&q, 1, &AtomicU64::new(1)).unwrap().unwrap();
        assert_eq!(result.text, "这个接口返回太慢了", "pinyin={}", q.pinyin);
    }
}
