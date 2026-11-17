//! Skeletons: what a model sees instead of source.
//!
//! A callable's skeleton carries its kind and flags, line count, signature, doc comment,
//! the imports it uses, the calls it makes and an outline of its call flow (blocks and exits
//! summarized, calls folded onto one line per block). A tiny function is sent verbatim,
//! because its code is shorter than any outline. Types carry their signature, doc and
//! members; the file outline carries imports, exports and names.
//!
//! The target is 40% of the source tokens or less. Type skeletons and the file outline are
//! built from facts only (no source, no line numbers), so they are stable cache keys: a
//! function-body edit changes neither.

use std::collections::{HashMap, HashSet};

use iso_core::{FileFacts, FlowItem, FlowUnit, Language, SymbolFact, SymbolKind};

use crate::tokens::estimate_tokens;

/// Limits that keep skeletons small.
#[derive(Clone, Debug)]
pub struct SkeletonOptions {
    /// Bodies with at most this many lines and `tiny_chars` characters are sent verbatim.
    pub tiny_lines: u32,
    pub tiny_chars: usize,
    pub max_flow_lines: usize,
    pub max_flow_depth: usize,
    pub max_calls: usize,
    pub max_doc_chars: usize,
    /// Block labels (`if (…)`) are clipped to this many characters.
    pub max_label_chars: usize,
    /// Return and throw expressions are clipped to this many characters.
    pub max_exit_chars: usize,
    /// Keyword outline lines when no call flow is available.
    pub max_outline_lines: usize,
    pub max_members: usize,
}

impl Default for SkeletonOptions {
    fn default() -> Self {
        SkeletonOptions {
            tiny_lines: 6,
            tiny_chars: 360,
            max_flow_lines: 18,
            max_flow_depth: 4,
            max_calls: 12,
            max_doc_chars: 240,
            max_label_chars: 36,
            max_exit_chars: 24,
            max_outline_lines: 8,
            max_members: 24,
        }
    }
}

/// The skeleton of one symbol.
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolSkeleton {
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub body_hash: u64,
    pub text: String,
}

impl SymbolSkeleton {
    pub fn tokens(&self) -> u64 {
        estimate_tokens(&self.text)
    }
}

/// Skeletons for one file.
#[derive(Clone, Debug, PartialEq)]
pub struct FileSkeleton {
    pub path: String,
    pub lang: Language,
    /// Estimated tokens of the whole source, for the size ratio.
    pub source_tokens: u64,
    /// Functions and methods, in source order.
    pub callables: Vec<SymbolSkeleton>,
    /// Classes, interfaces, type aliases, enums, structs and traits.
    pub types: Vec<SymbolSkeleton>,
    /// The input for the file overview.
    pub outline: String,
}

impl FileSkeleton {
    /// Estimated tokens of every skeleton text in the file.
    pub fn tokens(&self) -> u64 {
        self.callables.iter().map(|s| s.tokens()).sum::<u64>()
            + self.types.iter().map(|s| s.tokens()).sum::<u64>()
            + estimate_tokens(&self.outline)
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A signature with whitespace collapsed and the padding of multi-line parameter lists
/// removed: `f( a, b, )` becomes `f(a, b)`.
fn tidy_sig(s: &str) -> String {
    collapse(s)
        .replace("( ", "(")
        .replace(", )", ")")
        .replace(",)", ")")
        .replace(" )", ")")
}

/// Truncates to `max` characters, with an ellipsis when cut.
pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn kind_flags(sym: &SymbolFact) -> String {
    let mut s = sym.kind.as_str().to_string();
    if sym.is_async {
        s.push_str(", async");
    }
    if sym.exported {
        s.push_str(", exported");
    }
    s
}

/// Imported local names of a file: local name -> (specifier, type_only).
fn import_locals(facts: &FileFacts) -> HashMap<&str, (&str, bool)> {
    let mut m = HashMap::new();
    for imp in &facts.imports {
        for n in &imp.names {
            m.insert(
                n.local(),
                (imp.specifier.as_str(), imp.type_only || n.type_only),
            );
        }
    }
    m
}

fn call_head(callee: &str) -> &str {
    let head = callee.split(['.', '(', '?']).next().unwrap_or("");
    head.strip_prefix("new ").unwrap_or(head)
}

/// Imported names a symbol's body uses other than by calling them (types, values passed
/// around): "type A, b". Called imports already appear in the flow or the calls line, and
/// the file outline says which module each name comes from.
fn uses_line(facts: &FileFacts, locals: &HashMap<&str, (&str, bool)>, qn: &str) -> String {
    let called: HashSet<&str> = facts
        .calls
        .iter()
        .filter(|c| c.caller.as_deref() == Some(qn))
        .map(|c| call_head(&c.callee))
        .collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for r in facts.refs.iter().filter(|r| r.from.as_deref() == Some(qn)) {
        let Some((key, &(_, type_only))) = locals.get_key_value(r.name.as_str()) else {
            continue;
        };
        if called.contains(key) || !seen.insert(key) {
            continue;
        }
        out.push(if type_only || r.type_position {
            format!("type {key}")
        } else {
            key.to_string()
        });
    }
    out.join(", ")
}

fn calls_line(facts: &FileFacts, qn: &str, max: usize) -> String {
    let mut seen = HashSet::new();
    let calls: Vec<&str> = facts
        .calls
        .iter()
        .filter(|c| c.caller.as_deref() == Some(qn))
        .map(|c| c.callee.as_str())
        .filter(|c| seen.insert(*c))
        .collect();
    let mut s = calls
        .iter()
        .take(max)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if calls.len() > max {
        s.push_str(&format!(", +{} more", calls.len() - max));
    }
    s
}

struct FlowWriter<'a> {
    opts: &'a SkeletonOptions,
    lines: Vec<String>,
    truncated: bool,
}

impl FlowWriter<'_> {
    fn push(&mut self, depth: usize, text: String) {
        if self.lines.len() >= self.opts.max_flow_lines {
            self.truncated = true;
            return;
        }
        self.lines
            .push(format!("{}{}", "  ".repeat(depth + 1), text));
    }

    fn flush(&mut self, depth: usize, seq: &mut Vec<String>) {
        if seq.is_empty() {
            return;
        }
        let mut line = String::new();
        for step in seq.drain(..) {
            if !line.is_empty() && line.chars().count() + step.chars().count() > 150 {
                self.push(depth, std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push_str(", ");
            }
            line.push_str(&step);
        }
        if !line.is_empty() {
            self.push(depth, line);
        }
    }

    fn count(items: &[FlowItem]) -> usize {
        items
            .iter()
            .map(|i| match i {
                FlowItem::Call { inner, .. } => 1 + Self::count(inner),
                FlowItem::Block { children, .. } => 1 + Self::count(children),
                FlowItem::Exit { .. } => 1,
            })
            .sum()
    }

    fn items(&mut self, items: &[FlowItem], depth: usize) {
        if depth > self.opts.max_flow_depth {
            let n = Self::count(items);
            if n > 0 {
                self.push(
                    depth,
                    format!("… {n} nested step{}", if n == 1 { "" } else { "s" }),
                );
            }
            return;
        }
        let max = self.opts.max_label_chars;
        let mut seq: Vec<String> = Vec::new();
        for item in items {
            match item {
                FlowItem::Call {
                    label,
                    target,
                    awaited,
                    inner,
                    ..
                } => {
                    // Built-ins (Date.now, JSON.parse) say little about the algorithm.
                    if target.kind != iso_core::FlowTargetKind::Builtin {
                        // Call names are identifiers: never clipped.
                        let label = collapse(label);
                        let step = if *awaited {
                            format!("await {label}")
                        } else {
                            label
                        };
                        if seq.last() != Some(&step) {
                            seq.push(step);
                        }
                    }
                    // Callbacks and deferred bodies run as part of this call; inline bodies
                    // (local helpers, defaults) are other units and are not repeated here.
                    let nested: Vec<&FlowItem> = inner
                        .iter()
                        .filter(|i| {
                            !matches!(i, FlowItem::Block { kind, .. } if *kind == iso_core::BlockKind::Inline)
                        })
                        .collect();
                    if !nested.is_empty() {
                        self.flush(depth, &mut seq);
                        let nested: Vec<FlowItem> = nested.into_iter().cloned().collect();
                        self.items(&nested, depth + 1);
                    }
                }
                FlowItem::Exit {
                    kind,
                    label,
                    early,
                    methods,
                    ..
                } => {
                    let word = match kind {
                        iso_core::ExitKind::Return if early == &Some(true) => "return early",
                        iso_core::ExitKind::Return => "return",
                        iso_core::ExitKind::Throw => "throw",
                    };
                    let label = clip(&collapse(label), self.opts.max_exit_chars);
                    let mut step = if label.is_empty() {
                        word.to_string()
                    } else {
                        format!("{word} {label}")
                    };
                    if let Some(ms) = methods.as_ref().filter(|m| !m.is_empty()) {
                        let names: Vec<&str> = ms.iter().map(|m| m.name.as_str()).collect();
                        step.push_str(&format!(" with methods {}", names.join(", ")));
                    }
                    seq.push(step);
                }
                FlowItem::Block {
                    kw,
                    label,
                    children,
                    ..
                } => {
                    self.flush(depth, &mut seq);
                    let label = clip(&collapse(label), max);
                    let before = self.lines.len();
                    self.push(
                        depth,
                        if label.is_empty() {
                            kw.clone()
                        } else {
                            format!("{kw} {label}")
                        },
                    );
                    self.items(children, depth + 1);
                    // A block with a one-line body fits on its header line:
                    // "if (finalizing): createRunTrace".
                    if self.lines.len() == before + 2 && !self.truncated {
                        let body = self.lines.pop().unwrap_or_default();
                        if let Some(head) = self.lines.last_mut() {
                            head.push_str(": ");
                            head.push_str(body.trim_start());
                        }
                    }
                }
            }
        }
        self.flush(depth, &mut seq);
    }
}

fn flow_outline(unit: &FlowUnit, opts: &SkeletonOptions) -> Vec<String> {
    let mut w = FlowWriter {
        opts,
        lines: Vec::new(),
        truncated: false,
    };
    w.items(&unit.tree, 0);
    if w.truncated {
        w.lines.push("  … more steps".to_string());
    }
    w.lines
}

const OUTLINE_KEYWORDS: &[&str] = &[
    "if ", "if(", "} else", "else", "elif ", "for ", "for(", "foreach", "while ", "while(", "loop",
    "match ", "switch", "case ", "try", "catch", "except", "finally", "with ", "return", "throw ",
    "raise", "yield", "await ", "defer ", "go ", "select ", "unless ",
];

/// Control-flow lines of a body, for languages without a flow walker.
fn keyword_outline(body: &[&str], opts: &SkeletonOptions) -> Vec<String> {
    let mut picked: Vec<(usize, String)> = Vec::new();
    let mut total = 0;
    for line in body {
        let t = line.trim_start();
        if OUTLINE_KEYWORDS.iter().any(|k| t.starts_with(k)) {
            total += 1;
            if picked.len() < opts.max_outline_lines {
                let indent = line.len() - t.len();
                picked.push((indent, clip(&collapse(t), 64)));
            }
        }
    }
    let min = picked.iter().map(|(i, _)| *i).min().unwrap_or(0);
    let mut out: Vec<String> = picked
        .into_iter()
        .map(|(i, t)| format!("  {}{}", "  ".repeat(((i - min) / 2).min(4)), t))
        .collect();
    if total > out.len() {
        out.push(format!("  … {} more", total - out.len()));
    }
    out
}

fn dedent(lines: &[&str]) -> Vec<String> {
    let min = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            // Cut at most this line's own leading whitespace, so the cut is always on a
            // character boundary.
            let ws = l.len() - l.trim_start().len();
            l[min.min(ws)..].trim_end().to_string()
        })
        .collect()
}

fn callable_text(
    sym: &SymbolFact,
    facts: &FileFacts,
    locals: &HashMap<&str, (&str, bool)>,
    lines: &[&str],
    unit: Option<&FlowUnit>,
    opts: &SkeletonOptions,
) -> String {
    let n_lines = sym.end_line.saturating_sub(sym.start_line) + 1;
    let mut out = vec![format!("{}, {n_lines} lines", kind_flags(sym))];
    let lo = (sym.start_line.max(1) - 1) as usize;
    let hi = (sym.end_line as usize).min(lines.len());
    let body: &[&str] = if lo < hi { &lines[lo..hi] } else { &[] };
    if let Some(doc) = sym.doc.as_deref().map(collapse).filter(|d| !d.is_empty()) {
        out.push(format!("doc: {}", clip(&doc, opts.max_doc_chars)));
    }
    let body_chars: usize = body.iter().map(|l| l.trim().chars().count() + 1).sum();
    if !body.is_empty() && n_lines <= opts.tiny_lines && body_chars <= opts.tiny_chars {
        out.push("code:".to_string());
        out.extend(dedent(body).into_iter().map(|l| format!("  {l}")));
        return out.join("\n");
    }
    let sig = tidy_sig(&sym.signature);
    if !sig.is_empty() {
        out.push(format!("sig: {sig}"));
    } else {
        out.push(format!("sig: {}({})", sym.name, sym.params.join(", ")));
    }
    let uses = uses_line(facts, locals, &sym.qualified_name);
    if !uses.is_empty() {
        out.push(format!("uses: {uses}"));
    }
    // With a call flow, the flow already lists every call in run order.
    if unit.is_none() {
        let calls = calls_line(facts, &sym.qualified_name, opts.max_calls);
        if !calls.is_empty() {
            out.push(format!("calls: {calls}"));
        }
    }
    let flow = match unit {
        Some(u) => flow_outline(u, opts),
        None => keyword_outline(body.get(1..).unwrap_or(&[]), opts),
    };
    if !flow.is_empty() {
        out.push(if unit.is_some() { "flow:" } else { "outline:" }.to_string());
        out.extend(flow);
    }
    out.join("\n")
}

/// The type skeleton. Built from facts only, so it is a stable cache key.
fn type_text(sym: &SymbolFact, facts: &FileFacts, opts: &SkeletonOptions) -> String {
    let mut out = vec![kind_flags(sym)];
    let sig = tidy_sig(&sym.signature);
    if !sig.is_empty() {
        out.push(format!("sig: {sig}"));
    }
    if let Some(doc) = sym.doc.as_deref().map(collapse).filter(|d| !d.is_empty()) {
        out.push(format!("doc: {}", clip(&doc, opts.max_doc_chars)));
    }
    let members: Vec<String> = facts
        .symbols
        .iter()
        .filter(|s| s.parent.as_deref() == Some(sym.qualified_name.as_str()))
        .map(|s| {
            if s.kind.is_callable() {
                format!("{}()", s.name)
            } else {
                s.name.clone()
            }
        })
        .collect();
    if !members.is_empty() {
        let mut m = members
            .iter()
            .take(opts.max_members)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if members.len() > opts.max_members {
            m.push_str(&format!(", +{} more", members.len() - opts.max_members));
        }
        out.push(format!("members: {m}"));
    }
    out.join("\n")
}

/// The file outline, the overview's input. Built from facts only.
pub fn file_outline(lang: Language, facts: &FileFacts) -> String {
    let mut out = vec![format!("language: {}", lang.label())];
    let mut imports: Vec<String> = Vec::new();
    for imp in facts.imports.iter().take(16) {
        imports.push(imp.specifier.clone());
    }
    if facts.imports.len() > 16 {
        imports.push(format!("+{} more", facts.imports.len() - 16));
    }
    if !imports.is_empty() {
        out.push(format!("imports: {}", imports.join(", ")));
    }
    let top = |s: &&SymbolFact| s.parent.is_none();
    let exports: Vec<String> = facts
        .symbols
        .iter()
        .filter(top)
        .filter(|s| s.exported)
        .take(16)
        .map(|s| format!("{} {}", s.kind.as_str(), s.name))
        .filter(|s| !s.is_empty())
        .collect();
    if !exports.is_empty() {
        out.push(format!("exports: {}", exports.join(", ")));
    }
    let names = |pred: &dyn Fn(&SymbolFact) -> bool| -> Vec<&str> {
        facts
            .symbols
            .iter()
            .filter(top)
            .filter(|s| !s.exported && pred(s))
            .map(|s| s.name.as_str())
            .take(24)
            .collect()
    };
    let fns = names(&|s| s.kind.is_callable());
    if !fns.is_empty() {
        out.push(format!("functions: {}", fns.join(", ")));
    }
    let types = names(&|s| s.kind.is_type_like());
    if !types.is_empty() {
        out.push(format!("types: {}", types.join(", ")));
    }
    let methods = facts
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .count();
    if methods > 0 {
        out.push(format!("methods: {methods}"));
    }
    if facts.has_errors {
        out.push("parse errors: yes".to_string());
    }
    out.join("\n")
}

/// Builds the skeletons of one file. `flows` may be empty; callables without a flow unit
/// get a keyword outline from the source instead.
pub fn build_skeleton(
    path: &str,
    lang: Language,
    source: &str,
    facts: &FileFacts,
    flows: &[FlowUnit],
    opts: &SkeletonOptions,
) -> FileSkeleton {
    let lines: Vec<&str> = source.lines().collect();
    let locals = import_locals(facts);
    let units: HashMap<&str, &FlowUnit> = flows.iter().map(|u| (u.id.as_str(), u)).collect();
    let mut callables = Vec::new();
    let mut types = Vec::new();
    for sym in &facts.symbols {
        if sym.kind.is_callable() {
            let unit = units.get(sym.qualified_name.as_str()).copied();
            callables.push(SymbolSkeleton {
                qualified_name: sym.qualified_name.clone(),
                kind: sym.kind,
                body_hash: sym.body_hash,
                text: callable_text(sym, facts, &locals, &lines, unit, opts),
            });
        } else if sym.kind.is_type_like() {
            types.push(SymbolSkeleton {
                qualified_name: sym.qualified_name.clone(),
                kind: sym.kind,
                body_hash: sym.body_hash,
                text: type_text(sym, facts, opts),
            });
        }
    }
    FileSkeleton {
        path: path.to_string(),
        lang,
        source_tokens: estimate_tokens(source),
        callables,
        types,
        outline: file_outline(lang, facts),
    }
}

/// Type skeletons only (no source needed), for cache lookups.
pub(crate) fn type_skeletons(facts: &FileFacts, opts: &SkeletonOptions) -> Vec<SymbolSkeleton> {
    facts
        .symbols
        .iter()
        .filter(|s| s.kind.is_type_like())
        .map(|sym| SymbolSkeleton {
            qualified_name: sym.qualified_name.clone(),
            kind: sym.kind,
            body_hash: sym.body_hash,
            text: type_text(sym, facts, opts),
        })
        .collect()
}
