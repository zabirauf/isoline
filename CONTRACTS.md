# Isoline contracts

Frozen at the end of phase 0. Change them only through the lead: an agent that needs a change
stops and reports the exact change it needs.

The build brief is `docs/handoff/plan/implementation-plan.md`. This file records what phase 0
actually froze, including the places where it refines the plan.

## Repo layout

```text
crates/
  iso-core      shared types only, no IO                       (lead)
  iso-index     SQLite schema, migrations, typed queries       (lead)
  iso-walk      walk, content hash, Merkle tree, watcher       (lead)
  iso-parse     parser pool, extractors, call-flow walker      (see ownership below)
  iso-resolve   import resolution, one module per language     (see ownership below)
  iso-graph     layers, cycles, clusters, impact               (lead)
  iso-cache     versioned content-addressed store              (lead)
  iso-jobs      priority background queue                      (lead)
  iso-ai        Provider trait, skeletons, budget, passes      (workstream I)
  iso-git       git and gh through their command lines         (lead)
  iso-engine    wires the crates into operations               (lead)
  iso-cli       headless commands                              (lead)
sidecar/copilot Go: Copilot SDK behind JSON-RPC on stdio       (workstream F)
app/src-tauri   Tauri commands, thin wrappers over iso-engine  (lead)
app/ui          React + TypeScript + Vite, Monaco              (workstream E)
fixtures/<lang>/<case>/   source files + expected/*.json
tools/analyze   the prototype's analyze.js, generic copy, for TS goldens
eval/           repos.toml, results/<round>.json, reports
```

`iso-engine` is an addition to the plan: `iso-cli` and `app/src-tauri` both call it, so the
app and the headless tool can never disagree.

## File ownership

| Workstream | Owns |
| --- | --- |
| Lead | everything not listed below; `iso-parse/src/lib.rs`, `iso-parse/src/lang/mod.rs`, `iso-parse/src/util.rs` (agents may append helpers at the end of `util.rs`), `iso-resolve/src/lib.rs` |
| B + H. TS/JS and call flow | `iso-parse/src/lang/ts.rs`, `iso-parse/src/lang/js.rs`, `iso-parse/src/flow/**`, `iso-parse/queries/{typescript,javascript}/**`, `iso-resolve/src/js.rs`, `fixtures/ts/**`, `fixtures/js/**`, `tools/analyze/**`, `iso-parse/tests/ts_*.rs`, `iso-parse/tests/js_*.rs`, `iso-parse/tests/flow_*.rs`, `iso-resolve/tests/js_*.rs` |
| C + D + N. Python, Rust, HTML, CSS, generic | `iso-parse/src/lang/{python,rust,html,css,generic}.rs`, `iso-parse/queries/{python,rust,html,css,go,java,csharp}/**`, `iso-resolve/src/{python,rust,html}.rs`, `fixtures/{python,rust,html,css,go,java,csharp,text}/**`, `iso-parse/tests/{python,rust,html,css,generic}_*.rs`, `iso-resolve/tests/{python,rust,html}_*.rs` |
| E. UI | `app/ui/**` except `app/ui/src/ipc/bindings/**` (generated) |
| F + I. Sidecar and AI | `sidecar/copilot/**`, `crates/iso-ai/**` (the `Provider` trait and its request/response types are frozen) |

Each agent works in its own git worktree and branch. Done means: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings` and `cargo test` pass for the crates touched, and the
agent's golden files match.

## 1. Core types (`iso-core`)

- `FileId(u64)` = hash64 of the repo-relative path with `/` separators.
  `SymbolId(u64)` = hash64 of path + qualified name + kind. Both serialize as 16-char hex
  strings on the wire, because JavaScript numbers cannot hold a u64.
- `Language` (26 values) with `tier()`, `monaco_id()`, `detect(path, first_line)`.
- `Tier`: `full` (TS, TSX, JS, JSX, Python, Rust, HTML, CSS), `outline` (Go, Java, C#),
  `text` (everything else).
- `FileFacts { symbols, imports, calls, refs, has_errors }`. `refs` is an addition to the
  plan: references from a symbol's body to file-level names (imports and local
  declarations), with a `type_position` flag. The cards' "Imports used" and "type-only"
  chips and `TypeUse` edges come from it.
- `SymbolFact.qualified_name` is unique per file and kind: `Class.method`, `outer.inner`.
- `Confidence { exact, heuristic, inferred }`, `EdgeKind { import, call, typeuse }`,
  `Target { symbol, external, unresolved }`, `Edge`.
- Lines are 1-based everywhere.

### Call flow

`FlowItem` and `FlowUnit` serialize to exactly the JSON shape of the prototype's
`analyze.js`, so golden files compare with `serde_json::Value` equality:

```json
{"t":"call","label":"foo","target":{"kind":"fn","id":"foo"},"line":3,"await":false,"optional":false,"argOf":null,"inner":[]}
{"t":"block","kind":"cond","kw":"if","label":"(x)","line":4,"children":[]}
{"t":"exit","kind":"return","label":"x","line":5,"early":false}
{"id":"f","owner":"f","method":null,"start":1,"end":9,"async":false,"params":["a"],"tree":[]}
```

Flow targets carry the prototype's kinds (`fn`, `ext`, `method`, `local`, `param`, `builtin`,
`other`) rather than the plan's `Target` enum, because the view colors steps by those kinds.

**Golden rule for TS:** `fixtures/ts/**/expected/flows.json` is produced by
`node tools/analyze/analyze.js <file>`. That copy differs from the prototype's analyzer by one
deleted line: the prototype hard-coded `harness.runAttempt` and `harness.classify` as external
calls for its one input file. The prototype's original output is kept as
`fixtures/ts/lifecycle/expected/flows.prototype.json`; the only differences are those two
targets (`ext` in the prototype, `method` in the generic golden).

## 2. Extractor interface (`iso-parse`)

```rust
pub trait Extractor: Send + Sync {
    fn language(&self) -> Language;
    fn extract(&self, path: &Path, source: &str, tree: &tree_sitter::Tree) -> FileFacts;
    fn flow_units(&self, path: &Path, source: &str, tree: &tree_sitter::Tree) -> Vec<FlowUnit>; // default: empty
}
pub fn grammar(lang) -> Option<tree_sitter::Language>;
pub fn parse(lang, source, old_tree) -> Option<Tree>;   // per-thread parser pool
pub fn extractor(lang) -> Option<&'static dyn Extractor>;
pub fn extract_file(path, lang, source) -> FileFacts;
pub fn flow_units(path, lang, source) -> Vec<FlowUnit>;
```

The plan's per-symbol `flow(source, tree, symbol)` became per-file `flow_units`, because unit
discovery (methods on a controller object a factory returns) needs the whole file. It still
runs on demand when a file is opened, and its result is cached by content hash.

Extractor conventions:

- `exported`: TS/JS `export`, Python public module-level names (no leading `_`, or listed in
  `__all__`), Rust `pub`, CSS/HTML always false.
- `signature`: declaration head up to the body, whitespace collapsed, at most 200 chars.
- `doc`: JSDoc / docstring / `///` text without markers.
- `calls[].caller` and `refs[].from`: qualified name of the innermost enclosing
  function or method, or `None` at top level.
- Callables (function cards) are `function` and `method` symbols. Arrow functions and
  function expressions bound to a top-level `const` are `function` symbols.

## 3. Index schema (`iso-index`)

`crates/iso-index/migrations/0001_init.sql` is the schema: `files`, `tree_nodes`, `symbols`,
`imports`, `edges`, `modules`, `file_modules`, `derived`, `jobs`, `meta`. `ai_cache` lives in
a database shared by all repos (`migrations/ai_cache.sql`). Additions to the plan:
`files.lines`, `files.facts_hash`, `files.exports_hash` (early cutoff), `files.status`,
`imports.external`, `imports.unresolved`, `edges.from_file`, `modules.dir`.

Databases live under the app data folder: `<data>/repos/<hash of root>/index.sqlite` and
`<data>/ai_cache.sqlite`. `ISOLINE_DATA_DIR` overrides `<data>` (tests and eval use it).

## 4. IPC commands (`app/src-tauri`)

Payload types are in `crates/iso-core/src/ipc.rs`; TypeScript bindings are generated into
`app/ui/src/ipc/bindings/` by `cargo test -p iso-core`. Field names are camelCase.

| Command | Args | Returns | Events |
| --- | --- | --- | --- |
| `open_repo` | `path` | `RepoSummary`; starts background indexing | `index_progress: IndexProgress`, `index_done: RepoSummary` |
| `get_tree` | | `TreeNode` (root) | |
| `get_file_view` | `path` | `FileView` | |
| `get_flow` | `path`, `unit` | `FlowUnit` | |
| `get_source` | `path`, `start?`, `end?` | `SourceSlice` | |
| `get_impact` | `symbol` (hex id), `transitive` | `Impact` | |
| `get_module_map` | | `ModuleMap` | |
| `get_tree_packed` | | `TreePacked` | The tree of `get_tree` in parallel arrays (a sixth of the size); the UI loads this one. |
| `get_architecture` | | `Architecture \| null` | The last repo pass: overview, layers and major flows. Never calls a model. |
| `search` | `query`, `limit?` | `SearchHit[]` | |
| `ai_status` / `ai_sign_in` / `ai_sign_out` | | `AiStatus` | |
| `ai_estimate` | `scope`, `path?` | `AiEstimate` | |
| `ai_run` | `scope`, `path?`, `budgetUsd` | `AiRunHandle` | `ai_progress: AiProgress`, `summary_ready: { path }`, `ai_job_done: { jobId, stored, failed, costUsd, error }` once per run. Scope `architecture` skips file summaries: cluster and repo passes only. |
| `ai_enable` | `enabled` | `AiStatus` | |
| `ai_review` | `comparison` (as `compare` returned it) | `ReviewState` | The cached review of the change, else its estimate. Never calls a model. |
| `ai_review_run` | `comparison`, `budgetUsd` | `ChangeReview` | One request (the review model) with the change's facts and as much of the diff as fits 40k tokens; cached by prompt hash. |
| `ai_config` | | `AiConfig` | Each task's model (summaries, modules, architecture, review) and system prompt with its default, and the models to choose from. Lists the account's models when signed in; sends no code. |
| `ai_set_config` | `update: AiConfigUpdate` | `AiConfig` | Saves models, prompts (null: the default) and Economy to `<data>/ai_settings.json`, for every repo. |
| `ai_requests` | | `AiRequestRow[]` | Model requests of this session, newest first (in memory, at most 300). |
| `ai_request` | `id` | `AiRequestDetail \| null` | One request with its system message, prompt and answer. |
| `ai_requests_clear` | | `null` | Empties the request log. |
| `get_jobs` | | `JobsState` | `job_progress: JobsState` |
| `rename_module` | `id`, `name?`, `color?` | `ModuleInfo` | |
| `recent_repos` | | `string[]` | |
| `git_status` | | `GitStatus` (`isRepo: false` outside git) | `git_changed: GitStatus` |
| `git_refs` | | `GitRefs` | |
| `git_switch` | `name`, `create` | `GitStatus`; reindexes the files that differ | `git_changed`, `files_changed`, `tree_changed`, `index_done` |
| `git_fetch` | | `GitStatus` | `git_changed` |
| `git_prs` | | `PullRequests` (`available: false` and a `message` without `gh`) | |
| `compare` | `base`, `head`, `threeDot`, `pr?` | `Comparison` | `packages`: packages that changed files start or stop importing, with how many unchanged files import them. |
| `compare_file` | `base`, `head`, `path`, `oldPath?` | `FileDiffText` | |

`tree_changed` (no payload) fires when the set of files changes: right after the first walk
lists a new repo's files as pending, long before parsing ends, and when files are added or
deleted. The UI refetches `get_tree` on it.

`compare` takes any revision git understands for `base` and `head`; `head` may be `worktree`
for uncommitted changes, and `pr` (a pull request number, through `gh`) replaces both. With
`threeDot` the base side is the merge base of the two, as a pull request shows it. Nothing is
checked out: both sides are read with `git cat-file --batch` and parsed with the index's
extractors. `compare_file` takes the resolved `Comparison.base.sha` and `head.sha`.

`get_flow` takes a path and a unit id (`fn`, `Class.method`, `factory.method`) rather than a
`SymbolId`, because controller methods are units but not symbols.

## 5. Cache keys and job kinds

`iso_core::cache`: `derived_key(kind, content_hash, version)` for `facts`, `flow`, `graph`;
`ai_key(content_hash, prompt_version, model, kind)` for the AI cache. When a task has a
prompt the user wrote, `iso-ai` appends `~p<prompt hash>` to the content hash of that task's
kinds, so its answers never mix with the default prompt's. `EXTRACTOR_VERSION`
and `FLOW_WALKER_VERSION` invalidate only their own rows.

`iso_core::jobs`: `JobKind { parse, resolve, graph, precise, ai_leaf, ai_cluster, ai_repo }`,
`Priority { VisibleFile, Neighbors, ExpandedOrRecent, Repo, Precise, Ai }`.

## 6. Sidecar protocol (`sidecar/copilot` ↔ `iso-ai`)

Newline-delimited JSON-RPC 2.0 on the sidecar's stdin/stdout; logs go to stderr. Every reply
that used a model carries token counts. Methods:

| Method | Params | Result |
| --- | --- | --- |
| `status` | `{}` | `{ signedIn, user?, mock, version }` |
| `signIn` | `{ clientId? }` | `{ userCode, verificationUri, expiresIn }` (device flow starts) |
| `signInPoll` | `{}` | `{ signedIn, user? }` |
| `signOut` | `{}` | `{ signedIn: false }` |
| `listModels` | `{}` | `{ models: ModelInfo[] }` |
| `complete` | `CompletionRequest` | `Completion` |
| `shutdown` | `{}` | `{}` |

`--mock` (or `ISOLINE_SIDECAR_MOCK=1`) answers every method without a network call, with
deterministic text and token counts of `ceil(chars / 4)`. All built-in SDK tools are
excluded and the system message is replaced in live mode, so the model never reads the repo.

## Rules that apply to everyone

- No model calls in tests. No network outside the sidecar.
- The UI never parses code; it receives ready-to-render data.
- Golden files are written first, by hand or by the reference analyzer, and never
  regenerated just to make a test pass.
