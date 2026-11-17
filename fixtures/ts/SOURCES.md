# Fixture sources (TS/JS)

## Copied from open-source projects

The `real-*` fixtures are verbatim copies of files from
[openclaw](https://github.com/openclaw/openclaw), MIT License, Copyright (c) 2026 OpenClaw
Foundation. They were taken at commit `e22624239fdc0908fffc5553843b01d0ca99c7bf` and are kept
unmodified so they compare byte for byte with upstream. Their imports point into the
openclaw tree and do not resolve here, which is fine because the flow walker never
resolves imports.

| Fixture | Upstream path |
| --- | --- |
| `real-argv/argv.ts` | `src/cli/argv.ts` |
| `real-tui-formatters/tui-formatters.ts` | `src/tui/tui-formatters.ts` |
| `real-user-profile-events/user-profile-events.ts` | `src/state/user-profile-events.ts` |

Their `expected/flows.json` files are `node tools/analyze/analyze.js <file>` output, and
nothing else. `expected/flows.extra.json` holds the units Isoline adds, checked by hand
against the source (see DEVIATIONS.md, section 1).

## From the design handoff

- `lifecycle/lifecycle.ts` is `docs/handoff/prototype/lifecycle.ts`. Its
  `expected/flows.prototype.json` is the prototype's own `flows.json`.

## Written for Isoline

These were written by hand for this repo. Their goldens come from analyze.js for flows
(`expected/flows.json`) and are written by hand for everything else.

| Fixture | What it covers | Golden |
| --- | --- | --- |
| `control/` | `if`/`else`, `switch`, loops, `try`/`catch`/`finally`, early exits, short-circuit `&&`/`\|\|`/`??` | analyze.js |
| `callbacks/` | Callbacks, iteration methods, inline local functions, deferred definitions | analyze.js, plus extra |
| `classes/` | Class methods, constructors, arrow fields, an arrow-const factory | analyze.js, plus extra |
| `facts-basic/basic.ts` | Imports of every form, re-exports, JSDoc, interfaces, types, enums, async functions, classes, dynamic `import()` | Hand-written `facts.json` |
| `facts-tsx/panel.tsx` | TSX components, hooks, JSX element references, generic arrows, type-only imports | Hand-written `facts.json` |
| `monorepo/` | tsconfig `paths` (own and through `extends`), project references, `index.ts` barrels, ESM `.js` → `.ts`, workspace packages with `exports` / `main`, builtins, externals, unresolved imports | Hand-written `resolve.json` |
| `../js/plain/` | Plain JavaScript flow | analyze.js |
| `../js/facts-cjs/cjs.js` | CommonJS `require`, `module.exports`, nested functions, `#private` methods | Hand-written `facts.json` |

## Corpus runs (not checked in)

`crates/iso-parse/tests/flow_corpus.rs` and `ts_timing.rs` run against a local clone of
openclaw at the commit above. The results are in DEVIATIONS.md, section 5. No corpus file is
copied into the repo apart from the three `real-*` fixtures.
