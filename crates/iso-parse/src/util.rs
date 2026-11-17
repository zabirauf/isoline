//! Small helpers shared by extractors. Lead-owned; agents may add functions at the end.

use tree_sitter::Node;

/// 1-based start line.
pub fn line(n: Node) -> u32 {
    n.start_position().row as u32 + 1
}

/// 1-based end line.
pub fn end_line(n: Node) -> u32 {
    n.end_position().row as u32 + 1
}

/// Source text of a node.
pub fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

/// Text with runs of whitespace collapsed to one space, trimmed, cut to `max` chars with "…".
pub fn short(s: &str, max: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max {
        let mut out: String = collapsed.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        collapsed
    }
}
