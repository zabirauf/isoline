# iso-ai

AI orchestration for Isoline: skeletons instead of source, a shared cache, budgets, and the
leaf, cluster and repo passes. Every model call goes through the frozen `Provider` trait.
Tests use `MockProvider`; the app uses `SidecarProvider`, which talks to the Go Copilot
sidecar in `sidecar/copilot` (CONTRACTS.md section 6).

## Quick start for the engine

```rust
use std::sync::Arc;
use iso_ai::{AiEngine, AiSettings, FileInput, SidecarConfig, SidecarProvider};
use iso_core::ipc::AiScope;

let provider = Arc::new(SidecarProvider::new(
    SidecarConfig::new(sidecar_path).mock(std::env::var("ISOLINE_SIDECAR_MOCK").is_ok()),
));
let ai = AiEngine::open(&data_dir.join("ai_cache.sqlite"), provider.clone())?;
ai.set_settings(AiSettings { economy: repo_settings.economy, ..AiSettings::default() }
    .fitted_to(&ai.refresh_models().unwrap_or_default()));

// Build one FileInput per file from the index: path, lang, content hash, facts, flow units,
// and the source (only needed for files with callables that have no cached summary).
let est = ai.estimate(&files, &modules, &edges, AiScope::Repo);   // -> ipc::AiEstimate via est.to_ipc()
let report = ai.run(&files, &modules, &edges, &stored_colors, AiScope::Repo, budget_usd,
                    &mut |p| emit("ai_progress", p.to_ipc(job_id)))?;
for path in &report.leaf.updated_paths { emit("summary_ready", path); }

// Opening a file never waits on a model: serve whatever is cached.
let cached = ai.cached_file(&file_input);       // summaries, type sentences, overview
```

## Public API

### Inputs (plain structs, built from iso-core types)

| Type | Fields |
| --- | --- |
| `FileInput` | `path`, `lang`, `content_hash`, `source`, `facts: FileFacts`, `flows: Vec<FlowUnit>` |
| `ModuleInput` | `id`, `dir`, `files` (repo-relative paths) |
| edges | `iso_core::ipc::ModuleEdge { from, to, weight }`, `from` imports `to` |
| existing colors | `HashMap<module id, css color>`: stored or user-picked colors, always kept |

### `AiEngine`

| Method | What it does |
| --- | --- |
| `open(cache_path, provider)`, `in_memory(provider)`, `with_cache(cache, provider)` | One engine per app, one shared cache for all repos |
| `settings()`, `set_settings(AiSettings)` | Models per pass, Economy, batch sizes, skeleton limits |
| `refresh_models()` | Calls `listModels`, keeps prices. Called lazily the first time a cost is needed, so fully cached runs make no provider call at all |
| `price(model)` | `ModelInfo` for a model id (listed, else the analysis plan's defaults) |
| `estimate(files, modules, edges, scope)` | `Estimate { leaf, cluster?, repo? }`; `to_ipc()` gives `AiEstimate`. File scope is the leaf pass, module scope adds the cluster pass, repo scope adds the repo pass |
| `estimate_leaf(files)`, `estimate_cluster(..)`, `estimate_repo(..)` | One pass each |
| `run(files, modules, edges, colors, scope, budget_usd, on_progress)` | The passes a scope needs, in order, sharing one budget. Returns `FullReport` |
| `run_leaf(files, budget_usd, on_progress)` | Function and method summaries, type sentences, file overviews |
| `cluster_pass(modules, edges, files, colors, budget_usd, on_progress)` | Name, role, responsibilities and color for every module (`ClusterReport.modules`) |
| `repo_pass(module_summaries, edges, budget_usd, on_progress)` | Architecture overview, layer names, color legend |
| `cached_summary(&SymbolFact)` | `Option<ipc::Summary>` |
| `cached_file(&FileInput)` | `CachedFile { summaries, types, overview }`; facts are enough, the source may be empty |
| `cached_modules(modules, edges, files, colors)` | Module summaries from the cache only. Modules without an answer get a name from their directory and `ai: false`; colors are always assigned |
| `cached_repo(module_summaries, edges)` | `Option<RepoOverview>` |

Every run refuses with `AiError::Budget` when its estimate exceeds the budget, checks the
budget again before each request, and lowers each request's `max_tokens` so a longer answer
than expected cannot take the spend past the budget. A run that stops early returns its
report with `stopped_by_budget: true`. `AiError::NotSignedIn` stops a run; other provider
errors are recorded per item and the run goes on, until three requests in a row fail.

`on_progress` gets a `Progress` after every request and exactly one with `finished: true`.
`Progress::to_ipc(job_id)` gives the `ai_progress` payload.

### `SidecarProvider`

`SidecarProvider::new(SidecarConfig::new(path).mock(bool))` starts nothing until the first
call. It implements `Provider` and adds `status()`, `sign_in(client_id)`, `sign_in_poll()`,
`sign_out()`, `shutdown()`, `is_running()`. Requests carry ids, so threads can call at once.
When the process exits or a call times out, pending calls fail and the next call starts a new
process; more than `max_restarts` starts within `restart_window` fail fast. Dropping the
provider sends `shutdown`. Set `ISOLINE_SIDECAR_LOG=1` to echo the sidecar's stderr.

Sign-in needs a GitHub OAuth app client id (`sign_in(Some(id))` or
`ISOLINE_GITHUB_CLIENT_ID` in the sidecar's environment). Without one the sidecar answers
with error code -32002 and a message saying so. Registering the app is a human step.

### `MockProvider`

Deterministic answers shaped like the prompts ask for (the same rules and the same bytes as
the sidecar's `--mock` mode), token counts of `ceil(chars / 4)`, simulated prompt caching of
repeated system messages, `max_tokens` honored. Counters: `calls()`, `list_calls()`,
`tokens_in()`, `tokens_out()`, `cached_tokens_in()`, `requests()`, `reset_counts()`.
`push_response` and `push_error` queue canned results; `with_responder` replaces the answer
function. Usable from any crate's tests.

## How it works

**Skeletons** (`skeleton::build_skeleton`). A function's skeleton is its kind and flags, line
count, signature, doc comment, the imports it uses other than by calling them, and its call
flow: one line per block (`if`, `try`, `catch`, `loop`, `callback`) with the calls inside it
in run order, returns and throws, built-ins left out. Without a flow unit it gets its calls
and a keyword outline of its control lines. A function of at most 6 lines is sent as code.
On the prototype's `lifecycle.ts` the skeletons are 39% of the source tokens with call flow
and 37% without (`tests/skeleton_size.rs`).

**Cache** (`AiCache`). The `ai_cache` table from `crates/iso-index/migrations/ai_cache.sql`,
keyed by `iso_core::cache::ai_key(content, PROMPT_VERSION, model, kind)`:

| Answer | Kind | Content key |
| --- | --- | --- |
| Function or method summary | `leaf` | the symbol's `body_hash` |
| Type sentence | `leaf` | hash of the type skeleton (signature, doc, member names) |
| File overview | `file` | hash of the path and the file outline (imports, exports, names) |
| Module | `cluster` | hash of exactly what the model sees: dir, member summaries, edges |
| Architecture | `repo` | hash of the repo prompt |

None of these keys contain line numbers, so a body edit changes only that function's key: the
file overview, the type sentences and the other functions stay cached. A module reruns only
when a member summary or an edge changed, and the repo pass only when a module changed.
Identical bodies share one answer. Bad answers are never cached, so the next run asks again.

**Leaf pass.** Uncached items are packed into requests of up to 20 files (and at most 24k
prompt tokens, 120 items) behind one stable system message, so the provider's prompt cache
serves it after the first request. Items are `### <id> <name>` sections: `f` functions, `t`
types, `o` overviews. The answer is strict JSON (`{"items": [...]}`); code fences and
surrounding chatter are tolerated, a bad or missing item is skipped and reported in
`RunReport.errors`, and text is cleaned (whitespace, emoji, sentence-ending exclamation
marks, list markers; `algo` capped at 7 steps).

**Cluster and repo passes.** One request per module, with member file overviews and up to 6
symbol sentences per file, capped at 6k tokens, plus "depends on" and "used by" edges. Names
come from the model; colors come from code (`palette`): eleven hues, each with a light value
(at least 5:1 on `surface` and `bg`) and a dark value (at least 6.5:1 on the dark ones),
assigned by a hash of the module id with probing, in sorted order, keeping existing colors.
The repo pass gets every module's name, dir, size, role and dependencies, and returns the
overview and layers; modules the model leaves out go into the last layer.

**Estimates and cost.** Estimates build the exact requests a run would send, so input tokens
match the mock exactly; output uses per-item expectations (`prompts::OUT_TOKENS_*`). The
system message of every request after the first counts as cached input. Cluster estimates
made before the leaf pass count missing member summaries at a typical size. Prices come from
`listModels`; ids it lacks fall back to the analysis plan's table (Haiku 5.5 $0.10/$0.50,
Sonnet 5.5 $2/$10, Opus 5.5 $4/$20 per million tokens, cached input a tenth). Economy runs
the cluster pass on the leaf model and is the default, as the plan asks.

**Prompts** follow the design system's voice: one sentence that starts with a verb, 3 to 7
steps, notes that describe what was observed and never tell the reader what to do, `warn`
only for things that can go wrong at run time, sentence case, no emoji, no exclamation marks.
`PROMPT_VERSION` is part of every key; bump it whenever a prompt or a skeleton format changes.

## Tests

`cargo test -p iso-ai` runs without network or model:

- `tests/acceptance.rs`: a second run makes zero provider calls (also from a fresh engine on
  the same cache file); one body change makes exactly one leaf request with only that symbol;
  estimates are within 10% of counted tokens (measured: about 1-2% for output, 0% for input);
  budget refusal and mid-run stop; bad answers skipped and retried; stable colors.
- `tests/skeleton_size.rs`: the 40% target on the real fixture, tiny functions verbatim.
- `tests/sidecar_e2e.rs`: builds the mock-only Go sidecar with `GOPROXY=off` and drives it
  end to end (protocol, sign-in, concurrency, crash restart, the full pipeline). It skips with
  a message when Go or its module cache is missing.
