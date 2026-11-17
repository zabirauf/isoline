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
    let masked;
    let source =
        if matches!(lang, Language::TypeScript | Language::Tsx) && source.contains("import(") {
            masked = mask_type_imports(source);
            masked.as_str()
        } else {
            source
        };
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

/// Replace `import("path")` with an identifier of the same byte length where it is a type
/// (`import("x").T`, `typeof import("x")`), leaving dynamic `import()` calls untouched.
/// tree-sitter-typescript cannot parse those type imports, and its error recovery then loses
/// the declarations that follow.
pub fn mask_type_imports(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while let Some(off) = src[i..].find("import(") {
        let start = i + off;
        i = start + 7;
        if start > 0 {
            let p = b[start - 1];
            if p.is_ascii_alphanumeric() || p == b'_' || p == b'$' || p == b'.' {
                continue;
            }
        }
        let mut j = start + 7;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let Some(&q) = b.get(j) else { break };
        if !matches!(q, b'"' | b'\'' | b'`') {
            continue;
        }
        let Some(qe) = src[j + 1..].find(q as char).map(|k| j + 1 + k) else {
            break;
        };
        let mut k = qe + 1;
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if b.get(k) != Some(&b')') {
            continue;
        }
        let end = k + 1;
        let after_typeof = src[..start].trim_end().ends_with("typeof");
        let member_type = b.get(end) == Some(&b'.') && {
            let rest = &src[end + 1..];
            let ident: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            !ident.is_empty()
                && ident != "then"
                && !rest[ident.len()..].trim_start().starts_with('(')
        };
        if (after_typeof || member_type) && !src[start..end].contains('\n') {
            out[start..end].fill(b'_');
            i = end;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

#[cfg(test)]
mod mask_tests {
    use super::*;

    #[test]
    fn masks_type_imports_only() {
        let src = "type A = { c?: import(\"../x.js\").ChatType; d: readonly import('y').K[] };\nconst m = await import(\"./m.js\");\nimport(\"./n\").then(f);\nlet t: typeof import(\"z\");";
        let m = mask_type_imports(src);
        assert_eq!(m.len(), src.len());
        assert!(m.contains(&format!("c?: {}.ChatType", "_".repeat(17))));
        assert!(m.contains(&format!("readonly {}.K[]", "_".repeat(11))));
        assert!(m.contains("await import(\"./m.js\")"));
        assert!(m.contains("import(\"./n\").then(f)"));
        assert!(m.contains(&format!("typeof {};", "_".repeat(11))));
        let tree = parse(Language::TypeScript, src, None).unwrap();
        assert!(!tree.root_node().has_error());
    }
}
