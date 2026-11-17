//! JS/TS resolution against `fixtures/ts/monorepo`: tsconfig `paths` (own and inherited
//! through `extends`), project references, `index.ts` barrels, ESM `.js` → `.ts`, workspace
//! packages with and without `node_modules`, builtins and externals.
//!
//! `expected/resolve.json` is hand-written. iso-resolve never parses code, so each entry
//! names its import directly; `golden_imports_exist_in_sources` keeps the table honest.

use std::path::{Path, PathBuf};

use iso_core::{ImportFact, ImportKind};
use iso_resolve::{RepoFiles, Resolution, Resolver, js::JsResolver};
use serde_json::Value;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts/monorepo")
}

/// Every file under `root` except `expected/` and `node_modules/`, repo-relative.
fn walk(root: &Path) -> Vec<String> {
    fn go(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "expected" || rel.ends_with("node_modules") {
                continue;
            }
            if p.is_dir() {
                go(root, &p, out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    go(root, root, &mut out);
    out
}

fn golden() -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(fixture().join("expected/resolve.json")).unwrap())
        .unwrap()
}

fn import(e: &Value) -> ImportFact {
    let kind = match e["kind"].as_str().unwrap_or("static") {
        "dynamic" => ImportKind::Dynamic,
        "require" => ImportKind::Require,
        "reexport" => ImportKind::Reexport,
        _ => ImportKind::Static,
    };
    ImportFact {
        specifier: e["specifier"].as_str().unwrap().to_string(),
        kind,
        names: Vec::new(),
        type_only: false,
        line: 1,
    }
}

fn want(e: &Value) -> Resolution {
    if let Some(f) = e["file"].as_str() {
        Resolution::File(f.to_string())
    } else if let Some(x) = e["external"].as_str() {
        Resolution::External(x.to_string())
    } else {
        assert_eq!(e["unresolved"], true, "bad golden entry {e}");
        Resolution::Unresolved
    }
}

fn check_all(root: &Path) {
    let repo = RepoFiles::new(root, walk(root));
    let r = JsResolver::new(&repo);
    let mut bad = Vec::new();
    for e in golden() {
        let from = e["from"].as_str().unwrap();
        let got = r.resolve(from, &import(&e));
        let w = want(&e);
        if got != w {
            bad.push(format!(
                "{from}: {:?} -> {got:?}, want {w:?}",
                e["specifier"].as_str().unwrap()
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} mismatch(es)\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn monorepo_matches_golden() {
    check_all(&fixture());
}

/// With the workspace installed (`node_modules/@acme/*` symlinked to the packages, as npm,
/// pnpm and yarn do), the same imports resolve to the same repo files.
#[cfg(unix)]
#[test]
fn monorepo_with_node_modules_matches_golden() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    copy_dir(&fixture(), &root);
    let scope = root.join("node_modules/@acme");
    std::fs::create_dir_all(&scope).unwrap();
    for (name, dir) in [("core", "packages/core"), ("ui", "packages/ui")] {
        std::os::unix::fs::symlink(root.join(dir), scope.join(name)).unwrap();
    }
    check_all(&root);
}

#[test]
fn golden_imports_exist_in_sources() {
    let root = fixture();
    for e in golden() {
        let from = e["from"].as_str().unwrap();
        let spec = e["specifier"].as_str().unwrap();
        let src = std::fs::read_to_string(root.join(from)).unwrap();
        assert!(
            src.contains(&format!("\"{spec}\"")),
            "{from} does not import {spec:?}"
        );
    }
}

#[test]
fn every_relative_import_in_the_fixture_is_in_the_golden() {
    // Crude scan: quoted specifiers after `from`, `import(` and `require(`.
    let root = fixture();
    let g = golden();
    for rel in walk(&root) {
        if !(rel.ends_with(".ts") || rel.ends_with(".tsx") || rel.ends_with(".cjs")) {
            continue;
        }
        let src = std::fs::read_to_string(root.join(&rel)).unwrap();
        for pat in ["from \"", "import(\"", "require(\""] {
            for (i, _) in src.match_indices(pat) {
                let rest = &src[i + pat.len()..];
                let spec = &rest[..rest.find('"').unwrap()];
                assert!(
                    g.iter()
                        .any(|e| e["from"] == rel.as_str() && e["specifier"] == spec),
                    "{rel}: {spec:?} missing from expected/resolve.json"
                );
            }
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let dest = to.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_dir(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}
