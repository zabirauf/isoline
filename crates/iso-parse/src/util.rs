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

// ---------------------------------------------------------------------------------------
// Deep trees (added by workstream B + H).

/// Depth of the deepest node, counted iteratively (no recursion).
pub fn tree_depth(tree: &tree_sitter::Tree) -> usize {
    let mut c = tree.walk();
    let (mut depth, mut max) = (0usize, 0usize);
    loop {
        if c.goto_first_child() {
            depth += 1;
            max = max.max(depth);
            continue;
        }
        loop {
            if c.goto_next_sibling() {
                break;
            }
            if !c.goto_parent() {
                return max;
            }
            depth -= 1;
        }
    }
}

/// Trees up to this depth are visited on the caller's thread. Recursive visitors use a few
/// KiB of stack per tree level in debug builds, and rayon workers and test threads have
/// 2 MiB stacks.
pub const SHALLOW_TREE: usize = 192;

/// Stack bytes reserved per tree level for deep trees (generous for debug builds). The
/// reservation is virtual; only the pages a visitor touches are committed.
const STACK_PER_LEVEL: usize = 16 << 10;

/// Largest stack a deep tree gets. Visitors also cap their own recursion
/// ([`MAX_VISIT_DEPTH`]) so they stay inside it.
const MAX_STACK: usize = 1 << 30;

/// Recursion limit for recursive tree visitors: deeper subtrees are skipped. Matches
/// [`MAX_STACK`] / [`STACK_PER_LEVEL`].
pub const MAX_VISIT_DEPTH: usize = MAX_STACK / STACK_PER_LEVEL;

/// Run `f`, on a scoped thread with a stack sized for `depth` when the tree is deeper than
/// [`SHALLOW_TREE`] (deeply nested callbacks, long `a + b + …` chains in generated code).
pub fn with_stack_for<R: Send>(depth: usize, f: impl FnOnce() -> R + Send) -> R {
    if depth <= SHALLOW_TREE {
        return f();
    }
    let size = (depth.saturating_mul(STACK_PER_LEVEL)).clamp(8 << 20, MAX_STACK);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("iso-parse-deep".into())
            .stack_size(size)
            .spawn_scoped(s, f)
            .expect("spawn deep-tree thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e))
    })
}
