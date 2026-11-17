//! Facts an extractor produces for one file, and the graph edges built from them.
//!
//! Lines are 1-based everywhere. Columns are not stored.

use serde::{Deserialize, Serialize};

use crate::SymbolId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Type,
    Enum,
    Struct,
    Trait,
    Impl,
    Module,
    Variable,
    Constant,
    Field,
    /// CSS rule (selector) or HTML element with an id.
    Selector,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        use SymbolKind::*;
        match self {
            Function => "function",
            Method => "method",
            Class => "class",
            Interface => "interface",
            Type => "type",
            Enum => "enum",
            Struct => "struct",
            Trait => "trait",
            Impl => "impl",
            Module => "module",
            Variable => "variable",
            Constant => "constant",
            Field => "field",
            Selector => "selector",
        }
    }
    pub fn parse(s: &str) -> SymbolKind {
        use SymbolKind::*;
        match s {
            "function" => Function,
            "method" => Method,
            "class" => Class,
            "interface" => Interface,
            "type" => Type,
            "enum" => Enum,
            "struct" => Struct,
            "trait" => Trait,
            "impl" => Impl,
            "module" => Module,
            "constant" => Constant,
            "field" => Field,
            "selector" => Selector,
            _ => Variable,
        }
    }
    /// Callables get a function card and a call flow.
    pub fn is_callable(self) -> bool {
        matches!(self, SymbolKind::Function | SymbolKind::Method)
    }
    /// Declarations listed under "Local types".
    pub fn is_type_like(self) -> bool {
        matches!(
            self,
            SymbolKind::Class
                | SymbolKind::Interface
                | SymbolKind::Type
                | SymbolKind::Enum
                | SymbolKind::Struct
                | SymbolKind::Trait
        )
    }
}

/// One declared symbol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolFact {
    /// Bare name as written: `run`, `MyClass`.
    pub name: String,
    /// Dotted path inside the file: `MyClass.run`, `outer.inner`. Unique per file and kind.
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub start_line: u32,
    pub end_line: u32,
    /// The declaration head up to the body, whitespace collapsed, at most 200 chars.
    pub signature: String,
    /// Doc comment text with comment markers removed, if any.
    #[serde(default)]
    pub doc: Option<String>,
    pub exported: bool,
    #[serde(default)]
    pub is_async: bool,
    /// Parameter names as written (`execute = …` when it has a default).
    #[serde(default)]
    pub params: Vec<String>,
    /// Qualified name of the enclosing symbol, if nested (a method's class).
    #[serde(default)]
    pub parent: Option<String>,
    /// hash64 of the symbol's source text. Unchanged body hash means flow and summary stay.
    pub body_hash: u64,
}

/// One name brought in by an import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ImportedName {
    /// Name as exported by the target (`default` and `*` are allowed).
    pub name: String,
    /// Local binding, when different from `name`.
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub type_only: bool,
}

impl ImportedName {
    pub fn local(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum ImportKind {
    /// `import … from`, Python `import`/`from`, Rust `use`.
    Static,
    /// `import()` with a string literal.
    Dynamic,
    /// CommonJS `require("…")`.
    Require,
    /// `export … from` re-export.
    Reexport,
    /// Rust `mod foo;` declaration (resolves to a file).
    Mod,
    /// HTML `<script src>` / `<link href>`, CSS `@import`.
    Link,
}

/// One import statement (or equivalent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportFact {
    /// Module specifier exactly as written: `../a.js`, `react`, `.models`, `crate::a::b`.
    pub specifier: String,
    pub kind: ImportKind,
    pub names: Vec<ImportedName>,
    /// The whole statement is type-only (`import type …`).
    #[serde(default)]
    pub type_only: bool,
    pub line: u32,
}

/// One call site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallFact {
    /// Qualified name of the enclosing callable; `None` at module top level.
    pub caller: Option<String>,
    /// Callee text as written, whitespace collapsed: `foo`, `this.bar`, `a.b.c`, `new Foo`.
    pub callee: String,
    /// Last identifier of the callee: `foo`, `bar`, `c`, `Foo`.
    pub name: String,
    /// Receiver expression text when the callee is a member access (`a.b` for `a.b.c()`).
    #[serde(default)]
    pub receiver: Option<String>,
    pub line: u32,
}

/// A reference from a symbol's body to a name bound at file level (an import or a local declaration).
/// Calls are also references; `RefFact` additionally covers non-call uses such as type annotations
/// and values passed around.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefFact {
    /// Qualified name of the enclosing symbol; `None` at module top level.
    pub from: Option<String>,
    /// The identifier referenced.
    pub name: String,
    /// The reference sits in a type position (annotation, `implements`, generic argument).
    #[serde(default)]
    pub type_position: bool,
    pub line: u32,
}

/// Everything an extractor learns from one file. Order is source order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFacts {
    pub symbols: Vec<SymbolFact>,
    pub imports: Vec<ImportFact>,
    pub calls: Vec<CallFact>,
    #[serde(default)]
    pub refs: Vec<RefFact>,
    /// Parse errors were present (the tree still exists).
    #[serde(default)]
    pub has_errors: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// From a precise indexer (SCIP) or a same-file definition.
    Exact,
    /// Name plus import match.
    Heuristic,
    /// Proposed by a model.
    Inferred,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Exact => "exact",
            Confidence::Heuristic => "heuristic",
            Confidence::Inferred => "inferred",
        }
    }
    pub fn parse(s: &str) -> Confidence {
        match s {
            "exact" => Confidence::Exact,
            "inferred" => Confidence::Inferred,
            _ => Confidence::Heuristic,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Import,
    Call,
    TypeUse,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Import => "import",
            EdgeKind::Call => "call",
            EdgeKind::TypeUse => "typeuse",
        }
    }
    pub fn parse(s: &str) -> EdgeKind {
        match s {
            "import" => EdgeKind::Import,
            "typeuse" => EdgeKind::TypeUse,
            _ => EdgeKind::Call,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    Symbol { id: SymbolId },
    External { module: String, name: String },
    Unresolved { text: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Edge {
    pub from: SymbolId,
    pub to: Target,
    pub kind: EdgeKind,
    pub confidence: Confidence,
    pub line: u32,
}
