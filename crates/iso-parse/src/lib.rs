//! Tree-sitter parsing and per-language extraction.
//!
//! Contract (frozen in phase 0): one [`Extractor`] per language. An extractor sees one file
//! and never touches the database. `extract` runs at index time for every file; `flow_units`
//! runs on demand when a file is opened.
//!
//! File ownership (see CONTRACTS.md): this file and `lang/mod.rs` belong to the lead.

use std::path::Path;

use iso_core::{FileFacts, FlowUnit, Language};

pub mod flow;
pub mod lang;
pub mod util;

pub use tree_sitter;

/// One implementation per language.
pub trait Extractor: Send + Sync {
    fn language(&self) -> Language;
    /// Symbols, imports, calls and references for one file.
    fn extract(&self, path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts;
    /// Call-flow units for every callable in the file. Languages without a flow walker
    /// return an empty list.
    fn flow_units(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> Vec<FlowUnit> {
        Vec::new()
    }
}

/// Tree-sitter grammar for a language, if one is bundled.
pub fn grammar(lang: Language) -> Option<tree_sitter::Language> {
    use Language::*;
    Some(match lang {
        TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        JavaScript | Jsx => tree_sitter_javascript::LANGUAGE.into(),
        Python => tree_sitter_python::LANGUAGE.into(),
        Rust => tree_sitter_rust::LANGUAGE.into(),
        Html => tree_sitter_html::LANGUAGE.into(),
        Css => tree_sitter_css::LANGUAGE.into(),
        Go => tree_sitter_go::LANGUAGE.into(),
        Java => tree_sitter_java::LANGUAGE.into(),
        CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        _ => return None,
    })
}

thread_local! {
    static PARSERS: std::cell::RefCell<std::collections::HashMap<Language, tree_sitter::Parser>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Parse with a per-thread parser pool. Pass the previous tree for an incremental reparse
/// (the caller must have applied `Tree::edit` to it). Returns `None` when no grammar exists.
pub fn parse(
    lang: Language,
    source: &str,
    old: Option<&tree_sitter::Tree>,
) -> Option<tree_sitter::Tree> {
    let g = grammar(lang)?;
    PARSERS.with(|cell| {
        let mut map = cell.borrow_mut();
        let parser = map.entry(lang).or_insert_with(|| {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&g).expect("grammar ABI matches tree-sitter");
            p
        });
        parser.parse(source, old)
    })
}

/// The extractor for a language: a dedicated one for tier 1, the generic tags extractor for
/// tier 2, `None` for the text tier.
pub fn extractor(lang: Language) -> Option<&'static dyn Extractor> {
    lang::extractor(lang)
}

/// Parse and extract in one call. Text-tier files return empty facts.
pub fn extract_file(path: &Path, lang: Language, source: &str) -> FileFacts {
    let Some(ex) = extractor(lang) else {
        return FileFacts::default();
    };
    let Some(tree) = parse(lang, source, None) else {
        return FileFacts::default();
    };
    let mut facts = ex.extract(path, source, &tree);
    facts.has_errors |= tree.root_node().has_error();
    facts
}

/// Parse and compute call-flow units in one call.
pub fn flow_units(path: &Path, lang: Language, source: &str) -> Vec<FlowUnit> {
    let Some(ex) = extractor(lang) else {
        return Vec::new();
    };
    let Some(tree) = parse(lang, source, None) else {
        return Vec::new();
    };
    ex.flow_units(path, source, &tree)
}
