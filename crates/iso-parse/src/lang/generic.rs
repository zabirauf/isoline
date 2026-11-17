//! Generic tier-2 extractor driven by a grammar's tags query. Owned by workstream N.
//! Adding a language means adding a grammar and `queries/<lang>/tags.scm`, no new Rust code.

use std::path::Path;

use iso_core::{FileFacts, Language};

use crate::Extractor;

pub struct GenericExtractor {
    pub lang: Language,
}
pub static GO: GenericExtractor = GenericExtractor { lang: Language::Go };
pub static JAVA: GenericExtractor = GenericExtractor {
    lang: Language::Java,
};
pub static CSHARP: GenericExtractor = GenericExtractor {
    lang: Language::CSharp,
};

impl Extractor for GenericExtractor {
    fn language(&self) -> Language {
        self.lang
    }
    fn extract(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> FileFacts {
        FileFacts::default()
    }
}
