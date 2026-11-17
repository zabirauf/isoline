//! Skeletons stay at 40% of the source tokens or less on the prototype's real input file,
//! and never carry raw source except for tiny functions.

use std::collections::HashMap;
use std::path::PathBuf;

use iso_ai::{SkeletonOptions, build_skeleton};
use iso_core::{
    CallFact, FileFacts, FlowItem, FlowUnit, ImportFact, ImportKind, ImportedName, Language,
    SymbolFact, SymbolKind, hash64,
};

fn fixture() -> (String, Vec<FlowUnit>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts/lifecycle");
    let source = std::fs::read_to_string(root.join("lifecycle.ts")).unwrap();
    let flows: HashMap<String, FlowUnit> =
        serde_json::from_str(&std::fs::read_to_string(root.join("expected/flows.json")).unwrap())
            .unwrap();
    let mut units: Vec<FlowUnit> = flows.into_values().collect();
    units.sort_by_key(|u| u.start);
    (source, units)
}

fn imports(source: &str) -> Vec<ImportFact> {
    let mut out = Vec::new();
    let mut line = 1u32;
    for stmt in source.split(';') {
        let t = stmt.trim();
        let start = line + stmt.len() as u32 - stmt.trim_start().len() as u32; // approximate
        line += stmt.matches('\n').count() as u32;
        if !t.starts_with("import") {
            continue;
        }
        let type_only = t.starts_with("import type");
        let names = t
            .split_once('{')
            .and_then(|(_, r)| r.split_once('}'))
            .map(|(n, _)| n)
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| ImportedName {
                name: n.trim_start_matches("type ").to_string(),
                alias: None,
                type_only: n.starts_with("type "),
            })
            .collect();
        let spec = t.rsplit('"').nth(1).unwrap_or("").to_string();
        out.push(ImportFact {
            specifier: spec,
            kind: ImportKind::Static,
            names,
            type_only,
            line: start.min(line),
        });
    }
    out
}

fn calls(owner: &str, items: &[FlowItem], out: &mut Vec<CallFact>) {
    for i in items {
        match i {
            FlowItem::Call {
                label, line, inner, ..
            } => {
                out.push(CallFact {
                    caller: Some(owner.to_string()),
                    callee: label.clone(),
                    name: label.rsplit('.').next().unwrap_or(label).to_string(),
                    receiver: label.rsplit_once('.').map(|(r, _)| r.to_string()),
                    line: *line,
                });
                calls(owner, inner, out);
            }
            FlowItem::Block { children, .. } => calls(owner, children, out),
            FlowItem::Exit { .. } => {}
        }
    }
}

/// Facts for the fixture, built from its source and its golden call flow.
fn facts(source: &str, units: &[FlowUnit]) -> FileFacts {
    let lines: Vec<&str> = source.lines().collect();
    let mut f = FileFacts {
        imports: imports(source),
        ..Default::default()
    };
    for u in units.iter().filter(|u| u.method.is_none()) {
        let head: Vec<&str> = lines[(u.start - 1) as usize..u.end as usize]
            .iter()
            .copied()
            .take_while(|l| !l.trim_end().ends_with('{'))
            .collect();
        let sig = format!(
            "{} {}",
            head.join(" "),
            lines[(u.start as usize - 1 + head.len()).min(lines.len() - 1)]
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
        let body = lines[(u.start - 1) as usize..u.end as usize].join("\n");
        f.symbols.push(SymbolFact {
            name: u.id.clone(),
            qualified_name: u.id.clone(),
            kind: SymbolKind::Function,
            start_line: u.start,
            end_line: u.end,
            signature: sig.trim_end_matches('{').trim().chars().take(200).collect(),
            doc: None,
            exported: sig.starts_with("export"),
            is_async: u.is_async,
            params: u.params.clone(),
            parent: None,
            body_hash: hash64(body.as_bytes()),
        });
        calls(&u.id, &u.tree, &mut f.calls);
    }
    for (i, l) in lines.iter().enumerate() {
        if let Some(rest) = l.strip_prefix("type ") {
            let name = rest.split([' ', '=']).next().unwrap().to_string();
            f.symbols.push(SymbolFact {
                name: name.clone(),
                qualified_name: name,
                kind: SymbolKind::Type,
                start_line: i as u32 + 1,
                end_line: i as u32 + 1,
                signature: l.trim_end_matches(" = {").to_string(),
                doc: None,
                exported: false,
                is_async: false,
                params: vec![],
                parent: None,
                body_hash: hash64(l.as_bytes()),
            });
        }
    }
    f
}

#[test]
fn skeletons_are_at_most_forty_percent_of_the_source() {
    let (source, units) = fixture();
    let facts = facts(&source, &units);
    assert_eq!(
        facts
            .symbols
            .iter()
            .filter(|s| s.kind.is_callable())
            .count(),
        12
    );
    let opts = SkeletonOptions::default();
    for (label, flows) in [("with flow", units.as_slice()), ("without flow", &[][..])] {
        let sk = build_skeleton(
            "src/agents/harness/lifecycle.ts",
            Language::TypeScript,
            &source,
            &facts,
            flows,
            &opts,
        );
        if std::env::var("DUMP").is_ok() {
            for c in &sk.callables {
                eprintln!(
                    "### {} ({} tokens)\n{}",
                    c.qualified_name,
                    c.tokens(),
                    c.text
                );
            }
            eprintln!("### outline\n{}", sk.outline);
        }
        let ratio = sk.tokens() as f64 / sk.source_tokens as f64;
        eprintln!(
            "{label}: {} skeleton tokens for {} source tokens ({:.0}%)",
            sk.tokens(),
            sk.source_tokens,
            ratio * 100.0
        );
        assert!(
            ratio <= 0.40,
            "{label}: {:.0}% of the source",
            ratio * 100.0
        );
        assert_eq!(sk.callables.len(), 12);
        // Raw bodies of larger functions never appear.
        let all: String = sk
            .callables
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!all.contains("const canonicalWithAttemptProvenance"));
        assert!(!all.contains("code:"), "no function in this file is tiny");
    }

    let sk = build_skeleton(
        "lifecycle.ts",
        Language::TypeScript,
        &source,
        &facts,
        &units,
        &opts,
    );
    let run = sk
        .callables
        .iter()
        .find(|c| c.qualified_name == "runAgentHarnessLifecycleAttempt")
        .unwrap();
    eprintln!("{}", run.text);
    assert!(run.text.starts_with("function, async, exported, 67 lines"));
    assert!(
        run.text
            .contains("sig: export async function runAgentHarnessLifecycleAttempt(")
    );
    assert!(!run.text.contains("calls: "), "the flow lists the calls");
    assert!(
        run.text
            .contains("flow:\n  createAgentHarnessLifecycleDiagnostics")
    );
    assert!(run.text.contains("\n  try"));
    assert!(run.text.contains("catch (error)"));
    assert!(run.text.contains("throw error"));
    assert!(run.text.contains("finally always runs"));
    assert!(sk.outline.contains("imports: ../../config/config.js, "));
    assert!(sk.outline.contains("exports: function runAgentHarnessLifecycleAttempt, function runAgentHarnessLifecycleFinalization"));
}

#[test]
fn tiny_functions_are_sent_verbatim_and_outlines_stand_in_for_flow() {
    let source = "export const id = (x: number) => {\n\t  return x;\n};\n\nfunction big(a) {\n  if (a) {\n    log(\"é\");\n  }\n  for (const k of a) {\n    work(k);\n  }\n  try {\n    more();\n  } catch (e) {\n    throw e;\n  }\n  return a;\n}\n";
    let sym = |name: &str, start, end| SymbolFact {
        name: name.into(),
        qualified_name: name.into(),
        kind: SymbolKind::Function,
        start_line: start,
        end_line: end,
        signature: String::new(),
        doc: Some("  Does   the\nthing. ".into()),
        exported: name == "id",
        is_async: false,
        params: vec!["a".into()],
        parent: None,
        body_hash: 1,
    };
    let facts = FileFacts {
        symbols: vec![sym("id", 1, 3), sym("big", 5, 18)],
        ..Default::default()
    };
    let sk = build_skeleton(
        "a.ts",
        Language::TypeScript,
        source,
        &facts,
        &[],
        &SkeletonOptions::default(),
    );
    let tiny = &sk.callables[0].text;
    assert!(tiny.contains("code:\n  export const id"), "{tiny}");
    assert!(tiny.contains("doc: Does the thing."));
    let big = &sk.callables[1].text;
    assert!(big.contains("sig: big(a)"), "{big}");
    assert!(big.contains("outline:\n  if (a) {"), "{big}");
    assert!(big.contains("throw e;"));
    assert!(!big.contains("work(k)"));
}
