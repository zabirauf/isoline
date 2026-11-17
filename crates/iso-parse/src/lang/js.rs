//! JavaScript and JSX extractor. Owned by workstream B (see CONTRACTS.md).
//!
//! Shares its implementation with the TypeScript extractor (`ts.rs`); the JavaScript grammar
//! is a subset of the TypeScript one as far as the extractor is concerned.

use std::path::Path;

use iso_core::{FileFacts, FlowUnit, Language};

use crate::Extractor;

pub struct JsExtractor {
    pub lang: Language,
}
pub static JS: JsExtractor = JsExtractor {
    lang: Language::JavaScript,
};
pub static JSX: JsExtractor = JsExtractor {
    lang: Language::Jsx,
};

impl Extractor for JsExtractor {
    fn language(&self) -> Language {
        self.lang
    }
    fn extract(&self, path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts {
        super::ts::extract_facts(self.lang, path, source, tree)
    }
    fn flow_units(&self, _path: &Path, source: &str, tree: &tree_sitter::Tree) -> Vec<FlowUnit> {
        crate::flow::js::flow_units(self.lang, source, tree)
    }
}
