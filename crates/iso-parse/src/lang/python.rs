//! Python extractor. Owned by its language agent (see CONTRACTS.md).

use std::path::Path;

use iso_core::{FileFacts, Language};

use crate::Extractor;

pub struct PythonExtractor;
pub static PYTHON: PythonExtractor = PythonExtractor;

impl Extractor for PythonExtractor {
    fn language(&self) -> Language {
        Language::Python
    }
    fn extract(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> FileFacts {
        FileFacts::default()
    }
}
