# Status against the implementation plan

Plan: `docs/handoff/plan/implementation-plan.md`. Evidence for every number below is in
`docs/EVAL.md` and `eval/results/`.

## Workstreams

| Phase | Workstream | State | Acceptance test |
| --- | --- | --- | --- |
| 0 | Contracts | Done | Workspace, `CONTRACTS.md`, generated TypeScript bindings, CI on three systems |
| 1 | A. Walk, tree, index | Done | Reindexing unchanged files hashes and writes nothing; a three-file branch switch reports exactly three (`crates/iso-engine/tests/pipeline.rs`); history replay below |
| 1 | B. TS and JS | Done | Hand-written fact goldens (TS, TSX, CJS); tsconfig aliases, `extends`, barrels and workspace packages resolve in `fixtures/ts/monorepo` (26 cases) |
| 1 | C. Python | Done | Fact goldens; relative imports across packages (20 cases) |
| 1 | D. Rust, HTML, CSS | Done | Fact goldens; `use crate::a::b` and cross-crate uses in a two-crate workspace (24 cases); links (14 cases) |
| 1 | E. UI and Monaco | Done | Prototype views on mock and real data, Monaco behind `EditorHost`, flow-step line highlight, tier badges; 50 UI tests |
| 1 | F. Copilot sidecar | Done, live test pending | JSON-RPC round trips in mock mode; live Copilot SDK build compiles, never run against Copilot |
| 2 | G. Graph | Done | Layers by longest path over SCC condensation, cycles, impact, clustering (`crates/iso-graph`) |
| 2 | H. Call flow | Done | TS output equals `analyze.js` on `lifecycle.ts` (17 of 17 units) and on a fresh 600-file openclaw sample (2,026 of 2,026); Python and Rust match hand-written goldens |
| 2 | I. AI orchestrator | Done | Second run makes zero calls; one body edit makes one leaf request; estimate within 10% |
| 2 | M. Jobs and cache | Done | Body edit recomputes no dependents; export change recomputes only importers; restart redoes no finished file |
| 2 | N. Fallback tiers | Done | Go outline from query files only; unknown extensions open in the text tier |
| 3 | Integrate | Done | Every IPC command wired in Tauri; `iso-cli` exposes every feature; targets hold on a 14.6M-line repo |
| 4 | J. Modules and colors | Done | User renames survive reruns; colors stable; new files join their directory's module |
| 4 | K. Precise indexers | Done | A known wrong heuristic edge is corrected by a SCIP import (`pipeline.rs`); rust-analyzer and scip-typescript runs on real code |
| 4 | L. Scale rendering | Done | Canvas module map at 5,000 modules, slowest frame 2.2 ms in Chromium (not yet measured on WebView2 and WebKitGTK) |
| 5 | V. Validation loop | Three rounds plus history replay | See `docs/EVAL.md` |

Added after the plan, at the product owner's request: typed parameter signatures with
origin colors, definition peek on hover, go to definition with back and forward, links
without underlines, and Glance / Read / Dig detail levels (`docs/ux/progressive-disclosure.md`).

## Distribution

- **Releases:** `.github/workflows/release.yml` builds the app and `iso-cli` for macOS,
  Windows and Linux on arm64 and x64 (macOS x64 and Windows arm64 are cross-compiled) and
  publishes them with checksums when a `v*` tag is pushed.
- **Nix:** `flake.nix` builds `isoline` (Tauri, from source), `iso-cli` and
  `copilot-sidecar`; `.github/workflows/nix.yml` builds all three on Linux x64, Linux arm64
  and macOS.
- **Demo:** `app/ui/scripts/demo-video.mjs` records the annotated demo against the real
  engine through `iso-cli serve`.
- **Windows:** CI runs the whole test suite on `windows-latest`; this found that
  `std::fs::canonicalize`'s `\\?\` paths broke import resolution, now fixed with `dunce`.

## Decisions that refine the plan

- **One engine crate** (`iso-engine`) behind both the CLI and the app, so they cannot disagree.
- **Flow units per file** rather than per symbol, because controller methods on objects a
  factory returns need the whole file; still computed on demand and cached by content hash.
- **No git-seeded hashes.** A cold index reads every file to parse it anyway, and BLAKE3
  hashes faster than the read. Reopen uses a size and mtime check, then the Merkle tree.
- **tree-sitter stays the parser.** It indexes openclaw at about 20M lines per minute, 20
  times the target, so the Oxc swap the plan held in reserve is not needed. The one grammar
  gap that mattered (`import("x").T` in type positions) is masked before parsing.
- **Remote eval repos are pinned but unapproved** (`eval/repos.toml`); the rounds so far ran
  on local repos.

## What needs a human

These five items from the plan, unchanged:

1. A GitHub OAuth app client id for device-flow sign-in (`ISOLINE_GITHUB_CLIENT_ID`).
2. One live sign-in with a Copilot subscription, and a small credit budget for the first
   real AI passes.
3. Confirmation that Copilot's terms allow bulk background analysis from a third-party app.
4. Approval of the remote validation repos in `eval/repos.toml`.
5. Code-signing identities for macOS and Windows before distributing builds. The current
   `Isoline.app` and `.dmg` are unsigned local builds.

## Known gaps

- Rust `#[path]` module attributes and `cfg` are not followed by the Rust resolver.
- Python resolution cannot tell the standard library from third-party packages (both
  external).
- The 15 MB source-tree payload for a 64k-file repo loads in under a second; lazy
  per-directory loading is the next step for larger repos.
- Large-map frame times are measured in Chromium only.
- Live Copilot mode has never run against Copilot.
