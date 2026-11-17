//! Python resolver against `fixtures/python/packages` (expected/resolutions.json is written
//! by hand) plus a temp repo for pyproject-only layouts.

use std::path::{Path, PathBuf};

use iso_core::{ImportFact, ImportKind, ImportedName};
use iso_resolve::python::PythonResolver;
use iso_resolve::{RepoFiles, Resolution, Resolver};

fn repo_at(root: &Path) -> RepoFiles {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).expect("dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "expected") {
                    continue;
                }
                walk(&p, root, out);
            } else {
                let rel = p.strip_prefix(root).expect("inside");
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    RepoFiles::new(root, files)
}

fn import(spec: &str, names: &[&str], kind: ImportKind) -> ImportFact {
    ImportFact {
        specifier: spec.to_string(),
        kind,
        names: names
            .iter()
            .map(|n| ImportedName {
                name: n.to_string(),
                alias: None,
                type_only: false,
            })
            .collect(),
        type_only: false,
        line: 1,
    }
}

fn expected(v: &serde_json::Value) -> Resolution {
    if v == "unresolved" {
        return Resolution::Unresolved;
    }
    if let Some(f) = v.get("file").and_then(|f| f.as_str()) {
        return Resolution::File(f.to_string());
    }
    if let Some(m) = v.get("external").and_then(|f| f.as_str()) {
        return Resolution::External(m.to_string());
    }
    panic!("bad expectation {v}");
}

fn fixture(lang: &str, case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(lang)
        .join(case)
}

#[test]
fn python_packages_fixture() {
    let root = fixture("python", "packages");
    let repo = repo_at(&root);
    let r = PythonResolver::new(&repo);
    let cases: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("expected/resolutions.json")).expect("golden"),
    )
    .expect("json");
    let mut failures = Vec::new();
    for c in cases.as_array().expect("array") {
        let from = c["from"].as_str().expect("from");
        let spec = c["specifier"].as_str().expect("specifier");
        let names: Vec<&str> = c["names"]
            .as_array()
            .map(|a| a.iter().filter_map(|n| n.as_str()).collect())
            .unwrap_or_default();
        let got = r.resolve(from, &import(spec, &names, ImportKind::Static));
        let want = expected(&c["expect"]);
        if got != want {
            failures.push(format!(
                "{from}: {spec} {names:?}: got {got:?}, want {want:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn python_pyproject_layouts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, text).expect("write");
    };
    // Poetry project in a subdirectory with a namespace package (no __init__.py).
    write(
        "svc/pyproject.toml",
        "[tool.poetry]\nname = \"svc\"\npackages = [\n  { include = \"ns\", from = \"lib\" },\n]\n",
    );
    write("svc/lib/ns/api.py", "def f(): pass\n");
    write("svc/lib/ns/deep/x.py", "");
    write("svc/main.py", "from ns import api\n");
    // setuptools packages.find
    write(
        "other/pyproject.toml",
        "[tool.setuptools.packages.find]\nwhere = [\"code\"]\n",
    );
    write("other/code/thing.py", "");
    let repo = repo_at(root);
    let r = PythonResolver::new(&repo);
    let s = ImportKind::Static;
    assert_eq!(
        r.resolve("svc/main.py", &import("ns", &["api"], s)),
        Resolution::File("svc/lib/ns/api.py".into())
    );
    assert_eq!(
        r.resolve("svc/main.py", &import("ns.deep.x", &["ns.deep.x"], s)),
        Resolution::File("svc/lib/ns/deep/x.py".into())
    );
    assert_eq!(
        r.resolve("other/run.py", &import("thing", &["thing"], s)),
        Resolution::File("other/code/thing.py".into())
    );
    assert_eq!(
        r.resolve("svc/main.py", &import("ns.nope", &["ns.nope"], s)),
        Resolution::Unresolved
    );
    assert_eq!(
        r.resolve("svc/main.py", &import("json", &["json"], s)),
        Resolution::External("json".into())
    );
}
