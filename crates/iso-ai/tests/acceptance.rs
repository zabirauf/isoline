//! Workstream I acceptance tests, with MockProvider (no network, no model):
//! 1. a second run over unchanged code makes zero provider calls;
//! 2. changing one function body makes exactly one leaf request, containing only that symbol;
//! 3. the estimate is within 10% of the counted tokens.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use iso_ai::mock::prompt_items;
use iso_ai::{AiEngine, AiError, AiSettings, MockProvider, Pass, Progress, Provider};
use iso_core::ipc::AiScope;

fn engine_at(path: &std::path::Path, mock: &Arc<MockProvider>) -> AiEngine {
    let e = AiEngine::open(path, mock.clone() as Arc<dyn Provider>).unwrap();
    e.set_settings(AiSettings {
        economy: false,
        ..AiSettings::default()
    });
    e
}

fn noop() -> impl FnMut(&Progress) {
    |_: &Progress| {}
}

#[test]
fn second_run_makes_zero_provider_calls() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ai_cache.sqlite");
    let files = common::repo(30, 0);
    let modules = common::modules(&files);
    let edges = common::edges();
    let mock = Arc::new(MockProvider::new());

    let e = engine_at(&db, &mock);
    let first = e
        .run(
            &files,
            &modules,
            &edges,
            &HashMap::new(),
            AiScope::Repo,
            10.0,
            &mut noop(),
        )
        .unwrap();
    assert!(mock.calls() > 0);
    assert_eq!(first.leaf.failed, 0, "{:?}", first.leaf.errors);
    // 30 files: 5 functions, 1 type and 1 overview each.
    assert_eq!(first.leaf.items, 30 * 7);
    assert_eq!(first.leaf.stored, 30 * 7);
    // About 20 files per request.
    assert_eq!(first.leaf.requests, 2);
    let cluster = first.cluster.as_ref().unwrap();
    assert_eq!(cluster.run.stored, 3);
    assert!(cluster.modules.iter().all(|m| m.ai));
    let repo = first.repo.as_ref().unwrap();
    assert!(repo.overview.is_some());
    let overview = repo.overview.clone().unwrap();
    assert_eq!(
        overview
            .layers
            .iter()
            .map(|l| l.modules.len())
            .sum::<usize>(),
        3
    );

    // Same engine, same code.
    mock.reset_counts();
    let again = e
        .run(
            &files,
            &modules,
            &edges,
            &HashMap::new(),
            AiScope::Repo,
            10.0,
            &mut noop(),
        )
        .unwrap();
    assert_eq!(mock.calls(), 0);
    assert_eq!(mock.list_calls(), 0);
    assert_eq!(again.leaf.cached, again.leaf.items);
    assert_eq!(again.cost_usd(), 0.0);
    assert_eq!(again.cluster.unwrap().modules, cluster.modules);
    assert_eq!(again.repo.unwrap().overview.unwrap(), overview);
    drop(e);

    // A fresh engine on the same cache file (the app reopened): still nothing to send,
    // not even a model listing.
    let e = engine_at(&db, &mock);
    let est = e.estimate(&files, &modules, &edges, AiScope::Repo);
    assert_eq!(est.requests(), 0);
    assert_eq!(est.cost_usd(), 0.0);
    e.run(
        &files,
        &modules,
        &edges,
        &HashMap::new(),
        AiScope::Repo,
        0.0,
        &mut noop(),
    )
    .unwrap();
    assert_eq!(mock.calls(), 0);
    assert_eq!(mock.list_calls(), 0);

    // Lookups serve the cached answers without a request.
    let cached = e.cached_file(&files[3]);
    assert_eq!(cached.summaries.len(), 5);
    assert_eq!(cached.types.len(), 1);
    assert!(cached.overview.is_some());
    let s = &cached.summaries["step3_1"];
    assert!(s.ai && s.desc.starts_with("Handles") && (3..=7).contains(&s.algo.len()));
    assert_eq!(
        e.cached_summary(&files[3].facts.symbols[2]).as_ref(),
        Some(s)
    );
    // Lookups need facts only.
    let mut no_source = files[3].clone();
    no_source.source.clear();
    assert_eq!(e.cached_file(&no_source), cached);
    assert_eq!(mock.calls(), 0);
}

#[test]
fn one_body_change_makes_one_leaf_request_with_only_that_symbol() {
    let files = common::repo(30, 0);
    let mock = Arc::new(MockProvider::new());
    let e = AiEngine::in_memory(mock.clone()).unwrap();
    e.run_leaf(&files, 10.0, &mut noop()).unwrap();

    mock.reset_counts();
    let changed = common::repo(30, 1);
    assert_ne!(changed[0].content_hash, files[0].content_hash);
    let r = e.run_leaf(&changed, 10.0, &mut noop()).unwrap();
    assert_eq!(mock.calls(), 1);
    assert_eq!(r.requests, 1);
    assert_eq!(r.stored, 1);
    let req = &mock.requests()[0];
    let items = prompt_items(&req.prompt);
    assert_eq!(items.len(), 1, "{}", req.prompt);
    assert_eq!(items[0].0, "f1");
    assert_eq!(items[0].1, "step0_0");
    assert_eq!(r.updated_paths, vec![changed[0].path.clone()]);
    // The other functions of the same file were not sent again.
    assert!(!req.prompt.contains("step0_1"));
    // The new summary is served for the new body; the old one is still cached for the old.
    assert!(e.cached_summary(&changed[0].facts.symbols[1]).is_some());
    assert!(e.cached_summary(&files[0].facts.symbols[1]).is_some());
}

#[test]
fn estimate_is_within_ten_percent_of_counted_tokens() {
    for (n, economy) in [(30, false), (7, true), (55, false)] {
        let files = common::repo(n, 0);
        let modules = common::modules(&files);
        let edges = common::edges();
        let mock = Arc::new(MockProvider::new());
        let e = AiEngine::in_memory(mock.clone()).unwrap();
        e.set_settings(AiSettings {
            economy,
            ..AiSettings::default()
        });

        let leaf_est = e.estimate_leaf(&files);
        let report = e.run_leaf(&files, 10.0, &mut noop()).unwrap();
        let within = |est: u64, got: u64, what: &str| {
            let err = (est as f64 - got as f64).abs() / got as f64;
            eprintln!(
                "{n} files, {what}: estimated {est}, counted {got} ({:.1}%)",
                err * 100.0
            );
            assert!(
                err <= 0.10,
                "{n} files, {what}: estimated {est}, counted {got} ({:.1}%)",
                err * 100.0
            );
        };
        within(leaf_est.tokens_in, mock.tokens_in(), "leaf input");
        within(leaf_est.tokens_out, mock.tokens_out(), "leaf output");
        within(
            leaf_est.tokens_in + leaf_est.tokens_out,
            mock.tokens_in() + mock.tokens_out(),
            "leaf total",
        );
        assert_eq!(leaf_est.requests, report.requests);
        assert_eq!(leaf_est.cached_tokens_in, mock.cached_tokens_in());
        let cost_err = (leaf_est.cost_usd - report.cost_usd).abs() / report.cost_usd;
        assert!(
            cost_err <= 0.10,
            "leaf cost {} vs {}",
            leaf_est.cost_usd,
            report.cost_usd
        );

        // With leaf summaries cached, the cluster and repo estimates use the real inputs.
        mock.reset_counts();
        let est = e.estimate(&files, &modules, &edges, AiScope::Repo);
        assert_eq!(est.leaf.requests, 0);
        let c = e
            .cluster_pass(&modules, &edges, &files, &HashMap::new(), 10.0, &mut noop())
            .unwrap();
        let r = e.repo_pass(&c.modules, &edges, 10.0, &mut noop()).unwrap();
        let ce = est.cluster.unwrap();
        let re = est.repo.unwrap();
        within(
            ce.tokens_in + re.tokens_in,
            mock.tokens_in(),
            "cluster and repo input",
        );
        within(
            ce.tokens_in + re.tokens_in + ce.tokens_out + re.tokens_out,
            mock.tokens_in() + mock.tokens_out(),
            "cluster and repo total",
        );
        assert_eq!(c.run.requests + r.run.requests, ce.requests + re.requests);
        let expect_cluster = if economy {
            "claude-haiku-5.5"
        } else {
            "claude-sonnet-5.5"
        };
        assert_eq!(c.run.model, expect_cluster);
        assert_eq!(r.run.model, "claude-opus-5.5");
    }
}

#[test]
fn budget_refuses_and_stops() {
    let files = common::repo(60, 0);
    let mock = Arc::new(MockProvider::new());
    let e = AiEngine::in_memory(mock.clone()).unwrap();
    let est = e.estimate_leaf(&files);
    assert!(est.requests >= 3);

    // Refused up front: no call.
    match e.run_leaf(&files, est.cost_usd / 2.0, &mut noop()) {
        Err(AiError::Budget(msg)) => assert!(msg.contains("over the budget")),
        other => panic!("expected a budget refusal, got {other:?}"),
    }
    assert_eq!(mock.calls(), 0);

    // A budget that fits the estimate runs to the end and stays under it.
    let mut events: Vec<Progress> = Vec::new();
    let r = e
        .run_leaf(&files, est.cost_usd * 1.05, &mut |p: &Progress| {
            events.push(p.clone())
        })
        .unwrap();
    assert!(!r.stopped_by_budget);
    assert!(r.cost_usd <= est.cost_usd * 1.05);
    assert_eq!(events.iter().filter(|p| p.finished).count(), 1);
    let last = events.last().unwrap();
    assert!(last.finished && last.error.is_none() && last.pass == Pass::Leaf);
    assert_eq!(last.done, last.total);
    assert_eq!(last.tokens_in, mock.tokens_in());
    assert!((last.cost_usd - r.cost_usd).abs() < 1e-12);
    let ipc = last.to_ipc(7);
    assert_eq!((ipc.job_id, ipc.done, ipc.finished), (7, last.done, true));
}

#[test]
fn stops_mid_run_when_answers_cost_more_than_estimated() {
    let files = common::repo(60, 0);
    // Answers four times longer than the mock's: the estimate fits, the run does not.
    let mock = Arc::new(MockProvider::with_responder(|req| {
        let a = iso_ai::mock::mock_answer(&req.system, &req.prompt);
        let v: serde_json::Value = serde_json::from_str(&a).unwrap();
        let mut v = v;
        for item in v["items"].as_array_mut().unwrap() {
            if let Some(d) = item
                .get("desc")
                .and_then(|d| d.as_str())
                .map(|d| d.repeat(8))
            {
                item["desc"] = serde_json::Value::String(d);
            }
        }
        Ok(v.to_string())
    }));
    let e = AiEngine::in_memory(mock.clone()).unwrap();
    let est = e.estimate_leaf(&files);
    let mut finished = 0;
    let r = e
        .run_leaf(&files, est.cost_usd * 1.02, &mut |p: &Progress| {
            finished += p.finished as u32
        })
        .unwrap();
    assert!(r.stopped_by_budget);
    assert!(r.requests < est.requests);
    assert!(r.cost_usd <= est.cost_usd * 1.02);
    assert_eq!(finished, 1);
}

#[test]
fn bad_answers_are_skipped_and_retried_next_run() {
    let files = common::repo(3, 0);
    let mock = Arc::new(MockProvider::new());
    // First answer: fenced, with one malformed entry and one entry missing.
    let first = {
        let prompt_mock = Arc::new(MockProvider::new());
        let e = AiEngine::in_memory(prompt_mock.clone()).unwrap();
        e.run_leaf(&files, 10.0, &mut noop()).unwrap();
        let req = &prompt_mock.requests()[0];
        let a = iso_ai::mock::mock_answer(&req.system, &req.prompt);
        let mut v: serde_json::Value = serde_json::from_str(&a).unwrap();
        let items = v["items"].as_array_mut().unwrap();
        items[0]["desc"] = serde_json::Value::Null;
        items.remove(1);
        format!("Here are the summaries.\n```json\n{v}\n```\n")
    };
    mock.push_response(first);
    let e = AiEngine::in_memory(mock.clone()).unwrap();
    let r = e.run_leaf(&files, 10.0, &mut noop()).unwrap();
    assert_eq!(r.failed, 2, "{:?}", r.errors);
    assert_eq!(r.stored, r.items - 2);
    assert!(r.errors.iter().any(|x| x.message.contains("no desc")));
    assert!(r.errors.iter().any(|x| x.message.contains("no entry")));

    // The next run asks again for exactly the two failed items.
    mock.reset_counts();
    let r = e.run_leaf(&files, 10.0, &mut noop()).unwrap();
    assert_eq!(mock.calls(), 1);
    assert_eq!(prompt_items(&mock.requests()[0].prompt).len(), 2);
    assert_eq!(r.stored, 2);

    // A provider error is recorded and the pass continues; not signed in stops it.
    let mock = Arc::new(MockProvider::new());
    mock.push_error(AiError::Provider("boom".into()));
    let e = AiEngine::in_memory(mock.clone()).unwrap();
    let r = e.run_leaf(&files, 10.0, &mut noop()).unwrap();
    assert_eq!(r.failed, r.items);
    let mock = Arc::new(MockProvider::new());
    mock.push_error(AiError::NotSignedIn);
    let e = AiEngine::in_memory(mock.clone()).unwrap();
    assert!(matches!(
        e.run_leaf(&files, 10.0, &mut noop()),
        Err(AiError::NotSignedIn)
    ));
}

#[test]
fn cluster_colors_are_stable_and_keep_user_choices() {
    let files = common::repo(9, 0);
    let modules = common::modules(&files);
    let edges = common::edges();
    let mock = Arc::new(MockProvider::new());
    let e = AiEngine::in_memory(mock.clone()).unwrap();

    // Before any pass: names from directories, colors already assigned.
    let before = e.cached_modules(&modules, &edges, &files, &HashMap::new());
    assert!(before.iter().all(|m| !m.ai));
    assert_eq!(before[0].name, "Engine");

    e.run_leaf(&files, 10.0, &mut noop()).unwrap();
    let a = e
        .cluster_pass(&modules, &edges, &files, &HashMap::new(), 10.0, &mut noop())
        .unwrap();
    let mut users = HashMap::new();
    users.insert("src/ui".to_string(), "#000080".to_string());
    let b = e
        .cluster_pass(&modules, &edges, &files, &users, 10.0, &mut noop())
        .unwrap();
    assert_eq!(b.run.requests, 0);
    for (x, y) in a.modules.iter().zip(&b.modules) {
        assert_eq!(x.name, y.name);
        if x.id != "src/ui" {
            assert_eq!(x.color, y.color);
        }
    }
    assert_eq!(
        b.modules.iter().find(|m| m.id == "src/ui").unwrap().color,
        "#000080"
    );
    let distinct: std::collections::HashSet<_> = a.modules.iter().map(|m| &m.color).collect();
    assert_eq!(distinct.len(), 3);
    for m in &a.modules {
        assert_eq!(
            iso_ai::palette::by_value(&m.color).unwrap().dark,
            m.color_dark
        );
    }

    // The module prompt carries member summaries and edges, never raw source.
    let req = mock
        .requests()
        .into_iter()
        .find(|r| r.prompt.starts_with("Task: cluster"))
        .unwrap();
    assert!(req.prompt.contains("Handles step"));
    assert!(req.prompt.contains("imports)"));
    assert!(!req.prompt.contains("computeDefault"));
}
