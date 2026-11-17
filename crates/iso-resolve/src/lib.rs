//! Import resolution. One module per language; this file (the dispatch) belongs to the lead.
//!
//! Resolvers never parse code: they map an [`ImportFact`] written in one file to a file in the
//! repo, a package outside it, or nothing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use iso_core::{ImportFact, Language};

pub mod html;
pub mod js;
pub mod python;
pub mod rust;
pub mod scip;

/// Outcome of resolving one import.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// A file inside the repo (repo-relative path, `/` separators).
    File(String),
    /// A package or standard-library module outside the repo (`react`, `os`, `std::fmt`).
    External(String),
    /// Looked like a repo path but nothing matched.
    Unresolved,
}

/// Snapshot of the repo the resolvers work against.
#[derive(Clone, Debug, Default)]
pub struct RepoFiles {
    pub root: PathBuf,
    /// Every walked file, repo-relative with `/` separators.
    pub files: HashSet<String>,
}

impl RepoFiles {
    pub fn new(root: impl Into<PathBuf>, files: impl IntoIterator<Item = String>) -> Self {
        RepoFiles {
            root: root.into(),
            files: files.into_iter().collect(),
        }
    }
    pub fn contains(&self, rel: &str) -> bool {
        self.files.contains(rel)
    }
    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    /// Convert an absolute path back to repo-relative, if it is inside the root.
    pub fn rel(&self, abs: &Path) -> Option<String> {
        let r = abs.strip_prefix(&self.root).ok()?;
        Some(r.to_string_lossy().replace('\\', "/"))
    }
}

/// One resolver per language family.
pub trait Resolver: Send + Sync {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution;
}

/// All resolvers for one repo. Build once per index run; cheap to share across threads.
pub struct Resolvers {
    pub js: js::JsResolver,
    pub python: python::PythonResolver,
    pub rust: rust::RustResolver,
    pub html: html::LinkResolver,
}

impl Resolvers {
    pub fn new(repo: &RepoFiles) -> Self {
        Resolvers {
            js: js::JsResolver::new(repo),
            python: python::PythonResolver::new(repo),
            rust: rust::RustResolver::new(repo),
            html: html::LinkResolver::new(repo),
        }
    }

    pub fn resolve(&self, lang: Language, from_rel: &str, import: &ImportFact) -> Resolution {
        use Language::*;
        match lang {
            TypeScript | Tsx | JavaScript | Jsx => self.js.resolve(from_rel, import),
            Python => self.python.resolve(from_rel, import),
            Rust => self.rust.resolve(from_rel, import),
            Html | Css => self.html.resolve(from_rel, import),
            _ => generic_relative(from_rel, &import.specifier, &self.html.repo),
        }
    }
}

/// Fallback for tier-2 languages: a relative specifier that names a repo file resolves to it,
/// everything else is external.
pub fn generic_relative(from_rel: &str, spec: &str, repo: &RepoFiles) -> Resolution {
    if spec.starts_with('.') {
        let joined = join_rel(parent_dir(from_rel), spec);
        if repo.contains(&joined) {
            return Resolution::File(joined);
        }
        return Resolution::Unresolved;
    }
    Resolution::External(spec.to_string())
}

/// Directory part of a repo-relative path ("" for top-level files).
pub fn parent_dir(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// Join a repo-relative directory with a relative path, normalizing `.` and `..`.
pub fn join_rel(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins() {
        assert_eq!(join_rel("src/a", "../b/c.ts"), "src/b/c.ts");
        assert_eq!(join_rel("", "./x.ts"), "x.ts");
        assert_eq!(parent_dir("a/b/c.ts"), "a/b");
        assert_eq!(parent_dir("c.ts"), "");
    }
}
