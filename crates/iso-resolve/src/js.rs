//! JavaScript and TypeScript import resolution through `oxc_resolver`. Owned by workstream B
//! (see CONTRACTS.md).
//!
//! Order of attempts for one specifier:
//!
//! 1. `oxc_resolver` with tsconfig auto-discovery (nearest `tsconfig.json` that owns the
//!    importing file, its `extends` chain and project references), TypeScript extensions,
//!    the ESM `./x.js` → `./x.ts` mapping, `index` barrels and `node_modules`.
//! 2. Workspace packages: every `package.json` in the repo with a `name` is a package that
//!    resolves without `node_modules` being installed. Its `exports` map (or `main`, or an
//!    `index` file, or `src/index`) picks the file.
//!
//! A hit inside the repo that the walker saw is [`Resolution::File`]. A bare specifier that
//! does not land in the repo is [`Resolution::External`] with its package name. Everything
//! else (relative paths, tsconfig aliases and workspace subpaths that match nothing) is
//! [`Resolution::Unresolved`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iso_core::ImportFact;
use oxc_resolver::{
    EnforceExtension, ResolveError, ResolveOptions, Resolver as Oxc, TsconfigDiscovery,
};
use serde_json::Value;

use crate::{RepoFiles, Resolution, Resolver};

pub struct JsResolver {
    pub repo: RepoFiles,
    /// The root as the file system reports it (symlinks resolved), for mapping results back.
    real_root: PathBuf,
    oxc: Oxc,
    /// Workspace package name → (package directory, repo-relative; parsed package.json).
    packages: HashMap<String, (String, Value)>,
}

/// Extensions tried for extensionless specifiers, TypeScript first.
const EXTENSIONS: &[&str] = &[
    ".ts", ".tsx", ".d.ts", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json",
];

/// `exports` conditions, in the order a source-reading tool prefers them.
const CONDITIONS: &[&str] = &[
    "source", "types", "import", "module", "require", "node", "default",
];

impl JsResolver {
    pub fn new(repo: &RepoFiles) -> Self {
        let real_root = std::fs::canonicalize(&repo.root).unwrap_or_else(|_| repo.root.clone());
        let ext_alias = |from: &str, to: &[&str]| {
            (
                from.to_string(),
                to.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
        };
        let options = ResolveOptions {
            tsconfig: Some(TsconfigDiscovery::Auto),
            extensions: EXTENSIONS.iter().map(|s| s.to_string()).collect(),
            extension_alias: vec![
                ext_alias(".js", &[".ts", ".tsx", ".d.ts", ".js", ".jsx"]),
                ext_alias(".jsx", &[".tsx", ".jsx"]),
                ext_alias(".mjs", &[".mts", ".d.mts", ".mjs"]),
                ext_alias(".cjs", &[".cts", ".d.cts", ".cjs"]),
            ],
            enforce_extension: EnforceExtension::Disabled,
            condition_names: CONDITIONS.iter().map(|s| s.to_string()).collect(),
            main_fields: vec!["module".into(), "main".into()],
            builtin_modules: true,
            ..ResolveOptions::default()
        };
        let mut packages = HashMap::new();
        for rel in &repo.files {
            let Some(dir) = rel.strip_suffix("package.json") else {
                continue;
            };
            if !(dir.is_empty() || dir.ends_with('/')) || rel.contains("node_modules/") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(repo.abs(rel)) else {
                continue;
            };
            let Ok(pkg) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(name) = pkg.get("name").and_then(Value::as_str) {
                let dir = dir.trim_end_matches('/').to_string();
                packages.entry(name.to_string()).or_insert((dir, pkg));
            }
        }
        JsResolver {
            repo: repo.clone(),
            real_root,
            oxc: Oxc::new(options),
            packages,
        }
    }

    /// Repo-relative path of an absolute result, if the walker knows the file.
    fn in_repo(&self, abs: &Path) -> Option<String> {
        let rel = abs
            .strip_prefix(&self.real_root)
            .or_else(|_| abs.strip_prefix(&self.repo.root))
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        (self.repo.contains(&rel) && !rel.split('/').any(|s| s == "node_modules")).then_some(rel)
    }

    fn oxc_file(&self, from_abs: &Path, spec: &str) -> Result<Option<String>, ResolveError> {
        let r = self.oxc.resolve_file(from_abs, spec)?;
        Ok(self.in_repo(r.path()))
    }

    /// Resolve `name/sub` against a workspace package in the repo.
    fn workspace(&self, spec: &str) -> Option<Resolution> {
        let name = package_name(spec);
        let (dir, pkg) = self.packages.get(name)?;
        let sub = &spec[name.len()..]; // "" or "/x/y"
        let key = format!(".{sub}");
        let dir_abs = self.real_root.join(dir);
        let mut targets = Vec::new();
        if let Some(exports) = pkg.get("exports") {
            targets.extend(export_target(exports, &key));
        } else if sub.is_empty() {
            for f in ["source", "types", "module", "main"] {
                if let Some(m) = pkg.get(f).and_then(Value::as_str) {
                    targets.push(dot(m));
                }
            }
            targets.push("./index".into());
            targets.push("./src/index".into());
        } else {
            targets.push(key.clone());
            targets.push(format!("./src{sub}"));
        }
        for t in targets {
            if let Ok(r) = self.oxc.resolve(&dir_abs, &t)
                && let Some(rel) = self.in_repo(r.path())
            {
                return Some(Resolution::File(rel));
            }
        }
        Some(Resolution::Unresolved)
    }
}

impl Resolver for JsResolver {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution {
        let spec = import.specifier.as_str();
        if spec.is_empty() || spec.starts_with("data:") || spec.contains("://") {
            return Resolution::External(spec.to_string());
        }
        let from_abs = self.real_root.join(from_rel);
        let bare = is_bare(spec);
        match self.oxc_file(&from_abs, spec) {
            Ok(Some(rel)) => return Resolution::File(rel),
            Err(ResolveError::Builtin { resolved, .. }) => {
                return Resolution::External(
                    resolved
                        .strip_prefix("node:")
                        .unwrap_or(&resolved)
                        .to_string(),
                );
            }
            // Resolved outside the repo (an installed package) or to a file the walker skipped.
            Ok(None) if !bare => return Resolution::Unresolved,
            _ => {}
        }
        if !bare {
            return Resolution::Unresolved;
        }
        if let Some(r) = self.workspace(spec) {
            return r;
        }
        if looks_like_alias(spec) {
            return Resolution::Unresolved;
        }
        Resolution::External(package_name(spec).to_string())
    }
}

/// `./x`, `../x` and absolute paths are not package names.
fn is_bare(spec: &str) -> bool {
    !(spec.starts_with('.') || spec.starts_with('/'))
}

/// Specifiers that are tsconfig path aliases by convention, never npm packages
/// (`@/x`, `~/x`, `#internal`).
fn looks_like_alias(spec: &str) -> bool {
    spec.starts_with("@/") || spec.starts_with('~') || spec.starts_with('#')
}

/// `@scope/pkg/sub` → `@scope/pkg`, `pkg/sub` → `pkg`.
fn package_name(spec: &str) -> &str {
    let mut it = spec.match_indices('/');
    let cut = if spec.starts_with('@') {
        it.nth(1)
    } else {
        it.next()
    };
    cut.map(|(i, _)| &spec[..i]).unwrap_or(spec)
}

fn dot(p: &str) -> String {
    if p.starts_with("./") || p.starts_with("../") {
        p.to_string()
    } else {
        format!("./{}", p.trim_start_matches('/'))
    }
}

/// Targets an `exports` field gives for one subpath key (`.` or `./x`), best first.
fn export_target(exports: &Value, key: &str) -> Vec<String> {
    let is_subpath_map = exports
        .as_object()
        .is_some_and(|m| m.keys().any(|k| k.starts_with('.')));
    if !is_subpath_map {
        return if key == "." {
            conditions(exports)
        } else {
            Vec::new()
        };
    }
    let map = exports.as_object().unwrap();
    if let Some(v) = map.get(key) {
        return conditions(v);
    }
    // One `*` pattern: "./utils/*": "./src/utils/*.ts".
    for (k, v) in map {
        let Some((pre, post)) = k.split_once('*') else {
            continue;
        };
        if key.len() >= pre.len() + post.len()
            && let Some(rest) = key.strip_prefix(pre)
            && let Some(star) = rest.strip_suffix(post)
        {
            return conditions(v)
                .into_iter()
                .map(|t| t.replace('*', star))
                .collect();
        }
    }
    Vec::new()
}

/// Flatten a conditional export (`string`, `{ condition: … }` or an array of fallbacks).
fn conditions(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![dot(s)],
        Value::Array(a) => a.iter().flat_map(conditions).collect(),
        Value::Object(m) => {
            let mut out = Vec::new();
            for c in CONDITIONS {
                if let Some(x) = m.get(*c) {
                    out.extend(conditions(x));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn package_names() {
        assert_eq!(package_name("react"), "react");
        assert_eq!(package_name("react-dom/client"), "react-dom");
        assert_eq!(package_name("@acme/ui"), "@acme/ui");
        assert_eq!(package_name("@acme/ui/button"), "@acme/ui");
    }

    #[test]
    fn exports_maps() {
        let e = json!({
            ".": { "types": "./dist/index.d.ts", "source": "./src/index.ts" },
            "./card": "./src/card.tsx",
            "./icons/*": { "import": "./src/icons/*.tsx" }
        });
        assert_eq!(
            export_target(&e, "."),
            vec!["./src/index.ts", "./dist/index.d.ts"]
        );
        assert_eq!(export_target(&e, "./card"), vec!["./src/card.tsx"]);
        assert_eq!(
            export_target(&e, "./icons/star"),
            vec!["./src/icons/star.tsx"]
        );
        assert!(export_target(&e, "./nope").is_empty());
        assert_eq!(
            export_target(&json!("lib/main.js"), "."),
            vec!["./lib/main.js"]
        );
    }
}
