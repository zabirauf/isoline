//! Rust resolver against the two-crate `fixtures/rust/workspace`.

use iso_core::ImportKind;
use iso_resolve::rust::RustResolver;
use iso_resolve::{RepoFiles, Resolution, Resolver};

#[path = "rust_test_support.rs"]
mod support;
use support::{check_resolutions, fixture, import, repo_at};

#[test]
fn rust_workspace_fixture() {
    let root = fixture("rust", "workspace");
    let r = RustResolver::new(&repo_at(&root));
    assert!(check_resolutions(&root, &r) >= 20);
}

#[test]
fn rust_files_outside_crates() {
    let r = RustResolver::new(&RepoFiles::new("/nonexistent", ["scratch.rs".to_string()]));
    let s = ImportKind::Static;
    assert_eq!(
        r.resolve("scratch.rs", &import("crate::a", &["a"], s)),
        Resolution::Unresolved
    );
    assert_eq!(
        r.resolve("scratch.rs", &import("tokio::spawn", &["spawn"], s)),
        Resolution::External("tokio".into())
    );
    assert_eq!(
        r.resolve("scratch.rs", &import("::core::mem", &["mem"], s)),
        Resolution::External("core".into())
    );
}
