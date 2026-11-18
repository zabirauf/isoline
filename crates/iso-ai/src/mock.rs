//! A provider for tests: deterministic answers shaped like the prompts ask for (the same
//! rules as the sidecar's mock mode), token counts of `ceil(chars / 4)`, simulated prompt
//! caching of repeated system messages, and counters for calls and tokens.
//!
//! Other crates' tests can use it directly:
//!
//! ```
//! use std::sync::Arc;
//! use iso_ai::{MockProvider, Provider, CompletionRequest};
//! let mock = Arc::new(MockProvider::new());
//! let c = mock.complete(&CompletionRequest {
//!     model: "claude-haiku-5.5".into(), system: String::new(),
//!     prompt: "Task: leaf\n### f1 run\ncalls: a, b".into(), max_tokens: 500,
//! }).unwrap();
//! assert!(c.text.contains("\"f1\""));
//! assert_eq!(mock.calls(), 1);
//! ```

use std::collections::{HashSet, VecDeque};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::pricing::default_models;
use crate::tokens::estimate_tokens;
use crate::{AiError, Completion, CompletionRequest, ModelInfo, Provider};

type Responder = Box<dyn Fn(&CompletionRequest) -> Result<String, AiError> + Send + Sync>;

#[derive(Default)]
struct State {
    calls: u64,
    tokens_in: u64,
    tokens_out: u64,
    cached_in: u64,
    requests: Vec<CompletionRequest>,
    seen: HashSet<(String, String)>,
    queue: VecDeque<Result<String, AiError>>,
    list_calls: u64,
}

/// A deterministic, counting provider.
pub struct MockProvider {
    responder: Responder,
    models: Vec<ModelInfo>,
    state: Mutex<State>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockProvider {
    /// Answers every prompt with [`mock_answer`].
    pub fn new() -> Self {
        Self::with_responder(|req| Ok(mock_answer(&req.system, &req.prompt)))
    }

    /// Answers with a custom function.
    pub fn with_responder(
        f: impl Fn(&CompletionRequest) -> Result<String, AiError> + Send + Sync + 'static,
    ) -> Self {
        MockProvider {
            responder: Box::new(f),
            models: default_models(),
            state: Mutex::new(State::default()),
        }
    }

    /// Replaces the model list.
    pub fn with_models(mut self, models: Vec<ModelInfo>) -> Self {
        self.models = models;
        self
    }

    /// Queues a canned answer; queued answers are used before the responder.
    pub fn push_response(&self, text: impl Into<String>) {
        self.state.lock().queue.push_back(Ok(text.into()));
    }

    /// Queues an error.
    pub fn push_error(&self, e: AiError) {
        self.state.lock().queue.push_back(Err(e));
    }

    /// Completion calls made.
    pub fn calls(&self) -> u64 {
        self.state.lock().calls
    }
    /// `list_models` calls made.
    pub fn list_calls(&self) -> u64 {
        self.state.lock().list_calls
    }
    pub fn tokens_in(&self) -> u64 {
        self.state.lock().tokens_in
    }
    pub fn tokens_out(&self) -> u64 {
        self.state.lock().tokens_out
    }
    pub fn cached_tokens_in(&self) -> u64 {
        self.state.lock().cached_in
    }
    /// Every request received, in order.
    pub fn requests(&self) -> Vec<CompletionRequest> {
        self.state.lock().requests.clone()
    }
    /// Clears the counters and the request log (not the prompt-cache simulation).
    pub fn reset_counts(&self) {
        let mut s = self.state.lock();
        s.calls = 0;
        s.tokens_in = 0;
        s.tokens_out = 0;
        s.cached_in = 0;
        s.list_calls = 0;
        s.requests.clear();
    }
}

impl Provider for MockProvider {
    fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        self.state.lock().list_calls += 1;
        Ok(self.models.clone())
    }

    fn complete(&self, req: &CompletionRequest) -> Result<Completion, AiError> {
        let queued = {
            let mut s = self.state.lock();
            s.calls += 1;
            s.requests.push(req.clone());
            s.queue.pop_front()
        };
        let mut text = match queued {
            Some(r) => r?,
            None => (self.responder)(req)?,
        };
        // Like a real model, stop at max_tokens.
        if req.max_tokens > 0 && estimate_tokens(&text) > req.max_tokens as u64 {
            text = text.chars().take(req.max_tokens as usize * 4).collect();
        }
        let sys = estimate_tokens(&req.system);
        let tokens_in = sys + estimate_tokens(&req.prompt);
        let tokens_out = estimate_tokens(&text);
        let mut s = self.state.lock();
        let cached = if s.seen.insert((req.model.clone(), req.system.clone())) {
            0
        } else {
            sys
        };
        s.tokens_in += tokens_in;
        s.tokens_out += tokens_out;
        s.cached_in += cached;
        Ok(Completion {
            text,
            model: req.model.clone(),
            tokens_in,
            tokens_out,
            cached_tokens_in: cached,
        })
    }
}

struct Item<'a> {
    id: &'a str,
    name: &'a str,
    body: Vec<&'a str>,
}

/// Items of a prompt: `### <id> <name>` headers and the lines under them.
pub fn prompt_items(prompt: &str) -> Vec<(String, String, String)> {
    items(prompt)
        .into_iter()
        .map(|i| (i.id.to_string(), i.name.to_string(), i.body.join("\n")))
        .collect()
}

fn items(prompt: &str) -> Vec<Item<'_>> {
    let mut out: Vec<Item> = Vec::new();
    let mut open = false;
    for line in prompt.split('\n') {
        if let Some(rest) = line.strip_prefix("### ") {
            let (id, name) = rest.split_once(' ').unwrap_or((rest, ""));
            if !id.is_empty() {
                out.push(Item {
                    id,
                    name: name.trim(),
                    body: Vec::new(),
                });
                open = true;
                continue;
            }
        }
        if line.starts_with("## ") {
            open = false;
            continue;
        }
        if open && let Some(last) = out.last_mut() {
            last.body.push(line);
        }
    }
    out
}

fn field<'a>(body: &[&'a str], key: &str) -> Option<&'a str> {
    body.iter().find_map(|l| {
        l.trim()
            .strip_prefix(key)
            .and_then(|r| r.strip_prefix(':'))
            .map(str::trim)
    })
}

fn task(system: &str, prompt: &str) -> Option<String> {
    let find = |s: &str| {
        s.lines()
            .find_map(|l| l.strip_prefix("Task: "))
            .map(|t| t.split_whitespace().next().unwrap_or("").to_string())
    };
    find(prompt).or_else(|| find(system))
}

fn last_segment(s: &str) -> String {
    let s = s.trim_end_matches('/');
    let seg = s.rsplit('/').next().unwrap_or(s);
    let mut c = seg.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Root".to_string(),
    }
}

fn mock_fn(it: &Item) -> Value {
    let calls: Vec<&str> = field(&it.body, "calls")
        .map(|c| {
            c.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut algo: Vec<String> = calls
        .iter()
        .take(4)
        .map(|c| format!("Calls {c} and keeps what it returns for the next step."))
        .collect();
    let fillers = [
        "Reads the arguments it was given and checks that they are usable.",
        "Combines the intermediate values into one result.",
        "Returns the result to the caller.",
    ];
    let mut i = 0;
    while algo.len() < 3 {
        algo.push(fillers[i].to_string());
        i += 1;
    }
    let body = it.body.join("\n");
    let mut notes = Vec::new();
    if body.contains("catch") {
        notes.push(json!({"kind": "info", "text": "Errors raised inside the try block are caught here, so callers never see them."}));
    }
    if body.contains("throw") {
        notes.push(json!({"kind": "warn", "text": "Throws when a check fails, and the error reaches the caller unchanged."}));
    }
    json!({
        "id": it.id,
        "desc": format!("Handles {} for its callers and returns the outcome.", it.name),
        "algo": algo,
        "notes": notes,
    })
}

/// The deterministic answer for a prompt. Mirrors `MockAnswer` in the Go sidecar.
pub fn mock_answer(system: &str, prompt: &str) -> String {
    let items = items(prompt);
    let value = match task(system, prompt).as_deref() {
        Some("leaf") => {
            let list: Vec<Value> = items
                .iter()
                .filter_map(|it| match it.id.chars().next() {
                    Some('f') => Some(mock_fn(it)),
                    Some('t') => Some(json!({"id": it.id, "desc": format!("Describes the shape of {} values that this file passes around.", it.name)})),
                    Some('o') => Some(json!({"id": it.id, "overview": format!("Declares the functions and types of {} and the imports they rely on. Most of its work happens in the exported entry points.", it.name)})),
                    _ => None,
                })
                .collect();
            json!({ "items": list })
        }
        Some("cluster") => {
            let list: Vec<Value> = items
                .iter()
                .map(|it| {
                    let dir = field(&it.body, "dir").unwrap_or(it.name);
                    let mut resp: Vec<String> = it
                        .body
                        .iter()
                        .filter_map(|l| l.trim().strip_prefix("- "))
                        .take(3)
                        .map(|p| {
                            let p = p.split_once(':').map(|(a, _)| a).unwrap_or(p);
                            format!("Keeps the code in {p}.")
                        })
                        .collect();
                    if resp.is_empty() {
                        resp.push(format!("Keeps the code under {dir}."));
                    }
                    json!({
                        "id": it.id,
                        "name": last_segment(dir),
                        "role": format!("Groups the code under {dir} that works together."),
                        "responsibilities": resp,
                    })
                })
                .collect();
            json!({ "items": list })
        }
        Some("repo") => {
            let ids: Vec<&str> = items.iter().map(|i| i.id).collect();
            json!({
                "overview": format!("The repository is split into {} modules. Each module groups the files of one directory, and the module graph shows how they depend on each other.", items.len()),
                "layers": [{"name": "Core", "modules": ids}],
                "flows": [{"name": "Main flow", "text": "Data moves from the first module to the last.", "modules": [ids.first(), ids.last()]}],
            })
        }
        _ => {
            return format!(
                "Mock reply for a prompt of {} tokens.",
                estimate_tokens(prompt)
            );
        }
    };
    canonical(value).to_string()
}

/// Sort object keys at every level, so the text is identical whether or not serde_json's
/// `preserve_order` feature is enabled somewhere in the build (the Go mock sorts keys too).
fn canonical(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let sorted: std::collections::BTreeMap<String, serde_json::Value> =
                m.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.into_iter().map(canonical).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_like_the_go_mock() {
        let p = "Task: leaf\n## file a.ts (TypeScript)\n### f1 run\ncalls: load, save\nflow:\n  try\n  catch\n### t1 Config\n### o1 a.ts\n";
        let v: Value = serde_json::from_str(&mock_answer("", p)).unwrap();
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["algo"].as_array().unwrap().len(), 3);
        assert_eq!(items[0]["notes"].as_array().unwrap().len(), 1);
        assert!(items[2]["overview"].as_str().unwrap().contains("a.ts"));
        let v: Value = serde_json::from_str(&mock_answer(
            "Task: cluster",
            "### m1 src/engine\ndir: src/engine\n",
        ))
        .unwrap();
        assert_eq!(v["items"][0]["name"], "Engine");
        assert!(mock_answer("", "hello").starts_with("Mock reply"));
    }

    #[test]
    fn counts_tokens_and_simulates_prompt_cache() {
        let m = MockProvider::new();
        let req = CompletionRequest {
            model: "m".into(),
            system: "abcdefgh".into(),
            prompt: "Task: repo\n### m1 A".into(),
            max_tokens: 100,
        };
        let a = m.complete(&req).unwrap();
        let b = m.complete(&req).unwrap();
        assert_eq!(a.cached_tokens_in, 0);
        assert_eq!(b.cached_tokens_in, 2);
        assert_eq!(a.tokens_in, 2 + estimate_tokens(&req.prompt));
        assert_eq!(m.calls(), 2);
        assert_eq!(m.tokens_out(), a.tokens_out * 2);
        m.push_error(AiError::NotSignedIn);
        assert!(matches!(m.complete(&req), Err(AiError::NotSignedIn)));
        m.push_response("x");
        assert_eq!(m.complete(&req).unwrap().text, "x");
    }
}
