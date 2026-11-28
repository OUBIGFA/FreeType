//! Optional model boundary. The input thread only copies a bounded snapshot and
//! reads completed results; implementations must never run inference in submit.
use crate::Lexicon;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct Query {
    pub key: String,
    pub context: String,
    pub pinyin: String,
    pub candidates: Vec<String>,
    pub letters: Vec<Vec<char>>,
    pub variants: Vec<(String, Vec<Vec<char>>)>,
    pub blocks: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Answer {
    pub key: String,
    pub text: String,
    pub pinyin: String,
    pub words: Vec<(String, String)>,
    pub uncertain: Vec<bool>,
}
pub trait Backend: Send + Sync {
    fn submit(&self, query: Query);
    fn answer(&self, key: &str) -> Option<Answer>;
    fn available(&self) -> bool {
        true
    }
}
impl crate::Engine {
    pub fn set_enhancement(&self, backend: Option<Arc<dyn Backend>>) {
        *self.enhancement.write().unwrap() = backend;
    }
    pub fn enhancement_backend(&self) -> Option<Arc<dyn Backend>> {
        self.enhancement.read().unwrap().clone()
    }
    pub fn enhancement_query(
        &self,
        raw: &str,
        pinyin: &str,
        candidates: Vec<String>,
    ) -> Option<Query> {
        self.enhancement_backend()?;
        let syllables: Vec<_> = pinyin.split_whitespace().collect();
        if syllables.len() < 2 || syllables.len() > 96 || candidates.is_empty() {
            return None;
        }
        let mut letters = Vec::new();
        for s in &syllables {
            let parsed = crate::syllable::Syllable::from_text(s)?;
            let query = [crate::SyllableQuery::Full {
                id: parsed.id,
                erhua: false,
            }];
            let found = self.lexicon.lookup_common(&query);
            let mut chars = Vec::new();
            for entry in found {
                let mut it = entry.word.chars();
                if let (Some(ch), None) = (it.next(), it.next()) {
                    if !chars.contains(&ch) {
                        chars.push(ch);
                    }
                }
            }
            if chars.is_empty() {
                return None;
            }
            letters.push(chars);
        }
        let context = self.context();
        let key = format!(
            "{context}\u{1f}{raw}\u{1f}{pinyin}\u{1f}{}",
            candidates.join("\u{1e}")
        );
        let mut variants = Vec::new();
        if !self.config().shuangpin
            && pinyin.split_whitespace().collect::<String>() != raw.replace('\'', "")
        {
            if let Ok(graph) = crate::SyllableGraph::from_str(raw, crate::GraphOptions::default()) {
                fn walk(
                    g: &crate::SyllableGraph,
                    at: usize,
                    n: usize,
                    path: &mut Vec<String>,
                    out: &mut Vec<String>,
                    reachable: &[Vec<bool>],
                ) {
                    if out.len() >= 4 {
                        return;
                    }
                    if !reachable[n - path.len()][at] {
                        return;
                    }
                    if at == g.len() {
                        if path.len() == n {
                            out.push(path.join(" "));
                        }
                        return;
                    }
                    if path.len() >= n {
                        return;
                    }
                    for e in g.edges_from(at) {
                        if let crate::EdgeKind::Full {
                            syllable,
                            erhua: false,
                        } = e.kind
                        {
                            path.push(syllable.text.into());
                            walk(g, e.end, n, path, out, reachable);
                            path.pop();
                        }
                    }
                }
                let mut reachable = vec![vec![false; graph.len() + 1]; syllables.len() + 1];
                reachable[0][graph.len()] = true;
                for n in 1..=syllables.len() {
                    for at in (0..graph.len()).rev() {
                        reachable[n][at] = graph.edges_from(at).iter().any(|e| {
                            matches!(e.kind, crate::EdgeKind::Full { erhua: false, .. })
                                && reachable[n - 1][e.end]
                        });
                    }
                }
                let mut readings = Vec::new();
                walk(
                    &graph,
                    0,
                    syllables.len(),
                    &mut Vec::new(),
                    &mut readings,
                    &reachable,
                );
                for reading in readings.into_iter().filter(|p| p != pinyin) {
                    let mut options = Vec::new();
                    for s in reading.split_whitespace() {
                        let id = crate::syllable::Syllable::from_text(s)?.id;
                        let mut chars = Vec::new();
                        for entry in self
                            .lexicon
                            .lookup_common(&[crate::SyllableQuery::Full { id, erhua: false }])
                        {
                            let mut it = entry.word.chars();
                            if let (Some(c), None) = (it.next(), it.next()) {
                                if !chars.contains(&c) {
                                    chars.push(c);
                                }
                            }
                        }
                        options.push(chars);
                    }
                    if options.iter().all(|c| !c.is_empty()) {
                        variants.push((reading, options));
                    }
                }
            }
        }
        Some(Query {
            key,
            context,
            pinyin: pinyin.into(),
            candidates,
            letters,
            variants,
            blocks: Vec::new(),
        })
    }
}
