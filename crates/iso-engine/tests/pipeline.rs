//! End-to-end acceptance tests for workstreams A and M, on a small TypeScript repo.

use std::path::Path;
use std::sync::Arc;

use iso_engine::Engine;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn bump_mtime(root: &Path, rel: &str) {
    // Some file systems have coarse mtimes; make sure an edit is visible to the size+mtime check.
    let p = root.join(rel);
    let f = std::fs::File::options().write(true).open(&p).unwrap();
    let t = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    f.set_modified(t).unwrap();
}

fn repo() -> (tempfile::TempDir, tempfile::TempDir) {
    let src = tempfile::tempdir().unwrap();
    let r = src.path();
    write(
        r,
        "src/util.ts",
        "export function helper(x: number): number {\n  return x + 1;\n}\n\nexport function other(): void {}\n",
    );
    write(
        r,
        "src/api.ts",
        "import { helper } from \"./util\";\n\nexport function handle(n: number) {\n  return helper(n) * 2;\n}\n",
    );
    write(
        r,
        "src/main.ts",
        "import { handle } from \"./api\";\n\nexport function main() {\n  console.log(handle(1));\n}\n",
    );
    write(r, "src/lone.ts", "export const x = 1;\n");
    write(r, "README.md", "# demo\n");
    write(r, "notes.zzz", "unknown\n");
    (src, tempfile::tempdir().unwrap())
}

fn open(src: &Path, data: &Path) -> Arc<Engine> {
    Engine::open_with_data(src, data).unwrap()
}

#[test]
fn reindex_unchanged_hashes_and_writes_nothing() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    let a = e.index().unwrap();
    assert_eq!(a.files, 6);
    assert_eq!(a.hashed, 6);
    drop(e);
    // Reopen: a new engine, as after an app restart.
    let e = open(src.path(), data.path());
    let b = e.index().unwrap();
    assert_eq!(b.hashed, 0, "{b:?}");
    assert_eq!(b.parsed, 0);
    assert_eq!(b.facts_changed, 0);
    assert_eq!(b.edge_files, 0);
    assert_eq!(b.unchanged, 6);
    assert!(!b.reclustered);
    assert_eq!(a.root_hash, b.root_hash);
}

#[test]
fn branch_switch_touching_three_files_reports_exactly_those() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    e.index().unwrap();
    for f in ["src/util.ts", "README.md", "notes.zzz"] {
        let p = src.path().join(f);
        let t = std::fs::read_to_string(&p).unwrap();
        std::fs::write(&p, format!("{t}\n// changed\n")).unwrap();
        bump_mtime(src.path(), f);
    }
    let r = e.index().unwrap();
    assert_eq!(r.hashed, 3, "{r:?}");
    assert_eq!(r.unchanged, 3);
}

#[test]
fn touch_without_content_change_rehashes_but_does_not_reparse() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    e.index().unwrap();
    bump_mtime(src.path(), "src/api.ts");
    let r = e.index().unwrap();
    assert_eq!(r.hashed, 1);
    assert_eq!(r.parsed, 0);
    assert_eq!(r.facts_changed, 0);
    let r = e.index().unwrap();
    assert_eq!(r.hashed, 0, "mtime was stored after the touch");
}

#[test]
fn body_edit_reparses_one_file_and_recomputes_no_dependents() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    e.index().unwrap();
    write(
        src.path(),
        "src/util.ts",
        "export function helper(x: number): number {\n  const y = x * 3;\n  return y + 1;\n}\n\nexport function other(): void {}\n",
    );
    bump_mtime(src.path(), "src/util.ts");
    let r = e.index_paths(&["src/util.ts".into()]).unwrap();
    assert_eq!(r.parsed, 1, "{r:?}");
    assert_eq!(r.exports_changed, 0, "{r:?}");
    assert_eq!(r.dependents_recomputed, 0, "{r:?}");
}

#[test]
fn export_change_recomputes_only_importers() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    e.index().unwrap();
    write(
        src.path(),
        "src/util.ts",
        "export function helper(x: number, y = 2): number {\n  return x + y;\n}\n\nexport function other(): void {}\nexport function added() {}\n",
    );
    bump_mtime(src.path(), "src/util.ts");
    let r = e.index_paths(&["src/util.ts".into()]).unwrap();
    assert_eq!(r.exports_changed, 1, "{r:?}");
    // api.ts imports util.ts; main.ts and lone.ts do not.
    assert_eq!(r.dependents_recomputed, 1, "{r:?}");
}

#[test]
fn deleted_and_added_files_are_tracked() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    e.index().unwrap();
    std::fs::remove_file(src.path().join("src/lone.ts")).unwrap();
    write(src.path(), "src/new.ts", "export function fresh() {}\n");
    let r = e.index().unwrap();
    assert_eq!(r.deleted, 1);
    assert_eq!(r.added, 1);
    assert_eq!(r.files, 6);
    let tree = e.tree().unwrap();
    let src_dir = tree.children.iter().find(|c| c.name == "src").unwrap();
    let names: Vec<&str> = src_dir.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["api.ts", "main.ts", "new.ts", "util.ts"]);
}

#[test]
fn text_tier_and_unknown_files_open_without_errors() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    e.index().unwrap();
    let v = e.file_view("notes.zzz").unwrap();
    assert_eq!(v.tier, iso_core::Tier::Text);
    assert!(v.functions.is_empty());
    let s = e.source("notes.zzz", None, None).unwrap();
    assert_eq!(s.text, "unknown");
    assert_eq!(s.monaco_lang, "plaintext");
}

#[test]
fn cross_file_view_and_impact() {
    let (src, data) = repo();
    let e = open(src.path(), data.path());
    e.index().unwrap();
    let v = e.file_view("src/api.ts").unwrap();
    let handle = v
        .functions
        .iter()
        .find(|f| f.name == "handle")
        .expect("handle card");
    assert!(handle.exported);
    assert_eq!(handle.uses, vec!["helper"]);
    assert_eq!(handle.external_callers, 1, "main() calls handle()");
    assert_eq!(v.modules[0].resolved.as_deref(), Some("src/util.ts"));
    assert_eq!(v.importers.len(), 1);
    assert_eq!(v.importers[0].path, "src/main.ts");
    let imp = e.impact(handle.id, true).unwrap();
    assert_eq!(
        imp.dependents
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        vec!["main"]
    );
    assert_eq!(
        imp.dependencies
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        vec!["helper"]
    );
    let hits = e.search("hand", 10).unwrap();
    assert_eq!(hits[0].name, "handle");
}

#[test]
fn precise_index_corrects_a_wrong_heuristic_edge() {
    use protobuf::Message;
    use scip::types::{Document, Index, Occurrence, SymbolRole};
    let src = tempfile::tempdir().unwrap();
    let r = src.path();
    // `thing` is a default import; the name heuristic matches the unrelated export `thing`.
    write(
        r,
        "src/b.ts",
        "export default function create() {\n  return 1;\n}\n\nexport function thing() {\n  return 2;\n}\n",
    );
    write(
        r,
        "src/a.ts",
        "import thing from \"./b\";\n\nexport function main() {\n  return thing();\n}\n",
    );
    let data = tempfile::tempdir().unwrap();
    let e = open(r, data.path());
    e.index().unwrap();
    let main = e
        .file_view("src/a.ts")
        .unwrap()
        .functions
        .into_iter()
        .find(|f| f.name == "main")
        .unwrap();
    let before = e.impact(main.id, false).unwrap();
    assert_eq!(
        before.dependencies[0].name, "thing",
        "heuristic edge goes to the wrong export"
    );
    assert_eq!(
        before.dependencies[0].confidence,
        iso_core::Confidence::Heuristic
    );

    let sym = "scip-typescript npm demo 1.0.0 src/`b.ts`/create().";
    let occ = |line: i32, def: bool| {
        let mut o = Occurrence::new();
        o.range = vec![line, 9, 15];
        o.symbol = sym.into();
        o.symbol_roles = if def {
            SymbolRole::Definition as i32
        } else {
            0
        };
        o
    };
    let mut a = Document::new();
    a.relative_path = "src/a.ts".into();
    a.occurrences = vec![occ(3, false)];
    let mut b = Document::new();
    b.relative_path = "src/b.ts".into();
    b.occurrences = vec![occ(0, true)];
    let mut idx = Index::new();
    idx.documents = vec![a, b];
    let rep = e.import_scip(&idx.write_to_bytes().unwrap()).unwrap();
    assert_eq!(rep.corrected, 1, "{rep:?}");
    let after = e.impact(main.id, false).unwrap();
    assert_eq!(after.dependencies[0].name, "create");
    assert_eq!(
        after.dependencies[0].confidence,
        iso_core::Confidence::Exact
    );
}

/// Regression (found by `iso-cli history`): adding a Rust integration test together with
/// `tests/common/mod.rs` must resolve `mod common;` exactly as a cold index does, because
/// files under `tests/` are crate roots the resolver precomputes.
#[test]
fn adding_rust_crate_roots_matches_a_cold_index() {
    let src = tempfile::tempdir().unwrap();
    let r = src.path();
    write(r, "Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n");
    write(r, "src/lib.rs", "pub fn f() {}\n");
    let data = tempfile::tempdir().unwrap();
    let e = open(r, data.path());
    e.index().unwrap();
    write(r, "tests/common/mod.rs", "pub fn setup() {}\n");
    write(r, "tests/it.rs", "mod common;\n\n#[test]\nfn t() {\n    common::setup();\n}\n");
    let rep = e.index().unwrap();
    assert_eq!(rep.added, 2);
    let view = e.file_view("tests/it.rs").unwrap();
    let m = view.modules.iter().find(|m| m.id == "common").expect("mod common");
    assert_eq!(m.resolved.as_deref(), Some("tests/common/mod.rs"));
}
