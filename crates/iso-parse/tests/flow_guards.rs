//! Pathological inputs must not take the process down: test threads, like rayon workers,
//! have 2 MiB stacks. analyze.js overflows its stack on the second shape (a real file in a
//! large TS repo has it), so these have no golden; they only have to finish.

use std::path::Path;
use std::time::{Duration, Instant};

use iso_core::Language;

fn run(src: &str) -> Duration {
    let t = Instant::now();
    let path = Path::new("guard.ts");
    let _ = iso_parse::extract_file(path, Language::TypeScript, src);
    let _ = iso_parse::flow_units(path, Language::TypeScript, src);
    t.elapsed()
}

/// Callbacks nested 2000 deep (about 12 000 tree levels).
#[test]
fn deep_callback_nesting() {
    let n = 2000;
    let mut src = String::from("export function f() {\n");
    for _ in 0..n {
        src.push_str("g(() => {\n");
    }
    src.push_str("h();\n");
    for _ in 0..n {
        src.push_str("});\n");
    }
    src.push_str("}\n");
    run(&src);
    let facts = iso_parse::extract_file(Path::new("guard.ts"), Language::TypeScript, &src);
    assert_eq!(facts.calls.len(), n + 1);
}

/// A generated `a + a0 + a1 + …` chain: a left-leaning binary tree 20 000 levels deep.
#[test]
fn long_binary_chain() {
    let mut src = String::from("export function f() {\n  return a");
    for i in 0..20_000 {
        src.push_str(&format!(" + g{i}()"));
    }
    src.push_str(";\n}\n");
    run(&src);
    let facts = iso_parse::extract_file(Path::new("guard.ts"), Language::TypeScript, &src);
    assert_eq!(facts.calls.len(), 20_000);
}

/// A factory whose local functions call each other and define deferred helpers that call
/// back into them: inlining revisits the same bodies through every path.
#[test]
fn mutually_inlining_locals_finish() {
    let n = 14;
    let mut src = String::from("export function make() {\n");
    for i in 0..n {
        src.push_str(&format!("  const f{i} = () => {{\n"));
        src.push_str(&format!("    const later{i} = () => {{ "));
        for j in 0..n {
            if j != i {
                src.push_str(&format!("f{j}(); "));
            }
        }
        src.push_str("};\n");
        for j in 0..n {
            if j != i {
                src.push_str(&format!("    if (x{j}) f{j}();\n"));
            }
        }
        src.push_str("  };\n");
    }
    src.push_str("  return { f0, run() { f1(); } };\n}\n");
    let took = run(&src);
    assert!(took < Duration::from_secs(60), "took {took:?}");
}
