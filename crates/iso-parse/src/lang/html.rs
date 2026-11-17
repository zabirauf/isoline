//! HTML extractor. Owned by workstream D (see CONTRACTS.md).
//!
//! - `<script src>` and every `<link href>` become `link` imports (specifier as written).
//! - Elements with an `id` become `selector` symbols named `#id` spanning the element;
//!   each distinct class becomes a `selector` symbol `.class` on the line where it first
//!   appears (start and end line are that line). Duplicate ids keep the first. Values that
//!   look like template syntax (`{{ x }}`, `<%= %>`, `${x}`) are skipped.
//! - Inline `<script>` and `<style>` bodies are not analysed.

use std::collections::HashSet;
use std::path::Path;

use iso_core::{FileFacts, ImportFact, ImportKind, Language, SymbolFact, SymbolKind};
use tree_sitter::Node;

use crate::Extractor;
use crate::util::{clip, end_line, line, node_hash, text};

pub struct HtmlExtractor;
pub static HTML: HtmlExtractor = HtmlExtractor;

impl Extractor for HtmlExtractor {
    fn language(&self) -> Language {
        Language::Html
    }
    fn extract(&self, _path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts {
        let root = tree.root_node();
        let mut facts = FileFacts {
            has_errors: root.has_error(),
            ..FileFacts::default()
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            match n.kind() {
                "start_tag" | "self_closing_tag" => {
                    tag(n, source, &mut facts, &mut seen);
                    continue;
                }
                "raw_text" | "comment" | "text" | "end_tag" => continue,
                _ => {}
            }
            let mut c = n.walk();
            let children: Vec<Node> = n.named_children(&mut c).collect();
            stack.extend(children.into_iter().rev());
        }
        facts
    }
}

fn tag(n: Node, src: &str, facts: &mut FileFacts, seen: &mut HashSet<String>) {
    let mut name = String::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        match ch.kind() {
            "tag_name" => name = text(ch, src).to_ascii_lowercase(),
            "attribute" => {
                let mut key = String::new();
                let mut value = String::new();
                let mut ac = ch.walk();
                for part in ch.named_children(&mut ac) {
                    match part.kind() {
                        "attribute_name" => key = text(part, src).to_ascii_lowercase(),
                        "attribute_value" => value = text(part, src).to_string(),
                        "quoted_attribute_value" => {
                            value = part
                                .named_child(0)
                                .map(|v| text(v, src).to_string())
                                .unwrap_or_default();
                        }
                        _ => {}
                    }
                }
                attrs.push((key, value));
            }
            _ => {}
        }
    }
    let attr = |k: &str| {
        attrs
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.trim())
    };
    let ln = line(n);
    let link = match name.as_str() {
        "script" => attr("src"),
        "link" => attr("href"),
        _ => None,
    };
    if let Some(spec) = link.filter(|s| !s.is_empty()) {
        facts.imports.push(ImportFact {
            specifier: spec.to_string(),
            kind: ImportKind::Link,
            names: Vec::new(),
            type_only: false,
            line: ln,
        });
    }
    // The element the tag opens (a self-closing tag is its own element).
    let element = n
        .parent()
        .filter(|p| p.kind() != "document" && p.start_byte() == n.start_byte())
        .unwrap_or(n);
    let signature = clip(text(n, src), 200);
    if let Some(id) = attr("id").filter(|v| plain_token(v)) {
        let sel = format!("#{id}");
        if seen.insert(sel.clone()) {
            facts.symbols.push(selector(
                sel,
                line(element),
                end_line(element),
                &signature,
                element,
                src,
            ));
        }
    }
    if let Some(classes) = attr("class") {
        let classes = strip_templates(classes);
        for class in classes.split_whitespace().filter(|v| plain_token(v)) {
            let sel = format!(".{class}");
            if seen.insert(sel.clone()) {
                facts
                    .symbols
                    .push(selector(sel, ln, ln, &signature, n, src));
            }
        }
    }
}

fn selector(name: String, start: u32, end: u32, signature: &str, n: Node, src: &str) -> SymbolFact {
    SymbolFact {
        qualified_name: name.clone(),
        name,
        kind: SymbolKind::Selector,
        start_line: start,
        end_line: end,
        signature: signature.to_string(),
        doc: None,
        exported: false,
        is_async: false,
        params: Vec::new(),
        parent: None,
        body_hash: node_hash(n, src),
    }
}

/// Removes `{{ … }}`, `{% … %}`, `<% … %>` and `${ … }` spans.
fn strip_templates(v: &str) -> String {
    const PAIRS: [(&str, &str); 4] = [("{{", "}}"), ("{%", "%}"), ("<%", "%>"), ("${", "}")];
    let mut out = String::new();
    let mut rest = v;
    'outer: while !rest.is_empty() {
        for (open, close) in PAIRS {
            if let Some(after) = rest.strip_prefix(open) {
                rest = match after.find(close) {
                    Some(i) => &after[i + close.len()..],
                    None => "",
                };
                out.push(' ');
                continue 'outer;
            }
        }
        let ch = rest.chars().next().unwrap_or(' ');
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// Not empty and free of template syntax.
fn plain_token(v: &str) -> bool {
    !v.is_empty() && !v.contains(['{', '}', '<', '>', '$', '(', ')', '"', '\'', '=', ' '])
}
