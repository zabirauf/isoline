//! Python resolver against `fixtures/python/packages` plus a temp repo for pyproject-only
//! layouts.

use iso_core::ImportKind;
use iso_resolve::Resolution;
use iso_resolve::Resolver;
use iso_resolve::python::PythonResolver;

#[path = "rust_test_support.rs"]
mod support;
use support::{check_resolutions, fixture, import, repo_at};

#[test]
fn python_packages_fixture() {
    let root = fixture("python", "packages");
    let r = PythonResolver::new(&repo_at(&root));
    assert!(check_resolutions(&root, &r) >= 15);
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
    let r = PythonResolver::new(&repo_at(root));
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
