//! Python table and unit discovery for the shared walker. The mapping is documented in
//! `fixtures/python/FLOW.md`.
//!
//! Units: module-level functions (`f`) and the methods of module-level classes, nested
//! classes included (`Class.method`, `Outer.Inner.method`), named like the extractor's
//! qualified names so cards link to them. Nested `def`s and `name = lambda …` are locals:
//! inlined where they are called, deferred where they are only defined.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

use iso_core::{FileFacts, FlowUnit, ImportKind, Language};
use tree_sitter::{Node, Tree};

use super::spec::{FlowSpec, KindNames, SpecDef, Style};
use super::{
    Arm, FlowLang, Param, Prepared, Scope, SymbolNames, UnitSeed, end_line, line, named, walk_units,
};

pub static PY_DEF: SpecDef = SpecDef {
    skip: &[
        "comment",
        "decorator",
        "type",
        "class_definition",
        "import_statement",
        "import_from_statement",
        "future_import_statement",
        "global_statement",
        "nonlocal_statement",
        "pass_statement",
        "type_alias_statement",
    ],
    block: &["block", "module"],
    var_decl: &[],
    expr_stmt: &["expression_statement"],
    transparent: &["parenthesized_expression"],
    if_: &["if_statement"],
    ternary: &["conditional_expression"],
    binary: &["boolean_operator"],
    return_: &["return_statement"],
    throw: &["raise_statement"],
    try_: &["try_statement"],
    for_c: &[],
    while_: &["while_statement"],
    do_: &[],
    for_each: &["for_statement"],
    switch: &[],
    call: &["call"],
    new: &[],
    lambda: &["lambda"],
    object: &[],
    fn_decl: &["function_definition", "decorated_definition"],
    assign: &["assignment", "augmented_assignment"],
    match_: &["match_statement"],
    with: &["with_statement"],
    comprehension: &[
        "list_comprehension",
        "set_comprehension",
        "dictionary_comprehension",
        "generator_expression",
    ],
    try_op: &[],
    macro_: &[],
    logical_ops: &[("and", "only if"), ("or", "unless")],
    iter_methods: &["sort"],
    builtins: &[
        "str", "dict", "list", "set", "tuple", "int", "float", "bytes", "object",
    ],
    kinds: KindNames {
        identifier: "identifier",
        paren: "parenthesized_expression",
        member: "attribute",
        subscript: "subscript",
        await_: "await",
        declarator: "",
        pair: "",
        shorthand: "",
        spread: "",
        method: "",
        return_: "return_statement",
        object: "",
        else_clause: "else_clause",
        empty_statement: "",
        switch_case: "",
        template_string: "",
        optional_chain: &[],
    },
    style: Style {
        builtin_fns: BUILTIN_FNS,
        iter_fns: &["map", "filter", "sorted", "min", "max"],
        self_names: &["self", "cls"],
        member_roots_ext: true,
        parens: false,
        for_op: "in",
        else_line_from_clause: true,
        match_kw: "case",
        catch_kw: "except",
        catch_clauses: &["except_clause", "except_group_clause"],
        try_else_clauses: &["else_clause"],
        finally_clauses: &["finally_clause"],
        elif_clauses: &["elif_clause"],
        for_in_clause: "for_in_clause",
        path_kinds: &[],
        exit_macros: &[],
        builtin_macros: &[],
        member_object: "object",
        member_property: "attribute",
        subscript_object: "value",
    },
};

/// Built-in functions and exception classes (Python 3.13 `builtins`).
const BUILTIN_FNS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "BaseException",
    "Exception",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BlockingIOError",
    "BrokenPipeError",
    "ConnectionError",
    "EOFError",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "ImportError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PermissionError",
    "RecursionError",
    "ReferenceError",
    "RuntimeError",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SystemError",
    "SystemExit",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "ValueError",
    "ZeroDivisionError",
    "IOError",
    "DeprecationWarning",
    "UserWarning",
    "Warning",
];

pub struct PyFlow {
    spec: FlowSpec,
}

fn py_flow() -> &'static PyFlow {
    static CELL: OnceLock<PyFlow> = OnceLock::new();
    CELL.get_or_init(|| PyFlow {
        spec: FlowSpec::build(
            &crate::grammar(Language::Python).expect("bundled grammar"),
            &PY_DEF,
        ),
    })
}

/// Call-flow units for one Python file. `facts` are the extractor's facts for it: imports
/// classify `ext` calls, symbols name the units.
pub fn flow_units(src: &str, tree: &Tree, facts: &FileFacts) -> Vec<FlowUnit> {
    let pf = py_flow();
    crate::util::with_stack_for(crate::util::tree_depth(tree), || {
        let prepared = pf.prepare(src, tree.root_node(), facts);
        walk_units(pf, src, &prepared)
    })
}

fn has_token(n: Node, tok: &str) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .any(|ch| !ch.is_named() && ch.kind() == tok)
}

/// One parameter: binding name, default value, label as shown (`x`, `x = …`, `*args`).
fn param<'t, 's>(p: Node<'t>, src: &'s str) -> Option<(Option<&'s str>, Option<Node<'t>>, String)> {
    let text = |n: Node| &src[n.byte_range()];
    match p.kind() {
        "identifier" => Some((Some(text(p)), None, text(p).to_string())),
        "default_parameter" | "typed_default_parameter" => {
            let name = p.child_by_field_name("name")?;
            let ident = (name.kind() == "identifier").then(|| text(name));
            Some((
                ident,
                p.child_by_field_name("value"),
                format!("{} = …", text(name)),
            ))
        }
        "typed_parameter" => {
            let inner = named(p).find(|c| c.kind() != "type")?;
            param(inner, src)
        }
        "list_splat_pattern" | "dictionary_splat_pattern" => {
            let ident = named(p).find(|c| c.kind() == "identifier").map(text);
            Some((ident, None, text(p).to_string()))
        }
        _ => None,
    }
}

impl FlowLang for PyFlow {
    fn spec(&self) -> &FlowSpec {
        &self.spec
    }

    fn params<'t, 's>(&self, f: Node<'t>, src: &'s str) -> Vec<Param<'t, 's>> {
        let Some(list) = f.child_by_field_name("parameters") else {
            return Vec::new();
        };
        named(list)
            .filter_map(|p| param(p, src))
            .map(|(ident, default, _)| Param { ident, default })
            .collect()
    }

    fn body<'t>(&self, f: Node<'t>) -> Option<Node<'t>> {
        f.child_by_field_name("body")
    }

    fn is_plain_method(&self, _m: Node, _src: &str) -> bool {
        true
    }

    fn unwrap_arg<'t>(&self, arg: Node<'t>) -> Node<'t> {
        if arg.kind() == "keyword_argument"
            && let Some(v) = arg.child_by_field_name("value")
        {
            return v;
        }
        arg
    }

    fn match_arms<'t>(&self, node: Node<'t>, src: &str) -> (Option<Node<'t>>, Vec<Arm<'t>>) {
        let subject = node.child_by_field_name("subject");
        let mut arms = Vec::new();
        if let Some(body) = node.child_by_field_name("body") {
            for case in named(body).filter(|c| c.kind() == "case_clause") {
                let patterns: Vec<Node> =
                    named(case).filter(|c| c.kind() == "case_pattern").collect();
                let guard = case.child_by_field_name("guard");
                let label = match (patterns.first(), guard.or(patterns.last().copied())) {
                    (Some(a), Some(b)) => super::label(&src[a.start_byte()..b.end_byte()], 40),
                    _ => String::new(),
                };
                arms.push(Arm {
                    label,
                    guard,
                    body: case.child_by_field_name("consequence"),
                    line: line(case),
                });
            }
        }
        (subject, arms)
    }
}

impl PyFlow {
    fn param_labels(&self, f: Node, src: &str) -> Vec<String> {
        let Some(list) = f.child_by_field_name("parameters") else {
            return Vec::new();
        };
        named(list)
            .filter_map(|p| param(p, src))
            .map(|(_, _, l)| l)
            .collect()
    }

    /// Nested `def`s and `name = lambda …` anywhere in a body, not inside nested classes.
    fn scope_of<'t, 's>(&self, body: Option<Node<'t>>, src: &'s str) -> Scope<'t, 's> {
        let mut sc = Scope::default();
        let mut stack: Vec<Node<'t>> = body.into_iter().collect();
        while let Some(n) = stack.pop() {
            match n.kind() {
                "class_definition" => continue,
                "function_definition" => {
                    if let Some(name) = n.child_by_field_name("name") {
                        sc.locals.insert(&src[name.byte_range()], n);
                    }
                }
                "assignment" => {
                    if let (Some(l), Some(r)) = (
                        n.child_by_field_name("left"),
                        n.child_by_field_name("right"),
                    ) && l.kind() == "identifier"
                        && r.kind() == "lambda"
                    {
                        sc.locals.insert(&src[l.byte_range()], r);
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
        for imp in imports.iter().filter(|i| i.kind == ImportKind::Static) {
            for n in &imp.names {
                if n.name == "*" {
                    continue;
                }
                if n.name == imp.specifier {
                    // `import a.b` binds `a`; `import a.b as c` binds `c` to `a.b`.
                    let (local, module) = match &n.alias {
                        Some(a) => (a.clone(), imp.specifier.clone()),
                        None => {
                            let first = imp.specifier.split('.').next().unwrap_or("");
                            (first.to_string(), first.to_string())
                        }
                    };
                    prepared.import_paths.entry(local).or_insert(module);
                } else {
                    let local = n.local().to_string();
                    prepared
                        .imports
                        .insert(local.clone(), imp.specifier.clone());
                    prepared.import_paths.insert(local, imp.specifier.clone());
                }
            }
        }
        for st in named(root) {
            let (def, outer) = unwrap_decorated(st);
            match def.kind() {
                "function_definition" => {
                    if let Some(name) = def.child_by_field_name("name") {
                        let nm = &src[name.byte_range()];
                        prepared.top_fns.insert(nm);
                        let (id, owner, method) = names.unit(
                            nm,
                            end_line(outer),
                            false,
                            (nm.to_string(), nm.to_string()),
                        );
                        let u = self.seed(src, def, outer, id, owner, method);
                        prepared.units.push(u);
                    }
                }
                "class_definition" => self.class_units(src, def, None, &names, &mut prepared),
                _ => {}
            }
        }
        prepared
    }

    fn class_units<'t, 's>(
        &self,
        src: &'s str,
        class: Node<'t>,
        outer_qn: Option<&str>,
        names: &SymbolNames,
        prepared: &mut Prepared<'t, 's>,
    ) {
        let (Some(name), Some(body)) = (
            class.child_by_field_name("name"),
            class.child_by_field_name("body"),
        ) else {
            return;
        };
        let qn = match outer_qn {
            Some(o) => format!("{o}.{}", &src[name.byte_range()]),
            None => src[name.byte_range()].to_string(),
        };
        for st in named(body) {
            let (def, outer) = unwrap_decorated(st);
            match def.kind() {
                "function_definition" => {
                    let Some(m) = def.child_by_field_name("name") else {
                        continue;
                    };
                    let m = &src[m.byte_range()];
                    let (id, owner, method) =
                        names.unit(m, end_line(outer), true, (format!("{qn}.{m}"), qn.clone()));
                    if prepared.units.iter().any(|u| u.id == id) {
                        continue;
                    }
                    let u = self.seed(src, def, outer, id, owner, method);
                    prepared.units.push(u);
                }
                "class_definition" => self.class_units(src, def, Some(&qn), names, prepared),
                _ => {}
            }
        }
    }

    fn seed<'t, 's>(
        &self,
        src: &'s str,
        def: Node<'t>,
        outer: Node<'t>,
        id: String,
        owner: String,
        method: Option<String>,
    ) -> UnitSeed<'t, 's> {
        let self_type = method.as_ref().map(|_| owner.clone());
        UnitSeed {
            id,
            owner,
            method,
            node: def,
            scope: Rc::new(self.scope_of(def.child_by_field_name("body"), src)),
            params: self.params(def, src),
            start: line(outer),
            end: end_line(outer),
            is_async: has_token(def, "async"),
            param_labels: self.param_labels(def, src),
            factory: None,
            self_type,
            prefix: String::new(),
        }
    }
}

/// The definition inside `@decorator` wrappers, and the node whose lines the unit spans.
fn unwrap_decorated(st: Node) -> (Node, Node) {
    if st.kind() == "decorated_definition"
        && let Some(d) = st.child_by_field_name("definition")
    {
        return (d, st);
    }
    (st, st)
}
