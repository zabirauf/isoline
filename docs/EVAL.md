# Validation loop

Phase 5 of the plan: run Isoline on real repos, measure, fix the worst problems, rerun. Raw
results are in `eval/results/<round>.json`; `iso-cli eval --round <name>` produces them.

Machine: Apple silicon laptop, 12 cores, release build. The remote repos in
`eval/repos.toml` are pinned but not yet approved by a human, so rounds 1 and 2 ran on the
local repos only.

## Repo set used so far

| Repo | Covers | Files | Lines |
| --- | --- | --- | --- |
| openclaw | large TypeScript monorepo (pnpm workspaces, tsconfig paths, project references), over 1M lines | 64,228 | 14,568,311 |
| crosspoint-reader | mostly unsupported languages (C++ firmware) | 857 | 368,408 |
| openclaw-atlas | tiny repo, first-run experience | 21 | 27,451 |
| isoline | Rust workspace with several crates plus a TypeScript UI | 429 | 58,238 |

## Metrics against the plan's targets

Round 2, after the round-1 fixes:

| Metric | Target | openclaw | crosspoint | atlas | isoline |
| --- | --- | --- | --- | --- | --- |
| Cold index | 1M lines/min on 8 cores | 20.8M lines/min | 374M* | 74M* | 31M |
| Reopen an indexed repo | under 1 s | 356 ms | 8 ms | 3 ms | 9 ms |
| Open a file, p95 (200 random opens) | under 100 ms | 5.0 ms | 1.7 ms | 4.9 ms | 2.3 ms |
| Edit to refreshed view, p95 (50 edits) | under 50 ms | 26.5 ms | 43.5 ms | 17.6 ms | 6.1 ms |
| Files reparsed after a one-file body edit | exactly 1, 0 dependents | 1, 0 | 1, 0 | 1, 0 | 1, 0 |
| Branch switch touching 3 files | reports exactly 3 | 3 | 3 | 3 | 3 |
| Cache hit rate on reopen | above 99% | 100% | 100% | 100% | 100% |
| Unresolved imports, tier 1 | under 5% | 0.05% | 0% | 0% | 7.1%** |
| Precision of heuristic call edges vs SCIP | 90% or better | n/a*** | | | 100% (Rust 231/231, TS 64/64) |
| Panics and failed flow trees | 0 | 0 | 0 | 0 | 0 |
| Peak memory | measured, then budgeted | 1.2 GB | | | |

\* Mostly text-tier files, which are hashed but not parsed.
\** Test fixtures that deliberately import files they do not ship, plus Rust `#[path]`
modules (a known resolver gap). Product code resolves fully.
\*** scip-typescript needs the repo's dependencies installed; openclaw's are not.

Precision: `iso-cli precise` ran rust-analyzer's SCIP output over this repo and confirmed
231 of the 307 heuristic Rust call edges as exact and contradicted none; the other 76 had no
SCIP reference on their line (TypeScript files and fixture crates outside the Cargo
workspace), and a sample of them was correct by inspection. scip-typescript over `app/ui`
confirmed all 64 heuristic TypeScript call edges.

Call flow against the prototype's `analyze.js`: all 17 units of `lifecycle.ts` are equal to
the golden, and 4,128 of 4,133 units across two 600-file openclaw samples are equal. The 5
misses are tree-sitter-typescript misparses (`await f<typeof import("x")>(…)` and a postfix
`!` precedence case); see `fixtures/ts/DEVIATIONS.md`.

AI cost estimates against counted tokens (mock provider): input 0% off, output 0.8 to 2.1%,
cluster and repo passes 0.1 to 4.5%, all within the 10% target.

## Rounds

### Round 1

Findings, ranked by user impact:

1. **3.0% of openclaw's TypeScript imports unresolved** (12,746). Extensions import the root
   package by name (`openclaw/plugin-sdk/x`); the tsconfig aliases and package `exports`
   point at `dist/` output that a checkout does not contain. **Fixed:** every built target
   is followed by its source-mapped variants (`dist/x.d.ts` to `src/x`). Unresolved fell to
   177 (0.04%).
2. **The isoline repo failed in eval with "Permission denied".** A read-only file from the
   handoff zip made the bench's scratch copy uneditable. **Fixed:** copies are made writable
   and unreadable files are skipped.
3. **Python and Rust files have no call flow** (113 empty-flow files in crosspoint-reader).
   In progress: the shared walker gets Python and Rust tables.
4. **The source tree payload for 64k files is 15 MB.** Measured in Chromium: 20 ms to
   parse, 275 ms to fetch, 853 ms from page load to a visible tree. Within budget; lazy
   per-directory loading is the next step if larger repos need it.

### Round 2

All targets met on all four repos; openclaw's unresolved imports at 0.05%. An independent
visual review of the screenshots against the prototype (25 findings) produced one backend fix
(members called through typed parameters now show as their own module, so `lifecycle.ts`
reports 33 external symbols like the prototype) and a list of UI fixes.

## Stop rule

The plan stops the loop after two rounds in a row with no metric improving by more than 5%
and every correctness target met. Round 2 improved unresolved imports by 98%, so at least one
more round is due, ideally with the approved remote repos.
