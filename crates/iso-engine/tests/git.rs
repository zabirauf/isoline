//! Git through the engine: the tree event, comparisons in the map's terms, and switching
//! branches with the index following along. Skipped when git is not installed.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iso_core::ipc::ChangeKind;
use iso_engine::{Engine, Event};

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// main: lib/math.ts (add, sub), app/main.ts uses both, app/other.ts uses sub.
/// feature: sub deleted, add gains a parameter, a new util module, main imports it.
fn repo(r: &Path) {
    git(r, &["init", "-q", "-b", "main"]);
    write(
        r,
        "lib/math.ts",
        "export function add(a: number, b: number) {\n  return a + b;\n}\n\nexport function sub(a: number, b: number) {\n  return a - b;\n}\n",
    );
    write(
        r,
        "app/main.ts",
        "import { add, sub } from \"../lib/math\";\n\nexport function run() {\n  return add(1, sub(3, 2));\n}\n",
    );
    write(
        r,
        "app/other.ts",
        "import { sub } from \"../lib/math\";\n\nexport function other() {\n  return sub(5, 1);\n}\n",
    );
    git(r, &["add", "."]);
    git(r, &["commit", "-q", "-m", "first"]);
    git(r, &["switch", "-q", "-c", "feature"]);
    write(
        r,
        "lib/math.ts",
        "export function add(a: number, b: number, c = 0) {\n  return a + b + c;\n}\n",
    );
    write(
        r,
        "util/log.ts",
        "export function log(msg: string) {\n  console.log(msg);\n}\n",
    );
    write(
        r,
        "app/main.ts",
        "import { add } from \"../lib/math\";\nimport { log } from \"../util/log\";\n\nexport function run() {\n  log(\"run\");\n  return add(1, 2, 3);\n}\n",
    );
    git(r, &["add", "."]);
    git(r, &["commit", "-q", "-m", "feature work"]);
    git(r, &["switch", "-q", "main"]);
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn compare_switch_and_tree_events() {
    if iso_git::find_exe("git").is_none() {
        return;
    }
    let src = tempfile::tempdir().unwrap();
    let r = src.path();
    repo(r);
    let data = tempfile::tempdir().unwrap();
    let eng = Engine::open_with_data(r, data.path()).unwrap();
    let events: Arc<Mutex<Vec<&'static str>>> = Arc::default();
    let ev = Arc::clone(&events);
    eng.set_sink(Some(Arc::new(move |e: Event| {
        ev.lock().unwrap().push(e.name())
    })));
    eng.index().unwrap();
    {
        let ev = events.lock().unwrap();
        let tree = ev
            .iter()
            .position(|e| *e == "tree_changed")
            .expect("tree_changed");
        let done = ev.iter().position(|e| *e == "index_done").unwrap();
        assert!(
            tree < done,
            "the tree is announced before indexing ends: {ev:?}"
        );
    }

    let st = eng.git_status().unwrap();
    assert!(st.is_repo);
    assert_eq!(st.branch.as_deref(), Some("main"));
    let refs = eng.git_refs().unwrap();
    assert_eq!(refs.branches.len(), 2);

    // The index is on main; compare main with feature without checking it out.
    let c = eng.compare("main", "feature", true, None).unwrap();
    assert_eq!(c.totals.files, 3);
    assert_eq!(c.commits.len(), 1);
    let math = c.files.iter().find(|f| f.path == "lib/math.ts").unwrap();
    let ch: Vec<(&str, ChangeKind)> = math
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.change))
        .collect();
    assert!(ch.contains(&("sub", ChangeKind::Deleted)), "{ch:?}");
    assert!(ch.contains(&("add", ChangeKind::Modified)), "{ch:?}");
    let main = c.files.iter().find(|f| f.path == "app/main.ts").unwrap();
    let imp = main
        .imports
        .iter()
        .find(|i| i.specifier == "../util/log")
        .expect("new import");
    assert_eq!(imp.change, ChangeKind::Added);
    // util/log.ts exists on the feature side only, and still resolves there.
    assert_eq!(imp.file.as_deref(), Some("util/log.ts"));
    let run = main.symbols.iter().find(|s| s.name == "run").unwrap();
    assert!(run.calls_added.contains(&"log".to_string()), "{run:?}");
    assert!(run.calls_deleted.contains(&"sub".to_string()), "{run:?}");
    // sub is deleted and app/other.ts (unchanged) still calls it; add's signature changed and
    // main.ts calls it, but main.ts is part of the change, so only other.ts counts.
    let sub = c
        .impact
        .iter()
        .find(|i| i.symbol == "sub")
        .expect("impact of sub");
    assert_eq!(sub.change, ChangeKind::Deleted);
    assert_eq!(sub.caller_files, 1);
    assert!(
        !sub.by_import,
        "sub is indexed on main: its call edges count"
    );
    assert_eq!(sub.callers[0].path, "app/other.ts");
    assert!(
        !c.impact.iter().any(|i| i.symbol == "add"),
        "{:?}",
        c.impact
    );

    let f = eng
        .compare_file(&c.base.sha, &c.head.sha, "lib/math.ts", None)
        .unwrap();
    assert!(f.before.unwrap().contains("function sub"));
    assert!(!f.after.unwrap().contains("function sub"));
    assert_eq!(f.monaco_lang, "typescript");

    // Switch: the index follows without a watcher.
    events.lock().unwrap().clear();
    let st = eng.git_switch("feature", false).unwrap();
    assert_eq!(st.branch.as_deref(), Some("feature"));
    wait_for("the new file in the index", || {
        eng.tree()
            .unwrap()
            .children
            .iter()
            .any(|c| c.name == "util")
    });
    wait_for("index_done after the switch", || {
        events.lock().unwrap().contains(&"index_done")
    });
    assert!(events.lock().unwrap().contains(&"git_changed"));
    let v = eng.file_view("lib/math.ts").unwrap();
    assert!(v.functions.iter().all(|f| f.name != "sub"));

    // Uncommitted changes: head = worktree.
    write(
        r,
        "util/log.ts",
        "export function log(msg: string, level = 0) {\n  console.log(level, msg);\n}\n",
    );
    let w = eng.compare("HEAD", "worktree", false, None).unwrap();
    assert_eq!(w.head.label, "Working tree");
    assert_eq!(w.totals.files, 1);
    assert_eq!(w.totals.signatures_changed, 1);

    // Not a branch: git's own message comes back.
    let e = eng.git_switch("no-such-branch", false).unwrap_err();
    assert!(format!("{e:#}").contains("no-such-branch"));
}

#[test]
fn a_folder_outside_git() {
    let src = tempfile::tempdir().unwrap();
    write(src.path(), "a.ts", "export const a = 1;\n");
    let data = tempfile::tempdir().unwrap();
    let eng = Engine::open_with_data(src.path(), data.path()).unwrap();
    assert!(!eng.git_status().unwrap().is_repo);
    assert!(!eng.git_prs().available);
    assert!(eng.compare("HEAD", "worktree", true, None).is_err());
}

#[test]
fn review_of_a_comparison_sends_diffs_and_new_packages_and_is_cached() {
    use iso_ai::{AiEngine, MockProvider};
    if iso_git::find_exe("git").is_none() {
        return;
    }
    let src = tempfile::tempdir().unwrap();
    let r = src.path();
    repo(r);
    // A package new to the repo on the feature branch.
    git(r, &["switch", "-q", "feature"]);
    write(
        r,
        "util/log.ts",
        "import chalk from \"chalk\";\n\nexport function log(msg: string) {\n  console.log(chalk.blue(msg));\n}\n",
    );
    git(r, &["commit", "-q", "-am", "colored log"]);
    git(r, &["switch", "-q", "main"]);
    let data = tempfile::tempdir().unwrap();
    let eng = Engine::open_with_data(r, data.path()).unwrap();
    eng.index().unwrap();
    let c = eng.compare("main", "feature", true, None).unwrap();
    let p = c
        .packages
        .iter()
        .find(|p| p.name == "chalk")
        .expect("chalk");
    assert_eq!(p.change, ChangeKind::Added);
    assert_eq!(p.elsewhere, 0);
    assert_eq!(p.files, vec!["util/log.ts".to_string()]);

    let input = eng.review_input(&c).unwrap();
    let math = input
        .files
        .iter()
        .find(|f| f.path == "lib/math.ts")
        .unwrap();
    assert!(
        math.patch.contains("-export function sub"),
        "{}",
        math.patch
    );
    assert!(
        math.symbols
            .iter()
            .any(|s| s.starts_with("- ") && s.contains("sub"))
    );
    assert!(input.packages[0].contains("new to the repo"));

    let ai = AiEngine::in_memory(Arc::new(MockProvider::new())).unwrap();
    assert!(
        eng.ai_review_state(&ai, &c).is_err(),
        "AI is off for the repo"
    );
    eng.set_ai_enabled(true).unwrap();
    let st = eng.ai_review_state(&ai, &c).unwrap();
    assert!(st.review.is_none() && st.cost_usd > 0.0);
    assert!(
        eng.ai_review_run(&ai, &c, st.cost_usd / 10.0).is_err(),
        "over the budget"
    );
    let rv = eng.ai_review_run(&ai, &c, st.cost_usd * 2.0).unwrap();
    assert!(rv.summary.contains("feature"), "{}", rv.summary);
    assert!(rv.notes.iter().any(|n| n.path.is_some()));
    assert!(!rv.modules.is_empty());
    let again = eng.ai_review_state(&ai, &c).unwrap();
    assert_eq!(again.review, Some(rv));
}
