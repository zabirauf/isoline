//! Shared call-flow walker. A port of `docs/handoff/prototype/analyze.js` to tree-sitter,
//! driven by a node-kind table per language. Owned by workstream H.
//!
//! The walker ([`walk`]) only knows categories of nodes ([`spec::Cat`]) and field ids; every
//! grammar-specific name lives in a [`spec::SpecDef`]. Unit discovery (which functions are
//! units, what a file imports, which factories return controller objects) is per language
//! and produces a [`Prepared`] file for the walker. `js.rs`, `python.rs` and `rust.rs` hold
//! the tables, hooks ([`FlowLang`]) and discovery of each language.
//!
//! For TS/JS, semantics follow analyze.js exactly, including its quirks (see
//! `fixtures/ts/DEVIATIONS.md` for the few places where Isoline does more). Behavior that
//! only Python and Rust need is switched on by their [`spec::Style`]; [`spec::Style::JS`]
//! keeps analyze.js's. The Python and Rust mappings are in `fixtures/python/FLOW.md` and
//! `fixtures/rust/FLOW.md`.

pub mod js;
pub mod python;
pub mod rust;
pub mod spec;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use iso_core::{
    BlockKind, ExitKind, FlowItem, FlowTarget, FlowTargetKind, FlowUnit, ReturnedMethod,
};
use tree_sitter::Node;

use spec::{Cat, FlowSpec, is};

/// A parameter of a function-like node.
#[derive(Clone, Copy, Debug)]
pub struct Param<'t, 's> {
    /// Binding name when the parameter is a plain identifier (`a`, `...rest`).
    pub ident: Option<&'s str>,
    /// Default value, if any.
    pub default: Option<Node<'t>>,
}

/// Language hooks the walker calls back into.
pub trait FlowLang: Sync {
    fn spec(&self) -> &FlowSpec;
    /// Parameters of a function-like node, in order.
    fn params<'t, 's>(&self, f: Node<'t>, src: &'s str) -> Vec<Param<'t, 's>>;
    /// Body of a function-like node.
    fn body<'t>(&self, f: Node<'t>) -> Option<Node<'t>>;
    /// `true` for a getter/setter/constructor-like object member that is not a plain method.
    fn is_plain_method(&self, m: Node, src: &str) -> bool;
    /// Grammar quirk: a prefix operator the grammar attached to the base of a call's callee
    /// chain although it applies to the whole chain (tree-sitter-typescript parses
    /// `await f<T>(x)` as `(await f)<T>(x)`). Returns the operator node and its operand.
    fn misplaced_prefix<'t>(&self, _callee: Node<'t>) -> Option<(Node<'t>, Node<'t>)> {
        None
    }
    /// `true` when `n` is the outermost node of its postfix chain (`a.b().c()`).
    fn is_chain_top(&self, _n: Node) -> bool {
        true
    }
    /// The callee to label and classify (Rust `f::<T>` is `f`).
    fn unwrap_callee<'t>(&self, callee: Node<'t>) -> Node<'t> {
        callee
    }
    /// An argument's value (Python `key=value` is `value`).
    fn unwrap_arg<'t>(&self, arg: Node<'t>) -> Node<'t> {
        arg
    }
    /// Subject and arms of a [`Cat::Match`] node.
    fn match_arms<'t>(&self, _node: Node<'t>, _src: &str) -> (Option<Node<'t>>, Vec<Arm<'t>>) {
        (None, Vec::new())
    }
    /// The expression a function's body ends with when it is the return value (a Rust tail
    /// expression in a function that declares a return type).
    fn tail_value<'t>(&self, _f: Node<'t>, _src: &str) -> Option<Node<'t>> {
        None
    }
    /// Target of a path callee (`Type::f`, `module::f`), split at `::`.
    fn path_target(&self, _segs: &[&str], _cx: &PathCx) -> FlowTarget {
        other_target()
    }
    /// Name node and token tree of a [`Cat::Macro`] node.
    fn macro_parts<'t>(&self, _m: Node<'t>) -> Option<(Node<'t>, Option<Node<'t>>)> {
        None
    }
    /// Call shapes (`f(…)`, `a::b(…)`, `x.m(…)`) in a macro's token tree, in source order.
    fn token_calls<'t>(&self, _tree: Node<'t>, _src: &str) -> Vec<TokenCall<'t>> {
        Vec::new()
    }
}

/// One arm of a `match`.
pub struct Arm<'t> {
    pub label: String,
    /// `if` guard, walked as a condition.
    pub guard: Option<Node<'t>>,
    pub body: Option<Node<'t>>,
    pub line: u32,
}

/// What [`FlowLang::path_target`] can see.
pub struct PathCx<'a, 't, 's> {
    pub file: &'a Prepared<'t, 's>,
    pub unit_ids: &'a HashSet<&'a str>,
    /// The unit's own type (`Self`), qualified as unit ids are.
    pub owner: Option<&'a str>,
    /// Inline-module prefix of the unit (`tests.`).
    pub prefix: &'a str,
}

/// A call found in a macro's token tree.
pub struct TokenCall<'t> {
    pub label: String,
    pub callee: TokenCallee,
    pub line: u32,
    /// The parenthesized arguments, scanned for nested calls.
    pub args: Option<Node<'t>>,
}

pub enum TokenCallee {
    Ident(String),
    Path(Vec<String>),
    Method { name: String, on_self: bool },
}

/// Callable symbols by (name, end line) → qualified name, so a language's units carry the
/// extractor's names exactly (`Type.method`, `sync#2.wake`) and cards link to them.
pub struct SymbolNames<'f>(HashMap<(&'f str, u32), &'f str>);

impl<'f> SymbolNames<'f> {
    pub fn new(facts: &'f iso_core::FileFacts) -> Self {
        SymbolNames(
            facts
                .symbols
                .iter()
                .filter(|s| s.kind.is_callable())
                .map(|s| ((s.name.as_str(), s.end_line), s.qualified_name.as_str()))
                .collect(),
        )
    }
    /// Unit id, owner and method for a function named `name` ending on `end`, falling back
    /// to the computed `(id, owner)` when the extractor has no such symbol.
    pub fn unit(
        &self,
        name: &str,
        end: u32,
        method: bool,
        fallback: (String, String),
    ) -> (String, String, Option<String>) {
        let Some(qn) = self.0.get(&(name, end)) else {
            let m = method.then(|| name.to_string());
            return (fallback.0, fallback.1, m);
        };
        if !method {
            return (qn.to_string(), qn.to_string(), None);
        }
        let owner = qn
            .strip_suffix(name)
            .and_then(|o| o.strip_suffix('.'))
            .unwrap_or(&fallback.1);
        (qn.to_string(), owner.to_string(), Some(name.to_string()))
    }
}

/// An `other` target (no id).
pub fn other_target() -> FlowTarget {
    FlowTarget {
        kind: FlowTargetKind::Other,
        id: None,
        module: None,
        unit: None,
        of: None,
        note: None,
        ext: None,
    }
}

/// `FlowTarget` constructors for language modules.
pub fn target(kind: FlowTargetKind, id: impl Into<String>) -> FlowTarget {
    FlowTarget {
        kind,
        id: Some(id.into()),
        ..other_target()
    }
}

/// One factory's returned controller object: property name and what it is.
#[derive(Clone, Debug)]
pub enum Prop {
    Unit(String),
    From(String),
    Value,
}

/// Variables bound inside a unit's body (analyze.js `scopeOf`).
#[derive(Default)]
pub struct Scope<'t, 's> {
    /// Names bound to an arrow function or function expression.
    pub locals: HashMap<&'s str, Node<'t>>,
    /// Names bound to the result of calling a top-level function of this file.
    pub origins: HashMap<&'s str, &'s str>,
    /// Names bound to the result of calling an imported function.
    pub value_from: HashMap<&'s str, &'s str>,
}

/// A unit ready to walk.
pub struct UnitSeed<'t, 's> {
    pub id: String,
    pub owner: String,
    pub method: Option<String>,
    /// The function-like node whose parameters and body are walked.
    pub node: Node<'t>,
    pub scope: Rc<Scope<'t, 's>>,
    /// Parameters visible in the unit (a factory method also sees the factory's).
    pub params: Vec<Param<'t, 's>>,
    pub start: u32,
    pub end: u32,
    pub is_async: bool,
    /// Displayed parameter list (`name`, `name = …`).
    pub param_labels: Vec<String>,
    /// Set when the unit is a factory whose controller object is listed on return.
    pub factory: Option<String>,
    /// The type a method belongs to (qualified like unit ids), for `self.m()` / `Self::f()`.
    pub self_type: Option<String>,
    /// Inline-module prefix (`tests.`) for calls to functions of the same module.
    pub prefix: String,
}

/// One file, prepared by a language's discovery pass.
pub struct Prepared<'t, 's> {
    /// Local name of an imported function or value → module it comes from.
    pub imports: HashMap<String, String>,
    /// Local name of an imported module or type → its full path, for member and path
    /// callees rooted at it (`np.mean`, `HashMap::new`). Empty for TS/JS.
    pub import_paths: HashMap<String, String>,
    /// Functions declared at the top of the file.
    pub top_fns: HashSet<&'s str>,
    /// Factory name → its controller object's properties in source order.
    pub factory_props: HashMap<String, Vec<(String, Prop)>>,
    pub units: Vec<UnitSeed<'t, 's>>,
}

/// Walk every unit of a prepared file.
pub fn walk_units(lang: &dyn FlowLang, src: &str, file: &Prepared) -> Vec<FlowUnit> {
    let unit_ids: HashSet<&str> = file.units.iter().map(|u| u.id.as_str()).collect();
    file.units
        .iter()
        .map(|u| {
            let w = Walker {
                lang,
                spec: lang.spec(),
                src,
                file,
                scope: &u.scope,
                used: RefCell::new(HashSet::new()),
                depth: Cell::new(0),
                budget: Cell::new(MAX_WALKED),
                unit_ids: &unit_ids,
                owner: u.self_type.as_deref(),
                prefix: &u.prefix,
            };
            let inline: Vec<&str> = u.method.iter().map(|m| m.as_str()).collect();
            let ctx = Ctx {
                params: Rc::new(u.params.clone()),
                arg_of: None,
                cond: 0,
                inline: Rc::new(inline),
                factory: u.factory.as_deref(),
            };
            let raw = w.walk_fn(u.node, &ctx);
            let returned: HashSet<&str> = if u.method.is_some() {
                HashSet::new()
            } else {
                file.factory_props
                    .get(&u.id)
                    .map(|p| p.iter().map(|(k, _)| k.as_str()).collect())
                    .unwrap_or_default()
            };
            let tree = w.finalize(raw, &ctx, &returned);
            FlowUnit {
                id: u.id.clone(),
                owner: u.owner.clone(),
                method: u.method.clone(),
                start: u.start,
                end: u.end,
                is_async: u.is_async,
                params: u.param_labels.clone(),
                tree,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Labels

/// analyze.js `T`: node text, whitespace runs collapsed, trimmed, cut to `max` UTF-16 units
/// with a trailing "…".
pub fn label(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max * 4));
    for w in s.split(is_js_space).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    let units: usize = out.chars().map(char::len_utf16).sum();
    if units <= max {
        return out;
    }
    let mut cut = String::with_capacity(max * 2);
    let mut acc = 0;
    for ch in out.chars() {
        let l = ch.len_utf16();
        if acc + l > max.saturating_sub(1) {
            break;
        }
        acc += l;
        cut.push(ch);
    }
    cut.push('…');
    cut
}

/// Whitespace runs collapsed to one space, no length limit.
pub fn collapse(s: &str) -> String {
    s.split(is_js_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generic arguments removed from a path: `Vec::<u8>::new` → `Vec::new`.
pub fn strip_generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    while out.contains("::::") {
        out = out.replace("::::", "::");
    }
    out.trim_end_matches("::").to_string()
}

fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

#[inline]
pub fn line(n: Node) -> u32 {
    n.start_position().row as u32 + 1
}

#[inline]
pub fn end_line(n: Node) -> u32 {
    n.end_position().row as u32 + 1
}

/// Named, non-extra children.
pub fn named<'t>(n: Node<'t>) -> impl Iterator<Item = Node<'t>> {
    let mut c = n.walk();
    let mut ok = c.goto_first_child();
    std::iter::from_fn(move || {
        while ok {
            let node = c.node();
            ok = c.goto_next_sibling();
            if node.is_named() && !node.is_extra() {
                return Some(node);
            }
        }
        None
    })
}

/// Children under one field (tree-sitter allows repeated fields).
pub fn field_children<'t>(
    n: Node<'t>,
    field: Option<spec::FieldId>,
) -> impl Iterator<Item = Node<'t>> {
    let mut c = n.walk();
    let mut ok = c.goto_first_child();
    std::iter::from_fn(move || {
        let field = field?;
        while ok {
            let node = c.node();
            let f = c.field_id();
            ok = c.goto_next_sibling();
            if f == Some(field) && !node.is_extra() {
                return Some(node);
            }
        }
        None
    })
}

#[inline]
pub fn field<'t>(n: Node<'t>, f: Option<spec::FieldId>) -> Option<Node<'t>> {
    n.child_by_field_id(f?.get())
}

// ---------------------------------------------------------------------------------------
// Walker

enum Item<'t, 's> {
    Def {
        name: &'s str,
        line: u32,
        node: Node<'t>,
    },
    Call {
        label: String,
        target: FlowTarget,
        line: u32,
        awaited: bool,
        optional: bool,
        arg_of: Option<Rc<str>>,
        inner: Vec<Item<'t, 's>>,
    },
    Block {
        kind: BlockKind,
        kw: &'static str,
        label: String,
        line: u32,
        children: Vec<Item<'t, 's>>,
    },
    Exit {
        kind: ExitKind,
        label: String,
        line: u32,
        early: Option<bool>,
        methods: Option<Vec<ReturnedMethod>>,
    },
}

fn meaningful(items: &[Item]) -> bool {
    items.iter().any(|i| !matches!(i, Item::Def { .. }))
}

fn block<'t, 's>(
    kind: BlockKind,
    kw: &'static str,
    label: String,
    children: Vec<Item<'t, 's>>,
    line: u32,
) -> Item<'t, 's> {
    Item::Block {
        kind,
        kw,
        label,
        line,
        children,
    }
}

#[derive(Clone)]
struct Ctx<'t, 's, 'a> {
    params: Rc<Vec<Param<'t, 's>>>,
    arg_of: Option<Rc<str>>,
    cond: u32,
    inline: Rc<Vec<&'s str>>,
    factory: Option<&'a str>,
}

impl<'t, 's, 'a> Ctx<'t, 's, 'a> {
    fn with_cond(&self) -> Self {
        Ctx {
            cond: self.cond + 1,
            ..self.clone()
        }
    }
    fn with_arg_of(&self, a: Rc<str>) -> Self {
        Ctx {
            arg_of: Some(a),
            ..self.clone()
        }
    }
    fn with_inline(&self, name: &'s str) -> Self {
        let mut v = (*self.inline).clone();
        v.push(name);
        Ctx {
            inline: Rc::new(v),
            ..self.clone()
        }
    }
    fn has_param(&self, n: &str) -> bool {
        self.params.iter().any(|p| p.ident == Some(n))
    }
    /// The visible binding's default value (later frames shadow earlier ones).
    fn param_default(&self, n: &str) -> Option<Node<'t>> {
        self.params
            .iter()
            .rev()
            .find(|p| p.ident == Some(n))
            .and_then(|p| p.default)
    }
}

/// Recursion guard for pathological inputs (a default parameter that calls itself, local
/// functions that inline each other through deferred definitions). It bounds the native
/// stack: every level is a few `walk` frames, and worker threads have 2 MiB stacks.
/// analyze.js has no guard and overflows the V8 stack on the same inputs.
const MAX_DEPTH: u32 = 160;

/// Nodes one unit may walk. Inlining local functions into each other is exponential in the
/// worst case (analyze.js runs out of stack on such files); past the budget the walker stops
/// expanding and the unit's tree is cut short.
const MAX_WALKED: u32 = 200_000;

struct Walker<'w, 't, 's> {
    lang: &'w dyn FlowLang,
    spec: &'w FlowSpec,
    src: &'s str,
    file: &'w Prepared<'t, 's>,
    scope: &'w Scope<'t, 's>,
    used: RefCell<HashSet<&'s str>>,
    depth: Cell<u32>,
    /// Nodes left to walk (see [`MAX_WALKED`]).
    budget: Cell<u32>,
    unit_ids: &'w HashSet<&'w str>,
    /// The unit's own type, for `self.m()`.
    owner: Option<&'w str>,
    prefix: &'w str,
}

impl<'w, 't, 's> Walker<'w, 't, 's> {
    fn text(&self, n: Node) -> &'s str {
        &self.src[n.byte_range()]
    }
    fn t(&self, n: Node, max: usize) -> String {
        label(self.text(n), max)
    }
    fn is_fn(&self, n: Option<Node>) -> bool {
        n.is_some_and(|n| self.spec.cat(n) == Cat::Lambda)
    }
    /// First named, non-skipped child (the inner value of a wrapper).
    fn first_value(&self, n: Node<'t>) -> Option<Node<'t>> {
        named(n).find(|c| self.spec.cat(*c) != Cat::Skip)
    }
    fn unparen(&self, n: Node<'t>) -> Node<'t> {
        if is(&self.spec.k.paren, n)
            && let Some(inner) = self.first_value(n)
        {
            return inner;
        }
        n
    }

    fn walk_fn(&self, f: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let mut params = (*ctx.params).clone();
        params.extend(self.lang.params(f, self.src));
        let c = Ctx {
            params: Rc::new(params),
            arg_of: None,
            ..ctx.clone()
        };
        let mut out = match self.lang.body(f) {
            Some(b) => self.walk(Some(b), &c),
            None => Vec::new(),
        };
        if let Some(t) = self.lang.tail_value(f, self.src) {
            out.push(Item::Exit {
                kind: ExitKind::Return,
                label: self.t(t, 46),
                line: line(t),
                early: Some(ctx.cond > 0),
                methods: None,
            });
        }
        out
    }

    /// `(x)` for analyze.js, `x` for languages whose conditions have no parentheses.
    fn cond_label(&self, s: String) -> String {
        if self.spec.style.parens {
            format!("({s})")
        } else {
            s
        }
    }

    fn for_head(&self, left: String, op: &str, right: String) -> String {
        if self.spec.style.parens {
            format!("for ({left} {op} {right})")
        } else {
            format!("for {left} {op} {right}")
        }
    }

    fn walk_all(&self, n: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let mut out = Vec::new();
        for ch in named(n) {
            out.extend(self.walk(Some(ch), ctx));
        }
        out
    }

    fn walk(&self, node: Option<Node<'t>>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let Some(node) = node else {
            return Vec::new();
        };
        let d = self.depth.get();
        let budget = self.budget.get();
        if d > MAX_DEPTH || budget == 0 {
            return Vec::new();
        }
        self.budget.set(budget - 1);
        self.depth.set(d + 1);
        let out = self.walk_inner(node, ctx);
        self.depth.set(d);
        out
    }

    fn walk_inner(&self, node: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let sp = self.spec;
        let f = &sp.f;
        match sp.cat(node) {
            Cat::Skip => Vec::new(),
            Cat::Block => self.walk_all(node, ctx),
            Cat::VarDecl => {
                let mut out = Vec::new();
                for d in named(node) {
                    if !is(&sp.k.declarator, d) {
                        continue;
                    }
                    let init = field(d, f.value);
                    let name = field(d, f.name);
                    if self.is_fn(init)
                        && let Some(name) = name.filter(|n| is(&sp.k.identifier, *n))
                    {
                        out.push(Item::Def {
                            name: self.text(name),
                            line: line(d),
                            node: init.expect("checked"),
                        });
                        continue;
                    }
                    out.extend(self.walk(init, ctx));
                }
                out
            }
            Cat::ExprStmt | Cat::Transparent => {
                let mut out = Vec::new();
                for ch in named(node) {
                    if sp.cat(ch) != Cat::Skip {
                        out.extend(self.walk(Some(ch), ctx));
                    }
                }
                out
            }
            Cat::If => self.walk_if(node, ctx),
            Cat::Ternary => {
                let (cond_n, when_t, when_f) = match field(node, f.condition) {
                    Some(c) => (
                        Some(c),
                        field(node, f.consequence),
                        field(node, f.alternative),
                    ),
                    None => {
                        // Python: `consequence if condition else alternative`, no fields.
                        let mut kids = named(node).filter(|c| sp.cat(*c) != Cat::Skip);
                        let a = kids.next();
                        (kids.next(), a, kids.next())
                    }
                };
                let mut out = self.walk(cond_n, &ctx.with_arg_of(Rc::from("(condition)")));
                let inner = ctx.with_cond();
                let a = self.walk(when_t, &inner);
                let b = self.walk(when_f, &inner);
                let lbl = self.cond_label(cond_n.map(|c| self.t(c, 72)).unwrap_or_default());
                let (ma, mb) = (meaningful(&a), meaningful(&b));
                let lf = when_f.map(line).unwrap_or(line(node));
                if !ma && !mb {
                    return out;
                }
                if !mb {
                    out.push(block(BlockKind::Cond, "if", lbl, a, line(node)));
                } else if !ma {
                    out.push(block(BlockKind::Cond, "unless", lbl, b, lf));
                } else {
                    out.push(block(BlockKind::Cond, "if", lbl, a, line(node)));
                    out.push(block(BlockKind::Cond, "else", String::new(), b, lf));
                }
                out
            }
            Cat::Binary => {
                let left = field(node, f.left);
                let right = field(node, f.right);
                let op = field(node, f.operator).map(|o| o.kind()).unwrap_or("");
                if let Some(&(_, kw)) = sp.logical_ops.iter().find(|(o, _)| *o == op) {
                    let mut out = self.walk(left, ctx);
                    let r = self.walk(right, &ctx.with_cond());
                    if !meaningful(&r) {
                        return out;
                    }
                    let lbl = left.map(|l| self.t(l, 72)).unwrap_or_default();
                    out.push(block(BlockKind::Cond, kw, lbl, r, line(node)));
                    return out;
                }
                let mut out = self.walk(left, ctx);
                out.extend(self.walk(right, ctx));
                out
            }
            Cat::Return => self.walk_return(node, ctx),
            Cat::Throw => {
                let e = self.first_value(node);
                let mut out = self.walk(e, ctx);
                out.push(Item::Exit {
                    kind: ExitKind::Throw,
                    label: e.map(|e| self.t(e, 46)).unwrap_or_default(),
                    line: line(node),
                    early: None,
                    methods: None,
                });
                out
            }
            Cat::Try => self.walk_try(node, ctx),
            Cat::ForC => {
                let init = field(node, f.initializer)
                    .filter(|n| !is(&sp.k.empty_statement, *n) && n.is_named());
                let cond_n = field_children(node, f.condition)
                    .find(|n| n.is_named() && !is(&sp.k.empty_statement, *n));
                let incr = field(node, f.increment);
                let body = field(node, f.body);
                let init_label = match init {
                    Some(i) => label(self.text(i).trim_end().trim_end_matches(';'), 20),
                    None => self.t(node, 20),
                };
                let head = format!(
                    "for ({}; {}; …)",
                    init_label,
                    cond_n.map(|c| self.t(c, 30)).unwrap_or_default()
                );
                let mut out = self.walk(init, ctx);
                let mut b = self.walk(cond_n, ctx);
                b.extend(self.walk(body, &ctx.with_cond()));
                b.extend(self.walk(incr, ctx));
                if meaningful(&b) {
                    out.push(block(BlockKind::Loop, "loop", head, b, line(node)));
                }
                out
            }
            Cat::While | Cat::Do => {
                let cond_n = field(node, f.condition);
                let inner = cond_n.map(|c| self.unparen(c));
                let head = match inner {
                    Some(c) if sp.style.parens => format!("while ({})", self.t(c, 50)),
                    Some(c) => format!("while {}", self.t(c, 50)),
                    // Rust `loop { … }`.
                    None if !sp.style.parens => "loop".to_string(),
                    None => "while ()".to_string(),
                };
                let mut b = self.walk(cond_n, ctx);
                b.extend(self.walk(field(node, f.body), &ctx.with_cond()));
                let mut out = Vec::new();
                if meaningful(&b) {
                    out.push(block(BlockKind::Loop, "loop", head, b, line(node)));
                }
                out.extend(self.loop_else(node, ctx));
                out
            }
            Cat::ForEach => {
                let left = field(node, f.left).or_else(|| field(node, f.pattern));
                let right = field(node, f.right).or_else(|| field(node, f.value));
                let mut out = self.walk(right, ctx);
                let b = self.walk(field(node, f.body), &ctx.with_cond());
                if meaningful(&b) {
                    let init = match (field(node, f.kind), left) {
                        (Some(k), Some(l)) => label(&self.src[k.start_byte()..l.end_byte()], 24),
                        (None, Some(l)) => self.t(l, 24),
                        _ => String::new(),
                    };
                    let op = field(node, f.operator)
                        .map(|o| self.text(o))
                        .unwrap_or(sp.style.for_op);
                    let head =
                        self.for_head(init, op, right.map(|r| self.t(r, 30)).unwrap_or_default());
                    out.push(block(BlockKind::Loop, "loop", head, b, line(node)));
                }
                out.extend(self.loop_else(node, ctx));
                out
            }
            Cat::Switch => {
                let mut out = self.walk(field(node, f.value), ctx);
                let Some(body) = field(node, f.body) else {
                    return out;
                };
                let inner = ctx.with_cond();
                for cl in named(body) {
                    let is_case = is(&sp.k.switch_case, cl);
                    let value = if is_case { field(cl, f.value) } else { None };
                    let mut children = Vec::new();
                    for st in field_children(cl, f.body) {
                        children.extend(self.walk(Some(st), &inner));
                    }
                    if meaningful(&children) {
                        out.push(block(
                            BlockKind::Cond,
                            if is_case { "case" } else { "default" },
                            value.map(|v| self.t(v, 40)).unwrap_or_default(),
                            children,
                            line(cl),
                        ));
                    }
                }
                out
            }
            Cat::Call => {
                if field(node, f.arguments).is_some_and(|a| is(&sp.k.template_string, a)) {
                    // Tagged template: not a CallExpression in TypeScript's tree.
                    return self.walk_all(node, ctx);
                }
                self.call_item(node, ctx, false)
            }
            Cat::New => self.call_item(node, ctx, true),
            Cat::Lambda => {
                let body = self.walk_fn(node, ctx);
                if meaningful(&body) {
                    vec![block(
                        BlockKind::Deferred,
                        "later",
                        "inline function, runs when invoked".into(),
                        body,
                        line(node),
                    )]
                } else {
                    Vec::new()
                }
            }
            Cat::Object => {
                let mut out = Vec::new();
                for p in named(node) {
                    if is(&sp.k.pair, p) {
                        out.extend(self.walk(field(p, f.value), ctx));
                    } else if is(&sp.k.spread, p) {
                        out.extend(self.walk(self.first_value(p), ctx));
                    } else if is(&sp.k.method, p) && self.lang.is_plain_method(p, self.src) {
                        let b = self.walk_fn(p, ctx);
                        if meaningful(&b) {
                            let name = field(p, f.name).map(|n| self.t(n, 30)).unwrap_or_default();
                            out.push(block(
                                BlockKind::Deferred,
                                "later",
                                format!("method {name}()"),
                                b,
                                line(p),
                            ));
                        }
                    }
                }
                out
            }
            Cat::FnDecl => {
                let def = field(node, f.definition).unwrap_or(node);
                match (sp.cat(def), field(def, f.name)) {
                    (Cat::FnDecl, Some(name)) => vec![Item::Def {
                        name: self.text(name),
                        line: line(node),
                        node: def,
                    }],
                    // A decorated class, or a nameless declaration.
                    _ => Vec::new(),
                }
            }
            Cat::Assign => {
                let target = field(node, f.left).or_else(|| field(node, f.pattern));
                let value = field(node, f.right).or_else(|| field(node, f.value));
                let simple = target.filter(|t| is(&sp.k.identifier, *t));
                if self.is_fn(value)
                    && let Some(name) = simple
                {
                    return vec![Item::Def {
                        name: self.text(name),
                        line: line(node),
                        node: value.expect("checked"),
                    }];
                }
                let mut out = self.walk(value, ctx);
                if simple.is_none() {
                    out.extend(self.walk(target, ctx));
                }
                // Rust `let … else { … }`.
                if let Some(alt) = field(node, f.alternative) {
                    let b = self.walk(Some(alt), &ctx.with_cond());
                    if meaningful(&b) {
                        out.push(block(
                            BlockKind::Cond,
                            "else",
                            "if the pattern does not match".into(),
                            b,
                            line(alt),
                        ));
                    }
                }
                out
            }
            Cat::Match => {
                let (subject, arms) = self.lang.match_arms(node, self.src);
                let mut out = self.walk(subject, ctx);
                let inner = ctx.with_cond();
                for arm in arms {
                    let mut children =
                        self.walk(arm.guard, &inner.with_arg_of(Rc::from("(condition)")));
                    children.extend(self.walk(arm.body, &inner));
                    if meaningful(&children) {
                        out.push(block(
                            BlockKind::Cond,
                            sp.style.match_kw,
                            arm.label,
                            children,
                            arm.line,
                        ));
                    }
                }
                out
            }
            Cat::With => {
                let body = field(node, f.body);
                let clause = named(node).find(|c| Some(*c) != body && sp.cat(*c) != Cat::Skip);
                let mut out = self.walk(clause, ctx);
                let children = self.walk(body, ctx);
                if meaningful(&children) {
                    out.push(block(
                        BlockKind::Try,
                        "with",
                        clause.map(|c| self.t(c, 60)).unwrap_or_default(),
                        children,
                        line(node),
                    ));
                }
                out
            }
            Cat::Comprehension => {
                let body = field(node, f.body);
                let clauses: Vec<Node<'t>> = named(node)
                    .filter(|c| Some(*c) != body && sp.cat(*c) != Cat::Skip)
                    .collect();
                let first = clauses
                    .iter()
                    .position(|c| c.kind() == sp.style.for_in_clause);
                let Some(first) = first else {
                    return self.walk_all(node, ctx);
                };
                let head_clause = clauses[first];
                let left = field(head_clause, f.left);
                let right = field(head_clause, f.right);
                let mut out = self.walk(right, ctx);
                let inner = ctx.with_cond();
                let mut b = Vec::new();
                for (i, c) in clauses.iter().enumerate() {
                    if i == first {
                        continue;
                    }
                    if c.kind() == sp.style.for_in_clause {
                        b.extend(self.walk(field(*c, f.right), &inner));
                    } else {
                        b.extend(self.walk(Some(*c), &inner));
                    }
                }
                b.extend(self.walk(body, &inner));
                if meaningful(&b) {
                    let head = self.for_head(
                        left.map(|l| self.t(l, 24)).unwrap_or_default(),
                        sp.style.for_op,
                        right.map(|r| self.t(r, 30)).unwrap_or_default(),
                    );
                    out.push(block(BlockKind::Loop, "loop", head, b, line(node)));
                }
                out
            }
            Cat::TryOp => {
                let mut out = self.walk(self.first_value(node), ctx);
                out.push(Item::Exit {
                    kind: ExitKind::Throw,
                    label: self.t(node, 46),
                    line: line(node),
                    early: None,
                    methods: None,
                });
                out
            }
            Cat::Macro => self.macro_item(node, ctx),
            Cat::Other => self.walk_all(node, ctx),
        }
    }

    /// Python `for … else` / `while … else`: runs when the loop ends without `break`.
    fn loop_else(&self, node: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let Some(alt) = field(node, self.spec.f.alternative) else {
            return Vec::new();
        };
        let b = self.walk(field(alt, self.spec.f.body), &ctx.with_cond());
        if !meaningful(&b) {
            return Vec::new();
        }
        vec![block(
            BlockKind::Cond,
            "else",
            "if the loop ends without break".into(),
            b,
            line(alt),
        )]
    }

    /// A macro invocation: the calls in its token tree, then the macro itself as a call, or
    /// as an exit for macros that never return (`panic!`).
    fn macro_item(&self, node: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let st = self.spec.style;
        let Some((name_n, tree)) = self.lang.macro_parts(node) else {
            return Vec::new();
        };
        let name = self.text(name_n);
        let base = name.rsplit("::").next().unwrap_or(name);
        let label: Rc<str> = Rc::from(format!("{}!", label(name, 47)));
        let mut out = match tree {
            Some(t) => self.token_items(t, &label, ctx),
            None => Vec::new(),
        };
        if st.exit_macros.contains(&base) {
            out.push(Item::Exit {
                kind: ExitKind::Throw,
                label: self.t(node, 46),
                line: line(node),
                early: None,
                methods: None,
            });
            return out;
        }
        let target = if st.builtin_macros.contains(&base) {
            target(FlowTargetKind::Builtin, label.to_string())
        } else if let Some(m) = self.file.imports.get(name) {
            FlowTarget {
                module: Some(m.clone()),
                ..target(FlowTargetKind::Ext, label.to_string())
            }
        } else {
            other_target()
        };
        out.push(Item::Call {
            label: label.to_string(),
            target,
            line: line(node),
            awaited: false,
            optional: false,
            arg_of: ctx.arg_of.clone(),
            inner: Vec::new(),
        });
        out
    }

    /// Calls in a token tree, arguments first, each `argOf` the call or macro it sits in.
    fn token_items(
        &self,
        tree: Node<'t>,
        arg_of: &Rc<str>,
        ctx: &Ctx<'t, 's, '_>,
    ) -> Vec<Item<'t, 's>> {
        let d = self.depth.get();
        if d > MAX_DEPTH {
            return Vec::new();
        }
        self.depth.set(d + 1);
        let mut out = Vec::new();
        for tc in self.lang.token_calls(tree, self.src) {
            let lbl: Rc<str> = Rc::from(label(&tc.label, 48));
            if let Some(args) = tc.args {
                out.extend(self.token_items(args, &lbl, ctx));
            }
            let target = match &tc.callee {
                TokenCallee::Ident(n) => self.ident_target(n, ctx),
                TokenCallee::Path(segs) => {
                    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
                    self.path_target(&segs)
                }
                TokenCallee::Method { name, on_self } => {
                    let mut t = target(FlowTargetKind::Method, name.clone());
                    if *on_self {
                        t.unit = self.self_unit(name);
                    }
                    t
                }
            };
            out.push(Item::Call {
                label: lbl.to_string(),
                target,
                line: tc.line,
                awaited: false,
                optional: false,
                arg_of: Some(arg_of.clone()),
                inner: Vec::new(),
            });
        }
        self.depth.set(d);
        out
    }

    /// Target of a call by bare name.
    fn ident_target(&self, n: &str, ctx: &Ctx<'t, 's, '_>) -> FlowTarget {
        if !self.prefix.is_empty() {
            let scoped = format!("{}{n}", self.prefix);
            if self.unit_ids.contains(scoped.as_str()) {
                return target(FlowTargetKind::Fn, scoped);
            }
        }
        if self.file.top_fns.contains(n) {
            target(FlowTargetKind::Fn, n)
        } else if let Some(m) = self.file.imports.get(n) {
            FlowTarget {
                module: Some(m.clone()),
                ..target(FlowTargetKind::Ext, n)
            }
        } else if self.scope.locals.contains_key(n) {
            target(FlowTargetKind::Local, n)
        } else if ctx.has_param(n) {
            target(FlowTargetKind::Param, n)
        } else if self.spec.style.builtin_fns.contains(&n) {
            target(FlowTargetKind::Builtin, n)
        } else {
            other_target()
        }
    }

    fn path_target(&self, segs: &[&str]) -> FlowTarget {
        let cx = PathCx {
            file: self.file,
            unit_ids: self.unit_ids,
            owner: self.owner,
            prefix: self.prefix,
        };
        self.lang.path_target(segs, &cx)
    }

    /// The leftmost operand of a member/subscript chain (`np` in `np.linalg.norm`).
    fn chain_root(&self, mut n: Node<'t>) -> Node<'t> {
        let sp = self.spec;
        loop {
            let next = if is(&sp.k.member, n) {
                field(n, sp.f.member_object)
            } else if is(&sp.k.subscript, n) {
                field(n, sp.f.subscript_object)
            } else {
                None
            };
            match next {
                Some(x) => n = x,
                None => return n,
            }
        }
    }

    /// `Type.m` when the unit is a method of `Type` and `Type.m` is a unit too.
    fn self_unit(&self, m: &str) -> Option<String> {
        let id = format!("{}.{m}", self.owner?);
        self.unit_ids.contains(id.as_str()).then_some(id)
    }

    fn walk_if(&self, node: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let sp = self.spec;
        let f = &sp.f;
        let cond_n = field(node, f.condition);
        let mut out = self.walk(cond_n, &ctx.with_arg_of(Rc::from("(condition)")));
        let inner = ctx.with_cond();
        let a = self.walk(field(node, f.consequence), &inner);
        let lbl = self.cond_label(
            cond_n
                .map(|c| self.t(self.unparen(c), 72))
                .unwrap_or_default(),
        );
        let alts: Vec<Node<'t>> = field_children(node, f.alternative).collect();
        if alts
            .iter()
            .any(|a| sp.style.elif_clauses.contains(&a.kind()))
        {
            // Python `if … elif … else`: one flat block per clause.
            let mut blocks = vec![block(BlockKind::Cond, "if", lbl, a, line(node))];
            for alt in alts {
                if sp.style.elif_clauses.contains(&alt.kind()) {
                    let c = field(alt, f.condition);
                    let mut b = self.walk(c, &inner.with_arg_of(Rc::from("(condition)")));
                    b.extend(self.walk(field(alt, f.consequence), &inner));
                    let l =
                        self.cond_label(c.map(|c| self.t(self.unparen(c), 72)).unwrap_or_default());
                    blocks.push(block(BlockKind::Cond, "elif", l, b, line(alt)));
                } else {
                    let b = self.walk(self.first_value(alt), &inner);
                    blocks.push(block(BlockKind::Cond, "else", String::new(), b, line(alt)));
                }
            }
            if blocks.iter().any(|b| match b {
                Item::Block { children, .. } => meaningful(children),
                _ => false,
            }) {
                out.extend(blocks);
            }
            return out;
        }
        let else_clause = alts.first().copied();
        let else_stmt = else_clause.and_then(|e| {
            if is(&sp.k.else_clause, e) {
                self.first_value(e)
            } else {
                Some(e)
            }
        });
        match else_stmt {
            None => {
                if meaningful(&a) {
                    out.push(block(BlockKind::Cond, "if", lbl, a, line(node)));
                }
            }
            Some(e) => {
                let b = self.walk(Some(e), &inner);
                let else_line = match else_clause {
                    Some(c) if sp.style.else_line_from_clause => line(c),
                    _ => line(e),
                };
                if meaningful(&a) || meaningful(&b) {
                    out.push(block(BlockKind::Cond, "if", lbl, a, line(node)));
                    out.push(block(BlockKind::Cond, "else", String::new(), b, else_line));
                }
            }
        }
        out
    }

    fn walk_return(&self, node: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let sp = self.spec;
        let e = self.first_value(node);
        if let (Some(obj), Some(factory)) = (e, ctx.factory)
            && is(&sp.k.object, obj)
        {
            let methods = self
                .file
                .factory_props
                .get(factory)
                .map(|props| {
                    props
                        .iter()
                        .filter_map(|(k, v)| match v {
                            Prop::Unit(u) => Some(ReturnedMethod {
                                name: k.clone(),
                                unit: Some(u.clone()),
                                from: None,
                            }),
                            Prop::From(fr) => Some(ReturnedMethod {
                                name: k.clone(),
                                unit: None,
                                from: Some(fr.clone()),
                            }),
                            Prop::Value => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut out = Vec::new();
            for p in named(obj) {
                if is(&sp.k.pair, p) {
                    let v = field(p, sp.f.value);
                    if !self.is_fn(v) {
                        out.extend(self.walk(v, ctx));
                    }
                } else if is(&sp.k.spread, p) {
                    out.extend(self.walk(self.first_value(p), ctx));
                }
            }
            out.push(Item::Exit {
                kind: ExitKind::Return,
                label: "controller object".into(),
                line: line(node),
                early: Some(ctx.cond > 0),
                methods: Some(methods),
            });
            return out;
        }
        let mut out = self.walk(e, ctx);
        out.push(Item::Exit {
            kind: ExitKind::Return,
            label: e.map(|e| self.t(e, 46)).unwrap_or_default(),
            line: line(node),
            early: Some(ctx.cond > 0),
            methods: None,
        });
        out
    }

    fn walk_try(&self, node: Node<'t>, ctx: &Ctx<'t, 's, '_>) -> Vec<Item<'t, 's>> {
        let f = &self.spec.f;
        let mut out = vec![block(
            BlockKind::Try,
            "try",
            String::new(),
            self.walk(field(node, f.body), ctx),
            line(node),
        )];
        let st = self.spec.style;
        if !st.catch_clauses.is_empty() {
            // Python: `except`, `else` and `finally` clauses are children, not fields.
            for c in named(node) {
                let k = c.kind();
                let body = named(c).find(|b| self.spec.cat(*b) == Cat::Block);
                if st.catch_clauses.contains(&k) {
                    let head: Vec<Node> = named(c).filter(|n| Some(*n) != body).collect();
                    let lbl = match (head.first(), head.last()) {
                        (Some(a), Some(b)) => label(&self.src[a.start_byte()..b.end_byte()], 60),
                        _ => String::new(),
                    };
                    let children = self.walk(body, &ctx.with_cond());
                    if meaningful(&children) {
                        out.push(block(BlockKind::Catch, st.catch_kw, lbl, children, line(c)));
                    }
                } else if st.try_else_clauses.contains(&k) {
                    let children = self.walk(body, &ctx.with_cond());
                    if meaningful(&children) {
                        out.push(block(
                            BlockKind::Cond,
                            "else",
                            "if no exception".into(),
                            children,
                            line(c),
                        ));
                    }
                } else if st.finally_clauses.contains(&k) {
                    let children = self.walk(body, ctx);
                    if meaningful(&children) {
                        out.push(block(
                            BlockKind::Finally,
                            "finally",
                            "always runs".into(),
                            children,
                            line(c),
                        ));
                    }
                }
            }
            return out;
        }
        if let Some(h) = field(node, f.handler) {
            let lbl = field(h, f.parameter)
                .map(|p| format!("({})", self.text(p)))
                .unwrap_or_default();
            let children = self.walk(field(h, f.body), &ctx.with_cond());
            if meaningful(&children) {
                out.push(block(BlockKind::Catch, "catch", lbl, children, line(h)));
            }
        }
        if let Some(fin) = field(node, f.finalizer) {
            let b = field(fin, f.body);
            let children = self.walk(b, ctx);
            if meaningful(&children) {
                out.push(block(
                    BlockKind::Finally,
                    "finally",
                    "always runs".into(),
                    children,
                    b.map(line).unwrap_or(line(fin)),
                ));
            }
        }
        out
    }

    fn call_item(&self, node: Node<'t>, ctx: &Ctx<'t, 's, '_>, is_new: bool) -> Vec<Item<'t, 's>> {
        let sp = self.spec;
        let f = &sp.f;
        let callee = if is_new {
            field(node, f.constructor)
        } else {
            field(node, f.function)
        };
        let Some(callee) = callee else {
            return self.walk_all(node, ctx);
        };
        let mut callee = self.lang.unwrap_callee(callee);
        let misplaced = self.lang.misplaced_prefix(callee);
        let callee_text = match misplaced {
            Some((prefix, operand)) => {
                let t = &self.src[operand.start_byte()..callee.end_byte()];
                if prefix == callee {
                    callee = operand;
                }
                t
            }
            None => self.text(callee),
        };
        let mut pre = Vec::new();
        let is_member = is(&sp.k.member, callee);
        if !is_new && is_member {
            pre = self.walk(field(callee, f.member_object), ctx);
        } else if !is_new && is(&sp.k.subscript, callee) {
            pre = self.walk(field(callee, f.subscript_object), ctx);
        }
        let label: Rc<str> = if is_new {
            Rc::from(format!("new {}", label(callee_text, 48)))
        } else {
            Rc::from(label(callee_text, 48))
        };
        let mut target = other_target();
        // How a callback argument is shown: `each item of .map()` or `each item of map()`.
        let mut iter_name: Option<String> = None;
        let st = sp.style;
        if is(&sp.k.identifier, callee) {
            let n = self.text(callee);
            target = self.ident_target(n, ctx);
            if st.iter_fns.contains(&n) {
                iter_name = Some(format!("{n}()"));
            }
        } else if st.path_kinds.contains(&callee.kind()) {
            let text: String = callee_text.chars().filter(|c| !c.is_whitespace()).collect();
            let text = strip_generics(&text);
            let segs: Vec<&str> = text.split("::").collect();
            target = self.path_target(&segs);
        } else if is_member {
            let obj = field(callee, f.member_object);
            let m = field(callee, f.member_property)
                .map(|p| self.text(p))
                .unwrap_or("");
            if sp.iter_methods.contains(&m) {
                iter_name = Some(format!(".{m}()"));
            }
            target.kind = FlowTargetKind::Method;
            target.id = Some(m.into());
            let root = obj.map(|o| self.chain_root(o));
            let root_import = root
                .filter(|r| st.member_roots_ext && is(&sp.k.identifier, *r))
                .and_then(|r| self.file.import_paths.get(self.text(r)));
            if let Some(module) = root_import {
                target = FlowTarget {
                    module: Some(module.clone()),
                    ..self::target(FlowTargetKind::Ext, collapse(callee_text))
                };
            } else if obj.is_some_and(|o| st.self_names.contains(&self.text(o))) {
                target.unit = self.self_unit(m);
            } else if let Some(obj) = obj.filter(|o| is(&sp.k.identifier, *o)) {
                let o = self.text(obj);
                let origin = self.scope.origins.get(o).copied();
                let fp = origin.and_then(|origin| {
                    self.file
                        .factory_props
                        .get(origin)
                        .and_then(|props| props.iter().find(|(k, _)| k == m))
                        .map(|(_, v)| (origin, v))
                });
                match fp {
                    Some((origin, Prop::Unit(u))) => {
                        target.unit = Some(u.clone());
                        target.of = Some(origin.into());
                    }
                    Some((origin, Prop::From(fr))) => {
                        target.of = Some(origin.into());
                        target.note = Some(format!("function returned by {fr}()"));
                        target.ext = Some(fr.clone());
                    }
                    _ => {
                        if sp.builtins.contains(&o) {
                            target.kind = FlowTargetKind::Builtin;
                            target.id = Some(format!("{o}.{m}"));
                        }
                    }
                }
            }
        }
        let mut args = Vec::new();
        let mut inner = Vec::new();
        if let Some(arg_list) = field(node, f.arguments) {
            // Python `f(x for x in xs)`: the generator is the only argument.
            let list: Vec<Node<'t>> = if self.spec.cat(arg_list) == Cat::Comprehension {
                vec![arg_list]
            } else {
                named(arg_list).collect()
            };
            for a in list {
                let a = self.lang.unwrap_arg(a);
                if self.spec.cat(a) == Cat::Skip {
                    continue;
                }
                if self.is_fn(Some(a)) {
                    let body = self.walk_fn(a, ctx);
                    if meaningful(&body) {
                        inner.push(match &iter_name {
                            Some(it) => block(
                                BlockKind::Loop,
                                "loop",
                                format!("each item of {it}"),
                                body,
                                line(a),
                            ),
                            None => block(
                                BlockKind::Callback,
                                "callback",
                                format!("runs inside {label}()"),
                                body,
                                line(a),
                            ),
                        });
                    }
                    continue;
                }
                if is(&sp.k.identifier, a) {
                    let n = self.text(a);
                    if let Some(&local) = self.scope.locals.get(n)
                        && !ctx.inline.contains(&n)
                    {
                        self.used.borrow_mut().insert(n);
                        let body = self.walk_fn(local, &ctx.with_inline(n));
                        inner.push(block(
                            BlockKind::Callback,
                            "callback",
                            format!("{n}() runs inside {label}()"),
                            body,
                            line(local),
                        ));
                        continue;
                    }
                }
                args.extend(self.walk(Some(a), &ctx.with_arg_of(label.clone())));
            }
        }
        if target.kind == FlowTargetKind::Local {
            let id = self.text(callee);
            if !ctx.inline.contains(&id) {
                self.used.borrow_mut().insert(id);
                let fn_node = self.scope.locals[id];
                let body = self.walk_fn(fn_node, &ctx.with_inline(id));
                if meaningful(&body) {
                    inner.insert(
                        0,
                        block(
                            BlockKind::Inline,
                            "body",
                            format!("{id}()"),
                            body,
                            line(fn_node),
                        ),
                    );
                }
            }
        }
        if target.kind == FlowTargetKind::Param {
            let id = self.text(callee);
            if let Some(d) = ctx.param_default(id).filter(|d| self.is_fn(Some(*d))) {
                let body = self.walk_fn(d, ctx);
                if meaningful(&body) {
                    inner.push(block(
                        BlockKind::Inline,
                        "default",
                        format!("{id} = {} (unless the caller passes one)", self.t(d, 48)),
                        body,
                        line(d),
                    ));
                }
            }
        }
        let awaited = node.parent().is_some_and(|p| is(&sp.k.await_, p))
            || misplaced.is_some_and(|(prefix, _)| {
                is(&sp.k.await_, prefix) && self.lang.is_chain_top(node)
            });
        let optional = {
            let mut c = node.walk();
            let mut found = false;
            if c.goto_first_child() {
                loop {
                    let ch = c.node();
                    if sp.names.optional_chain.contains(&ch.kind()) {
                        found = true;
                        break;
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
            found
        };
        let item = Item::Call {
            label: label.to_string(),
            target,
            line: line(node),
            awaited,
            optional,
            arg_of: ctx.arg_of.clone(),
            inner,
        };
        pre.extend(args);
        pre.push(item);
        pre
    }

    fn finalize(
        &self,
        items: Vec<Item<'t, 's>>,
        ctx: &Ctx<'t, 's, '_>,
        returned: &HashSet<&str>,
    ) -> Vec<FlowItem> {
        let mut out = Vec::new();
        for it in items {
            match it {
                Item::Def { name, line, node } => {
                    if self.used.borrow().contains(name) || returned.contains(name) {
                        continue;
                    }
                    // The expansion nests: keep the depth raised while walking and
                    // finalizing the definition so the guard sees the whole chain.
                    let d = self.depth.get();
                    if d > MAX_DEPTH {
                        continue;
                    }
                    self.depth.set(d + 1);
                    let raw = self.walk_fn(node, &ctx.with_inline(name));
                    let body = self.finalize(raw, ctx, returned);
                    self.depth.set(d);
                    if !body.is_empty() {
                        out.push(FlowItem::Block {
                            kind: BlockKind::Deferred,
                            kw: "later".into(),
                            label: format!("{name}() defined here, runs when invoked"),
                            line,
                            children: body,
                        });
                    }
                }
                Item::Block {
                    kind,
                    kw,
                    label,
                    line,
                    children,
                } => out.push(FlowItem::Block {
                    kind,
                    kw: kw.into(),
                    label,
                    line,
                    children: self.finalize(children, ctx, returned),
                }),
                Item::Call {
                    label,
                    target,
                    line,
                    awaited,
                    optional,
                    arg_of,
                    inner,
                } => out.push(FlowItem::Call {
                    label,
                    target,
                    line,
                    awaited,
                    optional,
                    arg_of: arg_of.map(|a| a.to_string()),
                    inner: self.finalize(inner, ctx, returned),
                }),
                Item::Exit {
                    kind,
                    label,
                    line,
                    early,
                    methods,
                } => out.push(FlowItem::Exit {
                    kind,
                    label,
                    line,
                    early,
                    methods,
                }),
            }
        }
        out
    }
}
