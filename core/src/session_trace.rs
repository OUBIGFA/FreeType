//! Candidate diagnostic events, adapted from WowType af506728 (Apache-2.0).
//! FreeType keeps events in memory only; the Windows service owns background persistence.
use super::{keysym, modifier, Candidate, CandidateKind, Engine, Key, Session};
use serde_json::{json, Value};
use std::{sync::Arc, time::Instant};

#[derive(Clone, Debug, Default)]
pub(super) struct Trace {
    on: bool,
    out: Vec<Value>,
    // Preview copies share immutable earlier selections instead of copying all candidate strings per key.
    steps: Arc<Vec<Value>>,
    keys: String,
    started: Option<Instant>,
    via: String,
    last_commit: Option<String>,
    truncated: bool,
}

impl Session<'_> {
    pub(super) fn trace_sensitive(&mut self) {
        self.trace = Trace {
            on: self.trace.on,
            ..Trace::default()
        };
    }
    /// Explicitly opt in; disabling discards any unfinished in-memory record.
    pub fn set_trace(&mut self, enabled: bool) {
        if self.trace.on != enabled {
            self.trace = Trace {
                on: enabled,
                ..Trace::default()
            };
        }
    }

    /// Only the owner of an accepted session may drain these events; previews never write files.
    pub fn take_trace(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.trace.out)
    }

    pub(super) fn trace_key_before(&mut self, key: &Key) -> bool {
        if !self.trace.on || key.release || self.no_learn {
            return false;
        }
        self.trace.via = key_token(key);
        if !self.composing() {
            if let Some(text) = self.trace.last_commit.take() {
                if key.keysym == keysym::BACKSPACE && !key.has_command_modifier() {
                    self.trace.out.push(json!({"e":"backspace","text":text}));
                }
            }
            return false;
        }
        self.trace_add_key();
        true
    }

    pub(super) fn trace_key_after(&mut self, key: &Key, already: bool) {
        if !self.trace.on || key.release || self.no_learn {
            return;
        }
        if self.composing() && !already {
            self.trace_add_key();
        }
        if !self.composing() {
            self.trace_abandon();
        }
    }

    fn trace_add_key(&mut self) {
        if self.trace.started.is_none() {
            self.trace.started = Some(Instant::now());
        }
        if self.trace.keys.len() >= 8192 {
            self.trace.truncated = true;
            return;
        }
        if self.trace.via.len() == 1 {
            self.trace.keys.push_str(&self.trace.via);
        } else {
            self.trace.keys.push('<');
            self.trace.keys.push_str(&self.trace.via);
            self.trace.keys.push('>');
        }
    }

    pub(super) fn trace_via(&mut self, via: &str) {
        if self.trace.on {
            self.trace.via = via.to_owned();
        }
    }

    pub(super) fn trace_undo_selection(&mut self) {
        if self.trace.on {
            Arc::make_mut(&mut self.trace.steps).pop();
        }
    }

    pub(super) fn trace_character_selection(
        &mut self,
        index: usize,
        chosen: &Candidate,
        last: bool,
    ) {
        if !self.trace.on || self.no_learn {
            return;
        }
        self.trace_step(Some(index), Some(chosen));
        if let Some(step) = Arc::make_mut(&mut self.trace.steps).last_mut() {
            step["char"] = if last { "last" } else { "first" }.into();
        }
    }

    pub(super) fn trace_commit_backspace(&mut self) {
        if !self.trace.on || self.no_learn {
            return;
        }
        if let Some(text) = self.trace.last_commit.take() {
            self.trace.out.push(json!({"e":"backspace","text":text}));
        }
    }

    pub(super) fn trace_step(&mut self, index: Option<usize>, chosen: Option<&Candidate>) {
        if !self.trace.on || self.no_learn {
            return;
        }
        if self.trace.steps.len() >= 32 {
            self.trace.truncated = true;
            return;
        }
        let size = self.page_size();
        let keep = (size * 2).max(index.map_or(0, |i| i + 1)).min(60);
        let rest = self.remaining().to_owned();
        let list = self.candidates();
        let total = list.len();
        let cands: Vec<Value> = list
            .iter()
            .take(keep)
            .map(|c| {
                json!({
                    "t":clip(&c.text,256),"p":clip(&c.pinyin,256),"n":c.consumed,"k":kind(c.kind)
                })
            })
            .collect();
        let mut step = json!({"via":self.trace.via,"rest":clip(&rest,4096),
            "pick":index.map_or(0,|i| i+1),"page":self.page,"page_size":size,"total":total,"cands":cands});
        if let Some(c) = chosen {
            step["text"] = clip(&c.text, 4096).into();
            step["pinyin"] = c.pinyin.as_str().into();
            step["consumed"] = c.consumed.into();
            step["kind"] = kind(c.kind).into();
        } else {
            step["text"] = clip(&rest, 4096).into();
            step["kind"] = "raw".into();
        }
        if rest.chars().count() > 4096 || chosen.is_some_and(|c| c.text.chars().count() > 4096) {
            self.trace.truncated = true;
        }
        Arc::make_mut(&mut self.trace.steps).push(step);
    }

    fn trace_event(&mut self, event: &str) -> Value {
        let config = self.engine.map(Engine::config);
        let scheme = config
            .filter(|c| c.shuangpin)
            .map(|c| format!("shuangpin:{}", c.shuangpin_scheme.id()))
            .unwrap_or_else(|| "pinyin".into());
        let mut record = json!({"e":event,"mode":"zh","scheme":scheme,"input":clip(&self.input,4096),
            "context":clip(&self.context,64),"keys":std::mem::take(&mut self.trace.keys),
            "steps":Arc::unwrap_or_clone(std::mem::take(&mut self.trace.steps)),
            "truncated":self.trace.truncated || self.input.chars().count()>4096});
        if let Some(config) = config {
            record["cfg"] = json!({"fuzzy":config.fuzzy,"fuzzy_groups":config.fuzzy_groups,"typo":config.typo_correction,
                "traditional":config.traditional,"english":config.english_candidate,"nnlm":config.nnlm_rerank});
        }
        if let Some(started) = self.trace.started.take() {
            record["ms"] = (started.elapsed().as_millis() as u64).into();
        }
        self.trace.truncated = false;
        record
    }

    pub(super) fn trace_commit(&mut self, text: &str) {
        if !self.trace.on || self.no_learn || self.input.is_empty() {
            return;
        }
        let mut event = self.trace_event("commit");
        event["text"] = clip(text, 4096).into();
        if text.chars().count() > 4096 {
            event["truncated"] = true.into();
        }
        self.trace.out.push(event);
        self.trace.last_commit = Some(clip(text, 4096));
    }

    pub(super) fn trace_abandon(&mut self) {
        if !self.trace.on || self.no_learn || self.trace.keys.is_empty() {
            return;
        }
        let mut event = self.trace_event("abandon");
        event["via"] = self.trace.via.as_str().into();
        self.trace.out.push(event);
    }
}

fn clip(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}
fn kind(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Word => "word",
        CandidateKind::Plain => "plain",
        CandidateKind::Emoji => "emoji",
    }
}
fn key_token(key: &Key) -> String {
    let name = match key.keysym {
        keysym::SPACE => "space".into(),
        keysym::RETURN | keysym::KP_ENTER => "enter".into(),
        keysym::BACKSPACE => "backspace".into(),
        keysym::ESCAPE => "esc".into(),
        keysym::LEFT => "left".into(),
        keysym::RIGHT => "right".into(),
        keysym::UP => "up".into(),
        keysym::DOWN => "down".into(),
        keysym::PAGE_UP => "pgup".into(),
        keysym::PAGE_DOWN => "pgdn".into(),
        sym => char::from_u32(sym)
            .filter(char::is_ascii_graphic)
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("0x{sym:x}")),
    };
    let mut prefix = String::new();
    for (bit, label) in [
        (modifier::CTRL, "ctrl+"),
        (modifier::ALT, "alt+"),
        (modifier::SUPER, "super+"),
    ] {
        if key.modifiers & bit != 0 {
            prefix.push_str(label);
        }
    }
    prefix + &name
}
