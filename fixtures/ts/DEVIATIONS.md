# Where Isoline differs from analyze.js (TS/JS)

The call-flow walker in `crates/iso-parse/src/flow/` is a port of the prototype's
`analyze.js` to tree-sitter. For every unit analyze.js emits, Isoline must emit the same JSON
(`crates/iso-parse/tests/flow_ts_golden.rs`). This file lists every place where the output
differs, and why.

## 1. Extra units

analyze.js only makes units of top-level `function` declarations and of the methods on the
controller object a factory function returns. Isoline also makes units of these:

| Extra unit | Example | Where it is tested |
| --- | --- | --- |
| Top-level `const f = () => …` and `const f = function () {…}` | `callbacks.ts` `helper`, `user-profile-events.ts` `mutationKey` | `expected/flows.extra.json` |
| Class methods, constructors and arrow-function fields (`Class.method`) | `classes.ts` `Repo.open`, `user-profile-events.ts` `UserProfileMutationUnsettledError.constructor` | `expected/flows.extra.json` |
| Factories declared as arrow consts, and their controller methods | `classes.ts` `makeRepo`, `makeRepo.load` | `expected/flows.extra.json` |

These units are additions only. The calls inside the units analyze.js emits are classified
exactly as analyze.js classifies them, because analyze.js's `topFns` table is left
unchanged. A call to `helper()` stays `other`, as analyze.js has it, even though `helper`
now has a unit of its own.

## 2. The lifecycle golden

`lifecycle/expected/flows.json` comes from `tools/analyze/analyze.js`. That copy has one line
removed: the prototype hard-codes `harness.runAttempt` and `harness.classify` as external
calls. The prototype's original output is kept in `lifecycle/expected/flows.prototype.json`.
The two files differ only in those two targets (see CONTRACTS.md, "Golden rule for TS").

## 3. Guards on pathological input

analyze.js has no recursion limit. On a file whose local functions inline each other through
deferred definitions, it fails with `RangeError: Maximum call stack size exceeded`. One example
is openclaw `src/plugins/plugin-generation-capture.ts`, a factory with about 20 local lambdas
that call each other. The port first reproduced the same unbounded recursion, and it took the
whole process down. Isoline now stops in these places instead:

- **Nesting limit.** `MAX_DEPTH = 160` nested walk levels. `finalize` now counts the deferred
  definitions it expands against the same limit; before the fix it reset the count and could
  recurse forever.
- **Work budget.** `MAX_WALKED = 200 000` nodes per unit. Mutual inlining is exponential in
  the worst case, so past the budget the walker stops expanding and the unit's tree is cut
  short.
- **Deep trees.** Trees deeper than 192 levels are visited on a scoped thread whose stack is
  sized from the tree's depth (`util::with_stack_for`). This covers generated
  `a + b + c + …` chains and deeply nested callbacks. The facts extractor also stops
  descending past `util::MAX_VISIT_DEPTH` levels.

On every input where analyze.js finishes, these limits change nothing. Both 600-file
analyze.js samples of openclaw give the same results with the guards in place.
`crates/iso-parse/tests/flow_guards.rs` checks each of these shapes on a 2 MiB test thread.

## 4. Grammar quirk that is repaired

tree-sitter-typescript parses `await f<T>(x)` as `(await f)<T>(x)`. In that tree, `await`
wraps the callee instead of the call. `FlowLang::misplaced_prefix` moves the `await` back onto
the call, so the output matches analyze.js. The variable-origin scan in `flow/js.rs`
`scope_of` handles the same shape.

## 5. Known misparses (corpus)

A 600-file sample of openclaw (every 7th `.ts` file under `src/` and `extensions/`) was run
through `tests/flow_corpus.rs`. 2171 of 2174 units match analyze.js exactly. The 3 that
differ all come from tree-sitter-typescript building a different tree than TypeScript does,
so the walker cannot repair them:

| Unit (openclaw file) | Source shape | tree-sitter reads it as | Effect |
| --- | --- | --- | --- |
| `restoreRealMediaLoader` (`src/infra/outbound/message-action-params.attachments.test.ts`) | `await vi.importActual<typeof import("x")>("x")` | `(await vi.importActual) < (typeof import("x")) > ("x")`, two comparisons | The call is not marked `await` |
| `createManagedServiceManagerBoundary` (`src/infra/update-managed-service-handoff-boundary.test-support.ts`) | The same shape | The same | The same |
| `createUsageOverviewFold.finishInto` (`src/infra/session-cost-usage-quarters-fold.ts`) | `tools[b]! - tools[a]! \|\| ids.tools[a]!.localeCompare(…)` | Postfix `!` binds looser than `-` and `\|\|`, so the whole `a - b \|\| c` becomes the object of `.localeCompare` | analyze.js's `unless` block is missing, and the call is labeled with the whole expression |

A second, disjoint sample (every 7th file at offset 3, `packages/` included) matches on
1957 of 1959 units. The 2 that differ are in `src/infra/update-managed-service-handoff.test-support.ts`
(`registerPreparedCoordinatorAdmissionTest` and `writeConcurrentManagedHandoffParams`). Both
are the first shape again. Across both samples, 4128 of 4133 units (99.9 %) match.

The first shape only occurs with a `typeof import(…)` type argument. A plain `f<T>(x)` parses
correctly. Swapping in the Oxc parser, which phase 3 benchmarks, would remove all of these.

## 6. Facts conventions beyond CONTRACTS.md

These cover extraction rather than call flow. They are written down here because no
reference analyzer exists for them.

- **Export marking.** `export { a, b as c }`, `export default a` and the CommonJS forms
  `module.exports = a`, `module.exports = { a, b: c }`, `exports.x = a` and
  `module.exports.x = a` mark the named top-level declaration `exported`. A class marked
  this way also exports its non-private members, the same as `export class` does.
- **Export clauses are not references.** They emit no `RefFact`: they mark a binding instead
  of using it.
- **Plain variables are not symbols.** Top-level variables that are not functions become
  symbols only when exported (for example `export const TIMEOUT = 5000`).
- **CommonJS requires.** `require("x")` assigned to a plain name imports `*` under that alias.
  A destructuring pattern imports each property it names.

## 7. Import resolution (`iso-resolve/src/js.rs`)

This section describes resolver policy where plain tsc or Node behavior would give no answer.

- **Workspace packages without `node_modules`.** Every `package.json` in the repo that has a
  `name` is a workspace package. Its `exports` map is read with the conditions
  `source, types, import, module, require, node, default`, in that order. Without `exports`,
  the resolver tries `source`, `types`, `module` and `main`, then `index`, then `src/index`.
  That way a package whose `main` points at an unbuilt `dist/` still resolves to its source.
  With `node_modules` installed (symlinked workspaces), oxc_resolver takes the same path and
  lands on the same files.
- **Alias-looking specifiers.** Bare specifiers that start with `@/`, `~` or `#` and match
  nothing are `Unresolved`, not `External`, because by convention they are path aliases.
- **Node builtins.** These are `External` with the `node:` prefix removed, so `node:fs` and
  `fs` are the same module.
- **Inside the repo only.** A result counts as a repo `File` only if the walker saw it and it
  is not under `node_modules`.
