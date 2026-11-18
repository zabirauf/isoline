//! Robust parsing of model answers: tolerate code fences and chatter around the JSON, skip
//! bad items with a recorded reason, and clean the text so it follows the voice rules.

use iso_core::ipc::{Note, NoteKind, Summary};
use serde_json::Value;

/// Finds the JSON value in an answer. Accepts a bare object, a fenced block, or an object
/// surrounded by other text.
pub fn extract_json(text: &str) -> Result<Value, String> {
    let t = text.trim().trim_start_matches('\u{feff}');
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let body_start = after.find('\n').map(|i| i + 1).unwrap_or(0);
        let body = &after[body_start..];
        let body = body.find("```").map(|e| &body[..e]).unwrap_or(body);
        if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
            return Ok(v);
        }
    }
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(a), Some(b)) = (t.find(open), t.rfind(close))
            && a < b
            && let Ok(v) = serde_json::from_str::<Value>(&t[a..=b])
        {
            return Ok(v);
        }
    }
    Err("the answer holds no readable JSON".to_string())
}

/// The item list of an answer: `{"items": [...]}`, or a bare array.
pub fn items(v: &Value) -> Result<&Vec<Value>, String> {
    match v {
        Value::Array(a) => Ok(a),
        Value::Object(o) => match o.get("items") {
            Some(Value::Array(a)) => Ok(a),
            _ => Err("the answer has no \"items\" array".to_string()),
        },
        _ => Err("the answer is not a JSON object".to_string()),
    }
}

/// The `id` of an item.
pub fn item_id(v: &Value) -> Option<String> {
    match v.get("id")? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE0F | 0x200D | 0x2B50 | 0x2B55)
}

/// Cleans model text: collapses whitespace, drops emoji, turns sentence-ending exclamation
/// marks outside backticks into full stops.
pub fn clean_text(s: &str) -> String {
    let collapsed: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !is_emoji(*c))
        .collect();
    let chars: Vec<char> = collapsed.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut in_code = false;
    for (i, &c) in chars.iter().enumerate() {
        if c == '`' {
            in_code = !in_code;
        }
        if c == '!' && !in_code {
            let next = chars.get(i + 1).copied();
            let prev = if i > 0 {
                chars.get(i - 1).copied()
            } else {
                None
            };
            let ends = next.is_none_or(|n| n.is_whitespace() || n == '"' || n == ')');
            let after_word = prev.is_some_and(|p| p.is_alphanumeric() || p == ')' || p == '"');
            if ends && after_word {
                out.push('.');
                continue;
            }
        }
        out.push(c);
    }
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn sentence(s: &str) -> String {
    let mut t = clean_text(s);
    if !t.is_empty() && !t.ends_with(['.', '?', ':', ')', '"', '`']) {
        t.push('.');
    }
    t
}

fn strip_marker(s: &str) -> &str {
    let t = s.trim_start();
    for m in ["- ", "* ", "• "] {
        if let Some(r) = t.strip_prefix(m) {
            return r;
        }
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits < 3 {
        let rest = &t[digits..];
        for m in [". ", ") "] {
            if let Some(r) = rest.strip_prefix(m) {
                return r;
            }
        }
    }
    t
}

fn string_field(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn string_list(v: &Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .map(|s| sentence(strip_marker(s)))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .lines()
            .map(|l| sentence(strip_marker(l)))
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Maximum algorithm steps kept.
pub const MAX_STEPS: usize = 7;
/// Maximum notes kept.
pub const MAX_NOTES: usize = 3;

/// Parses a function item into a summary.
pub fn parse_summary(item: &Value, model: &str) -> Result<Summary, String> {
    let desc = string_field(item, "desc")
        .map(|s| sentence(&s))
        .filter(|s| !s.is_empty())
        .ok_or("no desc")?;
    let mut algo = string_list(item, "algo");
    algo.truncate(MAX_STEPS);
    let mut notes = Vec::new();
    if let Some(Value::Array(a)) = item.get("notes") {
        for n in a {
            let (kind, text) = match n {
                Value::String(s) => ("info".to_string(), s.clone()),
                Value::Object(_) => (
                    string_field(n, "kind")
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                    string_field(n, "text").unwrap_or_default(),
                ),
                _ => continue,
            };
            let text = sentence(&text);
            if text.is_empty() {
                continue;
            }
            let kind = match kind.as_str() {
                "warn" | "warning" | "bug" | "risk" => NoteKind::Warn,
                _ => NoteKind::Info,
            };
            notes.push(Note { kind, text });
        }
    }
    notes.truncate(MAX_NOTES);
    Ok(Summary {
        desc,
        algo,
        notes,
        model: model.to_string(),
        ai: true,
    })
}

/// Parses a type item: one sentence.
pub fn parse_type_desc(item: &Value) -> Result<String, String> {
    string_field(item, "desc")
        .or_else(|| string_field(item, "summary"))
        .map(|s| sentence(&s))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "no desc".to_string())
}

/// Parses an overview item.
pub fn parse_overview(item: &Value) -> Result<String, String> {
    string_field(item, "overview")
        .or_else(|| string_field(item, "desc"))
        .map(|s| sentence(&s))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "no overview".to_string())
}

/// A module answer: name, role, responsibilities.
pub fn parse_module(item: &Value) -> Result<(String, String, Vec<String>), String> {
    let name = string_field(item, "name")
        .map(|s| clean_text(&s).trim_end_matches('.').to_string())
        .filter(|s| !s.is_empty())
        .ok_or("no name")?;
    let role = string_field(item, "role")
        .map(|s| sentence(&s))
        .unwrap_or_default();
    let mut resp = string_list(item, "responsibilities");
    resp.truncate(6);
    Ok((name, role, resp))
}

/// A layer as the model wrote it: name and module ids.
pub type RawLayer = (String, Vec<String>);

/// A flow as the model wrote it: name, sentence and module ids in order.
pub type RawFlow = (String, String, Vec<String>);

/// A repo answer: overview, layers (name, module ids) and flows.
pub fn parse_repo(v: &Value) -> Result<(String, Vec<RawLayer>, Vec<RawFlow>), String> {
    let overview = string_field(v, "overview")
        .map(|s| clean_text(&s))
        .filter(|s| !s.is_empty())
        .ok_or("no overview")?;
    let mut layers = Vec::new();
    if let Some(Value::Array(a)) = v.get("layers") {
        for l in a {
            let Some(name) = string_field(l, "name").map(|s| clean_text(&s)) else {
                continue;
            };
            let mods = match l.get("modules") {
                Some(Value::Array(m)) => m
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                    .collect(),
                _ => Vec::new(),
            };
            layers.push((name, mods));
        }
    }
    let mut flows = Vec::new();
    if let Some(Value::Array(a)) = v.get("flows") {
        for f in a {
            let Some(name) = string_field(f, "name").map(|s| clean_text(&s)) else {
                continue;
            };
            let text = string_field(f, "text")
                .map(|s| clean_text(&s))
                .unwrap_or_default();
            let mods: Vec<String> = match f.get("modules") {
                Some(Value::Array(m)) => m
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                    .collect(),
                _ => Vec::new(),
            };
            flows.push((name, text, mods));
        }
    }
    Ok((overview, layers, flows))
}

/// A one-line summary from a doc comment, used when there is no model answer.
/// `ai` is false, so the UI shows it without the AI marker.
pub fn summary_from_doc(doc: &str) -> Option<Summary> {
    let text = clean_text(doc);
    if text.is_empty() {
        return None;
    }
    let end = text
        .char_indices()
        .find(|&(i, c)| c == '.' && text[i + 1..].starts_with(' ') || c == '\n')
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len());
    Some(Summary {
        desc: sentence(&text[..end]),
        algo: Vec::new(),
        notes: Vec::new(),
        model: String::new(),
        ai: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fences_and_chatter() {
        let v = extract_json("```json\n{\"items\":[]}\n```").unwrap();
        assert!(v["items"].is_array());
        let v = extract_json("Here you go:\n{\"items\":[{\"id\":\"f1\"}]}\nDone.").unwrap();
        assert_eq!(items(&v).unwrap().len(), 1);
        let v = extract_json("[{\"id\":\"f1\"}]").unwrap();
        assert_eq!(items(&v).unwrap().len(), 1);
        assert!(extract_json("no json here").is_err());
        assert!(items(&serde_json::json!({"x": 1})).is_err());
    }

    #[test]
    fn cleans_voice() {
        assert_eq!(clean_text("Runs  the   job! 🚀"), "Runs the job.");
        assert_eq!(clean_text("Calls `println!` once"), "Calls `println!` once");
        assert_eq!(clean_text("Checks !x first"), "Checks !x first");
        assert_eq!(strip_marker("1. Read it"), "Read it");
        assert_eq!(strip_marker("- Read it"), "Read it");
    }

    #[test]
    fn parses_summary_items() {
        let item = serde_json::json!({
            "id": "f1",
            "desc": "Builds the base fields",
            "algo": ["1. Read the trace", "- Add the ids!", "x", "x", "x", "x", "x", "x"],
            "notes": [{"kind": "WARNING", "text": "Can throw twice"}, {"kind": "info", "text": ""}, "plain"]
        });
        let s = parse_summary(&item, "m").unwrap();
        assert_eq!(s.desc, "Builds the base fields.");
        assert_eq!(s.algo.len(), MAX_STEPS);
        assert_eq!(s.algo[0], "Read the trace.");
        assert_eq!(s.algo[1], "Add the ids.");
        assert_eq!(s.notes.len(), 2);
        assert_eq!(s.notes[0].kind, NoteKind::Warn);
        assert_eq!(s.notes[1].kind, NoteKind::Info);
        assert!(s.ai);
        assert!(parse_summary(&serde_json::json!({"id": "f2"}), "m").is_err());
    }

    #[test]
    fn doc_fallback() {
        let s = summary_from_doc("Runs one attempt. Then more text.").unwrap();
        assert_eq!(s.desc, "Runs one attempt.");
        assert!(!s.ai);
        assert!(summary_from_doc("  ").is_none());
    }
}
