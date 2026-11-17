//! Html extractor. Owned by its language agent (see CONTRACTS.md).

use std::path::Path;

use iso_core::{FileFacts, Language};

use crate::Extractor;

pub struct HtmlExtractor;
pub static HTML: HtmlExtractor = HtmlExtractor;

impl Extractor for HtmlExtractor {
    fn language(&self) -> Language {
        Language::Html
    }
    fn extract(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> FileFacts {
        FileFacts::default()
    }
}
