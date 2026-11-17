//! Node-kind tables that drive the shared walker.
//!
//! A [`FlowSpec`] maps every node kind of one grammar to a [`Cat`] (what the walker does
//! with it) and resolves the field names the walker reads. A new language adds a
//! [`SpecDef`] (kind names per category, field names, a few hook functions) and calls
//! [`FlowSpec::build`]; the walker itself does not change.

use tree_sitter::{Language, Node};

/// What the walker does with a node kind. The comments name the TS/JS kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
    /// Walk the named children in order (the `ts.forEachChild` fallback).
    Other,
    /// Never walked: comments and type syntax.
    Skip,
    /// `statement_block`: its statements in order.
    Block,
    /// `lexical_declaration`, `variable_declaration`: their declarators.
    VarDecl,
    /// `expression_statement`.
    ExprStmt,
    /// `parenthesized_expression`, `as_expression`, `non_null_expression`, …: the inner value.
    Transparent,
    If,
    Ternary,
    /// `binary_expression`; short-circuit operators become conditional blocks.
    Binary,
    Return,
    Throw,
    Try,
    /// C-style `for (init; cond; step)`.
    ForC,
    While,
    Do,
    /// `for … of` / `for … in`.
    ForEach,
    Switch,
    Call,
    New,
    /// Arrow functions and function expressions.
    Lambda,
    /// Object literal.
    Object,
    /// A nested named function statement (`def f():` in Python, `fn f()` in Rust): a local.
    FnDecl,
    /// One binding statement (`x = …` in Python, `let x = …` in Rust): a lambda value makes
    /// a local, anything else is walked (value first, then a non-identifier target).
    Assign,
    /// `match` (Rust) / `match`-`case` (Python): arms come from [`super::FlowLang::match_arms`].
    Match,
    /// Python `with`: the context expressions run first, the body is a block.
    With,
    /// Python list/set/dict comprehensions and generator expressions: a loop.
    Comprehension,
    /// Rust `expr?`: an exit that propagates the error (or `None`) to the caller.
    TryOp,
    /// Rust macro invocation: token trees, scanned by [`super::FlowLang::token_calls`].
    Macro,
}

/// Kind names per category and field names, as written in one grammar.
pub struct SpecDef {
    pub skip: &'static [&'static str],
    pub block: &'static [&'static str],
    pub var_decl: &'static [&'static str],
    pub expr_stmt: &'static [&'static str],
    pub transparent: &'static [&'static str],
    pub if_: &'static [&'static str],
    pub ternary: &'static [&'static str],
    pub binary: &'static [&'static str],
    pub return_: &'static [&'static str],
    pub throw: &'static [&'static str],
    pub try_: &'static [&'static str],
    pub for_c: &'static [&'static str],
    pub while_: &'static [&'static str],
    pub do_: &'static [&'static str],
    pub for_each: &'static [&'static str],
    pub switch: &'static [&'static str],
    pub call: &'static [&'static str],
    pub new: &'static [&'static str],
    pub lambda: &'static [&'static str],
    pub object: &'static [&'static str],
    pub fn_decl: &'static [&'static str],
    pub assign: &'static [&'static str],
    pub match_: &'static [&'static str],
    pub with: &'static [&'static str],
    pub comprehension: &'static [&'static str],
    pub try_op: &'static [&'static str],
    pub macro_: &'static [&'static str],
    /// Short-circuit operators (operator token kind) and the keyword shown for them.
    pub logical_ops: &'static [(&'static str, &'static str)],
    /// Methods whose callback argument runs once per item (`map`, `forEach`, …).
    pub iter_methods: &'static [&'static str],
    /// Receivers whose methods are built-ins (`Object`, `JSON`, …).
    pub builtins: &'static [&'static str],
    pub kinds: KindNames,
    pub style: Style,
}

/// Language conventions beyond node kinds. [`Style::JS`] is analyze.js's behavior.
pub struct Style {
    /// Functions called by bare name that are built-ins (`print`, `len`, `Some`).
    pub builtin_fns: &'static [&'static str],
    /// Functions called by bare name whose callback argument runs once per item (`map`).
    pub iter_fns: &'static [&'static str],
    /// Receiver names that mean the unit's own type (`self`, `cls`).
    pub self_names: &'static [&'static str],
    /// A member call whose receiver chain starts at an imported module is `ext`
    /// (`os.path.join`, `np.mean`). analyze.js treats every member call as a method.
    pub member_roots_ext: bool,
    /// Wrap conditions in parentheses in labels (`if (x)`, `while (x)`), as analyze.js does.
    pub parens: bool,
    /// Keyword shown for a `for … of`/`for … in` loop's operator when the grammar has none.
    pub for_op: &'static str,
    /// An `else` block's line is the `else` keyword's line, not its first statement's.
    pub else_line_from_clause: bool,
    /// Keyword for `match` arms.
    pub match_kw: &'static str,
    /// Keyword for exception handlers (`catch`, `except`).
    pub catch_kw: &'static str,
    /// Clause kinds of a try statement without `handler`/`finalizer` fields (Python).
    pub catch_clauses: &'static [&'static str],
    pub try_else_clauses: &'static [&'static str],
    pub finally_clauses: &'static [&'static str],
    /// `elif` clause kinds (siblings under an if statement's `alternative` field).
    pub elif_clauses: &'static [&'static str],
    /// Comprehension `for` clause kind.
    pub for_in_clause: &'static str,
    /// Call-like nodes whose callee is a path (`Type::f`), classified by
    /// [`super::FlowLang::path_target`].
    pub path_kinds: &'static [&'static str],
    /// Macros that never return (`panic!`): an exit instead of a call.
    pub exit_macros: &'static [&'static str],
    /// Macros shown as built-in calls (`println!`, `vec!`).
    pub builtin_macros: &'static [&'static str],
    /// Field names of a member access (`a.b`) and of a subscript's object.
    pub member_object: &'static str,
    pub member_property: &'static str,
    pub subscript_object: &'static str,
}

impl Style {
    pub const JS: Style = Style {
        builtin_fns: &[],
        iter_fns: &[],
        self_names: &[],
        member_roots_ext: false,
        parens: true,
        for_op: "of",
        else_line_from_clause: false,
        match_kw: "case",
        catch_kw: "catch",
        catch_clauses: &[],
        try_else_clauses: &[],
        finally_clauses: &[],
        elif_clauses: &[],
        for_in_clause: "",
        path_kinds: &[],
        exit_macros: &[],
        builtin_macros: &[],
        member_object: "object",
        member_property: "property",
        subscript_object: "object",
    };
}

/// Individual kind names the walker matches directly.
pub struct KindNames {
    pub identifier: &'static str,
    /// Parenthesized expression (stripped from `if (…)` / `while (…)` labels).
    pub paren: &'static str,
    pub member: &'static str,
    pub subscript: &'static str,
    pub await_: &'static str,
    pub declarator: &'static str,
    pub pair: &'static str,
    pub shorthand: &'static str,
    pub spread: &'static str,
    pub method: &'static str,
    pub return_: &'static str,
    pub object: &'static str,
    pub else_clause: &'static str,
    pub empty_statement: &'static str,
    pub switch_case: &'static str,
    pub template_string: &'static str,
    /// Token or node that marks an optional call (`?.`).
    pub optional_chain: &'static [&'static str],
}

/// A grammar's table, resolved to kind ids and field ids.
pub struct FlowSpec {
    cats: Vec<Cat>,
    pub iter_methods: &'static [&'static str],
    pub builtins: &'static [&'static str],
    pub logical_ops: &'static [(&'static str, &'static str)],
    pub k: KindIds,
    pub f: Fields,
    pub names: &'static KindNames,
    pub style: &'static Style,
}

/// Kind ids (every grammar may alias one name to several ids, so these are lists).
#[derive(Default)]
pub struct KindIds {
    pub identifier: Vec<u16>,
    pub paren: Vec<u16>,
    pub member: Vec<u16>,
    pub subscript: Vec<u16>,
    pub await_: Vec<u16>,
    pub declarator: Vec<u16>,
    pub pair: Vec<u16>,
    pub shorthand: Vec<u16>,
    pub spread: Vec<u16>,
    pub method: Vec<u16>,
    pub return_: Vec<u16>,
    pub object: Vec<u16>,
    pub else_clause: Vec<u16>,
    pub empty_statement: Vec<u16>,
    pub switch_case: Vec<u16>,
    pub template_string: Vec<u16>,
}

pub type FieldId = std::num::NonZeroU16;

/// Field ids the walker reads. `None` when the grammar has no such field.
#[derive(Default)]
pub struct Fields {
    pub body: Option<FieldId>,
    pub condition: Option<FieldId>,
    pub consequence: Option<FieldId>,
    pub alternative: Option<FieldId>,
    pub left: Option<FieldId>,
    pub right: Option<FieldId>,
    pub operator: Option<FieldId>,
    pub value: Option<FieldId>,
    pub name: Option<FieldId>,
    pub function: Option<FieldId>,
    pub constructor: Option<FieldId>,
    pub arguments: Option<FieldId>,
    pub object: Option<FieldId>,
    pub property: Option<FieldId>,
    pub initializer: Option<FieldId>,
    pub increment: Option<FieldId>,
    pub handler: Option<FieldId>,
    pub finalizer: Option<FieldId>,
    pub parameter: Option<FieldId>,
    pub parameters: Option<FieldId>,
    pub pattern: Option<FieldId>,
    pub key: Option<FieldId>,
    pub kind: Option<FieldId>,
    pub source: Option<FieldId>,
    pub alias: Option<FieldId>,
    pub declaration: Option<FieldId>,
    pub definition: Option<FieldId>,
    pub member_object: Option<FieldId>,
    pub member_property: Option<FieldId>,
    pub subscript_object: Option<FieldId>,
}

fn ids(lang: &Language, name: &str) -> Vec<u16> {
    (0..lang.node_kind_count() as u16)
        .filter(|&id| lang.node_kind_is_named(id) && lang.node_kind_for_id(id) == Some(name))
        .collect()
}

impl FlowSpec {
    pub fn build(lang: &Language, def: &'static SpecDef) -> FlowSpec {
        let n = lang.node_kind_count();
        let mut cats = vec![Cat::Other; n];
        let groups: [(&[&str], Cat); 27] = [
            (def.skip, Cat::Skip),
            (def.block, Cat::Block),
            (def.var_decl, Cat::VarDecl),
            (def.expr_stmt, Cat::ExprStmt),
            (def.transparent, Cat::Transparent),
            (def.if_, Cat::If),
            (def.ternary, Cat::Ternary),
            (def.binary, Cat::Binary),
            (def.return_, Cat::Return),
            (def.throw, Cat::Throw),
            (def.try_, Cat::Try),
            (def.for_c, Cat::ForC),
            (def.while_, Cat::While),
            (def.do_, Cat::Do),
            (def.for_each, Cat::ForEach),
            (def.switch, Cat::Switch),
            (def.call, Cat::Call),
            (def.new, Cat::New),
            (def.lambda, Cat::Lambda),
            (def.object, Cat::Object),
            (def.fn_decl, Cat::FnDecl),
            (def.assign, Cat::Assign),
            (def.match_, Cat::Match),
            (def.with, Cat::With),
            (def.comprehension, Cat::Comprehension),
            (def.try_op, Cat::TryOp),
            (def.macro_, Cat::Macro),
        ];
        for (names, cat) in groups {
            for name in names {
                for id in ids(lang, name) {
                    cats[id as usize] = cat;
                }
            }
        }
        let kn = &def.kinds;
        let k = KindIds {
            identifier: ids(lang, kn.identifier),
            paren: ids(lang, kn.paren),
            member: ids(lang, kn.member),
            subscript: ids(lang, kn.subscript),
            await_: ids(lang, kn.await_),
            declarator: ids(lang, kn.declarator),
            pair: ids(lang, kn.pair),
            shorthand: ids(lang, kn.shorthand),
            spread: ids(lang, kn.spread),
            method: ids(lang, kn.method),
            return_: ids(lang, kn.return_),
            object: ids(lang, kn.object),
            else_clause: ids(lang, kn.else_clause),
            empty_statement: ids(lang, kn.empty_statement),
            switch_case: ids(lang, kn.switch_case),
            template_string: ids(lang, kn.template_string),
        };
        let fid = |name: &str| lang.field_id_for_name(name);
        let f = Fields {
            body: fid("body"),
            condition: fid("condition"),
            consequence: fid("consequence"),
            alternative: fid("alternative"),
            left: fid("left"),
            right: fid("right"),
            operator: fid("operator"),
            value: fid("value"),
            name: fid("name"),
            function: fid("function"),
            constructor: fid("constructor"),
            arguments: fid("arguments"),
            object: fid("object"),
            property: fid("property"),
            initializer: fid("initializer"),
            increment: fid("increment"),
            handler: fid("handler"),
            finalizer: fid("finalizer"),
            parameter: fid("parameter"),
            parameters: fid("parameters"),
            pattern: fid("pattern"),
            key: fid("key"),
            kind: fid("kind"),
            source: fid("source"),
            alias: fid("alias"),
            declaration: fid("declaration"),
            definition: fid("definition"),
            member_object: fid(def.style.member_object),
            member_property: fid(def.style.member_property),
            subscript_object: fid(def.style.subscript_object),
        };
        FlowSpec {
            cats,
            iter_methods: def.iter_methods,
            builtins: def.builtins,
            logical_ops: def.logical_ops,
            k,
            f,
            names: &def.kinds,
            style: &def.style,
        }
    }

    #[inline]
    pub fn cat(&self, n: Node) -> Cat {
        if n.is_extra() {
            return Cat::Skip;
        }
        self.cats
            .get(n.kind_id() as usize)
            .copied()
            .unwrap_or(Cat::Other)
    }
}

/// `true` when `n`'s kind id is in `set`.
#[inline]
pub fn is(set: &[u16], n: Node) -> bool {
    set.contains(&n.kind_id())
}
