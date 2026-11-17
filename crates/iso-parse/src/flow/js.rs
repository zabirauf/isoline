//! TypeScript / JavaScript table and unit discovery for the shared walker.
//!
//! Discovery mirrors analyze.js: units are the file's top-level function declarations plus
//! the methods of the controller object a factory function returns. Isoline adds two unit
//! kinds analyze.js ignores (see `fixtures/ts/DEVIATIONS.md`): top-level `const x = () => …`
//! functions and class methods. Neither changes how calls inside the analyze.js units are
//! classified.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use iso_core::{FlowUnit, Language};
use tree_sitter::{Node, Tree};

use super::spec::{FlowSpec, KindNames, SpecDef, Style, is};
use super::{
    FlowLang, Param, Prepared, Prop, Scope, UnitSeed, end_line, field, line, named, walk_units,
};

pub static TS_DEF: SpecDef = SpecDef {
    skip: &[
        "comment",
        "html_comment",
        "type_annotation",
        "type_arguments",
        "type_parameters",
        "type_query",
        "type_predicate_annotation",
        "asserts_annotation",
        "omitting_type_annotation",
        "adding_type_annotation",
        "opting_type_annotation",
        "interface_declaration",
        "type_alias_declaration",
    ],
    block: &["statement_block"],
    var_decl: &["lexical_declaration", "variable_declaration"],
    expr_stmt: &["expression_statement"],
    transparent: &[
        "parenthesized_expression",
        "as_expression",
        "non_null_expression",
        "satisfies_expression",
    ],
    if_: &["if_statement"],
    ternary: &["ternary_expression"],
    binary: &["binary_expression"],
    return_: &["return_statement"],
    throw: &["throw_statement"],
    try_: &["try_statement"],
    for_c: &["for_statement"],
    while_: &["while_statement"],
    do_: &["do_statement"],
    for_each: &["for_in_statement"],
    switch: &["switch_statement"],
    call: &["call_expression"],
    new: &["new_expression"],
    lambda: &[
        "arrow_function",
        "function_expression",
        "function",
        "generator_function",
    ],
    object: &["object"],
    fn_decl: &[],
    assign: &[],
    match_: &[],
    with: &[],
    comprehension: &[],
    try_op: &[],
    macro_: &[],
    logical_ops: &[("&&", "only if"), ("||", "unless"), ("??", "if nullish")],
    iter_methods: &[
        "map",
        "forEach",
        "filter",
        "reduce",
        "some",
        "every",
        "flatMap",
        "find",
        "findIndex",
        "sort",
    ],
    builtins: &[
        "Object", "Date", "JSON", "Math", "Array", "Promise", "Number", "String", "Reflect",
        "console", "Symbol", "Error",
    ],
    kinds: KindNames {
        identifier: "identifier",
        paren: "parenthesized_expression",
        member: "member_expression",
        subscript: "subscript_expression",
        await_: "await_expression",
        declarator: "variable_declarator",
        pair: "pair",
        shorthand: "shorthand_property_identifier",
        spread: "spread_element",
        method: "method_definition",
        return_: "return_statement",
        object: "object",
        else_clause: "else_clause",
        empty_statement: "empty_statement",
        switch_case: "switch_case",
        template_string: "template_string",
        optional_chain: &["?.", "optional_chain"],
    },
    style: Style::JS,
};

/// The flow table for one TS/JS grammar.
pub struct JsFlow {
    spec: FlowSpec,
}

/// The walker hooks for a TS/JS language (`None` for other languages).
pub fn js_flow(lang: Language) -> Option<&'static JsFlow> {
    static TS: OnceLock<JsFlow> = OnceLock::new();
    static TSX: OnceLock<JsFlow> = OnceLock::new();
    static JS: OnceLock<JsFlow> = OnceLock::new();
    let cell = match lang {
        Language::TypeScript => &TS,
        Language::Tsx => &TSX,
        Language::JavaScript | Language::Jsx => &JS,
        _ => return None,
    };
    Some(cell.get_or_init(|| JsFlow {
        spec: FlowSpec::build(&crate::grammar(lang).expect("bundled grammar"), &TS_DEF),
    }))
}

/// Call-flow units for one TS/JS file.
pub fn flow_units(lang: Language, src: &str, tree: &Tree) -> Vec<FlowUnit> {
    let Some(jf) = js_flow(lang) else {
        return Vec::new();
    };
    crate::util::with_stack_for(crate::util::tree_depth(tree), || {
        let prepared = jf.prepare(src, tree.root_node());
        walk_units(jf, src, &prepared)
    })
}

fn has_token(n: Node, tok: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .any(|ch| !ch.is_named() && ch.kind() == tok)
}

/// String literal content without its quotes.
pub fn string_value<'s>(n: Node, src: &'s str) -> &'s str {
    let t = &src[n.byte_range()];
    if t.len() >= 2 { &t[1..t.len() - 1] } else { t }
}

impl FlowLang for JsFlow {
    fn spec(&self) -> &FlowSpec {
        &self.spec
    }

    fn params<'t, 's>(&self, f: Node<'t>, src: &'s str) -> Vec<Param<'t, 's>> {
        let mut out = Vec::new();
        self.each_param(f, |p, _name| {
            let (ident, default) = self.param_parts(p, src);
            out.push(Param { ident, default });
        });
        out
    }

    fn body<'t>(&self, f: Node<'t>) -> Option<Node<'t>> {
        field(f, self.spec.f.body)
    }

    fn is_plain_method(&self, m: Node, _src: &str) -> bool {
        !has_token(m, "get") && !has_token(m, "set")
    }

    fn misplaced_prefix<'t>(&self, callee: Node<'t>) -> Option<(Node<'t>, Node<'t>)> {
        misplaced_prefix(callee)
    }

    fn is_chain_top(&self, n: Node) -> bool {
        let Some(p) = n.parent() else {
            return true;
        };
        let base = match p.kind() {
            "call_expression" => p.child_by_field_name("function"),
            "member_expression" | "subscript_expression" => p.child_by_field_name("object"),
            "non_null_expression" => return false,
            _ => None,
        };
        base != Some(n)
    }
}

/// tree-sitter-typescript attaches some prefix operators to the base of a postfix chain:
/// `await f<T>(x)` parses as `(await f)<T>(x)` and `!a[i]!.has(k)` as `(!a[i])!.has(k)`.
/// Neither shape can come from valid source (it would need parentheses), so finding a
/// prefix operator at the base of a callee chain means the grammar misplaced it. Returns the
/// operator node and its operand.
pub fn misplaced_prefix(callee: Node) -> Option<(Node, Node)> {
    let mut n = callee;
    loop {
        let next = match n.kind() {
            "call_expression" => n.child_by_field_name("function"),
            "member_expression" | "subscript_expression" => n.child_by_field_name("object"),
            "non_null_expression" => named(n).next(),
            "await_expression" | "unary_expression" => {
                let operand = named(n).last()?;
                return Some((n, operand));
            }
            _ => None,
        };
        n = next?;
    }
}

impl JsFlow {
    fn each_param<'t>(&self, f: Node<'t>, mut visit: impl FnMut(Node<'t>, Option<Node<'t>>)) {
        let sp = &self.spec;
        if let Some(p) = field(f, sp.f.parameter) {
            visit(p, Some(p));
            return;
        }
        if let Some(list) = field(f, sp.f.parameters) {
            for p in named(list) {
                visit(p, None);
            }
        }
    }

    /// The binding pattern of one parameter node and its default value.
    fn pattern_and_default<'t>(&self, p: Node<'t>) -> (Node<'t>, Option<Node<'t>>) {
        let f = &self.spec.f;
        match p.kind() {
            "required_parameter" | "optional_parameter" => {
                (field(p, f.pattern).unwrap_or(p), field(p, f.value))
            }
            "assignment_pattern" => (field(p, f.left).unwrap_or(p), field(p, f.right)),
            _ => (p, None),
        }
    }

    /// TypeScript's `ParameterDeclaration.name`: the pattern without a rest `...`.
    fn param_name<'t>(&self, p: Node<'t>) -> (Node<'t>, Option<Node<'t>>) {
        let (mut pat, default) = self.pattern_and_default(p);
        if pat.kind() == "rest_pattern"
            && let Some(inner) = named(pat).next()
        {
            pat = inner;
        }
        (pat, default)
    }

    fn param_parts<'t, 's>(
        &self,
        p: Node<'t>,
        src: &'s str,
    ) -> (Option<&'s str>, Option<Node<'t>>) {
        let (pat, default) = self.param_name(p);
        let ident = match pat.kind() {
            "identifier" | "this" => Some(&src[pat.byte_range()]),
            _ => None,
        };
        (ident, default)
    }

    /// analyze.js unit params: `name` or `name = …`, name text as written.
    pub fn param_labels(&self, f: Node, src: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.each_param(f, |p, _| {
            let (pat, default) = self.param_name(p);
            let mut s = src[pat.byte_range()].to_string();
            if default.is_some() {
                s.push_str(" = …");
            }
            out.push(s);
        });
        out
    }

    fn is_lambda(&self, n: Option<Node>) -> bool {
        n.is_some_and(|n| self.spec.cat(n) == super::spec::Cat::Lambda)
    }

    /// The declaration inside `export …` / `declare …` wrappers.
    fn unwrap_decl<'t>(&self, st: Node<'t>) -> Node<'t> {
        match st.kind() {
            "export_statement" => field(st, self.spec.f.declaration)
                .or_else(|| field(st, self.spec.f.value))
                .unwrap_or(st),
            "ambient_declaration" => named(st).next().unwrap_or(st),
            _ => st,
        }
    }

    fn scope_of<'t, 's>(
        &self,
        body: Option<Node<'t>>,
        src: &'s str,
        top_fns: &HashMap<&'s str, Node<'t>>,
        imports: &HashMap<String, String>,
    ) -> Scope<'t, 's> {
        let mut sc = Scope::default();
        let Some(body) = body else {
            return sc;
        };
        let sp = &self.spec;
        let mut c = body.walk();
        loop {
            let n = c.node();
            if is(&sp.k.declarator, n)
                && let (Some(name), Some(value)) = (field(n, sp.f.name), field(n, sp.f.value))
                && is(&sp.k.identifier, name)
            {
                let nm = &src[name.byte_range()];
                if self.is_lambda(Some(value)) {
                    sc.locals.insert(nm, value);
                }
                let mut init = value;
                if is(&sp.k.await_, init)
                    && let Some(inner) = named(init).next()
                {
                    init = inner;
                }
                if sp.cat(init) == super::spec::Cat::Call
                    && !field(init, sp.f.arguments).is_some_and(|a| is(&sp.k.template_string, a))
                    && let Some(callee) = field(init, sp.f.function)
                    // `await f<T>()` (see `misplaced_prefix`).
                    && let Some(callee) = if is(&sp.k.await_, callee) {
                        named(callee).last()
                    } else {
                        Some(callee)
                    }
                    && is(&sp.k.identifier, callee)
                {
                    let fname = &src[callee.byte_range()];
                    if let Some((k, _)) = top_fns.get_key_value(fname) {
                        sc.origins.insert(nm, k);
                    } else if imports.contains_key(fname) {
                        sc.value_from.insert(nm, fname);
                    }
                }
            }
            if c.goto_first_child() {
                continue;
            }
            loop {
                if c.node() == body {
                    return sc;
                }
                if c.goto_next_sibling() {
                    break;
                }
                if !c.goto_parent() {
                    return sc;
                }
            }
        }
    }

    /// analyze.js `returnedObject`: the first `return { … }` outside nested functions.
    fn returned_object<'t>(&self, n: Node<'t>, src: &str) -> Option<Node<'t>> {
        let sp = &self.spec;
        for c in named(n) {
            if self.is_lambda(Some(c))
                || (is(&sp.k.method, c) && self.is_method_declaration(c, src))
            {
                continue;
            }
            if is(&sp.k.return_, c)
                && let Some(e) = named(c).find(|e| sp.cat(*e) != super::spec::Cat::Skip)
                && is(&sp.k.object, e)
            {
                return Some(e);
            }
            if let Some(found) = self.returned_object(c, src) {
                return Some(found);
            }
        }
        None
    }

    /// TypeScript's `MethodDeclaration`: not an accessor, not a class constructor.
    fn is_method_declaration(&self, m: Node, src: &str) -> bool {
        if has_token(m, "get") || has_token(m, "set") {
            return false;
        }
        let in_class = m.parent().is_some_and(|p| p.kind() == "class_body");
        let is_ctor =
            field(m, self.spec.f.name).is_some_and(|n| &src[n.byte_range()] == "constructor");
        !(in_class && is_ctor)
    }

    pub fn prepare<'t, 's>(&self, src: &'s str, root: Node<'t>) -> Prepared<'t, 's> {
        let sp = &self.spec;
        let f = &sp.f;
        let text = |n: Node| -> &'s str { &src[n.byte_range()] };

        // Pass 1: imports and top-level functions.
        let mut imports: HashMap<String, String> = HashMap::new();
        let mut top_order: Vec<&'s str> = Vec::new();
        let mut top_fns: HashMap<&'s str, Node<'t>> = HashMap::new();
        let mut top_outer: HashMap<&'s str, Node<'t>> = HashMap::new();
        for st in named(root) {
            let decl = self.unwrap_decl(st);
            match decl.kind() {
                "import_statement" => {
                    let Some(source) = field(decl, f.source) else {
                        continue;
                    };
                    let module = string_value(source, src);
                    for clause in named(decl).filter(|c| c.kind() == "import_clause") {
                        for ni in named(clause).filter(|c| c.kind() == "named_imports") {
                            for spec in named(ni).filter(|c| c.kind() == "import_specifier") {
                                if let Some(local) = field(spec, f.alias).or(field(spec, f.name)) {
                                    imports.insert(text(local).to_string(), module.to_string());
                                }
                            }
                        }
                    }
                }
                "function_declaration"
                | "generator_function_declaration"
                | "function_signature" => {
                    if let Some(name) = field(decl, f.name) {
                        let nm = text(name);
                        if !top_fns.contains_key(nm) {
                            top_order.push(nm);
                        }
                        top_fns.insert(nm, decl);
                        top_outer.insert(nm, st);
                    }
                }
                _ => {}
            }
        }

        let mut prepared = Prepared {
            imports,
            import_paths: HashMap::new(),
            top_fns: top_fns.keys().copied().collect(),
            factory_props: HashMap::new(),
            units: Vec::new(),
        };
        let mut index: HashMap<String, usize> = HashMap::new();
        let push = |prepared: &mut Prepared<'t, 's>,
                    index: &mut HashMap<String, usize>,
                    u: UnitSeed<'t, 's>| {
            match index.get(&u.id) {
                Some(&i) => prepared.units[i] = u,
                None => {
                    index.insert(u.id.clone(), prepared.units.len());
                    prepared.units.push(u);
                }
            }
        };

        // Pass 2: top-level function units and their controller methods.
        for name in &top_order {
            let decl = top_fns[name];
            let outer = top_outer[name];
            self.add_function_unit(
                &mut prepared,
                &mut index,
                &push,
                src,
                &top_fns,
                name,
                decl,
                decl,
                line(outer),
                end_line(outer),
            );
        }

        // Isoline additions: top-level arrow-function consts and class methods.
        for st in named(root) {
            let decl = self.unwrap_decl(st);
            match decl.kind() {
                "lexical_declaration" | "variable_declaration" => {
                    let declarators: Vec<Node> =
                        named(decl).filter(|d| is(&sp.k.declarator, *d)).collect();
                    let single = declarators.len() == 1;
                    for d in declarators {
                        let (Some(name), Some(value)) = (field(d, f.name), field(d, f.value))
                        else {
                            continue;
                        };
                        if !is(&sp.k.identifier, name) || !self.is_lambda(Some(value)) {
                            continue;
                        }
                        let nm = text(name);
                        if index.contains_key(nm) {
                            continue;
                        }
                        let span = if single { st } else { d };
                        self.add_function_unit(
                            &mut prepared,
                            &mut index,
                            &push,
                            src,
                            &top_fns,
                            nm,
                            value,
                            value,
                            line(span),
                            end_line(span),
                        );
                    }
                }
                "class_declaration" | "abstract_class_declaration" => {
                    let (Some(cname), Some(body)) = (field(decl, f.name), field(decl, f.body))
                    else {
                        continue;
                    };
                    let cname = text(cname);
                    for m in named(body) {
                        let (fnode, mname) = match m.kind() {
                            "method_definition" => (Some(m), field(m, f.name)),
                            "public_field_definition" | "field_definition" => {
                                let v = field(m, f.value);
                                let n = field(m, f.name).or_else(|| field(m, f.property));
                                (v.filter(|v| self.is_lambda(Some(*v))), n)
                            }
                            _ => (None, None),
                        };
                        let (Some(fnode), Some(mname)) = (fnode, mname) else {
                            continue;
                        };
                        if field(fnode, f.body).is_none() {
                            continue;
                        }
                        let mname = text(mname);
                        let id = format!("{cname}.{mname}");
                        if index.contains_key(&id) {
                            continue;
                        }
                        let body_n = field(fnode, f.body);
                        let scope = self.scope_of(body_n, src, &top_fns, &prepared.imports);
                        let u = UnitSeed {
                            id,
                            owner: cname.to_string(),
                            method: Some(mname.to_string()),
                            node: fnode,
                            scope: Rc::new(scope),
                            params: self.params(fnode, src),
                            start: line(m),
                            end: end_line(m),
                            is_async: has_token(fnode, "async"),
                            param_labels: self.param_labels(fnode, src),
                            factory: None,
                            self_type: None,
                            prefix: String::new(),
                        };
                        push(&mut prepared, &mut index, u);
                    }
                }
                _ => {}
            }
        }
        prepared
    }

    /// One function unit plus, when it returns a controller object, a unit per method.
    #[allow(clippy::too_many_arguments)]
    fn add_function_unit<'t, 's>(
        &self,
        prepared: &mut Prepared<'t, 's>,
        index: &mut HashMap<String, usize>,
        push: &impl Fn(&mut Prepared<'t, 's>, &mut HashMap<String, usize>, UnitSeed<'t, 's>),
        src: &'s str,
        top_fns: &HashMap<&'s str, Node<'t>>,
        name: &'s str,
        fn_node: Node<'t>,
        async_node: Node<'t>,
        start: u32,
        end: u32,
    ) {
        let sp = &self.spec;
        let f = &sp.f;
        let body = field(fn_node, f.body);
        let scope = Rc::new(self.scope_of(body, src, top_fns, &prepared.imports));
        let params = self.params(fn_node, src);
        let unit = UnitSeed {
            id: name.to_string(),
            owner: name.to_string(),
            method: None,
            node: fn_node,
            scope: scope.clone(),
            params: params.clone(),
            start,
            end,
            is_async: has_token(async_node, "async"),
            param_labels: self.param_labels(fn_node, src),
            factory: None,
            self_type: None,
            prefix: String::new(),
        };
        push(prepared, index, unit);

        let Some(obj) = body.and_then(|b| self.returned_object(b, src)) else {
            return;
        };
        let mut props: Vec<(String, Prop)> = Vec::new();
        let set = |props: &mut Vec<(String, Prop)>, k: &str, v: Prop| {
            if let Some(slot) = props.iter_mut().find(|(kk, _)| kk == k) {
                slot.1 = v;
            } else {
                props.push((k.to_string(), v));
            }
        };
        for p in named(obj) {
            let (pn, body_node, shorthand) = if is(&sp.k.pair, p) {
                let Some(key) = field(p, f.key).filter(|k| k.kind() == "property_identifier")
                else {
                    continue;
                };
                let v = field(p, f.value);
                (
                    &src[key.byte_range()],
                    v.filter(|v| self.is_lambda(Some(*v))),
                    false,
                )
            } else if is(&sp.k.method, p) {
                let Some(key) = field(p, f.name).filter(|k| k.kind() == "property_identifier")
                else {
                    continue;
                };
                let plain = self.is_plain_method(p, src);
                (&src[key.byte_range()], plain.then_some(p), false)
            } else if is(&sp.k.shorthand, p) {
                let pn = &src[p.byte_range()];
                (pn, scope.locals.get(pn).copied(), true)
            } else {
                continue;
            };
            if let Some(b) = body_node {
                let id = format!("{name}.{pn}");
                let mut uparams = params.clone();
                uparams.extend(self.params(b, src));
                let u = UnitSeed {
                    id: id.clone(),
                    owner: name.to_string(),
                    method: Some(pn.to_string()),
                    node: b,
                    scope: scope.clone(),
                    params: uparams,
                    start: line(b),
                    end: end_line(b),
                    is_async: has_token(b, "async"),
                    param_labels: self.param_labels(b, src),
                    factory: None,
                    self_type: None,
                    prefix: String::new(),
                };
                push(prepared, index, u);
                set(&mut props, pn, Prop::Unit(id));
            } else if shorthand && let Some(fr) = scope.value_from.get(pn) {
                set(&mut props, pn, Prop::From(fr.to_string()));
            } else {
                set(&mut props, pn, Prop::Value);
            }
        }
        if props.iter().any(|(_, v)| matches!(v, Prop::Unit(_))) {
            if let Some(&i) = index.get(name) {
                prepared.units[i].factory = Some(name.to_string());
            }
            prepared.factory_props.insert(name.to_string(), props);
        }
    }
}
