//! Python import resolution. Owned by workstream C (see CONTRACTS.md).
//!
//! Search roots, nearest first for each importing file:
//! - the parent of every top-level package (a directory with `__init__.py` whose parent has
//!   none), which covers the `src/` layout;
//! - from each `pyproject.toml`: its directory, `src/` next to it when that exists, poetry
//!   `packages = [{ include, from }]`, setuptools `package-dir` and
//!   `[tool.setuptools.packages.find] where`;
//! - the repo root, and last the importing file's own directory (scripts import siblings).
//!
//! A module `a.b` is `a/b/__init__.py` (or a namespace directory) before `a/b.py`.
//! `from pkg import name` resolves to the submodule `pkg/name.py` when one of the names is a
//! submodule (the first that is), else to the package's `__init__.py`. Relative imports
//! (`.`, `..x`) start from the importing file's package. A top-level name that no root
//! provides is `External(top-level name)` (stdlib and third-party look the same); a name a
//! root does provide but whose path is missing is `Unresolved`.

use std::collections::HashSet;

use iso_core::ImportFact;

use crate::rust::toml_lite;
use crate::{RepoFiles, Resolution, Resolver, join_rel, parent_dir};

pub struct PythonResolver {
    pub repo: RepoFiles,
    /// Repo-relative search roots ("" is the repo root), deduplicated.
    roots: Vec<String>,
    /// Every directory that holds a file, repo-relative.
    dirs: HashSet<String>,
}

impl PythonResolver {
    pub fn new(repo: &RepoFiles) -> Self {
        let mut dirs = HashSet::new();
        for f in &repo.files {
            let mut d = parent_dir(f);
            while !d.is_empty() && dirs.insert(d.to_string()) {
                d = parent_dir(d);
            }
        }
        let mut roots: Vec<String> = Vec::new();
        let mut add = |r: String| {
            if !roots.contains(&r) {
                roots.push(r);
            }
        };
        let mut sorted: Vec<&String> = repo.files.iter().collect();
        sorted.sort();
        for f in sorted {
            let name = f.rsplit('/').next().unwrap_or(f);
            if name == "__init__.py" || name == "__init__.pyi" {
                let pkg = parent_dir(f);
                if pkg.is_empty() {
                    continue;
                }
                let parent = parent_dir(pkg);
                if !has_init(repo, parent) {
                    add(parent.to_string());
                }
            } else if name == "pyproject.toml" {
                let dir = parent_dir(f);
                for r in pyproject_roots(repo, f, &dirs) {
                    add(r);
                }
                add(dir.to_string());
            }
        }
        add(String::new());
        PythonResolver {
            repo: repo.clone(),
            roots,
            dirs,
        }
    }

    fn is_dir(&self, rel: &str) -> bool {
        rel.is_empty() || self.dirs.contains(rel)
    }

    fn module_file(&self, base: &str) -> Option<String> {
        [format!("{base}.py"), format!("{base}.pyi")]
            .into_iter()
            .find(|c| self.repo.contains(c))
    }

    fn init_file(&self, dir: &str) -> Option<String> {
        ["__init__.py", "__init__.pyi"]
            .iter()
            .map(|f| join_rel(dir, f))
            .find(|c| self.repo.contains(c))
    }

    /// Resolve dotted module `dotted` (may be empty: the package `base` itself) under `base`.
    fn lookup(&self, base: &str, dotted: &str, from_names: &[&str]) -> Option<String> {
        let dir = if dotted.is_empty() {
            base.to_string()
        } else {
            join_rel(base, &dotted.replace('.', "/"))
        };
        if self.is_dir(&dir) {
            for name in from_names {
                if *name == "*" || name.contains('.') {
                    continue;
                }
                let sub = join_rel(&dir, name);
                if let Some(f) = self.init_file(&sub) {
                    return Some(f);
                }
                if let Some(f) = self.module_file(&sub) {
                    return Some(f);
                }
            }
            if let Some(f) = self.init_file(&dir) {
                return Some(f);
            }
        }
        if !dotted.is_empty() {
            return self.module_file(&dir);
        }
        None
    }

    /// Roots to try for an absolute import from `from_rel`: ancestors of the file (nearest
    /// first), then the rest in discovery order, then the file's own directory.
    fn roots_for<'s>(&'s self, from_rel: &'s str) -> Vec<&'s str> {
        let mut ancestors: Vec<&str> = self
            .roots
            .iter()
            .map(String::as_str)
            .filter(|r| is_ancestor(r, from_rel))
            .collect();
        ancestors.sort_by_key(|r| std::cmp::Reverse(r.len()));
        let mut out = ancestors.clone();
        out.extend(
            self.roots
                .iter()
                .map(String::as_str)
                .filter(|r| !ancestors.contains(r)),
        );
        let own = parent_dir(from_rel);
        if !out.contains(&own) {
            out.push(own);
        }
        out
    }
}

impl Resolver for PythonResolver {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution {
        let spec = import.specifier.as_str();
        // `import a.b` carries the module itself as its only name; `from m import x` carries
        // the imported names, which may be submodules.
        let plain = import.names.len() == 1 && import.names[0].name == spec;
        let names: Vec<&str> = if plain {
            Vec::new()
        } else {
            import.names.iter().map(|n| n.name.as_str()).collect()
        };

        if spec.starts_with('.') {
            let dots = spec.chars().take_while(|c| *c == '.').count();
            let rest = &spec[dots..];
            let mut base = parent_dir(from_rel).to_string();
            for _ in 1..dots {
                if base.is_empty() {
                    return Resolution::Unresolved;
                }
                base = parent_dir(&base).to_string();
            }
            return match self.lookup(&base, rest, &names) {
                Some(f) => Resolution::File(f),
                None => Resolution::Unresolved,
            };
        }

        let top = spec.split('.').next().unwrap_or(spec);
        if top.is_empty() {
            return Resolution::Unresolved;
        }
        let roots = self.roots_for(from_rel);
        for root in &roots {
            if let Some(f) = self.lookup(root, spec, &names) {
                return Resolution::File(f);
            }
        }
        let provided = roots.iter().any(|root| {
            let base = join_rel(root, top);
            self.dirs.contains(&base) || self.module_file(&base).is_some()
        });
        if provided {
            Resolution::Unresolved
        } else {
            Resolution::External(top.to_string())
        }
    }
}

fn has_init(repo: &RepoFiles, dir: &str) -> bool {
    repo.contains(&join_rel(dir, "__init__.py")) || repo.contains(&join_rel(dir, "__init__.pyi"))
}

fn is_ancestor(dir: &str, file: &str) -> bool {
    dir.is_empty() || file.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// Package roots named by one `pyproject.toml` (repo-relative).
fn pyproject_roots(repo: &RepoFiles, rel: &str, dirs: &HashSet<String>) -> Vec<String> {
    let dir = parent_dir(rel);
    let mut out = Vec::new();
    let src = join_rel(dir, "src");
    if dirs.contains(&src) {
        out.push(src);
    }
    let Ok(text) = std::fs::read_to_string(repo.abs(rel)) else {
        return out;
    };
    for entry in toml_lite::parse(&text) {
        match (entry.table.as_str(), entry.key.as_str()) {
            ("tool.poetry", "packages") => {
                for item in entry.value.items() {
                    if let Some(from) = item.get("from").and_then(|v| v.as_str()) {
                        out.push(join_rel(dir, from));
                    }
                }
            }
            ("tool.setuptools.package-dir", _) => {
                if let Some(v) = entry.value.as_str() {
                    out.push(join_rel(dir, v));
                }
            }
            ("tool.setuptools", "package-dir") => {
                for (_, v) in entry.value.pairs() {
                    if let Some(v) = v.as_str() {
                        out.push(join_rel(dir, v));
                    }
                }
            }
            ("tool.setuptools.packages.find", "where") => {
                for item in entry.value.items() {
                    if let Some(v) = item.as_str() {
                        out.push(join_rel(dir, v));
                    }
                }
            }
            _ => {}
        }
    }
    out
}
