//! AI layer through the engine, with the mock provider (no network, no sidecar).

use std::path::Path;
use std::sync::Arc;

use iso_ai::{AiEngine, MockProvider};
use iso_core::ipc::AiScope;
use iso_engine::{AiLayer, Engine};

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn repo(r: &Path) {
    write(r, "pkg/__init__.py", "");
    write(
        r,
        "pkg/util.py",
        "def helper(x):\n    return x + 1\n\n\ndef other(y):\n    if y:\n        return helper(y)\n    return 0\n",
    );
    write(
        r,
        "pkg/api.py",
        "from .util import helper\n\n\nclass Api:\n    def handle(self, n):\n        try:\n            return helper(n) * 2\n        except ValueError:\n            raise\n",
    );
    write(
        r,
        "app/main.py",
        "from pkg.api import Api\n\n\ndef main():\n    print(Api().handle(1))\n",
    );
}

#[test]
fn ai_is_off_until_enabled_then_cached_summaries_reach_file_views() {
    let src = tempfile::tempdir().unwrap();
    repo(src.path());
    let data = tempfile::tempdir().unwrap();
    let eng = Engine::open_with_data(src.path(), data.path()).unwrap();
    eng.index().unwrap();
    let mock = Arc::new(MockProvider::new());
    let ai = Arc::new(AiEngine::open(&data.path().join("ai_cache.sqlite"), mock.clone()).unwrap());
    eng.set_ai_hook(Some(Arc::new(AiLayer {
        ai: Arc::clone(&ai),
    })));

    // Off by default: nothing is sent.
    assert!(eng.ai_estimate(&ai, AiScope::Repo, None).is_err());
    assert!(
        eng.ai_run(&ai, AiScope::Repo, None, 1.0, &mut |_| {})
            .is_err()
    );
    assert_eq!(mock.calls(), 0);

    eng.set_ai_enabled(true).unwrap();
    let est = eng.ai_estimate(&ai, AiScope::Repo, None).unwrap();
    assert!(est.symbols >= 4, "{est:?}");
    assert!(est.cost_usd > 0.0);
    let mut finished = 0;
    let rep = eng
        .ai_run(&ai, AiScope::Repo, None, 1.0, &mut |p| {
            finished += p.finished as u32
        })
        .unwrap();
    assert!(rep.leaf.stored > 0, "{:?}", rep.leaf);
    assert!(mock.calls() > 0);
    assert!(finished >= 1);

    let v = eng.file_view("pkg/util.py").unwrap();
    let helper = v.functions.iter().find(|f| f.name == "helper").unwrap();
    let s = helper.summary.as_ref().expect("a cached summary");
    assert!(s.ai);
    assert!(!s.desc.is_empty());
    assert!(v.overview.is_some());

    // Modules got names from the cluster pass; a user rename survives a rerun.
    let mm = eng.module_map().unwrap();
    let m0 = mm.modules[0].id.clone();
    eng.rename_module(&m0, Some("My module"), None).unwrap();
    let before = mock.calls();
    eng.ai_run(&ai, AiScope::Repo, None, 1.0, &mut |_| {})
        .unwrap();
    assert_eq!(
        mock.calls(),
        before,
        "a second run over unchanged code makes no calls"
    );
    let mm = eng.module_map().unwrap();
    assert_eq!(
        mm.modules.iter().find(|m| m.id == m0).unwrap().name,
        "My module"
    );
}
