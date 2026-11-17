//! A synthetic repo for the iso-ai tests: TypeScript-like files with real line ranges,
//! body hashes, calls, imports and (for half the files) call-flow units.

#![allow(dead_code)]

use iso_ai::{FileInput, ModuleInput};
use iso_core::ipc::ModuleEdge;
use iso_core::{
    CallFact, FileFacts, FlowItem, FlowTarget, FlowTargetKind, FlowUnit, ImportFact, ImportKind,
    ImportedName, Language, RefFact, SymbolFact, SymbolKind, hash64,
};

pub const DIRS: [&str; 3] = ["src/engine", "src/index", "src/ui"];

fn fn_body(file: usize, k: usize, variant: u32) -> (Vec<String>, Vec<String>) {
    let name = format!("step{file}_{k}");
    let callees = vec![
        format!("load{k}"),
        format!("check{file}"),
        "emitEvent".to_string(),
    ];
    let mut lines = vec![format!(
        "export async function {name}(input: Input, options = {{}}): Promise<Output> {{"
    )];
    lines.push(format!("  const value = await load{k}(input.id, options);"));
    lines.push("  if (!value) {".to_string());
    lines.push("    return null;".to_string());
    lines.push("  }".to_string());
    lines.push("  try {".to_string());
    lines.push(format!(
        "    check{file}(value, {{ strict: true, depth: {k} }});"
    ));
    for j in 0..(4 + (k % 5)) {
        lines.push(format!(
            "    const part{j} = value.items[{j}] ?? computeDefault(value, \"part{j}\", options.mode);"
        ));
    }
    lines.push("  } catch (error) {".to_string());
    lines.push("    emitEvent(\"failed\", { error, input });".to_string());
    lines.push("    throw error;".to_string());
    lines.push("  }".to_string());
    lines.push(format!(
        "  return {{ id: input.id, value, version: {variant} }};"
    ));
    lines.push("}".to_string());
    (lines, callees)
}

/// Builds file `i` of the synthetic repo. `variant` changes the body of function 0 only.
pub fn file(i: usize, variant: u32) -> FileInput {
    let dir = DIRS[i % DIRS.len()];
    let path = format!("{dir}/file{i}.ts");
    let mut src: Vec<String> = vec![
        "import { load0, load1, load2, load3, load4 } from \"../store/load.js\";".to_string(),
        "import type { Input, Output } from \"../types.js\";".to_string(),
        "import { emitEvent } from \"../events.js\";".to_string(),
        String::new(),
        format!("/** Options for file {i}. */"),
        format!("export interface Options{i} {{"),
        "  mode: string;".to_string(),
        "  strict: boolean;".to_string(),
        "}".to_string(),
        String::new(),
    ];
    let mut facts = FileFacts {
        imports: vec![
            ImportFact {
                specifier: "../store/load.js".into(),
                kind: ImportKind::Static,
                names: (0..5)
                    .map(|k| ImportedName {
                        name: format!("load{k}"),
                        alias: None,
                        type_only: false,
                    })
                    .collect(),
                type_only: false,
                line: 1,
            },
            ImportFact {
                specifier: "../types.js".into(),
                kind: ImportKind::Static,
                names: ["Input", "Output"]
                    .iter()
                    .map(|n| ImportedName {
                        name: n.to_string(),
                        alias: None,
                        type_only: true,
                    })
                    .collect(),
                type_only: true,
                line: 2,
            },
            ImportFact {
                specifier: "../events.js".into(),
                kind: ImportKind::Static,
                names: vec![ImportedName {
                    name: "emitEvent".into(),
                    alias: None,
                    type_only: false,
                }],
                type_only: false,
                line: 3,
            },
        ],
        ..Default::default()
    };
    facts.symbols.push(SymbolFact {
        name: format!("Options{i}"),
        qualified_name: format!("Options{i}"),
        kind: SymbolKind::Interface,
        start_line: 6,
        end_line: 9,
        signature: format!("export interface Options{i}"),
        doc: Some(format!("Options for file {i}.")),
        exported: true,
        is_async: false,
        params: vec![],
        parent: None,
        body_hash: hash64(format!("Options{i} mode strict").as_bytes()),
    });
    let mut flows = Vec::new();
    for k in 0..5 {
        let (body, callees) = fn_body(i, k, if k == 0 { variant } else { 0 });
        let start = src.len() as u32 + 1;
        let end = start + body.len() as u32 - 1;
        let name = format!("step{i}_{k}");
        let text = body.join("\n");
        facts.symbols.push(SymbolFact {
            name: name.clone(),
            qualified_name: name.clone(),
            kind: SymbolKind::Function,
            start_line: start,
            end_line: end,
            signature: body[0].trim_end_matches(" {").to_string(),
            doc: (k == 0).then(|| format!("Runs step {k} of file {i}.")),
            exported: true,
            is_async: true,
            params: vec!["input".into(), "options = …".into()],
            parent: None,
            body_hash: hash64(text.as_bytes()),
        });
        for (n, c) in callees.iter().enumerate() {
            facts.calls.push(CallFact {
                caller: Some(name.clone()),
                callee: c.clone(),
                name: c.clone(),
                receiver: None,
                line: start + 1 + n as u32,
            });
        }
        facts.refs.push(RefFact {
            from: Some(name.clone()),
            name: "Input".into(),
            type_position: true,
            line: start,
        });
        if i.is_multiple_of(2) {
            let call = |label: &str, line: u32| FlowItem::Call {
                label: label.into(),
                target: FlowTarget {
                    kind: FlowTargetKind::Ext,
                    id: Some(label.into()),
                    module: None,
                    unit: None,
                    of: None,
                    note: None,
                    ext: None,
                },
                line,
                awaited: false,
                optional: false,
                arg_of: None,
                inner: vec![],
            };
            flows.push(FlowUnit {
                id: name.clone(),
                owner: name.clone(),
                method: None,
                start,
                end,
                is_async: true,
                params: vec!["input".into(), "options = …".into()],
                tree: vec![
                    call(&format!("load{k}"), start + 1),
                    FlowItem::Block {
                        kind: iso_core::BlockKind::Cond,
                        kw: "if".into(),
                        label: "(!value)".into(),
                        line: start + 2,
                        children: vec![FlowItem::Exit {
                            kind: iso_core::ExitKind::Return,
                            label: "null".into(),
                            line: start + 3,
                            early: Some(true),
                            methods: None,
                        }],
                    },
                    FlowItem::Block {
                        kind: iso_core::BlockKind::Try,
                        kw: "try".into(),
                        label: String::new(),
                        line: start + 5,
                        children: vec![call(&format!("check{i}"), start + 6)],
                    },
                    FlowItem::Block {
                        kind: iso_core::BlockKind::Catch,
                        kw: "catch".into(),
                        label: "(error)".into(),
                        line: end - 4,
                        children: vec![
                            call("emitEvent", end - 3),
                            FlowItem::Exit {
                                kind: iso_core::ExitKind::Throw,
                                label: "error".into(),
                                line: end - 2,
                                early: None,
                                methods: None,
                            },
                        ],
                    },
                    FlowItem::Exit {
                        kind: iso_core::ExitKind::Return,
                        label: "{ id: input.id, value, version }".into(),
                        line: end - 1,
                        early: Some(false),
                        methods: None,
                    },
                ],
            });
        }
        src.extend(body);
        src.push(String::new());
    }
    let source = src.join("\n");
    FileInput {
        path,
        lang: Language::TypeScript,
        content_hash: iso_core::content_hash(source.as_bytes()),
        source,
        facts,
        flows,
    }
}

/// `n` files; file 0's first function body uses `variant`.
pub fn repo(n: usize, variant: u32) -> Vec<FileInput> {
    (0..n)
        .map(|i| file(i, if i == 0 { variant } else { 0 }))
        .collect()
}

pub fn modules(files: &[FileInput]) -> Vec<ModuleInput> {
    DIRS.iter()
        .map(|d| ModuleInput {
            id: d.to_string(),
            dir: d.to_string(),
            files: files
                .iter()
                .filter(|f| f.path.starts_with(d))
                .map(|f| f.path.clone())
                .collect(),
        })
        .collect()
}

pub fn edges() -> Vec<ModuleEdge> {
    vec![
        ModuleEdge {
            from: "src/ui".into(),
            to: "src/engine".into(),
            weight: 7,
        },
        ModuleEdge {
            from: "src/engine".into(),
            to: "src/index".into(),
            weight: 4,
        },
    ]
}
