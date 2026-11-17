//! Css extractor. Owned by its language agent (see CONTRACTS.md).

use std::path::Path;

use iso_core::{FileFacts, Language};

use crate::Extractor;

pub struct CssExtractor;
pub static CSS: CssExtractor = CssExtractor;

impl Extractor for CssExtractor {
    fn language(&self) -> Language {
        Language::Css
    }
    fn extract(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> FileFacts {
        FileFacts::default()
    }
}
