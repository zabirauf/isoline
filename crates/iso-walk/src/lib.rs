//! Gitignore-aware parallel walk, content hashing, the Merkle tree and the file watcher.
//!
//! Why no git-seeded hashes: the plan suggests taking hashes of files unchanged since HEAD from
//! git. Isoline must read every file once anyway to parse it on a cold index, and BLAKE3 hashes
//! far faster than the read, so seeding from git would save nothing there. On reopen the
//! size + mtime check below skips reading entirely, which is the case that matters.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

use iso_core::Language;

pub mod watch;

/// Files larger than this are kept in the text tier and never parsed (minified bundles,
/// generated data).
pub const MAX_PARSE_BYTES: u64 = 1_500_000;

/// Directories skipped even when no ignore file mentions them.
pub const ALWAYS_SKIP: &[&str] = &[
    ".git",
    "node_modules",
    ".hg",
    ".svn",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".next",
    ".turbo",
    ".gradle",
    ".idea",
    ".DS_Store",
];

/// One file found by the walk. Content is not read here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkedFile {
    /// Repo-relative, `/` separators.
    pub rel: String,
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub mtime: i64,
}

/// Walk `root`, honoring .gitignore, .ignore, .isolineignore and global git excludes.
/// Hidden files are skipped except a few well-known dotfiles. Results are sorted by path.
pub fn walk(root: &Path) -> Vec<WalkedFile> {
    let out = Mutex::new(Vec::with_capacity(4096));
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .add_custom_ignore_filename(".isolineignore")
        .threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(12),
        )
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_some_and(|t| t.is_dir()) && ALWAYS_SKIP.contains(&name.as_ref()))
        });
    builder.build_parallel().run(|| {
        let out = &out;
        Box::new(move |res| {
            let Ok(entry) = res else {
                return ignore::WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return ignore::WalkState::Continue;
            }
            let Ok(rel) = entry.path().strip_prefix(root) else {
                return ignore::WalkState::Continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.is_empty() {
                return ignore::WalkState::Continue;
            }
            let Ok(md) = entry.metadata() else {
                return ignore::WalkState::Continue;
            };
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);
            out.lock().unwrap().push(WalkedFile {
                rel,
                size: md.len(),
                mtime,
            });
            ignore::WalkState::Continue
        })
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by(|a, b| a.rel.cmp(&b.rel));
    v
}

/// Content read from disk with its hash and detected language.
#[derive(Clone, Debug)]
pub struct ReadFile {
    pub text: String,
    pub hash: String,
    pub lang: Language,
    pub lines: u32,
    /// The file looked binary (NUL byte in the first 8 KB) or was not valid UTF-8.
    pub binary: bool,
}

/// Read, hash and classify one file.
pub fn read_file(root: &Path, rel: &str) -> std::io::Result<ReadFile> {
    let bytes = std::fs::read(root.join(rel))?;
    Ok(classify(rel, bytes))
}

pub fn classify(rel: &str, bytes: Vec<u8>) -> ReadFile {
    let hash = iso_core::content_hash(&bytes);
    let head = &bytes[..bytes.len().min(8192)];
    let binary = head.contains(&0);
    let text = if binary {
        String::new()
    } else {
        match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        }
    };
    let first = text.lines().next();
    let lang = if binary {
        Language::Other
    } else {
        Language::detect(rel, first)
    };
    let lines = count_lines(&text);
    ReadFile {
        text,
        hash,
        lang,
        lines,
        binary,
    }
}

pub fn count_lines(s: &str) -> u32 {
    if s.is_empty() {
        return 0;
    }
    let n = s.as_bytes().iter().filter(|&&b| b == b'\n').count() as u32;
    if s.ends_with('\n') { n } else { n + 1 }
}

/// Merkle tree over the repo: a file's hash is its content hash; a directory's is the hash of
/// its children's names and hashes, sorted by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MerkleTree {
    /// path → node. The root directory is "".
    pub nodes: HashMap<String, MerkleNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleNode {
    pub is_dir: bool,
    pub hash: String,
    pub parent: Option<String>,
}

fn parent_of(p: &str) -> String {
    p.rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_default()
}

impl MerkleTree {
    /// Build from (repo-relative path, content hash) pairs.
    pub fn build<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> MerkleTree {
        let mut children: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        let mut nodes = HashMap::new();
        children.entry(String::new()).or_default();
        for (path, hash) in files {
            let parent = parent_of(path);
            nodes.insert(
                path.to_string(),
                MerkleNode {
                    is_dir: false,
                    hash: hash.to_string(),
                    parent: Some(parent.clone()),
                },
            );
            // Make sure every ancestor directory exists.
            let mut d = parent;
            loop {
                if children.contains_key(&d) {
                    break;
                }
                children.insert(d.clone(), Vec::new());
                if d.is_empty() {
                    break;
                }
                d = parent_of(&d);
            }
        }
        for (path, n) in &nodes {
            let name = path.rsplit('/').next().unwrap_or(path);
            children
                .get_mut(n.parent.as_deref().unwrap_or(""))
                .unwrap()
                .push((name.to_string(), n.hash.clone()));
        }
        // Directories deepest first so children are hashed before parents.
        let mut dirs: Vec<String> = children.keys().cloned().collect();
        dirs.sort_by_key(|d| {
            std::cmp::Reverse(if d.is_empty() {
                0
            } else {
                d.matches('/').count() + 1
            })
        });
        for d in dirs {
            let mut kids = children.remove(&d).unwrap_or_default();
            kids.sort();
            let mut h = blake3::Hasher::new();
            for (name, hash) in &kids {
                h.update(name.as_bytes());
                h.update(&[0]);
                h.update(hash.as_bytes());
                h.update(&[0]);
            }
            let hash = h.finalize().to_hex().to_string();
            let parent = if d.is_empty() {
                None
            } else {
                Some(parent_of(&d))
            };
            if let Some(p) = &parent {
                let name = d.rsplit('/').next().unwrap_or(&d).to_string();
                children
                    .entry(p.clone())
                    .or_default()
                    .push((name, hash.clone()));
            }
            nodes.insert(
                d,
                MerkleNode {
                    is_dir: true,
                    hash,
                    parent,
                },
            );
        }
        MerkleTree { nodes }
    }

    /// Apply file changes in place: `(path, Some(content hash))` for added or modified files,
    /// `(path, None)` for removed ones. Only the ancestor directories of changed files are
    /// rehashed. Returns the node paths to write and the node paths to delete, so storage can
    /// be updated without rewriting the whole tree.
    pub fn apply(&mut self, changes: &[(String, Option<String>)]) -> NodeChanges {
        let mut out = NodeChanges::default();
        let mut affected: HashSet<String> = HashSet::new();
        for (path, hash) in changes {
            let parent = parent_of(path);
            match hash {
                Some(h) => {
                    let n = self.nodes.entry(path.clone()).or_insert(MerkleNode {
                        is_dir: false,
                        hash: String::new(),
                        parent: Some(parent.clone()),
                    });
                    n.hash = h.clone();
                    out.upserts.insert(path.clone());
                }
                None => {
                    if self.nodes.remove(path).is_some() {
                        out.removed.insert(path.clone());
                    }
                }
            }
            // Every ancestor needs rehashing (and creating, for a new directory).
            let mut d = parent;
            loop {
                affected.insert(d.clone());
                if !self.nodes.contains_key(&d) {
                    let p = if d.is_empty() {
                        None
                    } else {
                        Some(parent_of(&d))
                    };
                    self.nodes.insert(
                        d.clone(),
                        MerkleNode {
                            is_dir: true,
                            hash: String::new(),
                            parent: p,
                        },
                    );
                }
                if d.is_empty() {
                    break;
                }
                d = parent_of(&d);
            }
        }
        // Children of the affected directories only.
        let mut kids: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (p, n) in &self.nodes {
            if let Some(par) = &n.parent
                && affected.contains(par)
            {
                let name = p.rsplit('/').next().unwrap_or(p).to_string();
                kids.entry(par.clone()).or_default().push((name, p.clone()));
            }
        }
        let mut dirs: Vec<String> = affected.into_iter().collect();
        dirs.sort_by_key(|d| {
            std::cmp::Reverse(if d.is_empty() {
                0
            } else {
                d.matches('/').count() + 1
            })
        });
        for d in dirs {
            let mut ks = kids.remove(&d).unwrap_or_default();
            if ks.is_empty() && !d.is_empty() {
                // An emptied directory disappears; its parent was already queued.
                if self.nodes.remove(&d).is_some() {
                    out.removed.insert(d.clone());
                    out.upserts.remove(&d);
                }
                if let Some(par) = Some(parent_of(&d))
                    && let Some(v) = kids.get_mut(&par)
                {
                    v.retain(|(_, p)| p != &d);
                }
                continue;
            }
            ks.sort();
            let mut h = blake3::Hasher::new();
            for (name, p) in &ks {
                let ch = self.nodes.get(p).map(|n| n.hash.as_str()).unwrap_or("");
                h.update(name.as_bytes());
                h.update(&[0]);
                h.update(ch.as_bytes());
                h.update(&[0]);
            }
            let hash = h.finalize().to_hex().to_string();
            let n = self.nodes.get_mut(&d).expect("created above");
            if n.hash != hash {
                n.hash = hash;
                out.upserts.insert(d.clone());
            }
            // Make sure the parent sees this directory as a child.
            if !d.is_empty() {
                let par = parent_of(&d);
                let name = d.rsplit('/').next().unwrap_or(&d).to_string();
                let v = kids.entry(par).or_default();
                if !v.iter().any(|(_, p)| p == &d) {
                    v.push((name, d.clone()));
                }
            }
        }
        out
    }

    pub fn root_hash(&self) -> Option<&str> {
        self.nodes.get("").map(|n| n.hash.as_str())
    }

    fn children_index(&self) -> HashMap<&str, Vec<&str>> {
        let mut idx: HashMap<&str, Vec<&str>> = HashMap::new();
        for (p, n) in &self.nodes {
            if let Some(par) = &n.parent {
                idx.entry(par.as_str()).or_default().push(p.as_str());
            }
        }
        idx
    }

    /// Files that differ between two trees (added, removed or modified), found by descending
    /// only into directories whose hashes differ. Sorted.
    pub fn diff(old: &MerkleTree, new: &MerkleTree) -> TreeDiff {
        let mut out = TreeDiff::default();
        let old_kids = old.children_index();
        let new_kids = new.children_index();
        let mut stack = vec![String::new()];
        while let Some(dir) = stack.pop() {
            out.dirs_visited += 1;
            let a = old_kids.get(dir.as_str()).cloned().unwrap_or_default();
            let b = new_kids.get(dir.as_str()).cloned().unwrap_or_default();
            let mut names: Vec<&str> = a.iter().chain(b.iter()).copied().collect();
            names.sort();
            names.dedup();
            for p in names {
                let (o, n) = (old.nodes.get(p), new.nodes.get(p));
                match (o, n) {
                    (Some(o), Some(n)) if o.hash == n.hash && o.is_dir == n.is_dir => {}
                    (Some(o), Some(n)) if o.is_dir && n.is_dir => stack.push(p.to_string()),
                    _ => {
                        for (node, side) in [(o, 0), (n, 1)] {
                            let Some(node) = node else { continue };
                            if node.is_dir {
                                let tree = if side == 0 { old } else { new };
                                collect_files(tree, p, &mut out.changed);
                            } else {
                                out.changed.push(p.to_string());
                            }
                        }
                    }
                }
            }
        }
        out.changed.sort();
        out.changed.dedup();
        out
    }
}

fn collect_files(tree: &MerkleTree, dir: &str, out: &mut Vec<String>) {
    let prefix = format!("{dir}/");
    for (p, n) in &tree.nodes {
        if !n.is_dir && (dir.is_empty() || p.starts_with(&prefix)) {
            out.push(p.clone());
        }
    }
}

/// Nodes changed by [`MerkleTree::apply`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeChanges {
    pub upserts: std::collections::BTreeSet<String>,
    pub removed: std::collections::BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeDiff {
    pub changed: Vec<String>,
    pub dirs_visited: usize,
}

/// Simple timing span for pipeline stages.
pub struct Span {
    name: &'static str,
    start: Instant,
}

impl Span {
    pub fn new(name: &'static str) -> Self {
        Span {
            name,
            start: Instant::now(),
        }
    }
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// Absolute path helper.
pub fn abs(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_diff_finds_exactly_changed_files() {
        let a = MerkleTree::build([
            ("src/a.ts", "1"),
            ("src/b.ts", "2"),
            ("lib/c.py", "3"),
            ("README.md", "4"),
        ]);
        let same = MerkleTree::build([
            ("README.md", "4"),
            ("lib/c.py", "3"),
            ("src/b.ts", "2"),
            ("src/a.ts", "1"),
        ]);
        assert_eq!(a.root_hash(), same.root_hash());
        assert!(MerkleTree::diff(&a, &same).changed.is_empty());

        let b = MerkleTree::build([
            ("src/a.ts", "1x"),
            ("src/b.ts", "2"),
            ("lib/c.py", "3"),
            ("README.md", "4"),
            ("new/d.rs", "5"),
        ]);
        let d = MerkleTree::diff(&a, &b);
        assert_eq!(d.changed, vec!["new/d.rs", "src/a.ts"]);
        // lib/ is identical, so it is never descended into.
        assert!(d.dirs_visited <= 2, "visited {}", d.dirs_visited);

        let c = MerkleTree::build([("src/a.ts", "1"), ("README.md", "4")]);
        assert_eq!(
            MerkleTree::diff(&a, &c).changed,
            vec!["lib/c.py", "src/b.ts"]
        );
    }

    #[test]
    fn apply_matches_a_full_rebuild() {
        let files = [
            ("src/a.ts", "1"),
            ("src/b.ts", "2"),
            ("lib/c.py", "3"),
            ("README.md", "4"),
            ("x/y/z.rs", "5"),
        ];
        let mut t = MerkleTree::build(files);
        let changes = vec![
            ("src/a.ts".to_string(), Some("1x".to_string())),
            ("x/y/z.rs".to_string(), None),
            ("new/deep/d.go".to_string(), Some("6".to_string())),
        ];
        let ch = t.apply(&changes);
        let rebuilt = MerkleTree::build([
            ("src/a.ts", "1x"),
            ("src/b.ts", "2"),
            ("lib/c.py", "3"),
            ("README.md", "4"),
            ("new/deep/d.go", "6"),
        ]);
        assert_eq!(t, rebuilt);
        assert!(
            ch.removed.contains("x/y/z.rs")
                && ch.removed.contains("x/y")
                && ch.removed.contains("x")
        );
        assert!(
            ch.upserts.contains("")
                && ch.upserts.contains("src")
                && ch.upserts.contains("new/deep")
        );
        assert!(
            !ch.upserts.contains("lib"),
            "untouched directories are not rewritten"
        );
    }

    #[test]
    fn walk_honors_gitignore_and_skips() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::create_dir_all(r.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(r.join("gen")).unwrap();
        std::fs::write(r.join("src/a.ts"), "export const a = 1;\n").unwrap();
        std::fs::write(r.join("node_modules/x/i.js"), "x").unwrap();
        std::fs::write(r.join("gen/out.js"), "x").unwrap();
        std::fs::write(r.join(".gitignore"), "gen/\n").unwrap();
        let files: Vec<String> = walk(r).into_iter().map(|f| f.rel).collect();
        assert_eq!(files, vec!["src/a.ts"]);
    }

    #[test]
    fn classify_detects_binary_and_lines() {
        let f = classify("a.ts", b"a\nb\n".to_vec());
        assert_eq!(f.lines, 2);
        assert!(!f.binary);
        let g = classify("x.bin", vec![0, 1, 2]);
        assert!(g.binary);
        assert_eq!(count_lines("a"), 1);
    }
}
