//! Rust extractor. Owned by workstream D (see CONTRACTS.md).
//!
//! Two passes, both iterative: file-level names (top-level items and `use` bindings), then
//! one depth-first walk for symbols, imports, calls and references.
//!
//! Choices where the contract leaves room:
//! - Symbols: `fn` (`function`, or `method` inside `impl`/`trait`), `struct`/`union`
//!   (`struct`), `enum`, `trait`, `type`, `const` and immutable `static` (`constant`),
//!   `static mut` (`variable`), `mod` (`module`, inline or `mod x;`). Trait methods without a
//!   body are included. No symbol is emitted for an `impl` block itself; its methods are
//!   `Type.method` with parent `Type` (prefixed by enclosing inline modules or functions).
//! - `exported` only for a plain `pub` visibility.
//! - `use` trees are expanded to one `ImportFact` per leaf: `use a::b::{c, d as e, f::*}`
//!   gives `a::b::c` [c], `a::b::d` [d as e] and `a::b::f` [*]; `{self}` names the prefix
//!   module. `pub use` (any visibility) is `reexport`.
//! - Inside inline modules, `self::` and `super::` paths are rewritten relative to the
//!   file's module (`mod tests { use super::*; }` gives `self` [*]), so the resolver, which
//!   only knows files, resolves them right. `mod x;` inside `mod outer { … }` has the
//!   specifier `outer::x`.
//! - `extern crate foo as bar;` is a static import of `foo` with alias `bar`.
//! - Macro invocations are not calls. Their token trees are scanned for `path(…)` call
//!   shapes and for identifiers naming file-level items, so `println!("{}", f(x))` still
//!   records the call to `f`.
//! - `refs` holds non-call uses of file-level names. A direct callee and the root of a path
//!   or method-call receiver are covered by the call fact. `type_identifier` nodes are type
//!   positions, except a struct literal's name.

use std::collections::HashSet;
use std::path::Path;

use iso_core::{
    CallFact, FileFacts, FlowUnit, ImportFact, ImportKind, ImportedName, Language, RefFact,
    SymbolFact, SymbolKind,
};
use tree_sitter::Node;

use crate::Extractor;
use crate::util::{
    UniqueNames, clip, end_line, head_until, line, node_hash, strip_comment_markers, text,
};

pub struct RustExtractor;
pub static RUST: RustExtractor = RustExtractor;

impl Extractor for RustExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }
    fn extract(&self, _path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts {
        let root = tree.root_node();
        let mut x = Rs::new(source);
        x.collect_file_names(root);
        x.walk(root);
        x.facts.has_errors = root.has_error();
        x.facts.calls.sort_by_key(|c| c.line);
        x.facts
    }
    /// Call flow (workstream H, `flow/rust.rs`); imports come from this extractor's facts.
    fn flow_units(&self, path: &Path, source: &str, tree: &tree_sitter::Tree) -> Vec<FlowUnit> {
        let facts = self.extract(path, source, tree);
        crate::flow::rust::flow_units(source, tree, &facts)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Container {
    #[default]
    Module,
    Impl,
    Trait,
    Fn,
}

#[derive(Clone, Copy, Default)]
struct Ctx {
    /// Prefix for new item names and their parent (index into `names`).
    scope: Option<usize>,
    /// Enclosing symbol for refs (index into `names`).
    from: Option<usize>,
    /// Enclosing function for calls (index into `names`).
    caller: Option<usize>,
    container: Container,
    type_pos: bool,
    /// Enclosing inline modules as a `a::b` path (index into `names`).
    mod_path: Option<usize>,
}

struct Rs<'a> {
    src: &'a str,
    facts: FileFacts,
    file_names: HashSet<String>,
    /// Interned qualified names and module paths referenced by `Ctx`.
    names: Vec<String>,
    unique: UniqueNames,
    seen_refs: HashSet<(Option<usize>, String, bool, u32)>,
    /// `names` entries that are `impl` block scopes.
    impl_scopes: HashSet<usize>,
}

type Stack<'a> = Vec<(Node<'a>, Ctx)>;

impl<'a> Rs<'a> {
    fn new(src: &'a str) -> Self {
        Rs {
            src,
            facts: FileFacts::default(),
            file_names: HashSet::new(),
            names: Vec::new(),
            unique: UniqueNames::default(),
            seen_refs: HashSet::new(),
            impl_scopes: HashSet::new(),
        }
    }

    fn t(&self, n: Node) -> &'a str {
        text(n, self.src)
    }

    fn intern(&mut self, s: String) -> usize {
        self.names.push(s);
        self.names.len() - 1
    }

    fn name_of(&self, i: Option<usize>) -> Option<String> {
        i.map(|i| self.names[i].clone())
    }

    // ---- pass 1 ----

    fn collect_file_names(&mut self, root: Node) {
        let mut c = root.walk();
        for n in root.named_children(&mut c) {
            match n.kind() {
                "function_item" | "struct_item" | "enum_item" | "union_item" | "trait_item"
                | "type_item" | "const_item" | "static_item" | "mod_item" => {
                    if let Some(name) = n.child_by_field_name("name") {
                        self.file_names.insert(self.t(name).to_string());
                    }
                }
                "use_declaration" => {
                    for imp in self.use_facts(n, None) {
                        for name in &imp.names {
                            if name.name != "*" {
                                self.file_names.insert(name.local().to_string());
                            }
                        }
                    }
                }
                "extern_crate_declaration" => {
                    let id = n
                        .child_by_field_name("alias")
                        .or_else(|| n.child_by_field_name("name"));
                    if let Some(id) = id {
                        self.file_names.insert(self.t(id).to_string());
                    }
                }
                _ => {}
            }
        }
        // `self`, `crate` and `super` are never refs.
        for kw in ["self", "crate", "super", "Self"] {
            self.file_names.remove(kw);
        }
    }

    // ---- pass 2 ----

    fn walk(&mut self, root: Node<'a>) {
        let mut stack: Stack<'a> = vec![(root, Ctx::default())];
        while let Some((n, ctx)) = stack.pop() {
            let before = stack.len();
            self.visit(n, ctx, &mut stack);
            stack[before..].reverse();
        }
    }

    fn visit(&mut self, n: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        match n.kind() {
            "function_item" | "function_signature_item" => self.function(n, ctx, stack),
            "struct_item" | "union_item" => self.item(n, SymbolKind::Struct, ctx, stack),
            "enum_item" => self.item(n, SymbolKind::Enum, ctx, stack),
            "trait_item" => self.item(n, SymbolKind::Trait, ctx, stack),
            "type_item" => self.item(n, SymbolKind::Type, ctx, stack),
            "const_item" => self.item(n, SymbolKind::Constant, ctx, stack),
            "static_item" => {
                let kind = if has_child_kind(n, "mutable_specifier") {
                    SymbolKind::Variable
                } else {
                    SymbolKind::Constant
                };
                self.item(n, kind, ctx, stack)
            }
            "mod_item" => self.module(n, ctx, stack),
            "impl_item" => self.impl_block(n, ctx, stack),
            "use_declaration" => {
                let facts = self.use_facts(n, ctx.mod_path.map(|i| self.names[i].as_str()));
                self.facts.imports.extend(facts);
            }
            "extern_crate_declaration" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let name = self.t(name).to_string();
                    let alias = n
                        .child_by_field_name("alias")
                        .map(|a| self.t(a).to_string());
                    self.facts.imports.push(ImportFact {
                        specifier: name.clone(),
                        kind: ImportKind::Static,
                        names: vec![ImportedName {
                            name,
                            alias,
                            type_only: false,
                        }],
                        type_only: false,
                        line: line(n),
                    });
                }
            }
            "attribute_item"
            | "inner_attribute_item"
            | "line_comment"
            | "block_comment"
            | "macro_definition"
            | "visibility_modifier"
            | "closure_parameters"
            | "lifetime"
            | "label" => {}
            "call_expression" => self.call(n, ctx, stack),
            "macro_invocation" => {
                let mut c = n.walk();
                for tt in n.named_children(&mut c) {
                    if tt.kind() == "token_tree" {
                        stack.push((tt, ctx));
                    }
                }
            }
            "token_tree" => self.token_tree(n, ctx, stack),
            "scoped_identifier" | "scoped_type_identifier" => {
                // Only the root of a path can name a file-level item.
                let mut root = n;
                while let Some(p) = root.child_by_field_name("path") {
                    if matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier") {
                        root = p;
                    } else {
                        root = p;
                        break;
                    }
                }
                if root.id() != n.id() {
                    stack.push((root, ctx));
                }
            }
            "field_expression" => {
                if let Some(v) = n.child_by_field_name("value") {
                    stack.push((v, ctx));
                }
            }
            "struct_expression" => {
                // A struct literal names a type but uses it as a value.
                match n.child_by_field_name("name") {
                    Some(name) if name.kind() == "type_identifier" => {
                        self.reference(name, ctx, false)
                    }
                    Some(name) => stack.push((name, ctx)),
                    None => {}
                }
                if let Some(b) = n.child_by_field_name("body") {
                    stack.push((b, ctx));
                }
            }
            "let_declaration" | "for_expression" => {
                let mut c = n.walk();
                for (i, ch) in n.children(&mut c).enumerate() {
                    if !ch.is_named() {
                        continue;
                    }
                    let field = n.field_name_for_child(i as u32);
                    match field {
                        Some("pattern") if ch.kind() == "identifier" => {}
                        Some("type") => stack.push((
                            ch,
                            Ctx {
                                type_pos: true,
                                ..ctx
                            },
                        )),
                        _ => stack.push((ch, ctx)),
                    }
                }
            }
            "enum_variant" | "field_declaration" => {
                let mut c = n.walk();
                for (i, ch) in n.children(&mut c).enumerate() {
                    if ch.is_named() && n.field_name_for_child(i as u32) != Some("name") {
                        stack.push((ch, ctx));
                    }
                }
            }
            "identifier" => self.reference(n, ctx, false),
            "type_identifier" => self.reference(n, ctx, true),
            _ => push_children(n, ctx, stack),
        }
    }

    /// Parent and qualified name for a new item named `name`.
    fn qualify(&self, ctx: Ctx, name: &str) -> (Option<String>, String) {
        let parent = self.name_of(ctx.scope);
        let q = match &parent {
            Some(p) => format!("{p}.{name}"),
            None => name.to_string(),
        };
        (parent, q)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_symbol(
        &mut self,
        n: Node,
        name: String,
        kind: SymbolKind,
        ctx: Ctx,
        signature: String,
        params: Vec<String>,
        is_async: bool,
        doc: Option<String>,
    ) -> usize {
        let (parent, q) = self.qualify(ctx, &name);
        let qualified_name = self.unique.claim(q, kind);
        let exported = n.named_children(&mut n.walk()).any(|ch| {
            ch.kind() == "visibility_modifier" && self.t(ch).split_whitespace().eq(["pub"])
        });
        self.facts.symbols.push(SymbolFact {
            name,
            qualified_name: qualified_name.clone(),
            kind,
            start_line: line(n),
            end_line: end_line(n),
            signature,
            doc,
            exported,
            is_async,
            params,
            parent,
            body_hash: node_hash(n, self.src),
        });
        self.intern(qualified_name)
    }

    fn function(&mut self, n: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        let Some(name) = n.child_by_field_name("name") else {
            push_children(n, ctx, stack);
            return;
        };
        let name = self.t(name).to_string();
        let kind = match ctx.container {
            Container::Impl | Container::Trait => SymbolKind::Method,
            _ => SymbolKind::Function,
        };
        let body = n.child_by_field_name("body");
        let is_async = n
            .named_children(&mut n.walk())
            .any(|ch| ch.kind() == "function_modifiers" && has_token(ch, "async"));
        let params = n
            .child_by_field_name("parameters")
            .map(|p| self.params(p))
            .unwrap_or_default();
        let signature = head_until(n.start_byte(), n, body, self.src);
        let doc = outer_doc(n, self.src);
        let idx = self.add_symbol(n, name, kind, ctx, signature, params, is_async, doc);
        let inner = Ctx {
            scope: Some(idx),
            from: Some(idx),
            caller: Some(idx),
            container: Container::Fn,
            type_pos: false,
            mod_path: ctx.mod_path,
        };
        let mut c = n.walk();
        for (i, ch) in n.children(&mut c).enumerate() {
            if !ch.is_named() {
                continue;
            }
            match n.field_name_for_child(i as u32) {
                Some("name") => {}
                Some("parameters") => self.push_params(ch, inner, stack),
                Some("return_type") | Some("type_parameters") => stack.push((
                    ch,
                    Ctx {
                        type_pos: true,
                        ..inner
                    },
                )),
                _ if ch.kind() == "where_clause" => stack.push((
                    ch,
                    Ctx {
                        type_pos: true,
                        ..inner
                    },
                )),
                Some("body") => stack.push((ch, inner)),
                _ => {}
            }
        }
    }

    fn params(&self, p: Node) -> Vec<String> {
        let mut out = Vec::new();
        let mut c = p.walk();
        for ch in p.named_children(&mut c) {
            match ch.kind() {
                "self_parameter" => out.push(clip(self.t(ch), 60)),
                "parameter" => {
                    let end = ch
                        .child_by_field_name("type")
                        .map(|t| t.start_byte())
                        .unwrap_or(ch.end_byte());
                    let raw = self.src.get(ch.start_byte()..end).unwrap_or("");
                    out.push(clip(raw.trim_end().trim_end_matches(':'), 60));
                }
                "variadic_parameter" => out.push("...".to_string()),
                "attribute_item" | "line_comment" | "block_comment" => {}
                _ => out.push(clip(self.t(ch), 60)),
            }
        }
        out
    }

    fn push_params(&self, p: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        let ty = Ctx {
            type_pos: true,
            ..ctx
        };
        let mut c = p.walk();
        for ch in p.named_children(&mut c) {
            if ch.kind() == "parameter"
                && let Some(t) = ch.child_by_field_name("type")
            {
                stack.push((t, ty));
            }
        }
    }

    fn item(&mut self, n: Node<'a>, kind: SymbolKind, ctx: Ctx, stack: &mut Stack<'a>) {
        let Some(name) = n.child_by_field_name("name") else {
            push_children(n, ctx, stack);
            return;
        };
        let name = self.t(name).to_string();
        let body = match kind {
            SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait => {
                n.child_by_field_name("body")
            }
            _ => None,
        };
        let signature = head_until(n.start_byte(), n, body, self.src);
        let doc = outer_doc(n, self.src);
        let idx = self.add_symbol(n, name, kind, ctx, signature, Vec::new(), false, doc);
        let inner = Ctx {
            scope: Some(idx),
            from: Some(idx),
            container: if kind == SymbolKind::Trait {
                Container::Trait
            } else {
                ctx.container
            },
            type_pos: false,
            ..ctx
        };
        let mut c = n.walk();
        for (i, ch) in n.children(&mut c).enumerate() {
            if !ch.is_named() || n.field_name_for_child(i as u32) == Some("name") {
                continue;
            }
            let field = n.field_name_for_child(i as u32);
            let child_ctx = match (field, ch.kind()) {
                (Some("type"), _)
                | (Some("type_parameters"), _)
                | (Some("bounds"), _)
                | (_, "where_clause")
                | (_, "trait_bounds") => Ctx {
                    type_pos: true,
                    ..inner
                },
                _ => inner,
            };
            stack.push((ch, child_ctx));
        }
    }

    fn module(&mut self, n: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        let Some(name_node) = n.child_by_field_name("name") else {
            return;
        };
        let name = self.t(name_node).to_string();
        let body = n.child_by_field_name("body");
        let signature = head_until(n.start_byte(), n, body, self.src);
        let doc = outer_doc(n, self.src).or_else(|| body.and_then(|b| inner_doc(b, self.src)));
        let path = match ctx.mod_path {
            Some(p) => format!("{}::{name}", self.names[p]),
            None => name.clone(),
        };
        let idx = self.add_symbol(
            n,
            name,
            SymbolKind::Module,
            ctx,
            signature,
            Vec::new(),
            false,
            doc,
        );
        match body {
            None => self.facts.imports.push(ImportFact {
                specifier: path,
                kind: ImportKind::Mod,
                names: Vec::new(),
                type_only: false,
                line: line(n),
            }),
            Some(b) => {
                let mod_path = self.intern(path);
                let inner = Ctx {
                    scope: Some(idx),
                    from: Some(idx),
                    caller: None,
                    container: Container::Module,
                    type_pos: false,
                    mod_path: Some(mod_path),
                };
                stack.push((b, inner));
            }
        }
    }

    fn impl_block(&mut self, n: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        let ty = n
            .child_by_field_name("type")
            .and_then(|t| base_type_name(t, self.src))
            .unwrap_or("_")
            .to_string();
        let q = match self.name_of(ctx.scope) {
            Some(p) => format!("{p}.{ty}"),
            None => ty,
        };
        let scope = self.intern(q);
        self.impl_scopes.insert(scope);
        let inner = Ctx {
            scope: Some(scope),
            from: Some(scope),
            container: Container::Impl,
            type_pos: false,
            ..ctx
        };
        let mut c = n.walk();
        for (i, ch) in n.children(&mut c).enumerate() {
            if !ch.is_named() {
                continue;
            }
            let child_ctx = match n.field_name_for_child(i as u32) {
                Some("body") => inner,
                _ => Ctx {
                    type_pos: true,
                    ..inner
                },
            };
            stack.push((ch, child_ctx));
        }
    }

    fn call(&mut self, n: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        let caller = self.name_of(ctx.caller);
        let Some(mut f) = n.child_by_field_name("function") else {
            push_children(n, ctx, stack);
            return;
        };
        if f.kind() == "generic_function" {
            if let Some(ta) = f.child_by_field_name("type_arguments") {
                stack.push((
                    ta,
                    Ctx {
                        type_pos: true,
                        ..ctx
                    },
                ));
            }
            if let Some(inner) = f.child_by_field_name("function") {
                f = inner;
            }
        }
        let collapse = |s: &str| clip(s, 200);
        let (callee, name, receiver, ln) = match f.kind() {
            "identifier" => {
                let s = self.t(f).to_string();
                (s.clone(), s, None, line(f))
            }
            "scoped_identifier" => {
                let name = f.child_by_field_name("name");
                let path = f.child_by_field_name("path");
                if let Some(p) = path {
                    // The path root is covered by this call; generic args inside are not.
                    self.push_path_extras(p, ctx, stack);
                }
                (
                    self.t(f).split_whitespace().collect::<String>(),
                    name.map(|x| self.t(x).to_string()).unwrap_or_default(),
                    path.map(|p| self.t(p).split_whitespace().collect::<String>()),
                    line(f),
                )
            }
            "field_expression" => {
                let field = f.child_by_field_name("field");
                let value = f.child_by_field_name("value");
                let mut root = value;
                while let Some(r) = root {
                    if r.kind() == "field_expression" {
                        root = r.child_by_field_name("value");
                    } else {
                        break;
                    }
                }
                if let Some(r) = root
                    && !matches!(r.kind(), "identifier" | "self")
                {
                    stack.push((r, ctx));
                }
                (
                    collapse(self.t(f)),
                    field.map(|x| self.t(x).to_string()).unwrap_or_default(),
                    value.map(|v| collapse(self.t(v))),
                    field.map(line).unwrap_or_else(|| line(f)),
                )
            }
            _ => {
                stack.push((f, ctx));
                let s = collapse(self.t(f));
                (s.clone(), s, None, line(f))
            }
        };
        self.facts.calls.push(CallFact {
            caller,
            callee,
            name,
            receiver,
            line: ln,
        });
        if let Some(a) = n.child_by_field_name("arguments") {
            stack.push((a, ctx));
        }
    }

    /// Generic arguments inside a call path (`Vec::<Foo>::new`) still reference types.
    fn push_path_extras(&self, p: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        let mut node = Some(p);
        while let Some(n) = node {
            match n.kind() {
                "scoped_identifier" => node = n.child_by_field_name("path"),
                "generic_type" | "generic_type_with_turbofish" => {
                    if let Some(ta) = n.child_by_field_name("type_arguments") {
                        stack.push((
                            ta,
                            Ctx {
                                type_pos: true,
                                ..ctx
                            },
                        ));
                    }
                    node = None;
                }
                _ => node = None,
            }
        }
    }

    /// Calls and refs inside a macro's token tree. Calls are `a(…)`, `a::b(…)`, `a.b(…)`
    /// and `x(…).b(…)` shapes; nested token trees are pushed for their own scan.
    fn token_tree(&mut self, tt: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
        let mut c = tt.walk();
        let toks: Vec<Node<'a>> = tt.children(&mut c).collect();
        let mut consumed = vec![false; toks.len()];
        for (i, tok) in toks.iter().enumerate() {
            if tok.kind() != "token_tree" {
                continue;
            }
            stack.push((*tok, ctx));
            if !self.t(*tok).starts_with('(') || i == 0 {
                continue;
            }
            // Walk the chain backwards from the token before the parenthesis.
            let last = i - 1;
            if !is_path_token(toks[last].kind()) {
                continue;
            }
            let mut start = last;
            while start >= 2 && matches!(toks[start - 1].kind(), "." | "::") {
                let prev = start - 2;
                if is_path_token(toks[prev].kind()) {
                    start = prev;
                } else if toks[start - 1].kind() == "."
                    && toks[prev].kind() == "token_tree"
                    && prev >= 1
                    && is_path_token(toks[prev - 1].kind())
                {
                    // `x(…).b(…)`: the receiver is the inner call.
                    start = prev - 1;
                } else {
                    break;
                }
            }
            if start > 0
                && matches!(
                    toks[start - 1].kind(),
                    "fn" | "!" | "." | "::" | "struct" | "enum" | "mod" | "trait" | "impl"
                )
            {
                continue;
            }
            let callee_src = self
                .src
                .get(toks[start].start_byte()..toks[last].end_byte())
                .unwrap_or("");
            let callee = clip(callee_src, 200)
                .replace(" :: ", "::")
                .replace(" . ", ".");
            let name = self.t(toks[last]).to_string();
            let receiver = (last > start).then(|| {
                let r = self
                    .src
                    .get(toks[start].start_byte()..toks[last - 1].start_byte())
                    .unwrap_or("");
                clip(r, 200)
            });
            for flag in consumed.iter_mut().take(last + 1).skip(start) {
                *flag = true;
            }
            self.facts.calls.push(CallFact {
                caller: self.name_of(ctx.caller),
                callee,
                name,
                receiver,
                line: line(toks[last]),
            });
        }
        for (i, tok) in toks.iter().enumerate() {
            if consumed[i] || tok.kind() != "identifier" {
                continue;
            }
            let after_sep = i > 0 && matches!(toks[i - 1].kind(), "." | "::");
            let is_macro = toks.get(i + 1).is_some_and(|n| n.kind() == "!");
            if !after_sep && !is_macro {
                let ty = self.t(*tok).chars().next().is_some_and(char::is_uppercase)
                    && toks.get(i + 1).is_some_and(|n| n.kind() == "<");
                self.reference(*tok, ctx, ty);
            }
        }
    }

    fn reference(&mut self, n: Node, ctx: Ctx, type_identifier: bool) {
        let name = self.t(n);
        if !self.file_names.contains(name) {
            return;
        }
        let from = self.name_of(ctx.from);
        // An item never references itself; an `impl Foo` header does not reference `Foo`.
        if let (Some(f), Some(i)) = (&from, ctx.from) {
            let last = f.rsplit('.').next().unwrap_or(f);
            if f == name || (last == name && self.impl_scopes.contains(&i)) {
                return;
            }
        }
        let type_position = type_identifier || ctx.type_pos;
        let ln = line(n);
        if !self
            .seen_refs
            .insert((ctx.from, name.to_string(), type_position, ln))
        {
            return;
        }
        self.facts.refs.push(RefFact {
            from,
            name: name.to_string(),
            type_position,
            line: ln,
        });
    }

    /// Expands a `use` declaration into one fact per leaf. `mod_path` is the enclosing
    /// inline module path (`a::b`) used to rewrite `self::`/`super::`.
    fn use_facts(&self, n: Node, mod_path: Option<&str>) -> Vec<ImportFact> {
        let kind = if has_child_kind(n, "visibility_modifier") {
            ImportKind::Reexport
        } else {
            ImportKind::Static
        };
        let Some(arg) = n.child_by_field_name("argument") else {
            return Vec::new();
        };
        let mut leaves: Vec<(Vec<String>, Option<String>, bool)> = Vec::new();
        let mut stack: Vec<(Node, Vec<String>)> = vec![(arg, Vec::new())];
        while let Some((node, prefix)) = stack.pop() {
            match node.kind() {
                "scoped_use_list" => {
                    let mut p = prefix.clone();
                    if let Some(path) = node.child_by_field_name("path") {
                        p.extend(path_segments(self.t(path)));
                    }
                    if let Some(list) = node.child_by_field_name("list") {
                        let mut c = list.walk();
                        let items: Vec<_> = list.named_children(&mut c).collect();
                        for item in items.into_iter().rev() {
                            stack.push((item, p.clone()));
                        }
                    }
                }
                "use_list" => {
                    let mut c = node.walk();
                    let items: Vec<_> = node.named_children(&mut c).collect();
                    for item in items.into_iter().rev() {
                        stack.push((item, prefix.clone()));
                    }
                }
                "use_as_clause" => {
                    let mut p = prefix.clone();
                    if let Some(path) = node.child_by_field_name("path") {
                        p.extend(path_segments(self.t(path)));
                    }
                    let alias = node
                        .child_by_field_name("alias")
                        .map(|a| self.t(a).to_string());
                    leaves.push((p, alias, false));
                }
                "use_wildcard" => {
                    let mut p = prefix.clone();
                    let t = self.t(node);
                    let t = t.trim_end().trim_end_matches('*').trim_end();
                    let t = t.trim_end_matches("::");
                    p.extend(path_segments(t));
                    leaves.push((p, None, true));
                }
                "line_comment" | "block_comment" | "attribute_item" => {}
                _ => {
                    let mut p = prefix.clone();
                    p.extend(path_segments(self.t(node)));
                    leaves.push((p, None, false));
                }
            }
        }
        let ln = line(n);
        leaves
            .into_iter()
            .filter(|(p, _, _)| !p.is_empty())
            .map(|(mut path, alias, glob)| {
                // `{self}` names the prefix module itself.
                if !glob && path.len() > 1 && path.last().is_some_and(|s| s == "self") {
                    path.pop();
                }
                let path = rewrite_relative(path, mod_path);
                let names = if glob {
                    vec![ImportedName {
                        name: "*".to_string(),
                        alias: None,
                        type_only: false,
                    }]
                } else {
                    let last = path.last().cloned().unwrap_or_default();
                    vec![ImportedName {
                        name: last,
                        alias,
                        type_only: false,
                    }]
                };
                ImportFact {
                    specifier: path.join("::"),
                    kind,
                    names,
                    type_only: false,
                    line: ln,
                }
            })
            .collect()
    }
}

fn push_children<'a>(n: Node<'a>, ctx: Ctx, stack: &mut Stack<'a>) {
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        stack.push((ch, ctx));
    }
}

fn is_path_token(kind: &str) -> bool {
    matches!(kind, "identifier" | "self" | "super" | "crate")
}

fn has_child_kind(n: Node, kind: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c).any(|ch| ch.kind() == kind)
}

fn has_token(n: Node, kind: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c).any(|ch| ch.kind() == kind)
}

fn path_segments(s: &str) -> Vec<String> {
    s.split("::")
        .map(|seg| seg.split_whitespace().collect::<String>())
        .filter(|seg| !seg.is_empty())
        .collect()
}

/// Rewrites `self::`/`super::` inside inline module `mod_path` so they are relative to the
/// file's own module.
fn rewrite_relative(path: Vec<String>, mod_path: Option<&str>) -> Vec<String> {
    let Some(mp) = mod_path else { return path };
    let mods: Vec<&str> = mp.split("::").collect();
    match path.first().map(String::as_str) {
        Some("self") => {
            let mut out = vec!["self".to_string()];
            out.extend(mods.iter().map(|s| s.to_string()));
            out.extend(path.into_iter().skip(1));
            out
        }
        Some("super") => {
            let ups = path.iter().take_while(|s| *s == "super").count();
            let mut out = Vec::new();
            if ups <= mods.len() {
                out.push("self".to_string());
                out.extend(mods[..mods.len() - ups].iter().map(|s| s.to_string()));
            } else {
                out.extend(std::iter::repeat_n("super".to_string(), ups - mods.len()));
            }
            out.extend(path.into_iter().skip(ups));
            out
        }
        _ => path,
    }
}

/// `Foo` for `Foo`, `Foo<T>`, `&mut Foo`, `a::Foo`, `dyn Foo`.
fn base_type_name<'a>(t: Node, src: &'a str) -> Option<&'a str> {
    let mut n = t;
    loop {
        match n.kind() {
            "type_identifier" | "primitive_type" => return Some(text(n, src)),
            "generic_type" | "reference_type" | "pointer_type" => {
                n = n.child_by_field_name("type")?;
            }
            "scoped_type_identifier" => n = n.child_by_field_name("name")?,
            "dynamic_type" | "abstract_type" => {
                n = n.child_by_field_name("trait")?;
            }
            _ => {
                let s = text(n, src);
                return Some(s.split('<').next().unwrap_or(s).trim());
            }
        }
    }
}

fn is_outer_doc(n: Node, src: &str) -> bool {
    let t = text(n, src);
    match n.kind() {
        "line_comment" => t.starts_with("///") && !t.starts_with("////"),
        "block_comment" => t.starts_with("/**") && !t.starts_with("/***") && t != "/**/",
        _ => false,
    }
}

/// `///` and `/** */` comments directly above an item (attributes may sit in between).
fn outer_doc(n: Node, src: &str) -> Option<String> {
    let mut docs = Vec::new();
    let mut prev = n.prev_sibling();
    while let Some(p) = prev {
        match p.kind() {
            "attribute_item" => {}
            "line_comment" | "block_comment" if is_outer_doc(p, src) => docs.push(text(p, src)),
            _ => break,
        }
        prev = p.prev_sibling();
    }
    docs.reverse();
    strip_comment_markers(docs)
}

/// `//!` and `/*! */` comments at the start of an inline module's body.
fn inner_doc(body: Node, src: &str) -> Option<String> {
    let mut docs = Vec::new();
    let mut c = body.walk();
    for ch in body.named_children(&mut c) {
        let t = text(ch, src);
        if matches!(ch.kind(), "line_comment" | "block_comment")
            && (t.starts_with("//!") || t.starts_with("/*!"))
        {
            docs.push(t);
        } else {
            break;
        }
    }
    strip_comment_markers(docs)
}
