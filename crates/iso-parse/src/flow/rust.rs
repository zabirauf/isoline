//! Rust table and unit discovery for the shared walker. The mapping is documented in
//! `fixtures/rust/FLOW.md`.
//!
//! Units: free functions (`f`), methods of `impl` blocks (`Type.method`, trait impls
//! included) and trait methods with a default body (`Trait.method`), inside inline modules
//! too (`tests.case`, `shapes.Circle.area`): the extractor's qualified names, so cards link
//! to them. Nested `fn` items and `let f = |…| …` closures are locals.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

use iso_core::{FileFacts, FlowTarget, FlowTargetKind, FlowUnit, ImportKind, Language};
use tree_sitter::{Node, Tree};

use super::spec::{FlowSpec, KindNames, SpecDef, Style};
use super::{
    Arm, FlowLang, Param, PathCx, Prepared, Scope, SymbolNames, TokenCall, TokenCallee, UnitSeed,
    collapse, end_line, line, named, other_target, target, walk_units,
};

pub static RS_DEF: SpecDef = SpecDef {
    skip: &[
        "line_comment",
        "block_comment",
        "attribute_item",
        "inner_attribute_item",
        "use_declaration",
        "struct_item",
        "enum_item",
        "union_item",
        "type_item",
        "trait_item",
        "impl_item",
        "mod_item",
        "const_item",
        "static_item",
        "macro_definition",
        "extern_crate_declaration",
        "foreign_mod_item",
        "function_signature_item",
        "type_arguments",
        "type_parameters",
        "where_clause",
        "lifetime",
        "primitive_type",
        "type_identifier",
        "generic_type",
        "reference_type",
        "scoped_type_identifier",
    ],
    block: &["block"],
    var_decl: &[],
    expr_stmt: &["expression_statement"],
    transparent: &["parenthesized_expression"],
    if_: &["if_expression"],
    ternary: &[],
    binary: &["binary_expression"],
    return_: &["return_expression"],
    throw: &[],
    try_: &[],
    for_c: &[],
    while_: &["while_expression", "loop_expression"],
    do_: &[],
    for_each: &["for_expression"],
    switch: &[],
    call: &["call_expression"],
    new: &[],
    lambda: &["closure_expression"],
    object: &[],
    fn_decl: &["function_item"],
    assign: &[
        "let_declaration",
        "assignment_expression",
        "compound_assignment_expr",
    ],
    match_: &["match_expression"],
    with: &[],
    comprehension: &[],
    try_op: &["try_expression"],
    macro_: &["macro_invocation"],
    logical_ops: &[("&&", "only if"), ("||", "unless")],
    iter_methods: &[
        "map",
        "for_each",
        "filter",
        "filter_map",
        "flat_map",
        "fold",
        "try_fold",
        "any",
        "all",
        "find",
        "find_map",
        "position",
        "take_while",
        "skip_while",
        "map_while",
        "inspect",
        "try_for_each",
        "retain",
        "sort_by",
        "sort_by_key",
        "sort_unstable_by",
        "sort_unstable_by_key",
        "max_by",
        "max_by_key",
        "min_by",
        "min_by_key",
        "partition",
        "scan",
        "reduce",
        "dedup_by_key",
        "chunk_by",
    ],
    builtins: &[],
    kinds: KindNames {
        identifier: "identifier",
        paren: "parenthesized_expression",
        member: "field_expression",
        subscript: "",
        await_: "await_expression",
        declarator: "",
        pair: "",
        shorthand: "",
        spread: "",
        method: "",
        return_: "return_expression",
        object: "",
        else_clause: "else_clause",
        empty_statement: "",
        switch_case: "",
        template_string: "",
        optional_chain: &[],
    },
    style: Style {
        builtin_fns: &["Some", "Ok", "Err", "drop"],
        iter_fns: &[],
        self_names: &["self"],
        member_roots_ext: false,
        parens: false,
        for_op: "in",
        else_line_from_clause: false,
        match_kw: "match",
        catch_kw: "catch",
        catch_clauses: &[],
        try_else_clauses: &[],
        finally_clauses: &[],
        elif_clauses: &[],
        for_in_clause: "",
        path_kinds: &["scoped_identifier"],
        exit_macros: EXIT_MACROS,
        builtin_macros: &[
            "println",
            "print",
            "eprintln",
            "eprint",
            "format",
            "format_args",
            "write",
            "writeln",
            "vec",
            "assert",
            "assert_eq",
            "assert_ne",
            "debug_assert",
            "debug_assert_eq",
            "debug_assert_ne",
            "dbg",
            "matches",
            "concat",
            "stringify",
            "include_str",
            "include_bytes",
            "env",
            "option_env",
            "cfg",
            "line",
            "column",
            "file",
            "module_path",
            "thread_local",
            "compile_error",
        ],
        member_object: "value",
        member_property: "field",
        subscript_object: "",
    },
};

/// Macros that never return normally. `bail!` (anyhow) returns an error, like `return Err`.
const EXIT_MACROS: &[&str] = &["panic", "unreachable", "todo", "unimplemented", "bail"];

/// Path roots whose functions are built-ins: the standard library and prelude types.
const BUILTIN_ROOTS: &[&str] = &[
    "std",
    "core",
    "alloc",
    "String",
    "Vec",
    "Box",
    "Option",
    "Result",
    "Rc",
    "Arc",
    "Cell",
    "RefCell",
    "Default",
    "Iterator",
    "IntoIterator",
    "From",
    "Into",
    "ToString",
    "ToOwned",
    "Clone",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "f32",
    "f64",
    "bool",
    "char",
    "str",
];

/// Tail expressions that are not a returned value worth an exit: control flow, whose arms
/// carry the values, and statements.
const NOT_A_TAIL_VALUE: &[&str] = &[
    "expression_statement",
    "let_declaration",
    "if_expression",
    "match_expression",
    "loop_expression",
    "while_expression",
    "for_expression",
    "block",
    "unsafe_block",
    "async_block",
    "const_block",
    "return_expression",
    "unit_expression",
    "break_expression",
    "continue_expression",
    "empty_statement",
    "function_item",
    "attribute_item",
];

pub struct RsFlow {
    spec: FlowSpec,
}

fn rs_flow() -> &'static RsFlow {
    static CELL: OnceLock<RsFlow> = OnceLock::new();
    CELL.get_or_init(|| RsFlow {
        spec: FlowSpec::build(
            &crate::grammar(Language::Rust).expect("bundled grammar"),
            &RS_DEF,
        ),
    })
}

/// Call-flow units for one Rust file. `facts` are the extractor's facts for it: imports
/// classify `ext` calls, symbols name the units.
pub fn flow_units(src: &str, tree: &Tree, facts: &FileFacts) -> Vec<FlowUnit> {
    let rf = rs_flow();
    crate::util::with_stack_for(crate::util::tree_depth(tree), || {
        let prepared = rf.prepare(src, tree.root_node(), facts);
        walk_units(rf, src, &prepared)
    })
}

/// Parameter binding name and label (`&self`, `mut n`, `(a, b)`), type left out.
fn param<'s>(p: Node, src: &'s str) -> Option<(Option<&'s str>, String)> {
    let text = |n: Node| &src[n.byte_range()];
    match p.kind() {
        "self_parameter" => Some((None, text(p).to_string())),
        "parameter" => {
            let pat = p.child_by_field_name("pattern")?;
            let ident = (pat.kind() == "identifier").then(|| text(pat));
            Some((ident, src[p.start_byte()..pat.end_byte()].to_string()))
        }
        "identifier" => Some((Some(text(p)), text(p).to_string())),
        "attribute_item" | "variadic_parameter" => None,
        k if k.ends_with("_type") || k == "type_identifier" => None,
        _ => Some((None, text(p).to_string())),
    }
}

/// An `impl` block's type name as the extractor writes it: `Foo<T>` → `Foo`,
/// `crate::a::Foo` → `Foo`, `&'a mut Foo` and `*const Foo` → `Foo`.
fn type_name<'s>(t: Node, src: &'s str) -> &'s str {
    let mut n = t;
    loop {
        let next = match n.kind() {
            "type_identifier" | "primitive_type" => return &src[n.byte_range()],
            "generic_type" | "reference_type" | "pointer_type" => n.child_by_field_name("type"),
            "scoped_type_identifier" => n.child_by_field_name("name"),
            "dynamic_type" | "abstract_type" => n.child_by_field_name("trait"),
            _ => None,
        };
        match next {
            Some(x) => n = x,
            None => {
                let s = &src[n.byte_range()];
                return s.split('<').next().unwrap_or(s).trim();
            }
        }
    }
}

fn is_seg(n: Node) -> bool {
    matches!(
        n.kind(),
        "identifier" | "self" | "super" | "crate" | "primitive_type" | "metavariable"
    )
}

fn is_paren_tree(n: Node) -> bool {
    n.kind() == "token_tree" && n.child(0).is_some_and(|c| c.kind() == "(")
}

impl FlowLang for RsFlow {
    fn spec(&self) -> &FlowSpec {
        &self.spec
    }

    fn params<'t, 's>(&self, f: Node<'t>, src: &'s str) -> Vec<Param<'t, 's>> {
        let Some(list) = f.child_by_field_name("parameters") else {
            return Vec::new();
        };
        named(list)
            .filter_map(|p| param(p, src))
            .map(|(ident, _)| Param {
                ident,
                default: None,
            })
            .collect()
    }

    fn body<'t>(&self, f: Node<'t>) -> Option<Node<'t>> {
        f.child_by_field_name("body")
    }

    fn is_plain_method(&self, _m: Node, _src: &str) -> bool {
        true
    }

    fn unwrap_callee<'t>(&self, callee: Node<'t>) -> Node<'t> {
        if callee.kind() == "generic_function"
            && let Some(f) = callee.child_by_field_name("function")
        {
            return f;
        }
        callee
    }

    fn match_arms<'t>(&self, node: Node<'t>, src: &str) -> (Option<Node<'t>>, Vec<Arm<'t>>) {
        let subject = node.child_by_field_name("value");
        let mut arms = Vec::new();
        if let Some(body) = node.child_by_field_name("body") {
            for arm in named(body).filter(|c| c.kind() == "match_arm") {
                let pat = arm.child_by_field_name("pattern");
                arms.push(Arm {
                    label: pat
                        .map(|p| super::label(&src[p.byte_range()], 40))
                        .unwrap_or_default(),
                    guard: pat.and_then(|p| p.child_by_field_name("condition")),
                    body: arm.child_by_field_name("value"),
                    line: line(arm),
                });
            }
        }
        (subject, arms)
    }

    fn tail_value<'t>(&self, f: Node<'t>, src: &str) -> Option<Node<'t>> {
        if f.kind() != "function_item" || f.child_by_field_name("return_type").is_none() {
            return None;
        }
        let last = named(f.child_by_field_name("body")?).last()?;
        if NOT_A_TAIL_VALUE.contains(&last.kind())
            || (last.kind() == "macro_invocation" && is_exit_macro(last, src))
        {
            return None;
        }
        Some(last)
    }

    fn path_target(&self, segs: &[&str], cx: &PathCx) -> FlowTarget {
        let (Some(&root), Some(&last)) = (segs.first(), segs.last()) else {
            return other_target();
        };
        let full = segs.join("::");
        let has = |id: &str| cx.unit_ids.contains(id);
        if segs.len() >= 2 {
            if root == "Self" {
                let mut t = target(FlowTargetKind::Method, last);
                if let Some(o) = cx.owner {
                    let id = format!("{o}.{last}");
                    if has(&id) {
                        t.unit = Some(id);
                    }
                }
                return t;
            }
            // `Type::f` for a type with an `impl` in this file.
            if segs.len() == 2 {
                for id in [
                    format!("{}{root}.{last}", cx.prefix),
                    format!("{root}.{last}"),
                ] {
                    if has(&id) {
                        let mut t = target(FlowTargetKind::Method, last);
                        t.unit = Some(id);
                        return t;
                    }
                }
            }
            // `module::f` for an inline module of this file.
            let dotted = segs.join(".");
            for id in [format!("{}{dotted}", cx.prefix), dotted] {
                if has(&id) {
                    return target(FlowTargetKind::Fn, id);
                }
            }
            if let Some(base) = cx.file.import_paths.get(root) {
                let mut module = base.clone();
                for s in &segs[1..segs.len() - 1] {
                    module.push_str("::");
                    module.push_str(s);
                }
                let mut t = target(FlowTargetKind::Ext, full);
                t.module = Some(module);
                return t;
            }
            if matches!(root, "crate" | "super" | "self") {
                let mut t = target(FlowTargetKind::Ext, full);
                t.module = Some(segs[..segs.len() - 1].join("::"));
                return t;
            }
            if BUILTIN_ROOTS.contains(&root) {
                return target(FlowTargetKind::Builtin, full);
            }
        }
        other_target()
    }

    fn macro_parts<'t>(&self, m: Node<'t>) -> Option<(Node<'t>, Option<Node<'t>>)> {
        let name = m.child_by_field_name("macro")?;
        let tree = named(m).find(|c| c.kind() == "token_tree");
        Some((name, tree))
    }

    fn token_calls<'t>(&self, tree: Node<'t>, src: &str) -> Vec<TokenCall<'t>> {
        let mut c = tree.walk();
        let mut kids: Vec<Node<'t>> = tree.children(&mut c).collect();
        // Drop the delimiters.
        if kids
            .first()
            .is_some_and(|k| matches!(k.kind(), "(" | "[" | "{"))
        {
            kids.remove(0);
        }
        if kids
            .last()
            .is_some_and(|k| matches!(k.kind(), ")" | "]" | "}"))
        {
            kids.pop();
        }
        let text = |n: Node| &src[n.byte_range()];
        let mut out = Vec::new();
        // Start of the call chain just completed, for `f(x).m(…)`.
        let mut chain: Option<usize> = None;
        let mut i = 0;
        while i < kids.len() {
            let k = kids[i];
            if is_seg(k) {
                let start = k.start_byte();
                let mut segs = vec![text(k).to_string()];
                let mut dotted = false;
                let mut j = i + 1;
                while j + 1 < kids.len()
                    && matches!(kids[j].kind(), "::" | ".")
                    && is_seg(kids[j + 1])
                {
                    dotted |= kids[j].kind() == ".";
                    segs.push(text(kids[j + 1]).to_string());
                    j += 2;
                }
                if j < kids.len() && is_paren_tree(kids[j]) {
                    let label = collapse(&src[start..kids[j - 1].end_byte()]);
                    let callee = if dotted {
                        let on_self = segs.len() == 2 && segs[0] == "self";
                        TokenCallee::Method {
                            name: segs.pop().unwrap_or_default(),
                            on_self,
                        }
                    } else if segs.len() == 1 {
                        TokenCallee::Ident(segs.pop().unwrap_or_default())
                    } else {
                        TokenCallee::Path(segs)
                    };
                    out.push(TokenCall {
                        label,
                        callee,
                        line: line(k),
                        args: Some(kids[j]),
                    });
                    chain = Some(start);
                    i = j + 1;
                    continue;
                }
                chain = None;
                i = j;
                continue;
            }
            if k.kind() == "."
                && let Some(start) = chain
                && i + 2 < kids.len()
                && is_seg(kids[i + 1])
                && is_paren_tree(kids[i + 2])
            {
                out.push(TokenCall {
                    label: collapse(&src[start..kids[i + 1].end_byte()]),
                    callee: TokenCallee::Method {
                        name: text(kids[i + 1]).to_string(),
                        on_self: false,
                    },
                    line: line(kids[i + 1]),
                    args: Some(kids[i + 2]),
                });
                i += 3;
                continue;
            }
            if k.kind() == "token_tree" {
                out.extend(self.token_calls(k, src));
            }
            chain = None;
            i += 1;
        }
        out
    }
}

/// `panic!`, `unreachable!`, … (also as `std::panic!`).
fn is_exit_macro(m: Node, src: &str) -> bool {
    m.child_by_field_name("macro").is_some_and(|name| {
        let t = &src[name.byte_range()];
        EXIT_MACROS.contains(&t.rsplit("::").next().unwrap_or(t))
    })
}

impl RsFlow {
    fn param_labels(&self, f: Node, src: &str) -> Vec<String> {
        let Some(list) = f.child_by_field_name("parameters") else {
            return Vec::new();
        };
        named(list)
            .filter_map(|p| param(p, src))
            .map(|(_, l)| l)
            .collect()
    }

    /// `let f = |…| …` closures and nested `fn` items anywhere in a body.
    fn scope_of<'t, 's>(&self, body: Option<Node<'t>>, src: &'s str) -> Scope<'t, 's> {
        let mut sc = Scope::default();
        let mut stack: Vec<Node<'t>> = body.into_iter().collect();
        while let Some(n) = stack.pop() {
            match n.kind() {
                "impl_item" | "trait_item" | "mod_item" => continue,
                "function_item" => {
                    if let Some(name) = n.child_by_field_name("name") {
                        sc.locals.insert(&src[name.byte_range()], n);
                    }
                }
                "let_declaration" => {
                    if let (Some(p), Some(v)) = (
                        n.child_by_field_name("pattern"),
                        n.child_by_field_name("value"),
                    ) && p.kind() == "identifier"
                        && v.kind() == "closure_expression"
                    {
                        sc.locals.insert(&src[p.byte_range()], v);
                    }
                }
                _ => {}
            }
            let mut c = n.walk();
            let kids: Vec<Node<'t>> = n.named_children(&mut c).collect();
            stack.extend(kids.into_iter().rev());
        }
        sc
    }

    pub fn prepare<'t, 's>(
        &self,
        src: &'s str,
        root: Node<'t>,
        facts: &FileFacts,
    ) -> Prepared<'t, 's> {
        let names = SymbolNames::new(facts);
        let imports = &facts.imports;
        let mut prepared = Prepared {
            imports: HashMap::new(),
            import_paths: HashMap::new(),
            top_fns: HashSet::new(),
            factory_props: HashMap::new(),
            units: Vec::new(),
        };
        for imp in imports
            .iter()
            .filter(|i| matches!(i.kind, ImportKind::Static | ImportKind::Reexport))
        {
            for n in &imp.names {
                match n.name.as_str() {
                    "*" => {}
                    "self" => {
                        let last = imp.specifier.rsplit("::").next().unwrap_or("");
                        let local = n.alias.clone().unwrap_or_else(|| last.to_string());
                        prepared
                            .imports
                            .insert(local.clone(), imp.specifier.clone());
                        prepared.import_paths.insert(local, imp.specifier.clone());
                    }
                    name => {
                        let local = n.local().to_string();
                        let parent = imp
                            .specifier
                            .strip_suffix(name)
                            .and_then(|p| p.strip_suffix("::"))
                            .unwrap_or(&imp.specifier)
                            .to_string();
                        prepared.imports.insert(local.clone(), parent);
                        prepared.import_paths.insert(local, imp.specifier.clone());
                    }
                }
            }
        }
        self.items(src, root, "", true, &names, &mut prepared);
        prepared
    }

    /// Units among the items of a file or inline module.
    fn items<'t, 's>(
        &self,
        src: &'s str,
        container: Node<'t>,
        prefix: &str,
        top: bool,
        names: &SymbolNames,
        prepared: &mut Prepared<'t, 's>,
    ) {
        let text = |n: Node| -> &'s str { &src[n.byte_range()] };
        for it in named(container) {
            match it.kind() {
                "function_item" => {
                    let Some(name) = it.child_by_field_name("name") else {
                        continue;
                    };
                    let nm = text(name);
                    if top {
                        prepared.top_fns.insert(nm);
                    }
                    let fallback = format!("{prefix}{nm}");
                    let (id, owner, _) =
                        names.unit(nm, end_line(it), false, (fallback.clone(), fallback));
                    self.push_unit(src, it, id, owner, None, None, prefix, prepared);
                }
                "impl_item" | "trait_item" => {
                    let tnode = if it.kind() == "impl_item" {
                        it.child_by_field_name("type")
                    } else {
                        it.child_by_field_name("name")
                    };
                    let (Some(tnode), Some(body)) = (tnode, it.child_by_field_name("body")) else {
                        continue;
                    };
                    let computed = format!("{prefix}{}", type_name(tnode, src));
                    for m in named(body).filter(|m| m.kind() == "function_item") {
                        let Some(mn) = m.child_by_field_name("name") else {
                            continue;
                        };
                        let mn = text(mn);
                        let (id, owner, method) = names.unit(
                            mn,
                            end_line(m),
                            true,
                            (format!("{computed}.{mn}"), computed.clone()),
                        );
                        self.push_unit(
                            src,
                            m,
                            id,
                            owner.clone(),
                            method,
                            Some(owner),
                            prefix,
                            prepared,
                        );
                    }
                }
                "mod_item" => {
                    if let (Some(name), Some(body)) = (
                        it.child_by_field_name("name"),
                        it.child_by_field_name("body"),
                    ) {
                        let inner = format!("{prefix}{}.", text(name));
                        self.items(src, body, &inner, false, names, prepared);
                    }
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_unit<'t, 's>(
        &self,
        src: &'s str,
        f: Node<'t>,
        id: String,
        owner: String,
        method: Option<String>,
        self_type: Option<String>,
        prefix: &str,
        prepared: &mut Prepared<'t, 's>,
    ) {
        let Some(body) = f.child_by_field_name("body") else {
            return;
        };
        if prepared.units.iter().any(|u| u.id == id) {
            return;
        }
        let is_async = named(f)
            .find(|c| c.kind() == "function_modifiers")
            .is_some_and(|m| src[m.byte_range()].contains("async"));
        prepared.units.push(UnitSeed {
            id,
            owner,
            method,
            node: f,
            scope: Rc::new(self.scope_of(Some(body), src)),
            params: self.params(f, src),
            start: line(f),
            end: end_line(f),
            is_async,
            param_labels: self.param_labels(f, src),
            factory: None,
            self_type,
            prefix: prefix.to_string(),
        });
    }
}
