//! Cache keys for the `derived` and `ai_cache` tables. A key is the hash of its inputs plus a
//! version, so upgrading one extractor or prompt invalidates only its own rows.

use crate::hash64;

/// Kinds of derived results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DerivedKind {
    /// File facts. Key: content hash + extractor version.
    Facts,
    /// Call-flow units for a file. Key: content hash + walker version.
    Flow,
    /// Layers, cycles, clusters for a connected component. Key: hash of its edge set.
    Graph,
}

impl DerivedKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DerivedKind::Facts => "facts",
            DerivedKind::Flow => "flow",
            DerivedKind::Graph => "graph",
        }
    }
}

/// Bumped when any extractor's output changes shape.
pub const EXTRACTOR_VERSION: u32 = 3;

/// Key for a derived result: kind + content hash + version.
pub fn derived_key(kind: DerivedKind, content_hash: &str, version: u32) -> u64 {
    hash64(format!("{}\u{0}{}\u{0}{}", kind.as_str(), content_hash, version).as_bytes())
}

/// Key for the AI cache: content (or body) hash + prompt version + model + kind.
pub fn ai_key(content_hash: &str, prompt_version: u32, model: &str, kind: &str) -> u64 {
    hash64(format!("{content_hash}\u{0}{prompt_version}\u{0}{model}\u{0}{kind}").as_bytes())
}
