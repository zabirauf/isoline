//! Drives the real Go sidecar in mock mode, end to end. The test builds the mock-only
//! sidecar from `sidecar/copilot` with the local Go toolchain and no module downloads
//! (`GOPROXY=off`), and skips with a message when Go or its module cache is missing.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use iso_ai::{
    AiEngine, AiError, AiSettings, Completion, CompletionRequest, ModelInfo, Progress, Provider,
    SidecarConfig, SidecarProvider,
};
use iso_core::ipc::AiScope;

fn find_go() -> Option<PathBuf> {
    let candidates = [
        "go",
        "/opt/homebrew/bin/go",
        "/usr/local/go/bin/go",
        "/usr/local/bin/go",
    ];
    candidates.iter().map(PathBuf::from).find(|p| {
        Command::new(p)
            .arg("version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Builds the sidecar once per test binary. None means skip.
fn sidecar_binary() -> Option<&'static Path> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        let Some(go) = find_go() else {
            eprintln!("SKIP: go is not installed; the sidecar end-to-end tests did not run");
            return None;
        };
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sidecar/copilot");
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "copilot-sidecar-test{}",
            std::env::consts::EXE_SUFFIX
        ));
        let res = Command::new(&go)
            .current_dir(&src)
            .args(["build", "-o"])
            .arg(&out)
            .arg(".")
            .env("GOPROXY", "off")
            .env("GOTOOLCHAIN", "local")
            .env("GOFLAGS", "-mod=readonly")
            .output()
            .expect("run go build");
        if !res.status.success() {
            let err = String::from_utf8_lossy(&res.stderr);
            if err.contains("GOPROXY=off") || err.contains("module lookup disabled") {
                eprintln!("SKIP: the Go module cache lacks the sidecar's dependencies: {err}");
                return None;
            }
            panic!("go build of the sidecar failed:\n{err}");
        }
        Some(out)
    })
    .as_deref()
}

fn provider(bin: &Path) -> SidecarProvider {
    let mut cfg = SidecarConfig::new(bin).mock(true);
    cfg.call_timeout = Duration::from_secs(20);
    cfg.complete_timeout = Duration::from_secs(20);
    SidecarProvider::new(cfg)
}

/// Counts completion calls on the way to another provider.
struct Counting<P> {
    inner: P,
    calls: AtomicU64,
}

impl<P: Provider> Provider for Counting<P> {
    fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        self.inner.list_models()
    }
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, AiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.complete(req)
    }
}

#[test]
fn protocol_round_trips() {
    let Some(bin) = sidecar_binary() else { return };
    let p = provider(bin);
    assert!(!p.is_running(), "the sidecar starts lazily");
    let st = p.status().unwrap();
    assert!(p.is_running());
    assert!(st.mock && st.signed_in && !st.live);
    assert_eq!(st.user.as_deref(), Some("mock-user"));

    let models = p.list_models().unwrap();
    assert_eq!(models, iso_ai::default_models());

    let req = CompletionRequest {
        model: "claude-haiku-5.5".into(),
        system: iso_ai::prompts::leaf_system(),
        prompt: "Task: leaf\n## file a.ts (TypeScript)\n### f1 run\ncalls: a, b\n### o1 a.ts\n"
            .into(),
        max_tokens: 2000,
    };
    let c = p.complete(&req).unwrap();
    // The Go mock and the Rust mock agree on text and token counts.
    let mock = iso_ai::MockProvider::new();
    let m = mock.complete(&req).unwrap();
    assert_eq!(c, m);
    let again = p.complete(&req).unwrap();
    assert_eq!(again.cached_tokens_in, iso_ai::estimate_tokens(&req.system));

    p.sign_out().unwrap();
    assert!(!p.status().unwrap().signed_in);
    assert!(matches!(p.complete(&req), Err(AiError::NotSignedIn)));
    let code = p.sign_in(None).unwrap();
    assert!(!code.user_code.is_empty() && code.expires_in > 0);
    assert_eq!(
        p.sign_in_poll().unwrap(),
        (true, Some("mock-user".to_string()))
    );
    p.complete(&req).unwrap();

    // Several threads at once: requests carry ids.
    std::thread::scope(|s| {
        for i in 0..4 {
            let p = &p;
            let mut r = req.clone();
            r.prompt = format!("Task: repo\n### m{i} X\n");
            s.spawn(move || assert!(p.complete(&r).unwrap().text.contains("overview")));
        }
    });
    assert_eq!(p.spawn_count(), 1);
    p.shutdown();
    assert!(!p.is_running());
}

#[test]
fn restarts_after_a_crash() {
    let Some(bin) = sidecar_binary() else { return };
    let p = provider(bin);
    let crash = CompletionRequest {
        model: "m".into(),
        system: String::new(),
        prompt: "__isoline_mock_crash__".into(),
        max_tokens: 10,
    };
    match p.complete(&crash) {
        Err(AiError::Provider(msg)) => assert!(msg.contains("exited"), "{msg}"),
        other => panic!("expected the sidecar to exit, got {other:?}"),
    }
    assert!(p.status().unwrap().mock);
    assert_eq!(p.spawn_count(), 2);

    // A missing binary is a clear error, not a panic.
    let missing = SidecarProvider::new(SidecarConfig::new("/nonexistent/copilot-sidecar"));
    match missing.status() {
        Err(AiError::Provider(msg)) => assert!(msg.contains("could not start")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn full_pipeline_through_the_sidecar() {
    let Some(bin) = sidecar_binary() else { return };
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ai_cache.sqlite");
    let files = common::repo(24, 0);
    let modules = common::modules(&files);
    let edges = common::edges();
    let p = Arc::new(Counting {
        inner: provider(bin),
        calls: AtomicU64::new(0),
    });
    let e = AiEngine::open(&db, p.clone()).unwrap();
    e.set_settings(AiSettings {
        economy: false,
        ..AiSettings::default()
    });
    let est = e.estimate(&files, &modules, &edges, AiScope::Repo);
    let mut events = 0;
    let r = e
        .run(
            &files,
            &modules,
            &edges,
            &HashMap::new(),
            AiScope::Repo,
            5.0,
            &mut |_: &Progress| events += 1,
        )
        .unwrap();
    assert!(events > 0);
    assert_eq!(r.leaf.failed, 0, "{:?}", r.leaf.errors);
    assert_eq!(r.leaf.stored, r.leaf.items);
    let cluster = r.cluster.as_ref().unwrap();
    assert!(cluster.modules.iter().all(|m| m.ai));
    assert_eq!(cluster.modules[0].name, "Engine");
    assert!(r.repo.as_ref().unwrap().overview.is_some());
    let sent = p.calls.load(Ordering::SeqCst);
    assert_eq!(sent as u32, est.requests());
    let counted = r.leaf.tokens_in + r.leaf.tokens_out;
    let estimated = est.leaf.tokens_in + est.leaf.tokens_out;
    assert!((estimated as f64 - counted as f64).abs() / counted as f64 <= 0.10);

    // Second run: zero calls.
    e.run(
        &files,
        &modules,
        &edges,
        &HashMap::new(),
        AiScope::Repo,
        5.0,
        &mut |_: &Progress| {},
    )
    .unwrap();
    assert_eq!(p.calls.load(Ordering::SeqCst), sent);

    // One body change: one leaf request.
    let changed = common::repo(24, 3);
    let r = e.run_leaf(&changed, 5.0, &mut |_: &Progress| {}).unwrap();
    assert_eq!(p.calls.load(Ordering::SeqCst), sent + 1);
    assert_eq!((r.requests, r.stored), (1, 1));
}
