//! Precise indexers (workstream K): detect and run scip-typescript, scip-python and
//! rust-analyzer, and read their SCIP output into exact reference → definition pairs.
//!
//! The engine uses [`ExactRef`]s to upgrade heuristic call edges to `exact`, and to correct
//! heuristic edges that pointed at the wrong symbol.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use protobuf::Message;

/// One resolved reference from a SCIP index. Lines are 1-based.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExactRef {
    /// Repo-relative path of the file containing the reference.
    pub file: String,
    pub line: u32,
    /// Last descriptor name of the referenced symbol (`run` for `pkg/a.ts/Foo#run().`).
    pub name: String,
    /// Where the referenced symbol is defined.
    pub def_file: String,
    pub def_line: u32,
}

/// Parse a SCIP index and pair every global reference with its definition.
pub fn read_index(bytes: &[u8]) -> anyhow::Result<Vec<ExactRef>> {
    let index = scip::types::Index::parse_from_bytes(bytes)?;
    let mut defs: HashMap<&str, (&str, u32)> = HashMap::new();
    for d in &index.documents {
        for o in &d.occurrences {
            if o.symbol_roles & scip::types::SymbolRole::Definition as i32 != 0
                && !scip::symbol::is_local_symbol(&o.symbol)
                && let Some(&l) = o.range.first()
            {
                defs.entry(o.symbol.as_str())
                    .or_insert((d.relative_path.as_str(), l as u32 + 1));
            }
        }
    }
    let mut out = Vec::new();
    for d in &index.documents {
        for o in &d.occurrences {
            if o.symbol_roles & scip::types::SymbolRole::Definition as i32 != 0
                || scip::symbol::is_local_symbol(&o.symbol)
            {
                continue;
            }
            let Some((def_file, def_line)) = defs.get(o.symbol.as_str()) else {
                continue;
            };
            let Some(&l) = o.range.first() else { continue };
            out.push(ExactRef {
                file: d.relative_path.replace('\\', "/"),
                line: l as u32 + 1,
                name: symbol_name(&o.symbol),
                def_file: def_file.replace('\\', "/"),
                def_line: *def_line,
            });
        }
    }
    Ok(out)
}

/// Last descriptor name of a SCIP symbol string.
pub fn symbol_name(sym: &str) -> String {
    match scip::symbol::parse_symbol(sym) {
        Ok(s) => s
            .descriptors
            .last()
            .map(|d| d.name.clone())
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// A precise indexer that can run on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexer {
    pub name: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
}

fn on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in ["", ".exe", ".cmd"] {
            let p = dir.join(format!("{program}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Indexers whose toolchain is present and whose language the repo contains. Nothing is
/// downloaded: an indexer that is not installed is simply skipped.
pub fn detect(root: &Path, has_ts: bool, has_py: bool, has_rust: bool) -> Vec<Indexer> {
    let mut v = Vec::new();
    if has_rust
        && root.join("Cargo.toml").is_file()
        && let Some(p) = on_path("rust-analyzer")
        && Command::new(&p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    {
        v.push(Indexer {
            name: "rust-analyzer",
            program: p,
            args: vec!["scip".into(), ".".into()],
        });
    }
    if has_ts && let Some(p) = on_path("scip-typescript") {
        v.push(Indexer {
            name: "scip-typescript",
            program: p,
            args: vec!["index".into(), "--infer-tsconfig".into()],
        });
    }
    if has_py && let Some(p) = on_path("scip-python") {
        v.push(Indexer {
            name: "scip-python",
            program: p,
            args: vec!["index".into(), ".".into()],
        });
    }
    v
}

/// Run one indexer in `root`, writing `index.scip` into `out_dir`. Returns the file's bytes.
pub fn run(ix: &Indexer, root: &Path, out_dir: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::create_dir_all(out_dir)?;
    let out = out_dir.join(format!("{}.scip", ix.name));
    let mut cmd = Command::new(&ix.program);
    cmd.current_dir(root).args(&ix.args);
    match ix.name {
        "rust-analyzer" => {
            cmd.arg("--output").arg(&out);
        }
        _ => {
            cmd.arg("--output").arg(&out);
        }
    }
    let st = cmd.output()?;
    anyhow::ensure!(
        st.status.success(),
        "{} failed: {}",
        ix.name,
        String::from_utf8_lossy(&st.stderr)
            .lines()
            .last()
            .unwrap_or("")
    );
    Ok(std::fs::read(&out)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scip::types::{Document, Index, Occurrence, SymbolRole};

    fn occ(line: i32, sym: &str, def: bool) -> Occurrence {
        let mut o = Occurrence::new();
        o.range = vec![line, 0, 5];
        o.symbol = sym.into();
        o.symbol_roles = if def {
            SymbolRole::Definition as i32
        } else {
            0
        };
        o
    }

    #[test]
    fn pairs_references_with_definitions() {
        let sym = "scip-typescript npm demo 1.0.0 src/`b.ts`/create().";
        let mut a = Document::new();
        a.relative_path = "src/a.ts".into();
        a.occurrences = vec![occ(4, sym, false), occ(5, "local 1", false)];
        let mut b = Document::new();
        b.relative_path = "src/b.ts".into();
        b.occurrences = vec![occ(2, sym, true)];
        let mut idx = Index::new();
        idx.documents = vec![a, b];
        let refs = read_index(&idx.write_to_bytes().unwrap()).unwrap();
        assert_eq!(
            refs,
            vec![ExactRef {
                file: "src/a.ts".into(),
                line: 5,
                name: "create".into(),
                def_file: "src/b.ts".into(),
                def_line: 3
            }]
        );
    }
}
