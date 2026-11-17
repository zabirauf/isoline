//! JavaScript and JSX extractor. Owned by workstream B (see CONTRACTS.md).

use std::path::Path;

use iso_core::{FileFacts, Language};

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
    fn extract(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> FileFacts {
        FileFacts::default()
    }
}
