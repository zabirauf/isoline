//! Generic tier-2 extractor driven only by query files. Owned by workstream N.
//!
//! Adding a language means adding its grammar (to `crate::grammar`), a
//! `queries/<lang>/tags.scm` and a `queries/<lang>/imports.scm`, and one row in [`SPECS`].
//! There is no per-language Rust code.
//!
//! `tags.scm` captures (tree-sitter tags conventions plus a few extras):
//! - `@definition.<kind>` on the declaration node and `@name` on its name. Kinds:
//!   `function`, `method`, `constructor` (→ method), `class`, `record` (→ class),
//!   `interface`, `module`/`namespace`/`package` (→ module), `type`/`delegate` (→ type),
//!   `struct`, `union` (→ struct), `enum`, `trait`, `constant`, `variable`, `field` and
//!   `property` (→ field); anything else is a variable.
//! - `@parent` (optional): text naming the owner when it is not lexically enclosing, such as
//!   a Go method's receiver type. Otherwise the parent is the innermost enclosing
//!   definition. Qualified names are dotted.
//! - `@exported` (optional) marks the definition exported; or the pattern property
//!   `(#set! exported "capitalized")` (name starts upper-case) or
//!   `(#set! exported "modifier:<word>")` (a child whose kind contains `modifier` has the
//!   word `<word>`).
//! - `@reference.call` on a call node with `@name` (the called identifier) and optional
//!   `@receiver`. The callee text is the call node's text up to its `arguments` field.
//!
//! `imports.scm` captures: `@import.specifier` (quotes stripped), optional `@import.alias`,
//! `@import.name` (repeatable) and `@import.wildcard` (names become `*`). Without names the
//! imported name is the specifier's last segment.
//!
//! Docs are the comment nodes directly above a definition (or above its parent when the
//! definition is its parent's first named child, as in Go's `type X struct`). Signatures are
//! the text up to the `body` field, or the first line. Repeated qualified names (overloads)
//! get `#2`, `#3`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use iso_core::{
    CallFact, FileFacts, ImportFact, ImportKind, ImportedName, Language, SymbolFact, SymbolKind,
};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

use crate::Extractor;
use crate::util::{
    UniqueNames, clip, end_line, head_until, line, node_hash, strip_comment_markers, text,
};

pub struct GenericExtractor {
    pub lang: Language,
}
pub static GO: GenericExtractor = GenericExtractor { lang: Language::Go };
pub static JAVA: GenericExtractor = GenericExtractor {
    lang: Language::Java,
};
pub static CSHARP: GenericExtractor = GenericExtractor {
    lang: Language::CSharp,
};

/// The only per-language table: language → query files.
pub struct Spec {
    pub lang: Language,
    pub tags: &'static str,
    pub imports: &'static str,
}

pub const SPECS: &[Spec] = &[
    Spec {
        lang: Language::Go,
        tags: include_str!("../../queries/go/tags.scm"),
        imports: include_str!("../../queries/go/imports.scm"),
    },
    Spec {
        lang: Language::Java,
        tags: include_str!("../../queries/java/tags.scm"),
        imports: include_str!("../../queries/java/imports.scm"),
    },
    Spec {
        lang: Language::CSharp,
        tags: include_str!("../../queries/csharp/tags.scm"),
        imports: include_str!("../../queries/csharp/imports.scm"),
    },
];

#[derive(Clone, Copy)]
enum Role {
    Def(SymbolKind),
    Name,
    Parent,
    Receiver,
    Exported,
    Call,
    Spec,
    Alias,
    ImportName,
    Wildcard,
    Ignore,
}

struct Compiled {
    tags: Query,
    tag_roles: Vec<Role>,
    imports: Query,
    import_roles: Vec<Role>,
}

/// Queries compiled once per process. A query that fails to compile leaves its language
/// with empty facts; `compile_errors` reports it (a unit test asserts there are none).
static COMPILED: LazyLock<Vec<(Language, Result<Compiled, String>)>> =
    LazyLock::new(|| SPECS.iter().map(|s| (s.lang, compile(s))).collect());

fn compile(s: &Spec) -> Result<Compiled, String> {
    let g = crate::grammar(s.lang).ok_or("no grammar")?;
    let tags = Query::new(&g, s.tags).map_err(|e| format!("{:?} tags.scm: {e}", s.lang))?;
    let imports =
        Query::new(&g, s.imports).map_err(|e| format!("{:?} imports.scm: {e}", s.lang))?;
    let tag_roles = tags.capture_names().iter().map(|n| role(n)).collect();
    let import_roles = imports.capture_names().iter().map(|n| role(n)).collect();
    Ok(Compiled {
        tags,
        tag_roles,
        imports,
        import_roles,
    })
}

/// Query compile errors, for tests.
pub fn compile_errors() -> Vec<String> {
    COMPILED
        .iter()
        .filter_map(|(_, c)| c.as_ref().err().cloned())
        .collect()
}

fn role(capture: &str) -> Role {
    if let Some(kind) = capture.strip_prefix("definition.") {
        return Role::Def(kind_of(kind));
    }
    match capture {
        "name" => Role::Name,
        "parent" => Role::Parent,
        "receiver" => Role::Receiver,
        "exported" => Role::Exported,
        "reference.call" => Role::Call,
        "import.specifier" => Role::Spec,
        "import.alias" => Role::Alias,
        "import.name" => Role::ImportName,
        "import.wildcard" => Role::Wildcard,
        _ => Role::Ignore,
    }
}

fn kind_of(tag: &str) -> SymbolKind {
    match tag {
        "constructor" => SymbolKind::Method,
        "record" => SymbolKind::Class,
        "namespace" | "package" => SymbolKind::Module,
        "delegate" | "typealias" => SymbolKind::Type,
        "union" => SymbolKind::Struct,
        "property" => SymbolKind::Field,
        "macro" => SymbolKind::Function,
        other => SymbolKind::parse(other),
    }
}

struct RawDef<'t> {
    node: Node<'t>,
    name: Node<'t>,
    kind: SymbolKind,
    parent: Option<Node<'t>>,
    exported: bool,
}

struct RawCall<'t> {
    node: Node<'t>,
    name: Node<'t>,
    receiver: Option<Node<'t>>,
}

impl Extractor for GenericExtractor {
    fn language(&self) -> Language {
        self.lang
    }
    fn extract(&self, _path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts {
        let root = tree.root_node();
        let mut facts = FileFacts {
            has_errors: root.has_error(),
            ..FileFacts::default()
        };
        let Some(Ok(q)) = COMPILED
            .iter()
            .find(|(l, _)| *l == self.lang)
            .map(|(_, c)| c.as_ref())
        else {
            return facts;
        };
        let (defs, calls) = collect_tags(q, root, source);
        outline(&defs, &calls, source, &mut facts);
        facts.imports = collect_imports(q, root, source);
        facts
    }
}

fn collect_tags<'t>(
    q: &Compiled,
    root: Node<'t>,
    src: &str,
) -> (Vec<RawDef<'t>>, Vec<RawCall<'t>>) {
    let mut defs: Vec<RawDef<'t>> = Vec::new();
    let mut index: HashMap<(usize, usize), usize> = HashMap::new();
    let mut calls = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&q.tags, root, src.as_bytes());
    while let Some(m) = matches.next() {
        let (mut def, mut name, mut parent, mut receiver, mut call) =
            (None, None, None, None, None);
        let mut exported = false;
        for c in m.captures() {
            match q.tag_roles[c.index as usize] {
                Role::Def(k) => def = Some((c.node, k)),
                Role::Name => name = Some(c.node),
                Role::Parent => parent = Some(c.node),
                Role::Receiver => receiver = Some(c.node),
                Role::Exported => exported = true,
                Role::Call => call = Some(c.node),
                _ => {}
            }
        }
        if let (Some((node, kind)), Some(name)) = (def, name) {
            for p in q.tags.property_settings(m.pattern_index) {
                if &*p.key == "exported" {
                    exported |= export_rule(p.value.as_deref().unwrap_or(""), node, name, src);
                }
            }
            match index.get(&(node.id(), name.id())) {
                Some(&i) => defs[i].exported |= exported,
                None => {
                    index.insert((node.id(), name.id()), defs.len());
                    defs.push(RawDef {
                        node,
                        name,
                        kind,
                        parent,
                        exported,
                    });
                }
            }
        } else if let (Some(node), Some(name)) = (call, name) {
            calls.push(RawCall {
                node,
                name,
                receiver,
            });
        }
    }
    defs.sort_by_key(|d| (d.node.start_byte(), std::cmp::Reverse(d.node.end_byte())));
    calls.sort_by_key(|c| (c.name.start_byte(), c.node.start_byte()));
    calls.dedup_by_key(|c| c.node.id());
    (defs, calls)
}

fn export_rule(rule: &str, def: Node, name: Node, src: &str) -> bool {
    if rule == "capitalized" {
        return text(name, src)
            .chars()
            .next()
            .is_some_and(char::is_uppercase);
    }
    if let Some(word) = rule.strip_prefix("modifier:") {
        let mut c = def.walk();
        return def.named_children(&mut c).any(|ch| {
            ch.kind().contains("modifier") && text(ch, src).split_whitespace().any(|w| w == word)
        });
    }
    false
}

fn outline(defs: &[RawDef], calls: &[RawCall], src: &str, facts: &mut FileFacts) {
    let mut unique = UniqueNames::default();
    // Open definitions: (end byte, symbol index).
    let mut open: Vec<(usize, usize)> = Vec::new();
    for d in defs {
        let start = d.node.start_byte();
        while open.last().is_some_and(|(end, _)| *end <= start) {
            open.pop();
        }
        let enclosing = open
            .last()
            .map(|(_, i)| facts.symbols[*i].qualified_name.clone());
        let name = text(d.name, src).split_whitespace().collect::<String>();
        let parent = match (d.parent, enclosing) {
            (Some(p), Some(e)) => Some(format!("{e}.{}", text(p, src))),
            (Some(p), None) => Some(text(p, src).to_string()),
            (None, e) => e,
        };
        let q = match &parent {
            Some(p) => format!("{p}.{name}"),
            None => name.clone(),
        };
        let body = d.node.child_by_field_name("body");
        let signature = match body {
            Some(_) => head_until(start, d.node, body, src),
            None => {
                let first = text(d.node, src).lines().next().unwrap_or("");
                clip(
                    first
                        .trim_end()
                        .trim_end_matches(['{', ';', ':'])
                        .trim_end(),
                    200,
                )
            }
        };
        facts.symbols.push(SymbolFact {
            qualified_name: unique.claim(q, d.kind),
            name,
            kind: d.kind,
            start_line: line(d.node),
            end_line: end_line(d.node),
            signature,
            doc: doc_above(d.node, src),
            exported: d.exported,
            is_async: false,
            params: Vec::new(),
            parent,
            body_hash: node_hash(d.node, src),
        });
        open.push((d.node.end_byte(), facts.symbols.len() - 1));
    }

    // Callers: sweep calls and definitions in byte order.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut di = 0;
    for c in calls {
        let pos = c.name.start_byte();
        while di < defs.len() && defs[di].node.start_byte() <= pos {
            let s = defs[di].node.start_byte();
            while open.last().is_some_and(|(end, _)| *end <= s) {
                open.pop();
            }
            open.push((defs[di].node.end_byte(), di));
            di += 1;
        }
        while open.last().is_some_and(|(end, _)| *end <= pos) {
            open.pop();
        }
        let caller = open
            .iter()
            .rev()
            .find(|(_, i)| facts.symbols[*i].kind.is_callable())
            .map(|(_, i)| facts.symbols[*i].qualified_name.clone());
        let end = c
            .node
            .child_by_field_name("arguments")
            .map(|a| a.start_byte())
            .unwrap_or(c.node.end_byte());
        let callee = clip(src.get(c.node.start_byte()..end).unwrap_or(""), 200);
        facts.calls.push(CallFact {
            caller,
            callee,
            name: text(c.name, src).to_string(),
            receiver: c.receiver.map(|r| clip(text(r, src), 200)),
            line: line(c.name),
        });
    }
    facts.calls.sort_by_key(|c| c.line);
}

/// Comment nodes directly above `n` (no blank line in between).
fn doc_above(n: Node, src: &str) -> Option<String> {
    let mut target = n;
    if n.prev_named_sibling().is_none()
        && let Some(p) = n.parent()
        && p.named_child(0).is_some_and(|c| c.id() == n.id())
        && p.parent().is_some()
    {
        target = p;
    }
    let mut docs = Vec::new();
    let mut next_row = target.start_position().row;
    let mut prev = target.prev_sibling();
    while let Some(p) = prev {
        if !p.kind().contains("comment") || p.end_position().row + 1 < next_row {
            break;
        }
        docs.push(text(p, src));
        next_row = p.start_position().row;
        prev = p.prev_sibling();
    }
    docs.reverse();
    strip_comment_markers(docs)
}

fn collect_imports(q: &Compiled, root: Node, src: &str) -> Vec<ImportFact> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&q.imports, root, src.as_bytes());
    while let Some(m) = matches.next() {
        let (mut spec, mut alias, mut wildcard) = (None, None, false);
        let mut names = Vec::new();
        for c in m.captures() {
            match q.import_roles[c.index as usize] {
                Role::Spec => spec = Some(c.node),
                Role::Alias => alias = Some(text(c.node, src).to_string()),
                Role::ImportName => names.push(text(c.node, src).to_string()),
                Role::Wildcard => wildcard = true,
                _ => {}
            }
        }
        let Some(spec) = spec else { continue };
        if !seen.insert(spec.id()) {
            continue;
        }
        let specifier = text(spec, src)
            .trim()
            .trim_matches(['"', '`', '\''])
            .split_whitespace()
            .collect::<String>();
        let names = if wildcard {
            vec![ImportedName {
                name: "*".to_string(),
                alias,
                type_only: false,
            }]
        } else if !names.is_empty() {
            names
                .into_iter()
                .map(|name| ImportedName {
                    name,
                    alias: None,
                    type_only: false,
                })
                .collect()
        } else {
            let last = specifier
                .rsplit(['/', '.', ':'])
                .next()
                .unwrap_or(&specifier)
                .to_string();
            vec![ImportedName {
                name: last,
                alias,
                type_only: false,
            }]
        };
        out.push(ImportFact {
            specifier,
            kind: ImportKind::Static,
            names,
            type_only: false,
            line: line(spec),
        });
    }
    out.sort_by_key(|i| i.line);
    out
}
