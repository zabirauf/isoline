//! Python extractor. Owned by workstream C (see CONTRACTS.md).
//!
//! Two passes over the tree, both iterative (no recursion, so deeply nested expressions
//! cannot overflow the stack):
//!
//! 1. Module-level names: imports, `def`, `class`, `type` aliases and UPPER_CASE constants,
//!    looking through `if`/`try`/`with` blocks at module level. Also reads `__all__`.
//! 2. One depth-first walk that emits symbols, imports, calls and references.
//!
//! Choices where the contract leaves room:
//! - `import a.b` gives one `ImportFact` per imported module (`import a, b` gives two), with
//!   `names = [{name: "a.b"}]`; the local binding Python creates is the first segment `a`.
//!   `import a.b as c` gives `{name: "a.b", alias: "c"}`. `from m import x as y, z` is one
//!   fact with two names; `from m import *` has the name `*`.
//! - Imports inside `if TYPE_CHECKING:` are `type_only`.
//! - Decorators belong to the function they decorate: they are part of its signature and
//!   start line, and calls and references in them have that function as caller.
//! - `refs` holds non-call uses only. A direct callee (`foo()`) and the root of an attribute
//!   callee (`np` in `np.array()`) are covered by the call fact instead.
//! - Parameters (including those of enclosing functions) shadow file-level names.
//! - Methods, nested functions and class members are never `exported`.

use std::collections::HashSet;
use std::path::Path;

use iso_core::{
    CallFact, FileFacts, ImportFact, ImportKind, ImportedName, Language, RefFact, SymbolFact,
    SymbolKind,
};
use tree_sitter::Node;

use crate::Extractor;
use crate::util::{UniqueNames, clean_doc, clip, head_until, line, node_hash, text};

pub struct PythonExtractor;
pub static PYTHON: PythonExtractor = PythonExtractor;

impl Extractor for PythonExtractor {
    fn language(&self) -> Language {
        Language::Python
    }
    fn extract(&self, _path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts {
        let root = tree.root_node();
        let mut x = Py::new(source);
        x.collect_module_names(root);
        x.walk(root);
        x.facts.has_errors = root.has_error();
        x.facts.calls.sort_by_key(|c| c.line);
        x.facts
    }
}

/// Context carried down the walk. Small and `Copy`.
#[derive(Clone, Copy, Default)]
struct Ctx {
    /// Innermost enclosing symbol (function, method, class or constant), for refs and nesting.
    sym: Option<usize>,
    /// Innermost enclosing function or method, for calls.
    func: Option<usize>,
    /// Definitions here are module-level.
    module_level: bool,
    /// Directly inside a class body (functions here are methods).
    in_class: bool,
    /// Inside a type annotation.
    type_pos: bool,
    /// Inside `if TYPE_CHECKING:`.
    type_checking: bool,
    /// Index into `Py::shadows` (0 is the empty set).
    shadow: usize,
}

struct Py<'a> {
    src: &'a str,
    facts: FileFacts,
    /// Names bound at module level (import locals and top-level declarations).
    module_names: HashSet<String>,
    /// Names listed in `__all__`, when the module defines it.
    all: Option<HashSet<String>>,
    shadows: Vec<HashSet<String>>,
    unique: UniqueNames,
    seen_refs: HashSet<(Option<usize>, String, bool, u32)>,
}

impl<'a> Py<'a> {
    fn new(src: &'a str) -> Self {
        Py {
            src,
            facts: FileFacts::default(),
            module_names: HashSet::new(),
            all: None,
            shadows: vec![HashSet::new()],
            unique: UniqueNames::default(),
            seen_refs: HashSet::new(),
        }
    }

    fn t(&self, n: Node) -> &'a str {
        text(n, self.src)
    }

    // ---- pass 1 ----

    fn collect_module_names(&mut self, root: Node) {
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            match n.kind() {
                "module"
                | "block"
                | "if_statement"
                | "elif_clause"
                | "else_clause"
                | "try_statement"
                | "except_clause"
                | "except_group_clause"
                | "finally_clause"
                | "with_statement"
                | "for_statement"
                | "while_statement" => {
                    let mut c = n.walk();
                    for ch in n.named_children(&mut c) {
                        stack.push(ch);
                    }
                }
                "decorated_definition" => {
                    if let Some(d) = n.child_by_field_name("definition") {
                        stack.push(d);
                    }
                }
                "function_definition" | "class_definition" => {
                    if let Some(name) = n.child_by_field_name("name") {
                        self.module_names.insert(self.t(name).to_string());
                    }
                }
                "type_alias_statement" => {
                    if let Some(name) = type_alias_name(n) {
                        self.module_names.insert(self.t(name).to_string());
                    }
                }
                "import_statement" | "import_from_statement" | "future_import_statement" => {
                    for imp in self.import_facts(n, false) {
                        for name in &imp.names {
                            let local = name.local();
                            if local != "*" {
                                let first = local.split('.').next().unwrap_or(local);
                                self.module_names.insert(first.to_string());
                            }
                        }
                    }
                }
                "expression_statement" => {
                    let Some(a) = n.named_child(0) else { continue };
                    if a.kind() != "assignment" && a.kind() != "augmented_assignment" {
                        continue;
                    }
                    let Some(left) = a.child_by_field_name("left") else {
                        continue;
                    };
                    let name = self.t(left);
                    if left.kind() == "identifier" && is_constant_name(name) {
                        self.module_names.insert(name.to_string());
                    }
                    if name == "__all__"
                        && let Some(right) = a.child_by_field_name("right")
                    {
                        let all = self.all.get_or_insert_with(HashSet::new);
                        let mut c = right.walk();
                        for s in right.named_children(&mut c) {
                            if s.kind() == "string" {
                                all.insert(string_value(s, self.src));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // ---- pass 2 ----

    fn walk(&mut self, root: Node<'a>) {
        let top = Ctx {
            module_level: true,
            ..Ctx::default()
        };
        let mut stack: Vec<(Node<'a>, Ctx)> = vec![(root, top)];
        while let Some((n, ctx)) = stack.pop() {
            let before = stack.len();
            self.visit(n, ctx, &mut stack);
            stack[before..].reverse();
        }
    }

    /// Handles one node and pushes the children to visit (in source order; the caller
    /// reverses them).
    fn visit(&mut self, n: Node<'a>, ctx: Ctx, stack: &mut Vec<(Node<'a>, Ctx)>) {
        match n.kind() {
            "decorated_definition" => {
                let Some(def) = n.child_by_field_name("definition") else {
                    push_children(n, ctx, stack);
                    return;
                };
                let mark = stack.len();
                let inner = self.define(def, n, ctx, stack);
                // Decorators come first in source order: insert them before the parts
                // `define` pushed.
                let mut c = n.walk();
                let decorators: Vec<_> = n
                    .named_children(&mut c)
                    .filter(|d| d.kind() == "decorator")
                    .map(|d| (d, inner))
                    .collect();
                stack.splice(mark..mark, decorators);
            }
            "function_definition" | "class_definition" => {
                self.define(n, n, ctx, stack);
            }
            "type_alias_statement" => {
                let inner = match type_alias_name(n) {
                    Some(name) if ctx.module_level => {
                        let name = self.t(name).to_string();
                        let idx = self.add_symbol(
                            name,
                            SymbolKind::Type,
                            n,
                            n,
                            None,
                            ctx,
                            Vec::new(),
                            None,
                            false,
                        );
                        Ctx {
                            sym: Some(idx),
                            ..ctx
                        }
                    }
                    _ => ctx,
                };
                if let Some(v) = n.child_by_field_name("right") {
                    stack.push((
                        v,
                        Ctx {
                            type_pos: true,
                            module_level: false,
                            ..inner
                        },
                    ));
                }
            }
            "expression_statement" if ctx.module_level => {
                let a = n.named_child(0);
                let left = a
                    .filter(|a| a.kind() == "assignment")
                    .and_then(|a| a.child_by_field_name("left"));
                match (a, left) {
                    (Some(a), Some(left))
                        if left.kind() == "identifier" && is_constant_name(self.t(left)) =>
                    {
                        let name = self.t(left).to_string();
                        let idx = self.add_symbol(
                            name,
                            SymbolKind::Constant,
                            n,
                            n,
                            None,
                            ctx,
                            Vec::new(),
                            None,
                            false,
                        );
                        let inner = Ctx {
                            sym: Some(idx),
                            module_level: false,
                            ..ctx
                        };
                        self.push_assignment_parts(a, inner, stack);
                    }
                    _ => push_children(n, ctx, stack),
                }
            }
            "assignment" | "augmented_assignment" => self.push_assignment_parts(n, ctx, stack),
            "import_statement" | "import_from_statement" | "future_import_statement" => {
                let imports = self.import_facts(n, ctx.type_checking);
                self.facts.imports.extend(imports);
            }
            "if_statement" => {
                let cond = n.child_by_field_name("condition");
                let guarded = cond
                    .is_some_and(|c| matches!(self.t(c), "TYPE_CHECKING" | "typing.TYPE_CHECKING"));
                let mut c = n.walk();
                for ch in n.named_children(&mut c) {
                    let is_consequence = n
                        .child_by_field_name("consequence")
                        .is_some_and(|b| b.id() == ch.id());
                    let child_ctx = if guarded && is_consequence {
                        Ctx {
                            type_checking: true,
                            ..ctx
                        }
                    } else {
                        ctx
                    };
                    stack.push((ch, child_ctx));
                }
            }
            "call" => self.visit_call(n, ctx, stack),
            "attribute" => {
                if let Some(o) = n.child_by_field_name("object") {
                    stack.push((o, ctx));
                }
            }
            "keyword_argument" => {
                if let Some(v) = n.child_by_field_name("value") {
                    stack.push((v, ctx));
                }
            }
            "type" => push_children(
                n,
                Ctx {
                    type_pos: true,
                    ..ctx
                },
                stack,
            ),
            "identifier" => self.reference(n, ctx),
            "global_statement" | "nonlocal_statement" | "comment" => {}
            _ => push_children(n, ctx, stack),
        }
    }

    fn push_assignment_parts(&mut self, a: Node<'a>, ctx: Ctx, stack: &mut Vec<(Node<'a>, Ctx)>) {
        if let Some(left) = a.child_by_field_name("left")
            && left.kind() != "identifier"
        {
            stack.push((left, ctx));
        }
        if let Some(ty) = a.child_by_field_name("type") {
            stack.push((ty, ctx));
        }
        if let Some(right) = a.child_by_field_name("right") {
            stack.push((right, ctx));
        }
    }

    /// Emits the symbol for a `def` or `class` (`outer` is the decorated definition when
    /// there are decorators) and pushes its parts. Returns the context inside it.
    fn define(
        &mut self,
        def: Node<'a>,
        outer: Node<'a>,
        ctx: Ctx,
        stack: &mut Vec<(Node<'a>, Ctx)>,
    ) -> Ctx {
        let Some(name_node) = def.child_by_field_name("name") else {
            push_children(def, ctx, stack);
            return ctx;
        };
        let name = self.t(name_node).to_string();
        let body = def.child_by_field_name("body");
        let doc = body.and_then(|b| docstring(b, self.src));
        // The signature ends at the `:` before the body (a trailing comment sits between).
        let head_end = colon_before_body(def).or(body);
        if def.kind() == "class_definition" {
            let idx = self.add_symbol(
                name,
                SymbolKind::Class,
                def,
                outer,
                head_end,
                ctx,
                Vec::new(),
                doc,
                false,
            );
            let inner = Ctx {
                sym: Some(idx),
                module_level: false,
                in_class: true,
                type_pos: false,
                ..ctx
            };
            if let Some(sc) = def.child_by_field_name("superclasses") {
                stack.push((sc, inner));
            }
            if let Some(b) = body {
                stack.push((b, inner));
            }
            return inner;
        }

        let kind = if ctx.in_class {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let is_async = has_token(def, "async");
        let params_node = def.child_by_field_name("parameters");
        let (params, bound) = params_node.map(|p| self.params(p)).unwrap_or_default();
        let idx = self.add_symbol(name, kind, def, outer, head_end, ctx, params, doc, is_async);
        let shadow = if bound.is_empty() {
            ctx.shadow
        } else {
            let mut set = self.shadows[ctx.shadow].clone();
            set.extend(bound);
            self.shadows.push(set);
            self.shadows.len() - 1
        };
        let inner = Ctx {
            sym: Some(idx),
            func: Some(idx),
            module_level: false,
            in_class: false,
            type_pos: false,
            shadow,
            type_checking: ctx.type_checking,
        };
        if let Some(p) = params_node {
            self.push_param_parts(p, inner, stack);
        }
        if let Some(r) = def.child_by_field_name("return_type") {
            stack.push((r, inner));
        }
        if let Some(b) = body {
            stack.push((b, inner));
        }
        inner
    }

    /// Parameter labels as written, plus the names they bind.
    fn params(&self, p: Node) -> (Vec<String>, Vec<String>) {
        let mut labels = Vec::new();
        let mut bound = Vec::new();
        let mut c = p.walk();
        for ch in p.named_children(&mut c) {
            let (label, name) = match ch.kind() {
                "identifier" => (self.t(ch).to_string(), Some(ch)),
                "typed_parameter" => {
                    let id = first_named(ch);
                    let label = id.map(|i| self.t(i).to_string()).unwrap_or_default();
                    let id = id.filter(|i| i.kind() == "identifier");
                    (label, id)
                }
                "default_parameter" | "typed_default_parameter" => {
                    let id = ch.child_by_field_name("name");
                    let label = id.map(|i| self.t(i)).unwrap_or("");
                    (format!("{label} = …"), id)
                }
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    (self.t(ch).to_string(), first_named(ch))
                }
                "keyword_separator" => ("*".to_string(), None),
                "positional_separator" => ("/".to_string(), None),
                _ => (clip(self.t(ch), 60), None),
            };
            if !label.is_empty() {
                labels.push(label);
            }
            if let Some(id) = name.filter(|i| i.kind() == "identifier") {
                bound.push(self.t(id).to_string());
            }
        }
        (labels, bound)
    }

    /// Annotations and default values inside a parameter list (never the names).
    fn push_param_parts(&self, p: Node<'a>, ctx: Ctx, stack: &mut Vec<(Node<'a>, Ctx)>) {
        let mut c = p.walk();
        for ch in p.named_children(&mut c) {
            match ch.kind() {
                "typed_parameter" => {
                    if let Some(t) = ch.child_by_field_name("type") {
                        stack.push((t, ctx));
                    }
                }
                "default_parameter" | "typed_default_parameter" => {
                    if let Some(t) = ch.child_by_field_name("type") {
                        stack.push((t, ctx));
                    }
                    if let Some(v) = ch.child_by_field_name("value") {
                        stack.push((v, ctx));
                    }
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_symbol(
        &mut self,
        name: String,
        kind: SymbolKind,
        def: Node,
        outer: Node,
        body: Option<Node>,
        ctx: Ctx,
        params: Vec<String>,
        doc: Option<String>,
        is_async: bool,
    ) -> usize {
        let parent = ctx
            .sym
            .map(|i| self.facts.symbols[i].qualified_name.clone());
        let qualified = match &parent {
            Some(p) => format!("{p}.{name}"),
            None => name.clone(),
        };
        let qualified_name = self.unique.claim(qualified, kind);
        let exported = ctx.module_level
            && match &self.all {
                Some(all) => all.contains(&name),
                None => !name.starts_with('_'),
            };
        let signature = match kind {
            SymbolKind::Constant | SymbolKind::Type => clip(self.t(def), 200),
            _ => head_until(outer.start_byte(), def, body, self.src),
        };
        self.facts.symbols.push(SymbolFact {
            name,
            qualified_name,
            kind,
            start_line: line(outer),
            end_line: crate::util::end_line(outer),
            signature,
            doc,
            exported,
            is_async,
            params,
            parent,
            body_hash: node_hash(outer, self.src),
        });
        self.facts.symbols.len() - 1
    }

    fn visit_call(&mut self, n: Node<'a>, ctx: Ctx, stack: &mut Vec<(Node<'a>, Ctx)>) {
        let caller = ctx
            .func
            .map(|i| self.facts.symbols[i].qualified_name.clone());
        if let Some(f) = n.child_by_field_name("function") {
            match f.kind() {
                "identifier" => {
                    let name = self.t(f).to_string();
                    self.facts.calls.push(CallFact {
                        caller,
                        callee: name.clone(),
                        name,
                        receiver: None,
                        line: line(f),
                    });
                }
                "attribute" => {
                    let obj = f.child_by_field_name("object");
                    let attr = f.child_by_field_name("attribute");
                    self.facts.calls.push(CallFact {
                        caller,
                        callee: clip(self.t(f), 200),
                        name: attr.map(|a| self.t(a).to_string()).unwrap_or_default(),
                        receiver: obj.map(|o| clip(self.t(o), 200)),
                        line: attr.map(line).unwrap_or_else(|| line(f)),
                    });
                    // Visit the receiver chain except a bare root identifier.
                    let mut root = obj;
                    while let Some(r) = root {
                        if r.kind() == "attribute" {
                            root = r.child_by_field_name("object");
                        } else {
                            break;
                        }
                    }
                    if let Some(r) = root
                        && r.kind() != "identifier"
                    {
                        stack.push((r, ctx));
                    }
                }
                _ => {
                    let callee = clip(self.t(f), 200);
                    self.facts.calls.push(CallFact {
                        caller,
                        name: callee.clone(),
                        callee,
                        receiver: None,
                        line: line(f),
                    });
                    stack.push((f, ctx));
                }
            }
        }
        if let Some(a) = n.child_by_field_name("arguments") {
            stack.push((a, ctx));
        }
    }

    fn reference(&mut self, n: Node, ctx: Ctx) {
        let name = self.t(n);
        if !self.module_names.contains(name) || self.shadows[ctx.shadow].contains(name) {
            return;
        }
        let ln = line(n);
        if !self
            .seen_refs
            .insert((ctx.sym, name.to_string(), ctx.type_pos, ln))
        {
            return;
        }
        let from = ctx
            .sym
            .map(|i| self.facts.symbols[i].qualified_name.clone());
        self.facts.refs.push(RefFact {
            from,
            name: name.to_string(),
            type_position: ctx.type_pos,
            line: ln,
        });
    }

    fn import_facts(&self, n: Node, type_only: bool) -> Vec<ImportFact> {
        let ln = line(n);
        let mut out = Vec::new();
        match n.kind() {
            "import_statement" => {
                let mut c = n.walk();
                for ch in n.children_by_field_name("name", &mut c) {
                    let (spec, alias) = self.dotted_and_alias(ch);
                    out.push(ImportFact {
                        specifier: spec.clone(),
                        kind: ImportKind::Static,
                        names: vec![ImportedName {
                            name: spec,
                            alias,
                            type_only: false,
                        }],
                        type_only,
                        line: ln,
                    });
                }
            }
            "import_from_statement" | "future_import_statement" => {
                let specifier = match n.child_by_field_name("module_name") {
                    Some(m) => self.t(m).split_whitespace().collect::<String>(),
                    None => "__future__".to_string(),
                };
                let mut names = Vec::new();
                let mut c = n.walk();
                for ch in n.children_by_field_name("name", &mut c) {
                    let (name, alias) = self.dotted_and_alias(ch);
                    names.push(ImportedName {
                        name,
                        alias,
                        type_only: false,
                    });
                }
                let mut c = n.walk();
                if n.named_children(&mut c)
                    .any(|ch| ch.kind() == "wildcard_import")
                {
                    names.push(ImportedName {
                        name: "*".to_string(),
                        alias: None,
                        type_only: false,
                    });
                }
                out.push(ImportFact {
                    specifier,
                    kind: ImportKind::Static,
                    names,
                    type_only,
                    line: ln,
                });
            }
            _ => {}
        }
        out
    }

    fn dotted_and_alias(&self, n: Node) -> (String, Option<String>) {
        if n.kind() == "aliased_import" {
            let name = n
                .child_by_field_name("name")
                .map(|x| self.t(x).split_whitespace().collect::<String>())
                .unwrap_or_default();
            let alias = n
                .child_by_field_name("alias")
                .map(|x| self.t(x).to_string());
            (name, alias)
        } else {
            (self.t(n).split_whitespace().collect(), None)
        }
    }
}

fn push_children<'a>(n: Node<'a>, ctx: Ctx, stack: &mut Vec<(Node<'a>, Ctx)>) {
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        stack.push((ch, ctx));
    }
}

/// The `:` token that opens a `def`/`class` body.
fn colon_before_body(def: Node) -> Option<Node> {
    let body_start = def
        .child_by_field_name("body")
        .map(|b| b.start_byte())
        .unwrap_or(def.end_byte());
    let mut c = def.walk();
    def.children(&mut c)
        .filter(|ch| !ch.is_named() && ch.kind() == ":" && ch.start_byte() < body_start)
        .last()
}

fn first_named(n: Node) -> Option<Node> {
    n.named_child(0)
}

fn has_token(n: Node, kind: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .any(|ch| !ch.is_named() && ch.kind() == kind)
}

fn type_alias_name(n: Node) -> Option<Node> {
    let left = n.child_by_field_name("left")?;
    if left.kind() == "type" {
        let id = left.named_child(0)?;
        if id.kind() == "identifier" {
            return Some(id);
        }
        if id.kind() == "generic_type" {
            return id.named_child(0).filter(|i| i.kind() == "identifier");
        }
        return None;
    }
    Some(left)
}

/// UPPER_CASE (letters, digits and `_`, at least one letter, not starting with a digit).
fn is_constant_name(s: &str) -> bool {
    let mut has_letter = false;
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            has_letter = true;
        } else if ch.is_ascii_digit() {
            if i == 0 {
                return false;
            }
        } else if ch != '_' {
            return false;
        }
    }
    has_letter
}

/// The value of a plain string literal (contents between the quotes).
fn string_value(n: Node, src: &str) -> String {
    let mut out = String::new();
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        if ch.kind() == "string_content" {
            out.push_str(text(ch, src));
        }
    }
    out
}

/// The docstring of a block: its first statement, when that is a string literal.
fn docstring(body: Node, src: &str) -> Option<String> {
    let first = body.named_child(0)?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let s = first.named_child(0)?;
    if s.kind() != "string" || first.named_child_count() != 1 {
        return None;
    }
    clean_doc(&string_value(s, src))
}
