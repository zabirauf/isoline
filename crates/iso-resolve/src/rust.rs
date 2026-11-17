//! Rust import resolution. Owned by workstream D (see CONTRACTS.md).
//!
//! Crates: every `Cargo.toml` in the repo with a `[package]` is a crate (this covers
//! `[workspace] members`, globs included, and also crates outside any workspace). Its lib
//! name is `[lib] name` or the package name with `-` → `_`. Crate roots: `[lib] path` or
//! `src/lib.rs`, `src/main.rs`, `[[bin]] path`, `src/bin/*.rs`, and the files directly in
//! `tests/`, `examples/` and `benches/`, plus `build.rs`.
//!
//! Module tree: a file's module path comes from its place under its crate root's directory
//! (`src/a/b.rs` and `src/a/b/mod.rs` are `a::b`). `mod x;` in `src/lib.rs` or a `mod.rs`
//! looks in the same directory, in `src/a.rs` it looks in `src/a/`. Specifiers on `mod`
//! facts may carry the enclosing inline modules (`outer::x`), see the extractor.
//!
//! `use` paths: `crate::`, `self::`, `super::` and a first segment naming a child module are
//! local; a first segment naming a crate in the repo (or a dependency renamed with
//! `package = …`) goes to that crate's lib root. The result is the file of the deepest module
//! on the path that exists (`crate::a::b::Item` → `a/b.rs`, else `a.rs`, else the root).
//! `std`, `core`, `alloc` and unknown crates are `External(crate name)`.

use std::collections::HashMap;

use iso_core::{ImportFact, ImportKind};

use crate::{RepoFiles, Resolution, Resolver, join_rel, parent_dir};

#[derive(Clone, Debug)]
struct Crate {
    /// Directory holding `Cargo.toml` ("" for the repo root).
    dir: String,
    /// Name used in paths (`my_crate`).
    name: String,
    /// Library root file, if any.
    lib: Option<String>,
    /// Every crate root file (lib first).
    roots: Vec<String>,
    /// Dependency key → crate name, for `foo = { package = "bar" }`.
    renames: HashMap<String, String>,
}

pub struct RustResolver {
    pub repo: RepoFiles,
    crates: Vec<Crate>,
}

const BUILTIN: &[&str] = &["std", "core", "alloc", "proc_macro", "test"];

impl RustResolver {
    pub fn new(repo: &RepoFiles) -> Self {
        let mut manifests: Vec<&String> = repo
            .files
            .iter()
            .filter(|f| f.rsplit('/').next() == Some("Cargo.toml"))
            .collect();
        manifests.sort();
        let crates = manifests
            .into_iter()
            .filter_map(|m| read_crate(repo, m))
            .collect();
        RustResolver {
            repo: repo.clone(),
            crates,
        }
    }

    /// The crate whose directory is the deepest ancestor of `file`.
    fn crate_of(&self, file: &str) -> Option<&Crate> {
        self.crates
            .iter()
            .filter(|c| is_ancestor(&c.dir, file))
            .max_by_key(|c| c.dir.len())
    }

    /// The crate root `file` belongs to: itself when it is a root, else the root whose
    /// directory is its deepest ancestor (lib wins ties).
    fn root_of<'c>(&self, krate: &'c Crate, file: &str) -> Option<&'c str> {
        if let Some(r) = krate.roots.iter().find(|r| *r == file) {
            return Some(r);
        }
        let mut best: Option<&str> = None;
        for r in &krate.roots {
            let d = parent_dir(r);
            if is_ancestor(d, file) && best.is_none_or(|b| parent_dir(b).len() < d.len()) {
                best = Some(r);
            }
        }
        best
    }

    /// Module path of `file` under `root` (`[]` for the root itself).
    fn module_path(root: &str, file: &str) -> Vec<String> {
        if root == file {
            return Vec::new();
        }
        let dir = parent_dir(root);
        let rel = if dir.is_empty() {
            file
        } else {
            file.strip_prefix(dir)
                .map(|r| r.trim_start_matches('/'))
                .unwrap_or(file)
        };
        let rel = rel.strip_suffix(".rs").unwrap_or(rel);
        let mut segs: Vec<String> = rel.split('/').map(str::to_string).collect();
        if segs.last().is_some_and(|s| s == "mod") {
            segs.pop();
        }
        segs
    }

    /// The file defining module `path` under `root`, if it exists.
    fn module_file(&self, root: &str, path: &[String]) -> Option<String> {
        if path.is_empty() {
            return Some(root.to_string());
        }
        let base = join_rel(parent_dir(root), &path.join("/"));
        let a = format!("{base}.rs");
        if self.repo.contains(&a) {
            return Some(a);
        }
        let b = format!("{base}/mod.rs");
        if self.repo.contains(&b) {
            return Some(b);
        }
        None
    }

    /// Deepest existing module on `start + rest`.
    fn deepest(&self, root: &str, start: &[String], rest: &[&str]) -> Option<String> {
        for k in (0..=rest.len()).rev() {
            let mut path = start.to_vec();
            path.extend(rest[..k].iter().map(|s| s.to_string()));
            if let Some(f) = self.module_file(root, &path) {
                return Some(f);
            }
        }
        None
    }

    fn resolve_mod(&self, from_rel: &str, spec: &str) -> Resolution {
        let name = from_rel.rsplit('/').next().unwrap_or(from_rel);
        let is_root = self
            .crate_of(from_rel)
            .is_some_and(|c| c.roots.iter().any(|r| r == from_rel));
        let dir = if is_root || name == "mod.rs" {
            parent_dir(from_rel).to_string()
        } else {
            from_rel.strip_suffix(".rs").unwrap_or(from_rel).to_string()
        };
        let rel = spec
            .split("::")
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("/");
        let base = join_rel(&dir, &rel);
        for cand in [format!("{base}.rs"), format!("{base}/mod.rs")] {
            if self.repo.contains(&cand) {
                return Resolution::File(cand);
            }
        }
        Resolution::Unresolved
    }

    /// A crate named `name` as seen from `from_rel` (nearest by shared path prefix).
    fn crate_named(&self, from_rel: &str, name: &str) -> Option<&Crate> {
        let wanted = self
            .crate_of(from_rel)
            .and_then(|c| c.renames.get(name))
            .map(String::as_str)
            .unwrap_or(name);
        self.crates
            .iter()
            .filter(|c| c.name == wanted && c.lib.is_some())
            .max_by_key(|c| common_prefix(&c.dir, from_rel))
    }
}

impl Resolver for RustResolver {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution {
        let spec = import.specifier.trim();
        if import.kind == ImportKind::Mod {
            return self.resolve_mod(from_rel, spec);
        }
        let spec = spec.strip_prefix("::").unwrap_or(spec);
        let segs: Vec<&str> = spec
            .split("::")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let Some(first) = segs.first().copied() else {
            return Resolution::Unresolved;
        };
        let here = self
            .crate_of(from_rel)
            .and_then(|c| self.root_of(c, from_rel).map(|r| (c, r)));

        match first {
            "crate" | "$crate" | "self" | "super" => {
                let Some((_, root)) = here else {
                    return Resolution::Unresolved;
                };
                let current = Self::module_path(root, from_rel);
                let (start, rest): (Vec<String>, &[&str]) = match first {
                    "crate" | "$crate" => (Vec::new(), &segs[1..]),
                    "self" => (current, &segs[1..]),
                    _ => {
                        let ups = segs.iter().take_while(|s| **s == "super").count();
                        if ups > current.len() {
                            return Resolution::Unresolved;
                        }
                        (current[..current.len() - ups].to_vec(), &segs[ups..])
                    }
                };
                match self.deepest(root, &start, rest) {
                    Some(f) => Resolution::File(f),
                    None => Resolution::Unresolved,
                }
            }
            _ if BUILTIN.contains(&first) => Resolution::External(first.to_string()),
            _ => {
                // A child module of the current module (2018 uniform paths).
                if let Some((_, root)) = here {
                    let mut current = Self::module_path(root, from_rel);
                    current.push(first.to_string());
                    if self.module_file(root, &current).is_some() {
                        current.pop();
                        if let Some(f) = self.deepest(root, &current, &segs) {
                            return Resolution::File(f);
                        }
                    }
                }
                if let Some(c) = self.crate_named(from_rel, first)
                    && let Some(lib) = &c.lib
                    && let Some(f) = self.deepest(lib, &[], &segs[1..])
                {
                    return Resolution::File(f);
                }
                Resolution::External(first.to_string())
            }
        }
    }
}

fn is_ancestor(dir: &str, file: &str) -> bool {
    dir.is_empty() || file.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y)
        .count()
}

fn read_crate(repo: &RepoFiles, manifest: &str) -> Option<Crate> {
    let text = std::fs::read_to_string(repo.abs(manifest)).ok()?;
    let dir = parent_dir(manifest).to_string();
    let mut package = None;
    let mut lib_name = None;
    let mut lib_path = None;
    let mut bins = Vec::new();
    let mut renames = HashMap::new();
    for e in toml_lite::parse(&text) {
        match (e.table.as_str(), e.key.as_str()) {
            ("package", "name") => package = e.value.as_str().map(str::to_string),
            ("lib", "name") => lib_name = e.value.as_str().map(str::to_string),
            ("lib", "path") => lib_path = e.value.as_str().map(str::to_string),
            ("bin", "path") => bins.extend(e.value.as_str().map(str::to_string)),
            (t, k) if t.ends_with("dependencies") => {
                if let Some(p) = e.value.get("package").and_then(|v| v.as_str()) {
                    renames.insert(k.replace('-', "_"), p.replace('-', "_"));
                }
            }
            _ => {}
        }
    }
    let package = package?;
    let name = lib_name.unwrap_or(package).replace('-', "_");
    let in_dir = |p: &str| join_rel(&dir, p);
    let lib =
        Some(in_dir(lib_path.as_deref().unwrap_or("src/lib.rs"))).filter(|p| repo.contains(p));
    let mut roots: Vec<String> = lib.iter().cloned().collect();
    let mut push = |p: String| {
        if repo.contains(&p) && !roots.contains(&p) {
            roots.push(p);
        }
    };
    push(in_dir("src/main.rs"));
    for b in bins {
        push(in_dir(&b));
    }
    push(in_dir("build.rs"));
    let mut extra: Vec<&String> = repo
        .files
        .iter()
        .filter(|f| {
            f.ends_with(".rs")
                && ["src/bin", "tests", "examples", "benches"]
                    .iter()
                    .any(|d| parent_dir(f) == in_dir(d))
        })
        .collect();
    extra.sort();
    for f in extra {
        push(f.clone());
    }
    Some(Crate {
        dir,
        name,
        lib,
        roots,
        renames,
    })
}

/// A tiny TOML reader: enough for `Cargo.toml` and `pyproject.toml` lookups (tables, array
/// tables, strings, arrays, inline tables). Malformed input never panics; unknown values are
/// kept as raw text.
pub(crate) mod toml_lite {
    #[derive(Clone, Debug, PartialEq)]
    pub enum Value {
        Str(String),
        Array(Vec<Value>),
        Table(Vec<(String, Value)>),
        Other(String),
    }

    impl Value {
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Value::Str(s) => Some(s),
                _ => None,
            }
        }
        pub fn items(&self) -> &[Value] {
            match self {
                Value::Array(v) => v,
                _ => &[],
            }
        }
        pub fn pairs(&self) -> &[(String, Value)] {
            match self {
                Value::Table(v) => v,
                _ => &[],
            }
        }
        pub fn get(&self, key: &str) -> Option<&Value> {
            self.pairs().iter().find(|(k, _)| k == key).map(|(_, v)| v)
        }
    }

    /// One `key = value` line, with the table header it sits under (`""` at the top).
    #[derive(Clone, Debug, PartialEq)]
    pub struct Entry {
        pub table: String,
        pub key: String,
        pub value: Value,
    }

    pub fn parse(text: &str) -> Vec<Entry> {
        let mut p = P {
            s: text.as_bytes(),
            i: 0,
            text,
        };
        let mut table = String::new();
        let mut out = Vec::new();
        loop {
            p.skip_ws(true);
            let Some(c) = p.peek() else { break };
            if c == b'[' {
                let double = p.s.get(p.i + 1) == Some(&b'[');
                p.i += if double { 2 } else { 1 };
                let start = p.i;
                while p.peek().is_some_and(|c| c != b']' && c != b'\n') {
                    p.i += 1;
                }
                table = normalize_key(&text[start..p.i]);
                p.skip_line();
                continue;
            }
            let start = p.i;
            while p.peek().is_some_and(|c| c != b'=' && c != b'\n') {
                p.i += 1;
            }
            if p.peek() != Some(b'=') {
                p.skip_line();
                continue;
            }
            let key = normalize_key(&text[start..p.i]);
            p.i += 1;
            let value = p.value(0);
            out.push(Entry {
                table: table.clone(),
                key,
                value,
            });
            p.skip_line();
        }
        out
    }

    /// `"a" . b` → `a.b`.
    fn normalize_key(k: &str) -> String {
        k.split('.')
            .map(|s| s.trim().trim_matches(['"', '\'']))
            .collect::<Vec<_>>()
            .join(".")
    }

    struct P<'a> {
        s: &'a [u8],
        i: usize,
        text: &'a str,
    }

    impl P<'_> {
        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }
        /// Skips spaces and comments; newlines too when `newlines`.
        fn skip_ws(&mut self, newlines: bool) {
            while let Some(c) = self.peek() {
                match c {
                    b' ' | b'\t' | b'\r' => self.i += 1,
                    b'\n' if newlines => self.i += 1,
                    b'#' => {
                        while self.peek().is_some_and(|c| c != b'\n') {
                            self.i += 1;
                        }
                    }
                    _ => break,
                }
            }
        }
        fn skip_line(&mut self) {
            while self.peek().is_some_and(|c| c != b'\n') {
                self.i += 1;
            }
        }
        fn value(&mut self, depth: usize) -> Value {
            self.skip_ws(depth > 0);
            match self.peek() {
                Some(b'"') | Some(b'\'') => Value::Str(self.string()),
                Some(b'[') if depth < 32 => {
                    self.i += 1;
                    let mut items = Vec::new();
                    loop {
                        self.skip_ws(true);
                        match self.peek() {
                            None => break,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            Some(b',') => self.i += 1,
                            _ => {
                                let before = self.i;
                                items.push(self.value(depth + 1));
                                if self.i == before {
                                    self.i += 1;
                                }
                            }
                        }
                    }
                    Value::Array(items)
                }
                Some(b'{') if depth < 32 => {
                    self.i += 1;
                    let mut pairs = Vec::new();
                    loop {
                        self.skip_ws(true);
                        match self.peek() {
                            None => break,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            Some(b',') => self.i += 1,
                            _ => {
                                let start = self.i;
                                while self
                                    .peek()
                                    .is_some_and(|c| !matches!(c, b'=' | b'}' | b'\n'))
                                {
                                    self.i += 1;
                                }
                                if self.peek() != Some(b'=') {
                                    continue;
                                }
                                let key = normalize_key(&self.text[start..self.i]);
                                self.i += 1;
                                let v = self.value(depth + 1);
                                pairs.push((key, v));
                            }
                        }
                    }
                    Value::Table(pairs)
                }
                _ => {
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| !matches!(c, b'\n' | b',' | b']' | b'}' | b'#'))
                    {
                        self.i += 1;
                    }
                    Value::Other(self.text[start..self.i].trim().to_string())
                }
            }
        }
        fn string(&mut self) -> String {
            let q = self.s[self.i];
            let triple = self.s.get(self.i..self.i + 3) == Some(&[q, q, q][..]);
            self.i += if triple { 3 } else { 1 };
            let mut out = Vec::new();
            while let Some(c) = self.peek() {
                if triple {
                    if self.s.get(self.i..self.i + 3) == Some(&[q, q, q][..]) {
                        self.i += 3;
                        break;
                    }
                } else if c == q {
                    self.i += 1;
                    break;
                } else if c == b'\n' {
                    break;
                }
                if c == b'\\'
                    && q == b'"'
                    && let Some(n) = self.s.get(self.i + 1)
                {
                    out.push(match n {
                        b'n' => b'\n',
                        b't' => b'\t',
                        other => *other,
                    });
                    self.i += 2;
                    continue;
                }
                out.push(c);
                self.i += 1;
            }
            String::from_utf8_lossy(&out).into_owned()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn reads_cargo_and_pyproject_shapes() {
            let t = r#"
# comment
[package]
name = "my-crate" # trailing
version = "0.1.0"

[lib]
path = 'src/my.rs'

[[bin]]
name = "x"
path = "src/x.rs"

[dependencies]
foo = { package = "bar-baz", path = "../bar" }
serde = "1"

[workspace]
members = [
  "crates/*",
  "tools/a", # c
]

[tool.poetry]
packages = [{ include = "pkg", from = "src" }]

[tool.setuptools.package-dir]
"" = "lib"
"#;
            let e = parse(t);
            let get = |table: &str, key: &str| {
                e.iter()
                    .find(|x| x.table == table && x.key == key)
                    .map(|x| x.value.clone())
            };
            assert_eq!(get("package", "name"), Some(Value::Str("my-crate".into())));
            assert_eq!(get("lib", "path"), Some(Value::Str("src/my.rs".into())));
            assert_eq!(get("bin", "path"), Some(Value::Str("src/x.rs".into())));
            let foo = get("dependencies", "foo").unwrap();
            assert_eq!(foo.get("package").and_then(|v| v.as_str()), Some("bar-baz"));
            let members = get("workspace", "members").unwrap();
            let m: Vec<_> = members.items().iter().filter_map(|v| v.as_str()).collect();
            assert_eq!(m, ["crates/*", "tools/a"]);
            let pk = get("tool.poetry", "packages").unwrap();
            assert_eq!(
                pk.items()[0].get("from").and_then(|v| v.as_str()),
                Some("src")
            );
            assert_eq!(
                get("tool.setuptools.package-dir", ""),
                Some(Value::Str("lib".into()))
            );
            // Garbage never panics.
            parse("[[[\n= = =\n\"unterminated\nx = [1, {a = \n");
        }
    }
}
