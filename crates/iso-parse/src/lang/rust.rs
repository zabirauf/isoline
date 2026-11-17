//! Rust extractor. Owned by its language agent (see CONTRACTS.md).

use std::path::Path;

use iso_core::{FileFacts, Language};

use crate::Extractor;

pub struct RustExtractor;
pub static RUST: RustExtractor = RustExtractor;

impl Extractor for RustExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }
    fn extract(&self, _path: &Path, _source: &str, _tree: &tree_sitter::Tree) -> FileFacts {
        FileFacts::default()
    }
}
