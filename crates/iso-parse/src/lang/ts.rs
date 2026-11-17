//! TypeScript and TSX extractor. Owned by workstream B (see CONTRACTS.md).
//!
//! One implementation serves TS, TSX, JS and JSX (`js.rs` delegates here). It is a single
//! recursive pass over the tree with a small context (enclosing symbol, enclosing callable,
//! type position) plus a pre-pass over the top-level statements that collects file-level
//! bindings for references.

use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

use iso_core::{
    CallFact, FileFacts, FlowUnit, ImportFact, ImportKind, ImportedName, Language, RefFact,
    SymbolFact, SymbolKind, hash64,
};
use tree_sitter::Node;

use crate::Extractor;
use crate::flow::js::{JsFlow, js_flow, string_value};
use crate::util::short;

pub struct TsExtractor {
    pub lang: Language,
}
pub static TS: TsExtractor = TsExtractor {
    lang: Language::TypeScript,
};
pub static TSX: TsExtractor = TsExtractor {
    lang: Language::Tsx,
};

impl Extractor for TsExtractor {
    fn language(&self) -> Language {
        self.lang
    }
    fn extract(&self, path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts {
        extract_facts(self.lang, path, source, tree)
    }
    fn flow_units(&self, _path: &Path, source: &str, tree: &tree_sitter::Tree) -> Vec<FlowUnit> {
        crate::flow::js::flow_units(self.lang, source, tree)
    }
}

/// Facts for one TS/TSX/JS/JSX file.
pub fn extract_facts(
    lang: Language,
    _path: &Path,
    src: &str,
    tree: &tree_sitter::Tree,
) -> FileFacts {
    let Some(jf) = js_flow(lang) else {
        return FileFacts::default();
    };
    crate::util::with_stack_for(crate::util::tree_depth(tree), || facts(jf, src, tree))
}

fn facts(jf: &'static JsFlow, src: &str, tree: &tree_sitter::Tree) -> FileFacts {
    let root = tree.root_node();
    let mut ex = Ex {
        src,
        jf,
        facts: FileFacts {
            has_errors: root.has_error(),
            ..FileFacts::default()
        },
        bindings: HashSet::new(),
        shadow: Vec::new(),
        seen_symbols: HashSet::new(),
        seen_refs: HashSet::new(),
        export_names: Vec::new(),
        private_members: HashSet::new(),
        export_only: HashSet::new(),
        depth: 0,
    };
    ex.collect_bindings(root);
    let cx = Cx {
        top: true,
        ..Cx::default()
    };
    // Error recovery can wrap a run of statements in an ERROR node; the declarations inside are
    // still top-level, so flatten top-level ERROR nodes instead of losing them.
    let mut stack: Vec<tree_sitter::Node> = children(root)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    while let Some(st) = stack.pop() {
        if st.is_error() {
            stack.extend(children(st).collect::<Vec<_>>().into_iter().rev());
            continue;
        }
        ex.visit(st, &cx);
    }
    // `export { a, b as c }`, `export default a` and CommonJS `module.exports = { a }` mark
    // earlier declarations exported, and with a class its public members.
    let names = std::mem::take(&mut ex.export_names);
    let marked: HashSet<String> = ex
        .facts
        .symbols
        .iter()
        .filter(|s| s.parent.is_none() && names.contains(&s.name.as_str()))
        .map(|s| s.qualified_name.clone())
        .collect();
    for s in &mut ex.facts.symbols {
        let member_of_marked = s.kind == SymbolKind::Method
            && s.parent.as_ref().is_some_and(|p| marked.contains(p))
            && !ex.private_members.contains(&s.qualified_name);
        if (s.parent.is_none() && marked.contains(&s.qualified_name)) || member_of_marked {
            s.exported = true;
        }
    }
    ex.facts
}

#[derive(Clone, Default)]
struct Cx {
    /// Qualified-name prefix for nested symbols.
    container: Option<Rc<str>>,
    /// Innermost enclosing symbol (refs come from it).
    symbol: Option<Rc<str>>,
    /// Innermost enclosing function or method (calls come from it).
    callable: Option<Rc<str>>,
    /// Statement level of the file or of a namespace body.
    top: bool,
    in_type: bool,
}

impl Cx {
    fn nested(&self) -> Cx {
        Cx {
            top: false,
            ..self.clone()
        }
    }
    fn typed(&self) -> Cx {
        Cx {
            top: false,
            in_type: true,
            ..self.clone()
        }
    }
    fn inside(&self, qn: &Rc<str>, callable: bool) -> Cx {
        Cx {
            container: Some(qn.clone()),
            symbol: Some(qn.clone()),
            callable: if callable {
                Some(qn.clone())
            } else {
                self.callable.clone()
            },
            top: false,
            in_type: false,
        }
    }
}

struct Ex<'s> {
    src: &'s str,
    jf: &'static JsFlow,
    facts: FileFacts,
    /// Names bound at file level: import locals and top-level declarations.
    bindings: HashSet<&'s str>,
    /// Local bindings (parameters, local variables) that hide file-level names.
    shadow: Vec<&'s str>,
    seen_symbols: HashSet<(String, SymbolKind)>,
    seen_refs: HashSet<(Option<Rc<str>>, &'s str, bool)>,
    export_names: Vec<&'s str>,
    /// Class members that stay unexported when their class is exported later.
    private_members: HashSet<String>,
    /// Identifier nodes that only name an export (CommonJS `module.exports = { a }`); they
    /// mark the binding and are not references.
    export_only: HashSet<usize>,
    /// Current `visit` recursion depth (see [`crate::util::MAX_VISIT_DEPTH`]).
    depth: usize,
}

/// Named, non-comment children.
fn children<'t>(n: Node<'t>) -> impl Iterator<Item = Node<'t>> {
    crate::flow::named(n)
}

fn has_token(n: Node, tok: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .any(|ch| !ch.is_named() && ch.kind() == tok)
}

fn is_lambda(n: Node) -> bool {
    matches!(
        n.kind(),
        "arrow_function" | "function_expression" | "function" | "generator_function"
    )
}

fn join(prefix: &Option<Rc<str>>, name: &str) -> String {
    match prefix {
        Some(p) => format!("{p}.{name}"),
        None => name.to_string(),
    }
}

/// JSDoc text without markers.
fn jsdoc(text: &str) -> Option<String> {
    if !text.starts_with("/**") || text == "/**/" {
        return None;
    }
    let inner = text
        .strip_prefix("/**")
        .unwrap_or(text)
        .strip_suffix("*/")
        .unwrap_or(text);
    let lines: Vec<&str> = inner
        .lines()
        .map(|l| {
            let l = l.trim();
            let l = l.strip_prefix('*').unwrap_or(l);
            l.strip_prefix(' ').unwrap_or(l).trim_end()
        })
        .collect();
    let start = lines.iter().position(|l| !l.is_empty())?;
    let end = lines.iter().rposition(|l| !l.is_empty())?;
    Some(lines[start..=end].join("\n"))
}

impl<'s> Ex<'s> {
    fn text(&self, n: Node) -> &'s str {
        &self.src[n.byte_range()]
    }

    fn doc_for(&self, outer: Node) -> Option<String> {
        let mut prev = outer.prev_sibling();
        // Decorators sit between a class member's doc comment and the member.
        while let Some(p) = prev {
            if p.kind() == "decorator" {
                prev = p.prev_sibling();
                continue;
            }
            break;
        }
        let p = prev?;
        if p.kind() != "comment" {
            return None;
        }
        jsdoc(self.text(p))
    }

    /// Declaration text from `outer`'s start up to `body` (or the whole node).
    fn signature(&self, outer: Node, body: Option<Node>) -> String {
        let end = body.map(|b| b.start_byte()).unwrap_or(outer.end_byte());
        let s = &self.src[outer.start_byte()..end.max(outer.start_byte())];
        let s = s.trim_end();
        let s = s.strip_suffix(';').unwrap_or(s);
        short(s, 200)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_symbol(
        &mut self,
        name: &str,
        qn: &str,
        kind: SymbolKind,
        outer: Node,
        signature: String,
        exported: bool,
        func: Option<Node>,
        parent: Option<&Rc<str>>,
    ) -> bool {
        if !self.seen_symbols.insert((qn.to_string(), kind)) {
            return false;
        }
        let (is_async, params) = match func {
            Some(f) => (has_token(f, "async"), self.jf.param_labels(f, self.src)),
            None => (false, Vec::new()),
        };
        self.facts.symbols.push(SymbolFact {
            name: name.to_string(),
            qualified_name: qn.to_string(),
            kind,
            start_line: crate::util::line(outer),
            end_line: crate::util::end_line(outer),
            signature,
            doc: self.doc_for(outer),
            exported,
            is_async,
            params,
            parent: parent.map(|p| p.to_string()),
            body_hash: hash64(self.text(outer).as_bytes()),
        });
        true
    }

    // -------------------------------------------------------------------------------
    // Pre-pass: file-level bindings.

    fn collect_bindings(&mut self, root: Node) {
        for st in children(root) {
            self.collect_stmt_bindings(st);
        }
    }

    fn collect_stmt_bindings(&mut self, st: Node) {
        let decl = match st.kind() {
            "export_statement" => match st.child_by_field_name("declaration") {
                Some(d) => d,
                None => return,
            },
            "ambient_declaration" => match children(st).next() {
                Some(d) => d,
                None => return,
            },
            "expression_statement" => match children(st).next() {
                Some(d) if d.kind() == "internal_module" => d,
                _ => return,
            },
            _ => st,
        };
        match decl.kind() {
            "import_statement" => {
                let mut stack = vec![decl];
                while let Some(n) = stack.pop() {
                    match n.kind() {
                        "import_specifier" => {
                            if let Some(l) = n
                                .child_by_field_name("alias")
                                .or_else(|| n.child_by_field_name("name"))
                            {
                                self.bindings.insert(self.text(l));
                            }
                        }
                        "string" => {}
                        "identifier" => {
                            self.bindings.insert(self.text(n));
                        }
                        _ => stack.extend(children(n)),
                    }
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                for d in children(decl).filter(|d| d.kind() == "variable_declarator") {
                    if let Some(name) = d.child_by_field_name("name") {
                        self.pattern_names(name, &mut |s: &mut Self, n| {
                            s.bindings.insert(n);
                        });
                    }
                }
            }
            "function_declaration"
            | "generator_function_declaration"
            | "function_signature"
            | "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "internal_module" => {
                if let Some(name) = decl.child_by_field_name("name") {
                    let t = self.text(name);
                    let t = t.split('.').next().unwrap_or(t);
                    self.bindings.insert(t);
                }
            }
            _ => {}
        }
    }

    /// Every identifier a binding pattern binds.
    fn pattern_names(&mut self, pat: Node, f: &mut impl FnMut(&mut Self, &'s str)) {
        match pat.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                let t = self.text(pat);
                f(self, t);
            }
            "pair_pattern" => {
                if let Some(v) = pat.child_by_field_name("value") {
                    self.pattern_names(v, f);
                }
            }
            "assignment_pattern" | "object_assignment_pattern" => {
                if let Some(l) = pat.child_by_field_name("left") {
                    self.pattern_names(l, f);
                }
            }
            "object_pattern" | "array_pattern" | "rest_pattern" => {
                for c in children(pat) {
                    self.pattern_names(c, f);
                }
            }
            _ => {}
        }
    }

    fn shadow_pattern(&mut self, pat: Node) {
        self.pattern_names(pat, &mut |s: &mut Self, n| s.shadow.push(n));
    }

    // -------------------------------------------------------------------------------
    // Main pass.

    fn visit_children(&mut self, n: Node, cx: &Cx) {
        for c in children(n) {
            self.visit(c, cx);
        }
    }

    fn visit(&mut self, n: Node, cx: &Cx) {
        if self.depth >= crate::util::MAX_VISIT_DEPTH {
            return;
        }
        self.depth += 1;
        self.visit_node(n, cx);
        self.depth -= 1;
    }

    fn visit_node(&mut self, n: Node, cx: &Cx) {
        match n.kind() {
            "comment" | "html_comment" | "hash_bang_line" => {}
            "import_statement" => self.import_statement(n),
            "export_statement" => self.export_statement(n, cx),
            "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "lexical_declaration"
            | "variable_declaration"
            | "internal_module"
            | "module"
            | "ambient_declaration" => self.declaration(n, n, false, cx),
            "expression_statement" if cx.top => {
                self.commonjs_exports(n);
                for c in children(n) {
                    if c.kind() == "internal_module" {
                        self.declaration(c, n, false, cx);
                    } else {
                        self.visit(c, &cx.nested());
                    }
                }
            }
            "call_expression" | "new_expression" => {
                self.call(n, cx);
                self.visit_children(n, &cx.nested());
            }
            "identifier" | "shorthand_property_identifier" => {
                self.reference(n, cx.in_type, cx);
            }
            "type_identifier" => self.reference(n, true, cx),
            "arrow_function" | "function_expression" | "function" | "generator_function" => {
                self.function_inside(n, &cx.nested());
            }
            "method_definition" => self.function_inside(n, &cx.nested()),
            "variable_declarator" => self.declarator(n, &cx.nested()),
            "catch_clause" => {
                let mark = self.shadow.len();
                if let Some(p) = n.child_by_field_name("parameter") {
                    self.shadow_pattern(p);
                }
                for c in children(n) {
                    if Some(c) != n.child_by_field_name("parameter") {
                        self.visit(c, &cx.nested());
                    }
                }
                self.shadow.truncate(mark);
            }
            "for_in_statement" => {
                let left = n.child_by_field_name("left");
                let declares = n.child_by_field_name("kind").is_some();
                for c in children(n) {
                    if declares && Some(c) == left {
                        self.shadow_pattern(c);
                    } else {
                        self.visit(c, &cx.nested());
                    }
                }
            }
            "type_annotation"
            | "type_arguments"
            | "type_parameters"
            | "implements_clause"
            | "extends_type_clause"
            | "type_predicate_annotation"
            | "asserts_annotation"
            | "opting_type_annotation"
            | "omitting_type_annotation"
            | "adding_type_annotation" => self.visit_children(n, &cx.typed()),
            "type_parameter" => {
                let name = n.child_by_field_name("name");
                for c in children(n) {
                    if Some(c) != name {
                        self.visit(c, &cx.typed());
                    }
                }
            }
            "as_expression" | "satisfies_expression" => {
                for (i, c) in children(n).enumerate() {
                    if i == 0 {
                        self.visit(c, &cx.nested());
                    } else {
                        self.visit(c, &cx.typed());
                    }
                }
            }
            "class" => {
                // Class expression: walk its members like a class body without symbols.
                let name = n.child_by_field_name("name");
                for c in children(n) {
                    if Some(c) != name {
                        self.visit(c, &cx.nested());
                    }
                }
            }
            _ => self.visit_children(n, &cx.nested()),
        }
    }

    fn reference(&mut self, n: Node, type_position: bool, cx: &Cx) {
        let name = self.text(n);
        if !self.bindings.contains(name)
            || self.shadow.contains(&name)
            || self.export_only.contains(&n.id())
        {
            return;
        }
        if self
            .seen_refs
            .insert((cx.symbol.clone(), name, type_position))
        {
            self.facts.refs.push(RefFact {
                from: cx.symbol.as_deref().map(str::to_string),
                name: name.to_string(),
                type_position,
                line: crate::util::line(n),
            });
        }
    }

    /// Parameters, type parameters, return type and body of a function-like node.
    fn function_inside(&mut self, f: Node, cx: &Cx) {
        let mark = self.shadow.len();
        let name = f.child_by_field_name("name");
        if let Some(p) = f.child_by_field_name("parameter") {
            self.shadow_pattern(p);
        }
        for c in children(f) {
            if Some(c) == name && f.kind() != "method_definition" {
                continue;
            }
            match c.kind() {
                "formal_parameters" => self.parameters(c, cx),
                _ if f.child_by_field_name("parameter") == Some(c) => {}
                _ => self.visit(c, cx),
            }
        }
        self.shadow.truncate(mark);
    }

    fn parameters(&mut self, list: Node, cx: &Cx) {
        for p in children(list) {
            match p.kind() {
                "required_parameter" | "optional_parameter" => {
                    let pat = p.child_by_field_name("pattern");
                    for c in children(p) {
                        if Some(c) == pat {
                            self.shadow_pattern(c);
                        } else {
                            self.visit(c, cx);
                        }
                    }
                }
                "assignment_pattern" => {
                    if let Some(l) = p.child_by_field_name("left") {
                        self.shadow_pattern(l);
                    }
                    if let Some(r) = p.child_by_field_name("right") {
                        self.visit(r, cx);
                    }
                }
                "identifier" | "object_pattern" | "array_pattern" | "rest_pattern" => {
                    self.shadow_pattern(p)
                }
                _ => self.visit(p, cx),
            }
        }
    }

    fn declarator(&mut self, d: Node, cx: &Cx) {
        let name = d.child_by_field_name("name");
        for c in children(d) {
            if Some(c) == name {
                if !cx.top {
                    self.shadow_pattern(c);
                }
            } else {
                self.visit(c, &cx.nested());
            }
        }
    }

    fn declaration(&mut self, decl: Node, outer: Node, exported: bool, cx: &Cx) {
        let kind = decl.kind();
        let name_node = decl.child_by_field_name("name");
        match kind {
            "function_declaration" | "generator_function_declaration" => {
                let Some(nn) = name_node else {
                    return self.function_inside(decl, &cx.nested());
                };
                let name = self.text(nn);
                let qn: Rc<str> = Rc::from(join(&cx.container, name));
                let sig = self.signature(outer, decl.child_by_field_name("body"));
                self.add_symbol(
                    name,
                    &qn,
                    SymbolKind::Function,
                    outer,
                    sig,
                    exported,
                    Some(decl),
                    cx.container.as_ref(),
                );
                self.function_inside(decl, &cx.inside(&qn, true));
            }
            "class_declaration" | "abstract_class_declaration" if cx.top => {
                let name = name_node.map(|n| self.text(n)).unwrap_or("default");
                self.class(decl, outer, name, exported, cx);
            }
            "interface_declaration" | "type_alias_declaration" | "enum_declaration" if cx.top => {
                let Some(nn) = name_node else {
                    return self.visit_children(decl, &cx.nested());
                };
                let name = self.text(nn);
                let qn: Rc<str> = Rc::from(join(&cx.container, name));
                let (sk, body) = match kind {
                    "interface_declaration" => {
                        (SymbolKind::Interface, decl.child_by_field_name("body"))
                    }
                    "enum_declaration" => (SymbolKind::Enum, decl.child_by_field_name("body")),
                    _ => (SymbolKind::Type, None),
                };
                let sig = self.signature(outer, body);
                self.add_symbol(
                    name,
                    &qn,
                    sk,
                    outer,
                    sig,
                    exported,
                    None,
                    cx.container.as_ref(),
                );
                let inner = if sk == SymbolKind::Enum {
                    cx.inside(&qn, false)
                } else {
                    Cx {
                        in_type: true,
                        ..cx.inside(&qn, false)
                    }
                };
                for c in children(decl) {
                    if Some(c) != name_node {
                        self.visit(c, &inner);
                    }
                }
            }
            "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration" => {
                let inner = if matches!(kind, "interface_declaration" | "type_alias_declaration") {
                    cx.typed()
                } else {
                    cx.nested()
                };
                for c in children(decl) {
                    if Some(c) != name_node {
                        self.visit(c, &inner);
                    }
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                self.variables(decl, outer, exported, cx)
            }
            "internal_module" | "module" => {
                let body = decl.child_by_field_name("body");
                let named_ns = name_node.filter(|n| n.kind() != "string");
                let inner = match (named_ns, cx.top) {
                    (Some(nn), true) => {
                        let name = self.text(nn);
                        let qn: Rc<str> = Rc::from(join(&cx.container, name));
                        let sig = self.signature(outer, body);
                        self.add_symbol(
                            name,
                            &qn,
                            SymbolKind::Module,
                            outer,
                            sig,
                            exported,
                            None,
                            cx.container.as_ref(),
                        );
                        Cx {
                            top: true,
                            ..cx.inside(&qn, false)
                        }
                    }
                    _ => Cx {
                        top: cx.top,
                        ..cx.nested()
                    },
                };
                if let Some(b) = body {
                    for st in children(b) {
                        self.visit(st, &inner);
                    }
                }
            }
            "ambient_declaration" => {
                for c in children(decl) {
                    match c.kind() {
                        "function_signature" => self.visit_children(c, &cx.nested()),
                        _ => self.visit(c, cx),
                    }
                }
            }
            _ => self.visit(decl, cx),
        }
    }

    fn variables(&mut self, decl: Node, outer: Node, exported: bool, cx: &Cx) {
        let declarators: Vec<Node> = children(decl)
            .filter(|d| d.kind() == "variable_declarator")
            .collect();
        let single = declarators.len() == 1;
        let is_const = decl
            .child_by_field_name("kind")
            .is_some_and(|k| k.kind() == "const");
        for d in declarators {
            let name = d.child_by_field_name("name");
            let value = d.child_by_field_name("value");
            let span = if single { outer } else { d };
            let simple_name = name.filter(|n| n.kind() == "identifier");
            if cx.top
                && let (Some(nn), Some(v)) = (simple_name, value)
                && is_lambda(v)
            {
                let nm = self.text(nn);
                let qn: Rc<str> = Rc::from(join(&cx.container, nm));
                let sig = if single {
                    self.signature(outer, v.child_by_field_name("body"))
                } else {
                    self.signature(d, v.child_by_field_name("body"))
                };
                self.add_symbol(
                    nm,
                    &qn,
                    SymbolKind::Function,
                    span,
                    sig,
                    exported,
                    Some(v),
                    cx.container.as_ref(),
                );
                if let Some(t) = d.child_by_field_name("type") {
                    self.visit(t, &cx.inside(&qn, false));
                }
                self.function_inside(v, &cx.inside(&qn, true));
                continue;
            }
            if cx.top
                && exported
                && let Some(nn) = simple_name
            {
                let nm = self.text(nn);
                let qn: Rc<str> = Rc::from(join(&cx.container, nm));
                let kind = if is_const {
                    SymbolKind::Constant
                } else {
                    SymbolKind::Variable
                };
                let head_end = value.map(|v| v.start_byte()).unwrap_or(d.end_byte());
                let start = if single {
                    outer.start_byte()
                } else {
                    d.start_byte()
                };
                let mut sig = self.src[start..head_end]
                    .trim_end()
                    .trim_end_matches('=')
                    .trim_end()
                    .to_string();
                if let Some(v) = value {
                    let vt = short(self.text(v), 200);
                    if vt.chars().count() <= 60 {
                        sig = format!("{sig} = {vt}");
                    }
                }
                self.add_symbol(
                    nm,
                    &qn,
                    kind,
                    span,
                    short(&sig, 200),
                    exported,
                    None,
                    cx.container.as_ref(),
                );
                let inner = cx.inside(&qn, false);
                for c in children(d) {
                    if Some(c) != name {
                        self.visit(c, &inner);
                    }
                }
                continue;
            }
            self.declarator(
                d,
                &Cx {
                    top: cx.top,
                    ..cx.nested()
                },
            );
        }
    }

    fn class(&mut self, decl: Node, outer: Node, name: &str, exported: bool, cx: &Cx) {
        let qn: Rc<str> = Rc::from(join(&cx.container, name));
        let body = decl.child_by_field_name("body");
        let sig = self.signature(outer, body);
        self.add_symbol(
            name,
            &qn,
            SymbolKind::Class,
            outer,
            sig,
            exported,
            None,
            cx.container.as_ref(),
        );
        let ccx = cx.inside(&qn, false);
        let name_node = decl.child_by_field_name("name");
        for c in children(decl) {
            if Some(c) == name_node || Some(c) == body {
                continue;
            }
            self.visit(c, &ccx);
        }
        let Some(body) = body else {
            return;
        };
        for m in children(body) {
            match m.kind() {
                "method_definition" => {
                    let Some(mn) = m.child_by_field_name("name") else {
                        self.visit(m, &ccx);
                        continue;
                    };
                    if mn.kind() == "computed_property_name" {
                        self.visit(mn, &ccx);
                    }
                    let mname = self.text(mn);
                    let mqn: Rc<str> = Rc::from(format!("{qn}.{mname}"));
                    let private = mname.starts_with('#') || self.is_private(m);
                    if private {
                        self.private_members.insert(mqn.to_string());
                    }
                    let sig = self.signature(m, m.child_by_field_name("body"));
                    self.add_symbol(
                        mname,
                        &mqn,
                        SymbolKind::Method,
                        m,
                        sig,
                        exported && !private,
                        Some(m),
                        Some(&qn),
                    );
                    self.function_inside(m, &ccx.inside(&mqn, true));
                }
                "public_field_definition" | "field_definition" => {
                    let value = m.child_by_field_name("value");
                    let fname = m
                        .child_by_field_name("name")
                        .or_else(|| m.child_by_field_name("property"));
                    if let (Some(v), Some(fname)) = (value, fname)
                        && is_lambda(v)
                    {
                        let mname = self.text(fname);
                        let mqn: Rc<str> = Rc::from(format!("{qn}.{mname}"));
                        let private = mname.starts_with('#') || self.is_private(m);
                        if private {
                            self.private_members.insert(mqn.to_string());
                        }
                        let sig = self.signature(m, v.child_by_field_name("body"));
                        self.add_symbol(
                            mname,
                            &mqn,
                            SymbolKind::Method,
                            m,
                            sig,
                            exported && !private,
                            Some(v),
                            Some(&qn),
                        );
                        for c in children(m) {
                            if c == v {
                                self.function_inside(v, &ccx.inside(&mqn, true));
                            } else if Some(c) != Some(fname) {
                                self.visit(c, &ccx);
                            }
                        }
                    } else {
                        for c in children(m) {
                            if Some(c) != fname
                                || fname.is_some_and(|f| f.kind() == "computed_property_name")
                            {
                                self.visit(c, &ccx);
                            }
                        }
                    }
                }
                _ => self.visit(m, &ccx),
            }
        }
    }

    /// `module.exports = x`, `module.exports = { a, b: c }`, `exports.a = b` and
    /// `module.exports.a = b` at top level mark the named declarations exported.
    fn commonjs_exports(&mut self, st: Node) {
        let Some(a) = children(st).find(|c| c.kind() == "assignment_expression") else {
            return;
        };
        let (Some(left), Some(right)) = (
            a.child_by_field_name("left"),
            a.child_by_field_name("right"),
        ) else {
            return;
        };
        let target = self.text(left);
        let whole = target == "module.exports";
        let member = left.kind() == "member_expression"
            && left
                .child_by_field_name("object")
                .is_some_and(|o| matches!(self.text(o), "exports" | "module.exports"));
        if !whole && !member {
            return;
        }
        match right.kind() {
            "identifier" => self.export_only(right),
            "object" if whole => {
                for p in children(right) {
                    match p.kind() {
                        "shorthand_property_identifier" => self.export_only(p),
                        "pair" => {
                            if let Some(v) = p
                                .child_by_field_name("value")
                                .filter(|v| v.kind() == "identifier")
                            {
                                self.export_only(v);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn export_only(&mut self, n: Node) {
        self.export_names.push(self.text(n));
        self.export_only.insert(n.id());
    }

    fn is_private(&self, m: Node) -> bool {
        children(m).any(|c| c.kind() == "accessibility_modifier" && self.text(c) == "private")
    }

    fn export_statement(&mut self, n: Node, cx: &Cx) {
        let type_only = has_token(n, "type");
        if let Some(source) = n.child_by_field_name("source") {
            let mut names = Vec::new();
            for c in children(n) {
                match c.kind() {
                    "export_clause" => {
                        for s in children(c).filter(|s| s.kind() == "export_specifier") {
                            let Some(nm) = s.child_by_field_name("name") else {
                                continue;
                            };
                            let alias = s.child_by_field_name("alias").map(|a| self.text(a));
                            let name = self.text(nm).trim_matches(['"', '\'']).to_string();
                            names.push(ImportedName {
                                alias: alias.filter(|a| *a != name).map(str::to_string),
                                name,
                                type_only: type_only || has_token(s, "type"),
                            });
                        }
                    }
                    "namespace_export" => {
                        let alias = children(c).next().map(|a| self.text(a).to_string());
                        names.push(ImportedName {
                            name: "*".into(),
                            alias,
                            type_only,
                        });
                    }
                    _ => {}
                }
            }
            if names.is_empty() && has_token(n, "*") {
                names.push(ImportedName {
                    name: "*".into(),
                    alias: None,
                    type_only,
                });
            }
            self.facts.imports.push(ImportFact {
                specifier: string_value(source, self.src).to_string(),
                kind: ImportKind::Reexport,
                names,
                type_only,
                line: crate::util::line(n),
            });
            return;
        }
        if let Some(decl) = n.child_by_field_name("declaration") {
            return self.declaration(decl, n, true, cx);
        }
        let is_default = has_token(n, "default");
        for c in children(n) {
            match c.kind() {
                "export_clause" => {
                    for s in children(c).filter(|s| s.kind() == "export_specifier") {
                        // An export clause is not a use: it only marks the binding.
                        if let Some(nm) = s.child_by_field_name("name") {
                            self.export_names.push(self.text(nm));
                        }
                    }
                }
                "identifier" if is_default => self.export_names.push(self.text(c)),
                _ if is_default && is_lambda(c) && cx.top => {
                    let qn: Rc<str> = Rc::from(join(&cx.container, "default"));
                    let sig = self.signature(n, c.child_by_field_name("body"));
                    self.add_symbol(
                        "default",
                        &qn,
                        SymbolKind::Function,
                        n,
                        sig,
                        true,
                        Some(c),
                        cx.container.as_ref(),
                    );
                    self.function_inside(c, &cx.inside(&qn, true));
                }
                "class" if is_default && cx.top => {
                    let name = c
                        .child_by_field_name("name")
                        .map(|nn| self.text(nn))
                        .unwrap_or("default");
                    self.class(c, n, name, true, cx);
                }
                _ => self.visit(c, &cx.nested()),
            }
        }
    }

    fn import_statement(&mut self, n: Node) {
        let type_only = has_token(n, "type");
        let mut names = Vec::new();
        let mut kind = ImportKind::Static;
        let mut source = n.child_by_field_name("source");
        for c in children(n) {
            match c.kind() {
                "import_clause" => {
                    for part in children(c) {
                        match part.kind() {
                            "identifier" => names.push(ImportedName {
                                name: "default".into(),
                                alias: Some(self.text(part).to_string()),
                                type_only,
                            }),
                            "namespace_import" => {
                                if let Some(id) = children(part).next() {
                                    names.push(ImportedName {
                                        name: "*".into(),
                                        alias: Some(self.text(id).to_string()),
                                        type_only,
                                    });
                                }
                            }
                            "named_imports" => {
                                for s in children(part).filter(|s| s.kind() == "import_specifier") {
                                    let Some(nm) = s.child_by_field_name("name") else {
                                        continue;
                                    };
                                    let name = self.text(nm).trim_matches(['"', '\'']).to_string();
                                    let alias = s
                                        .child_by_field_name("alias")
                                        .map(|a| self.text(a))
                                        .filter(|a| *a != name)
                                        .map(str::to_string);
                                    names.push(ImportedName {
                                        name,
                                        alias,
                                        type_only: type_only || has_token(s, "type"),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "import_require_clause" => {
                    kind = ImportKind::Require;
                    source = c.child_by_field_name("source");
                    if let Some(id) = children(c).find(|x| x.kind() == "identifier") {
                        names.push(ImportedName {
                            name: "*".into(),
                            alias: Some(self.text(id).to_string()),
                            type_only,
                        });
                    }
                }
                _ => {}
            }
        }
        let Some(source) = source else {
            return;
        };
        self.facts.imports.push(ImportFact {
            specifier: string_value(source, self.src).to_string(),
            kind,
            names,
            type_only,
            line: crate::util::line(n),
        });
    }

    /// The single string-literal argument of a call, if that is all it has.
    fn literal_arg(&self, call: Node) -> Option<&'s str> {
        let args = call.child_by_field_name("arguments")?;
        let mut it = children(args);
        let first = it.next()?;
        let ok = match first.kind() {
            "string" => true,
            "template_string" => children(first).all(|c| c.kind() == "string_fragment"),
            _ => false,
        };
        ok.then(|| string_value(first, self.src))
    }

    fn call(&mut self, n: Node, cx: &Cx) {
        let is_new = n.kind() == "new_expression";
        let callee = if is_new {
            n.child_by_field_name("constructor")
        } else {
            n.child_by_field_name("function")
        };
        let Some(callee) = callee else {
            return;
        };
        if !is_new {
            if callee.kind() == "import" {
                if let Some(spec) = self.literal_arg(n) {
                    self.facts.imports.push(ImportFact {
                        specifier: spec.to_string(),
                        kind: ImportKind::Dynamic,
                        names: Vec::new(),
                        type_only: false,
                        line: crate::util::line(n),
                    });
                }
                return;
            }
            if callee.kind() == "identifier"
                && self.text(callee) == "require"
                && !self.shadow.contains(&"require")
                && let Some(spec) = self.literal_arg(n)
            {
                let names = self.require_names(n);
                self.facts.imports.push(ImportFact {
                    specifier: spec.to_string(),
                    kind: ImportKind::Require,
                    names,
                    type_only: false,
                    line: crate::util::line(n),
                });
                return;
            }
        }
        let (name, receiver) = match callee.kind() {
            "identifier" | "super" | "this" => (self.text(callee).to_string(), None),
            "member_expression" => {
                let Some(p) = callee.child_by_field_name("property") else {
                    return;
                };
                let recv = callee
                    .child_by_field_name("object")
                    .map(|o| short(self.text(o), 200));
                (self.text(p).to_string(), recv)
            }
            "subscript_expression" => {
                let Some(idx) = callee
                    .child_by_field_name("index")
                    .filter(|i| i.kind() == "string")
                else {
                    return;
                };
                let recv = callee
                    .child_by_field_name("object")
                    .map(|o| short(self.text(o), 200));
                (string_value(idx, self.src).to_string(), recv)
            }
            _ => return,
        };
        let text = short(self.text(callee), 200);
        self.facts.calls.push(CallFact {
            caller: cx.callable.as_deref().map(str::to_string),
            callee: if is_new { format!("new {text}") } else { text },
            name,
            receiver,
            line: crate::util::line(n),
        });
    }

    /// Names bound by `const { a, b: c } = require("x")` / `const x = require("x")`.
    fn require_names(&self, call: Node) -> Vec<ImportedName> {
        let Some(parent) = call.parent().filter(|p| p.kind() == "variable_declarator") else {
            return Vec::new();
        };
        let Some(pat) = parent.child_by_field_name("name") else {
            return Vec::new();
        };
        match pat.kind() {
            "identifier" => vec![ImportedName {
                name: "*".into(),
                alias: Some(self.text(pat).to_string()),
                type_only: false,
            }],
            "object_pattern" => children(pat)
                .filter_map(|p| match p.kind() {
                    "shorthand_property_identifier_pattern" => Some(ImportedName {
                        name: self.text(p).to_string(),
                        alias: None,
                        type_only: false,
                    }),
                    "pair_pattern" => {
                        let k = p.child_by_field_name("key")?;
                        let v = p.child_by_field_name("value")?;
                        let name = self.text(k).to_string();
                        let alias = Some(self.text(v).to_string()).filter(|a| *a != name);
                        Some(ImportedName {
                            name,
                            alias,
                            type_only: false,
                        })
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}
