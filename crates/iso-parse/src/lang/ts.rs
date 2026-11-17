//! TypeScript and TSX extractor. Owned by workstream B (see CONTRACTS.md).

use std::path::Path;

use iso_core::{FileFacts, Language};

use crate::Extractor;

pub struct TsExtractor {
    pub lang: Language,
}
pub static TS: TsExtractor = TsExtractor {
    lang: Language::TypeScript,
};
pub static TSX: TsExtractor = TsExtractor {
    lang: Language::Tsx,
};

impl Extractor for TsExtractor {
    fn language(&self) -> Language {
        self.lang
    }
    fn extract(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> FileFacts {
        FileFacts::default()
    }
}
