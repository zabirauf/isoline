//! Change review: one request that explains a comparison (a branch, a pull request or the
//! working tree) for a reviewer. The prompt holds what Isoline already knows about the change
//! (changed modules, module links, packages, deleted functions that are still called, the
//! changed symbols of every file) and as much of the patch as fits a token budget, most
//! changed files first. The answer is a summary, the notes a reviewer must see, and one
//! sentence per changed module. Answers are cached by the prompt's hash, so a review is paid
//! for once per version of the change.

use std::collections::HashMap;

use iso_core::hash64;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache::kind;
use crate::engine::{
    AiEngine, Meter, Pass, PassEstimate, Planned, Progress, RunReport, sum_requests,
};
use crate::prompts::{STE_REMINDER, review_system};
use crate::tokens::{estimate_tokens, output_limit};
use crate::{AiError, parse};

/// Upper bound on the review prompt, in estimated tokens.
pub const REVIEW_MAX_PROMPT_TOKENS: u64 = 40_000;
/// Upper bound on one file's patch in the prompt, in characters.
const MAX_PATCH_CHARS: usize = 12_000;
const OUT_TOKENS_REVIEW: u64 = 450;
const OUT_TOKENS_REVIEW_PER_MODULE: u64 = 30;
const MAX_MODULES: usize = 40;
const MAX_COMMITS: usize = 30;
const MAX_NOTES: usize = 10;

/// What the review request is written from. The engine builds it from a comparison.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewInput {
    /// The pull request title, or "base ← head" with the head commit's subject.
    pub title: String,
    /// Commit subjects, newest first.
    pub commits: Vec<String>,
    pub modules: Vec<ReviewModule>,
    /// Module links that changed: "from → to: before → after imports".
    pub links: Vec<String>,
    /// Packages imported or no longer imported, with where.
    pub packages: Vec<String>,
    /// Deleted or re-signatured functions that unchanged files call.
    pub impact: Vec<String>,
    pub files: Vec<ReviewFile>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewModule {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub role: Option<String>,
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewFile {
    pub path: String,
    /// added, modified, deleted, renamed (from <old path>).
    pub change: String,
    /// Module id.
    pub module: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    /// One line per changed symbol: "+ fn run(a)", "- class A", "~ fn b: (x) → (x, y)".
    pub symbols: Vec<String>,
    /// "+ ./store", "- lodash".
    pub imports: Vec<String>,
    /// Unified diff hunks; empty for binary files or when git has none.
    pub patch: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewNoteKind {
    /// Can break callers, data or behavior.
    Risk,
    /// Something to verify or test.
    Check,
    /// A design decision worth knowing.
    Info,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewNote {
    pub kind: ReviewNoteKind,
    pub text: String,
    pub path: Option<String>,
    pub symbol: Option<String>,
}

/// The model's review of a change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeReview {
    pub summary: String,
    pub notes: Vec<ReviewNote>,
    /// Module id and one sentence about what changed in it.
    pub modules: Vec<(String, String)>,
    pub model: String,
    /// Changed files whose patch was left out of the prompt to stay under the token budget.
    pub files_without_patch: u32,
}

/// What a review costs, and how much of the change the prompt holds.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewEstimate {
    pub pass: PassEstimate,
    pub cached: bool,
    pub files_without_patch: u32,
}

struct Built {
    content: String,
    req: crate::CompletionRequest,
    est_out: u64,
    files: Vec<String>,
    modules: Vec<String>,
    files_without_patch: u32,
}

impl AiEngine {
    fn review_request(&self, input: &ReviewInput) -> Built {
        let s = self.settings();
        let mut p = String::from(
            "Task: review\nReview the code change below. Answer with the JSON object described in the instructions.\n\n",
        );
        p.push_str(&format!("Title: {}\n", input.title.trim()));
        if !input.commits.is_empty() {
            p.push_str("Commits:\n");
            for c in input.commits.iter().take(MAX_COMMITS) {
                p.push_str(&format!("- {}\n", clip(c, 160)));
            }
            if input.commits.len() > MAX_COMMITS {
                p.push_str(&format!(
                    "- and {} older commits\n",
                    input.commits.len() - MAX_COMMITS
                ));
            }
        }
        let modules: Vec<&ReviewModule> = input.modules.iter().take(MAX_MODULES).collect();
        let mkey: HashMap<&str, String> = modules
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.as_str(), format!("m{}", i + 1)))
            .collect();
        if !modules.is_empty() {
            p.push_str("\n## Changed modules\n");
            for (i, m) in modules.iter().enumerate() {
                p.push_str(&format!("### m{} {}\n", i + 1, m.name));
                p.push_str(&format!(
                    "dir: {}\nchanged files: {}, lines +{} -{}\n",
                    if m.dir.is_empty() { "/" } else { &m.dir },
                    m.files,
                    m.additions,
                    m.deletions
                ));
                if let Some(r) = m.role.as_deref().filter(|r| !r.is_empty()) {
                    p.push_str(&format!("role: {}\n", clip(r, 200)));
                }
            }
        }
        let section = |p: &mut String, title: &str, lines: &[String], max: usize| {
            if lines.is_empty() {
                return;
            }
            p.push_str(&format!("\n## {title}\n"));
            for l in lines.iter().take(max) {
                p.push_str(&format!("- {}\n", clip(l, 240)));
            }
            if lines.len() > max {
                p.push_str(&format!("- and {} more\n", lines.len() - max));
            }
        };
        section(&mut p, "Module links that changed", &input.links, 30);
        section(&mut p, "Packages", &input.packages, 20);
        section(
            &mut p,
            "Deleted or changed functions that unchanged files still call",
            &input.impact,
            15,
        );

        // Files: headers for all, patches for the most changed ones while the budget lasts.
        let mut order: Vec<usize> = (0..input.files.len()).collect();
        let weight =
            |f: &ReviewFile| f.symbols.len() as u64 * 40 + (f.additions + f.deletions) as u64;
        order.sort_by(|&a, &b| {
            weight(&input.files[b])
                .cmp(&weight(&input.files[a]))
                .then(input.files[a].path.cmp(&input.files[b].path))
        });
        let mut budget = REVIEW_MAX_PROMPT_TOKENS.saturating_sub(estimate_tokens(&p) + 2_000);
        // Headers take at most half the budget, most changed files first: with their symbols
        // and imports while that fits, then the line alone, then not at all.
        let mut header = vec![Header::Omit; input.files.len()];
        let mut header_budget = budget / 2;
        for &i in &order {
            let f = &input.files[i];
            for mode in [Header::Full, Header::Short] {
                let t = estimate_tokens(&file_header(f, "f0000", &mkey, mode)) + 8;
                if t <= header_budget {
                    header_budget -= t;
                    header[i] = mode;
                    break;
                }
            }
        }
        budget -= budget / 2 - header_budget;
        let mut with_patch = vec![false; input.files.len()];
        for &i in &order {
            let f = &input.files[i];
            if f.patch.is_empty() || header[i] == Header::Omit {
                continue;
            }
            let t = estimate_tokens(&clip(&f.patch, MAX_PATCH_CHARS));
            if t <= budget {
                budget -= t;
                with_patch[i] = true;
            }
        }
        p.push_str("\n## Files\n");
        let files: Vec<String> = input.files.iter().map(|f| f.path.clone()).collect();
        let mut without = 0u32;
        let mut unlisted = 0u32;
        for (i, f) in input.files.iter().enumerate() {
            if header[i] == Header::Omit {
                unlisted += 1;
                continue;
            }
            p.push_str(&file_header(f, &format!("f{}", i + 1), &mkey, header[i]));
            if with_patch[i] {
                p.push_str("```diff\n");
                p.push_str(&clip(&f.patch, MAX_PATCH_CHARS));
                if !p.ends_with('\n') {
                    p.push('\n');
                }
                p.push_str("```\n");
            } else if !f.patch.is_empty() {
                without += 1;
                p.push_str("(diff left out: the change is too large)\n");
            }
        }
        if unlisted > 0 {
            p.push_str(&format!(
                "(and {unlisted} more changed files with fewer changes, not listed)\n"
            ));
            without += unlisted;
        }
        p.push_str(STE_REMINDER);
        let content = format!("review1:{:016x}", hash64(p.as_bytes()));
        let est_out = OUT_TOKENS_REVIEW + OUT_TOKENS_REVIEW_PER_MODULE * modules.len() as u64;
        let req = crate::CompletionRequest {
            model: s.repo_model.clone(),
            system: review_system(),
            prompt: p,
            max_tokens: output_limit(est_out, s.max_output_tokens),
        };
        Built {
            content,
            req,
            est_out,
            files,
            modules: modules.iter().map(|m| m.id.clone()).collect(),
            files_without_patch: without,
        }
    }

    /// The cached review of this change, if any.
    pub fn cached_review(&self, input: &ReviewInput) -> Option<ChangeReview> {
        let b = self.review_request(input);
        self.cache.get_json(&b.content, &b.req.model, kind::REVIEW)
    }

    /// What reviewing this change costs.
    pub fn estimate_review(&self, input: &ReviewInput) -> ReviewEstimate {
        let b = self.review_request(input);
        let cached = self.cache.contains(&b.content, &b.req.model, kind::REVIEW);
        let pass = if cached {
            PassEstimate {
                model: b.req.model.clone(),
                items: 1,
                cached: 1,
                ..Default::default()
            }
        } else {
            let price = self.price(&b.req.model);
            let mut e = sum_requests(
                &price,
                self.calibration(),
                &[Planned {
                    request: &b.req,
                    est_out: b.est_out,
                }],
            );
            e.items = 1;
            e
        };
        ReviewEstimate {
            pass,
            cached,
            files_without_patch: b.files_without_patch,
        }
    }

    /// Reviews the change: from the cache when this version was reviewed before. Refuses when
    /// the estimate is over the budget.
    pub fn review(
        &self,
        input: &ReviewInput,
        budget_usd: f64,
    ) -> Result<(ChangeReview, RunReport), AiError> {
        let b = self.review_request(input);
        let model = b.req.model.clone();
        if let Some(r) = self
            .cache
            .get_json::<ChangeReview>(&b.content, &model, kind::REVIEW)
        {
            let mut rep = RunReport::new(Pass::Repo, &model);
            rep.items = 1;
            rep.cached = 1;
            return Ok((r, rep));
        }
        let price = self.price(&model);
        let est = sum_requests(
            &price,
            self.calibration(),
            &[Planned {
                request: &b.req,
                est_out: b.est_out,
            }],
        );
        if est.cost_usd > budget_usd + 1e-12 {
            return Err(AiError::Budget(format!(
                "the review is estimated at ${:.4}, over the budget of ${:.4}",
                est.cost_usd, budget_usd
            )));
        }
        let mut sink = |_: &Progress| {};
        let mut m = Meter::new(
            Pass::Repo,
            price,
            self.calibration(),
            budget_usd,
            1,
            &mut sink,
        );
        m.report.items = 1;
        let c = self.provider.complete(&m.capped(&b.req))?;
        m.record(&b.req, &c);
        let v = parse::extract_json(&c.text)
            .map_err(|e| AiError::Provider(format!("the review answer could not be read: {e}")))?;
        let review = parse_review(&v, &b.files, &b.modules, &c.model, b.files_without_patch)
            .map_err(|e| AiError::Provider(format!("the review answer could not be read: {e}")))?;
        let _ = self.cache.put_json(
            &b.content,
            &model,
            kind::REVIEW,
            &review,
            c.tokens_in,
            c.tokens_out,
        );
        m.report.stored = 1;
        Ok((review, m.report))
    }
}

fn file_header(f: &ReviewFile, id: &str, mkey: &HashMap<&str, String>, mode: Header) -> String {
    let mut h = format!("### {id} {} ({}", f.path, f.change);
    if let Some(m) = f.module.as_deref().and_then(|m| mkey.get(m)) {
        h.push_str(&format!(", module {m}"));
    }
    h.push_str(&format!(", +{} -{})\n", f.additions, f.deletions));
    if mode == Header::Short {
        return h;
    }
    for s in f.symbols.iter().take(40) {
        h.push_str(&format!("symbol: {}\n", clip(s, 200)));
    }
    if f.symbols.len() > 40 {
        h.push_str(&format!("symbol: and {} more\n", f.symbols.len() - 40));
    }
    for i in f.imports.iter().take(20) {
        h.push_str(&format!("import: {}\n", clip(i, 160)));
    }
    h
}

/// How much of a file's header goes into the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Header {
    /// Path, change, module, line counts, changed symbols and imports.
    Full,
    /// The first line only.
    Short,
    /// Not listed: counted in a closing line.
    Omit,
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…", &s[..end])
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Reads the answer. File ids (`f3`) become paths and module ids (`m2`) become module ids;
/// notes about files that are not in the change keep their text without the path.
fn parse_review(
    v: &Value,
    files: &[String],
    modules: &[String],
    model: &str,
    files_without_patch: u32,
) -> Result<ChangeReview, String> {
    let summary = text(v, "summary").ok_or("no summary")?;
    let file_of = |k: &str| {
        let k = k.trim();
        k.strip_prefix('f')
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| files.get(n.wrapping_sub(1)))
            .or_else(|| files.iter().find(|p| p.as_str() == k))
            .cloned()
    };
    let module_of = |k: &str| {
        k.trim()
            .strip_prefix('m')
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| modules.get(n.wrapping_sub(1)))
            .cloned()
    };
    let notes = v
        .get("notes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|n| {
                    let t = text(n, "text")?;
                    let kind = match text(n, "kind").as_deref() {
                        Some("risk") => ReviewNoteKind::Risk,
                        Some("check") => ReviewNoteKind::Check,
                        _ => ReviewNoteKind::Info,
                    };
                    Some(ReviewNote {
                        kind,
                        text: t,
                        path: text(n, "file").as_deref().and_then(file_of),
                        symbol: text(n, "symbol"),
                    })
                })
                .take(MAX_NOTES)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut notes = notes;
    // Risks first, then checks, then the rest, each in the model's order.
    notes.sort_by_key(|n| match n.kind {
        ReviewNoteKind::Risk => 0,
        ReviewNoteKind::Check => 1,
        ReviewNoteKind::Info => 2,
    });
    let mods = v
        .get("modules")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| Some((module_of(&text(m, "id")?)?, text(m, "text")?)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(ChangeReview {
        summary,
        notes,
        modules: mods,
        model: model.to_string(),
        files_without_patch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answers_map_ids_to_paths_and_modules() {
        let v = json!({
            "summary": "Adds a cache.",
            "notes": [
                {"kind": "info", "text": "Uses a map.", "file": "f2"},
                {"kind": "risk", "text": "Deletes run.", "file": "f1", "symbol": "run"},
                {"kind": "check", "text": "Test it.", "file": "f9"},
            ],
            "modules": [{"id": "m1", "text": "Gets a cache."}, {"id": "m7", "text": "Unknown."}],
        });
        let r = parse_review(
            &v,
            &["a.ts".into(), "b.ts".into()],
            &["core".into()],
            "m",
            0,
        )
        .unwrap();
        assert_eq!(r.notes[0].kind, ReviewNoteKind::Risk);
        assert_eq!(r.notes[0].path.as_deref(), Some("a.ts"));
        assert_eq!(r.notes[0].symbol.as_deref(), Some("run"));
        assert_eq!(r.notes[1].path, None);
        assert_eq!(r.notes[2].path.as_deref(), Some("b.ts"));
        assert_eq!(
            r.modules,
            vec![("core".to_string(), "Gets a cache.".to_string())]
        );
    }

    #[test]
    fn large_changes_keep_every_file_and_drop_patches() {
        let e = AiEngine::in_memory(std::sync::Arc::new(crate::mock::MockProvider::new())).unwrap();
        let big = "+x\n".repeat(4_000);
        let input = ReviewInput {
            title: "Big".into(),
            files: (0..60)
                .map(|i| ReviewFile {
                    path: format!("src/f{i}.ts"),
                    change: "modified".into(),
                    additions: 4_000,
                    patch: big.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let b = e.review_request(&input);
        assert!(estimate_tokens(&b.req.prompt) <= REVIEW_MAX_PROMPT_TOKENS);
        assert!(b.files_without_patch > 0);
        assert!(b.req.prompt.contains("src/f59.ts"));
    }

    #[test]
    fn thousands_of_files_stay_under_the_budget() {
        let e = AiEngine::in_memory(std::sync::Arc::new(crate::mock::MockProvider::new())).unwrap();
        let input = ReviewInput {
            title: "Huge".into(),
            files: (0..5000)
                .map(|i| ReviewFile {
                    path: format!("src/very/long/directory/name/number/{i}/file.ts"),
                    change: "modified".into(),
                    additions: (i % 50) as u32,
                    symbols: (0..30)
                        .map(|k| {
                            format!("~ fn f{k}: signature (a: string) → (a: string, b: number)")
                        })
                        .collect(),
                    patch: "+x\n".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let b = e.review_request(&input);
        assert!(estimate_tokens(&b.req.prompt) <= REVIEW_MAX_PROMPT_TOKENS);
        assert!(
            b.req
                .prompt
                .contains("more changed files with fewer changes")
        );
        assert!(b.files_without_patch > 0);
    }
}
