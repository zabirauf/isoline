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

// ---- Helpers added by workstream C + D + N (Python, Rust, HTML, CSS, generic). ----

/// Like [`short`], but only reads as much of `s` as it needs, so clipping a long text costs
/// O(max) instead of O(len). Use it for texts that nest (callees, receivers).
pub fn clip(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_space = count > 0;
            continue;
        }
        let needed = if pending_space { 2 } else { 1 };
        if count + needed > max {
            // Drop back to max - 1 chars and add the ellipsis.
            while count > max.saturating_sub(1) {
                out.pop();
                count -= 1;
            }
            out.push('…');
            return out;
        }
        if pending_space {
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        out.push(ch);
        count += 1;
    }
    out
}

/// hash64 of a node's source text, used for `SymbolFact::body_hash`.
pub fn node_hash(n: Node, src: &str) -> u64 {
    iso_core::hash64(text(n, src).as_bytes())
}

/// Declaration head: source from `start` up to `body` (or the end of `n`), whitespace
/// collapsed, trailing `:`, `{` and `;` removed, at most 200 chars.
pub fn head_until(start: usize, n: Node, body: Option<Node>, src: &str) -> String {
    let end = body.map(|b| b.start_byte()).unwrap_or(n.end_byte());
    let raw = src.get(start..end.max(start)).unwrap_or("");
    let trimmed = raw.trim_end().trim_end_matches([':', '{', ';']).trim_end();
    // Multi-line parameter lists collapse to `( a, b, )`; tidy them to `(a, b)`.
    let head = clip(trimmed, 400)
        .replace("( ", "(")
        .replace(", )", ")")
        .replace(",)", ")")
        .replace(" )", ")");
    clip(&head, 200)
}

/// Makes qualified names unique per kind by appending `#2`, `#3`, … to repeats.
#[derive(Default)]
pub struct UniqueNames {
    seen: std::collections::HashSet<(String, iso_core::SymbolKind)>,
}

impl UniqueNames {
    pub fn claim(&mut self, qualified: String, kind: iso_core::SymbolKind) -> String {
        if self.seen.insert((qualified.clone(), kind)) {
            return qualified;
        }
        let mut k = 2;
        loop {
            let candidate = format!("{qualified}#{k}");
            if self.seen.insert((candidate.clone(), kind)) {
                return candidate;
            }
            k += 1;
        }
    }
}

/// Removes common leading indentation (after the first line) and blank edge lines, like
/// Python's `inspect.cleandoc`. Returns `None` for empty text.
pub fn clean_doc(raw: &str) -> Option<String> {
    let lines: Vec<&str> = raw.lines().collect();
    let indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out: Vec<&str> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.trim()
            } else {
                l.get(indent..).unwrap_or("").trim_end()
            }
        })
        .collect();
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let first = out.iter().position(|l| !l.is_empty())?;
    Some(out[first..].join("\n"))
}

/// Strips comment markers (`///`, `//!`, `//`, `#`, `/** */`, leading `*`) from comment
/// texts and joins them. Used for doc comments in Rust and tier-2 languages.
pub fn strip_comment_markers<'a>(comments: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut lines = Vec::new();
    for c in comments {
        let c = c.trim();
        if let Some(body) = c.strip_prefix("/*") {
            let body = body.trim_start_matches(['*', '!']);
            let body = body.strip_suffix("*/").unwrap_or(body);
            for l in body.lines() {
                let l = l.trim();
                let l = l.strip_prefix('*').unwrap_or(l);
                lines.push(l.strip_prefix(' ').unwrap_or(l).trim_end());
            }
        } else {
            for l in c.lines() {
                let l = l.trim().trim_start_matches(['/', '#']);
                let l = l.strip_prefix('!').unwrap_or(l);
                lines.push(l.strip_prefix(' ').unwrap_or(l).trim_end());
            }
        }
    }
    clean_doc(&lines.join("\n"))
}

// ---- End of workstream C + D + N helpers. ----

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
