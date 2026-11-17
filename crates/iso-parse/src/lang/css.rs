//! CSS extractor. Owned by workstream D (see CONTRACTS.md).
//!
//! - Every rule set is a `selector` symbol named by its selector text (whitespace collapsed,
//!   at most 80 chars). `@media`, `@supports`, `@container`, `@layer` and other block at-rules
//!   are `selector` symbols named `@media <query>`; rules inside them (and nested rules) have
//!   them as parent. `@keyframes name` is a symbol without children.
//! - CSS and HTML qualified names join nesting levels with a space, not a dot (selectors
//!   contain dots): `@media (max-width: 600px) .nav a`. Repeats get `#2`, `#3`.
//! - `@import "x.css"` and `@import url(x.css)` are `link` imports.

use std::path::Path;

use iso_core::{FileFacts, ImportFact, ImportKind, Language, SymbolFact, SymbolKind};
use tree_sitter::Node;

use crate::Extractor;
use crate::util::{UniqueNames, clip, end_line, line, node_hash, text};

pub struct CssExtractor;
pub static CSS: CssExtractor = CssExtractor;

impl Extractor for CssExtractor {
    fn language(&self) -> Language {
        Language::Css
    }
    fn extract(&self, _path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts {
        let root = tree.root_node();
        let mut facts = FileFacts {
            has_errors: root.has_error(),
            ..FileFacts::default()
        };
        let mut unique = UniqueNames::default();
        // (node, index of the parent symbol)
        let mut stack: Vec<(Node, Option<usize>)> = vec![(root, None)];
        while let Some((n, parent)) = stack.pop() {
            let mut inner = parent;
            let mut descend = true;
            match n.kind() {
                "rule_set" => {
                    let sel = n
                        .named_children(&mut n.walk())
                        .find(|c| c.kind() == "selectors")
                        .map(|s| text(s, source))
                        .unwrap_or("");
                    let name = clip(sel, 80);
                    let sig = clip(sel, 200);
                    inner = Some(push(&mut facts, &mut unique, n, name, sig, parent, source));
                }
                "media_statement" | "supports_statement" | "at_rule" | "scope_statement" => {
                    let block = n
                        .named_children(&mut n.walk())
                        .find(|c| c.kind() == "block");
                    let end = block.map(|b| b.start_byte()).unwrap_or(n.end_byte());
                    let head = source.get(n.start_byte()..end).unwrap_or("");
                    let head = head.trim_end().trim_end_matches(';').trim_end();
                    let name = clip(head, 80);
                    let sig = clip(head, 200);
                    if block.is_some() {
                        inner = Some(push(&mut facts, &mut unique, n, name, sig, parent, source));
                    } else {
                        descend = false;
                    }
                }
                "keyframes_statement" => {
                    let end = n
                        .named_children(&mut n.walk())
                        .find(|c| c.kind() == "keyframe_block_list")
                        .map(|b| b.start_byte())
                        .unwrap_or(n.end_byte());
                    let head = source.get(n.start_byte()..end).unwrap_or("").trim_end();
                    let name = clip(head, 80);
                    let sig = clip(head, 200);
                    push(&mut facts, &mut unique, n, name, sig, parent, source);
                    descend = false;
                }
                "import_statement" => {
                    if let Some(spec) = import_specifier(n, source) {
                        facts.imports.push(ImportFact {
                            specifier: spec,
                            kind: ImportKind::Link,
                            names: Vec::new(),
                            type_only: false,
                            line: line(n),
                        });
                    }
                    descend = false;
                }
                "declaration" | "comment" | "js_comment" | "selectors" => descend = false,
                _ => {}
            }
            if descend {
                let mut c = n.walk();
                let children: Vec<Node> = n.named_children(&mut c).collect();
                stack.extend(children.into_iter().rev().map(|ch| (ch, inner)));
            }
        }
        facts
    }
}

fn push(
    facts: &mut FileFacts,
    unique: &mut UniqueNames,
    n: Node,
    name: String,
    signature: String,
    parent: Option<usize>,
    src: &str,
) -> usize {
    let parent_name = parent.map(|i| facts.symbols[i].qualified_name.clone());
    let q = match &parent_name {
        Some(p) => format!("{p} {name}"),
        None => name.clone(),
    };
    let qualified_name = unique.claim(q, SymbolKind::Selector);
    facts.symbols.push(SymbolFact {
        name,
        qualified_name,
        kind: SymbolKind::Selector,
        start_line: line(n),
        end_line: end_line(n),
        signature,
        doc: None,
        exported: false,
        is_async: false,
        params: Vec::new(),
        parent: parent_name,
        body_hash: node_hash(n, src),
    });
    facts.symbols.len() - 1
}

/// The URL of `@import "x"` / `@import url(x)` / `@import url("x")`.
fn import_specifier(n: Node, src: &str) -> Option<String> {
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        match ch.kind() {
            "string_value" => return Some(unquote(text(ch, src))),
            "call_expression" => {
                let args = ch
                    .named_children(&mut ch.walk())
                    .find(|a| a.kind() == "arguments")?;
                let first = args.named_child(0)?;
                return Some(unquote(text(first, src)));
            }
            _ => {}
        }
    }
    None
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches(['"', '\'']).trim().to_string()
}
