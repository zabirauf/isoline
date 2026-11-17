# Progressive disclosure, typed signatures and quiet links

Status: spec for implementation. Reviewer: UX. Applies to the file view (`Workbench`, `FunctionCard`, `DepMap`, `CallFlow`, `FileHeader`), the app shell and the repo overview. Binding rules come from `docs/handoff/design-system/project/README.md`. Where this spec changes a design-system rule, it says so in "Design-system changes" at the end.

Evidence: in `openclaw-lifecycle-map-light.png`, the first card's description sits below 6 stat pills, an empty overview placeholder, 4 underlined paths, 2 toolbar rows and a layer chip. The map's text is unreadable because 33 external symbols are fitted in. Most of it is correct; it is shown too early.

---

## 1. Type-colored signatures in cards

### Layout

The signature is a new block, `iso-sig`, under the description. It replaces `card-sig`. It is mono 12px on `surface`.

- **Inline** when the function has 3 or fewer params and the rendered text is 56 characters or fewer: `(harness: AgentHarness, opts: RunOpts) → Result`.
- **Stacked** otherwise. Use one param per line in a two-column grid (name | type), indented `space-8`, with the return type on its own last line. Read level shows the first 4 params, then a reveal "+ 3 more params". Dig shows all of them.

Read level with AI off (the description line is the doc comment):

```
 runAgentHarnessLifecycleAttempt  EXPORT                          L357–423
 Runs one harness attempt inside full diagnostics, then normalizes the result.
   harness   AgentHarness                         ← ext-soft fill (repo import)
   params    AgentHarnessAttemptParamsV2          ← ext-soft fill
   execute?  (params) => Promise<…>   = …         ← Promise muted, = … muted
   → Promise<EmbeddedRunAttemptResult>            ← Promise muted, inner ext-soft fill
 7 calls · 6 imports · 2 callers in other files   ← reveal buttons, muted
 Show code · 67 lines
```

| Part | Rendering |
|---|---|
| Param name | `ink`. Optional marker `?` in `muted`. |
| Rest param | `...` prefix in `muted`, then the name: `...args  string[]`. |
| Destructured param | Name shown as `{…}` in `muted`. The peek on hover lists the keys. |
| Default | Read: `= …` in `muted` after the type, with the full text in a peek on hover. Dig: the literal default, cut at 32 characters. |
| Type text | Tokenize `typeText` by identifier and look up each one in `FileView.targets` by name. Punctuation (`<>[]`, `\|`, `&`, `(),=>`) is `muted`. Cut types longer than 40 characters with `…`. The full text is in the peek of the param row. |
| Function-typed params | Show the parameter names only: `(params) => Promise<…>`. Generics are cut at the first level. |
| Generic params (`<T extends X>`) | Hidden at Read. Shown before the parens at Dig. |
| Return | Its own line, `→ Type`, colored by the same rules. Omitted when `returns` is null or `void`. Dig shows `→ void`. |

### Token per origin

Use existing tokens only. Each origin has a non-color cue at rest. The cursor and the peek are a second cue, not the only one.

| Origin (`targets[].origin`) | Color | Non-color cue at rest | Cursor | Click |
|---|---|---|---|---|
| Type in this file (`local`) | `accent` | weight 600, no fill. This matches `tk-def` and the CodeBlock rule "defined in this file = accent". | pointer | Go to definition in this file |
| Imported from the repo (`imported`) | `ext` | `ext-soft` fill, `radius-4`, padding 0 3px. Same shape as `iso-chip-ext`. | pointer | Open the defining file at the type |
| External package (`external`) | `ext` | 1px `line` outline, no fill, `radius-4` | pointer | Pin the peek. There is no file to open. |
| Builtin or primitive (`builtin`) | `muted` | plain, regular weight. Same token as built-ins in FlowStep (`iso-k-param`). | default | None. Peek on hover. |
| Unknown (no target) | `ink` | plain, regular weight | default | None. The peek says "Not resolved". |

Rationale: `ext` already means "from outside the file", so repo and package types share it and fill versus outline separates them, with no new hue ("Color is meaning"). Builtins say nothing about this codebase, so they get the quietest token. Builtin and unknown differ only in color, but neither is interactive and a builtin's name identifies it.

---

## 2. Clickable without underlines

### Cursor

| Cursor | Where |
|---|---|
| `pointer` | Every element with a click action: buttons, chips, card headers, signature types, step rows, graph nodes, file paths, linked identifiers in Monaco, segmented options |
| `text` | Prose (descriptions, algorithm steps, notes, the overview) and non-link code in Monaco. Readers copy from these. |
| `default` | Non-interactive labels, builtin and unknown types, event and phase chips, stat pills, disabled controls |
| `help` | Removed. The `AI` mark and tier badge get a peek instead of a native `title`. |

### Hover state

- **Inline links** (signature types, code identifiers, paths): no underline at rest. On hover, show a 1px underline in the element's own color, offset 2px, plus the pointer. Remove the native `title` from anything that has a peek, because two tooltips fight.
- **Chips and nodes:** the border changes to `accent`, as today ("Hover changes a border to accent").
- **Rows** (step rows, sidebar rows, importer rows): a `bg` fill, as the sidebar does today.
- **Hold ⌘** (Ctrl on Windows and Linux) to underline every go-to-definition target in the pane under the pointer. Release ⌘ to remove the underlines. This is the discoverability path that adds nothing to the screen at rest.

### What a click does

There is one rule: names focus; types and paths go to their definition; **⌘-click always goes to the definition.**

| Element | Click | ⌘-click |
|---|---|---|
| Signature type (local) | Expands "Local types", reveals that type with its code, highlights its lines | same |
| Signature type (repo import) | Opens the defining file focused on the type (`openFile(path,{symbol,line})`) | same |
| Signature type (external) | Pins the peek | same |
| Card header | Focuses the card in both panes (unchanged) | Opens the card's code and scrolls to it |
| Call, caller and "Start at" chips | Focus (unchanged, per the Chip README) | Go to definition |
| Imported-symbol chips | Focus the symbol in the map (unchanged) | Open the defining file at the symbol |
| Monaco identifier (fn / type / import) | Go to definition, on mouse-up only if the pointer moved 4px or less, so drag-select still works. Today it fires on `onMouseDown`. | same |
| Step row (call flow) | Highlights its line in the code (unchanged) | Go to the callee's definition |
| Step meta path (`infra/diagnostic-trace-context.js`) | Opens that file at the callee | same |
| "Open ›" on a step | Drills into the callee's flow (unchanged) | n/a |
| Import path, "Imported by" path, graph module header | Opens the file at the import line | same |
| Graph function node | Focus (unchanged) | Go to definition |

Keyboard: every target is a focusable button. Enter does the click and ⌘Enter does the ⌘-click; in Monaco, F12 goes to the definition under the caret. Go to definition always resolves the same way: a local function selects its card, opens its code and scrolls to it; a local type is revealed in "Local types"; a repo symbol opens its file; an external pins the peek; a builtin does nothing.

**History.** `history` in `App.tsx` (files only, Back only) becomes a stack of `{path, symbol, line, selection}`. Every go-to-definition and file open pushes; plain focus clicks do not. Back is ⌘[ or the mouse back button; Forward is ⌘] or mouse forward. Both are quiet buttons left of search, each shown only when its stack is non-empty, titled with the destination ("Back to lifecycle.ts · startAgentRun").

**Discoverability** uses no permanent decoration: hover underline and pointer, the ⌘-hold reveal, and a one-time hint (§6).

---

## 3. Definition peek (new component `iso-peek`)

| Aspect | Decision |
|---|---|
| Trigger | 400ms of pointer rest on a target, or keyboard focus resting for 400ms. Once a peek has been open, it is "warm" for 800ms: hovering another target swaps the content with no delay. |
| Grace | 150ms to move the pointer from the token into the peek. The peek closes 200ms after the pointer leaves both. |
| Position | Below the token. It flips above when space runs out and never covers the token. It uses the existing `.popover` surface (`surface`, `line` border, `radius-9`). |
| Size | Width 320–520px. Max height 360px. The code area shows up to 14 lines and scrolls inside the peek. |
| Pin | Clicking inside the peek, scrolling its code, or Shift+Enter pins it. A pinned peek shows "Close (Esc)". Only one peek is pinned at a time. |
| Dismiss | Unpinned: pointer leaves, Esc, or the pane under it scrolls. Pinned: Esc, "Close (Esc)", or a click outside. |
| Nesting | Types inside the peek's signature or code are links. Clicking one replaces the peek content in place and adds a breadcrumb (`AgentHarness › RunAttempt`), the same model as the call-flow breadcrumb. Hovering inside a peek never opens a second peek. ⌘-click or "Open ›" does a real go-to-definition. |
| Monaco | Turn off Monaco's own hover (`hover: {enabled:false}`). Drive the same `iso-peek` from `onMouseMove` + `linkAt`, so code and cards show one peek. This replaces the markdown `registerHoverProvider` content. |

Content, top to bottom:

```
┌─────────────────────────────────────────────────────────┐
│ INTERFACE  AgentHarnessAttemptParamsV2                  │  mono-tag kind · name in its origin token
│ src/agents/harness/types.ts · L41–58                    │  caption, muted; path is a link
│ Parameters for one attempt: session, prompt and policy. │  doc, clamped to 3 lines + "More"
│ ┌──────────────────────────────────────────────────────┐│
│ │41 export interface AgentHarnessAttemptParamsV2 {     ││  iso-code, real line numbers,
│ │42   session: SessionRef;                             ││  types inside are links
│ │…                                                     ││
│ └──────────────────────────────────────────────────────┘│
│ Show all · 18 lines                          Open ›     │
└─────────────────────────────────────────────────────────┘
```

- **Header** renders at once from `targets[]`. **Source** loads lazily via `get_source(path,start,end)`, cached by range, with "Loading lines 41–58…" meanwhile.
- **Origin line:** local "this file · L23–41"; imported: repo path and range; external "`zod` · package · node_modules/zod/lib/types.d.ts · L88"; builtin "built-in · TypeScript".
- **External with a path:** declaration source labeled "Declaration (package)", no "Open ›" (outside the index). **Builtin or no path:** kind, name, doc, signature, then muted "No source in this repo."
- **Unknown:** "Not resolved. Isoline found no definition for `Foo` in this file's imports."
- **Local function:** the card's description and line range instead of code; "Open ›" focuses the card.

---

## 4. Declutter audit

| Region | Current | Decision |
|---|---|---|
| Top bar | 11 items | Keep: wordmark (browser only), repo name, "Files", Back/Forward (conditional), search, indexing progress (only while indexing, plus "Indexed N files in X s" for 4s), AI button. |
| | "Jobs idle · 100% cache hits" | Remove at idle. While jobs run, show "Jobs · 1 running" merged into the indexing text. Cache stats go into that text's peek. |
| | "Open repo…" | Move to ⌘O, the native File menu, and the repo-name menu. The repo name becomes a menu button: Repo overview, Open repo…, recent repos. |
| | Theme segmented control | Move to the native View › Theme menu. The browser build gets a quiet "Theme" button that opens the 3-way control in a popover. |
| | AI button on the start screen | Hide it until a repo is open, because AI is turned on per repo. |
| Sidebar | Legend "indexed / pending / failed / badge" | Remove. Show status dots only for pending and failed rows; indexed is the norm and shows no dot. The meaning is in each dot's peek. |
| | "FULL" badge on every row (desktop dark) | Show tier badges only for OUTLINE and TEXT. |
| File header | 6 stat pills | Merge into the caption line: `src/agents/harness/lifecycle.ts · TypeScript · 486 lines · 12 functions · 16 imports`. The pills return only at Dig. |
| | "FULL" badge | Show only for outline and text tiers. Keep the module dot and name. |
| Overview | AI-off paragraph in a full box | Replace with one line (§6). |
| | "Imported by" with 4 underlined full paths | "Imported by 4 files" plus the first 2 basenames inline (`index.ts`, `runner.ts`) and "+ 2 more". Paths are `ink` mono with no underline. Hover shows the full path in a peek. |
| | Long AI overview | Clamp to 3 lines at Glance with "More". Show in full at Read and Dig. |
| Toolbar | Filter, Order (2), Detail (2), "Expand all code" on 2 rows | One row: Filter, Detail `Glance / Read / Dig`, and an order menu button "Entry points first ▾". "Expand all code" appears only at Dig. |
| Focus bar | 2-line sentence explaining blue and amber | One line: `Focused on startAgentRun · 4 depends on · 1 used by   Clear (Esc)`. "depends on" is in `accent` and "used by" in `user`, with the words as the cue. The color explanation lives in the map legend. |
| Card | `L3` layer chip | Remove it; the group heading already says "Layer 3". It returns at Dig. |
| | "No summary yet." | Remove. Show the doc comment if there is one. Otherwise the signature is the summary. |
| | Raw `signature` with ellipsis | Replace with the typed `iso-sig` (§1). |
| | Calls / Imports used / Called by rows | Hidden by default. They are replaced by one muted line of reveal buttons, `7 calls · 6 imports · 2 callers in other files`. Each count expands only its own row (`aria-expanded`). Hovering a count previews that set in the map through the existing `onHover`. At Dig, all rows start open. |
| | Info notes | Shown at Read. At Glance, collapsed to a count. |
| | Warning notes | Always visible at Read and Dig. At Glance, a square `!` and "1 possible bug" on the card row. |
| Dependency map | Toolbar with 6 buttons | Keep: tabs, "External imports" toggle, "Fit". Add "More ▾" containing Type-only imports, Transitive, Zoom in (+), Zoom out (−). The −/+ keys and scroll zoom still work. |
| | External imports column on by default | Off by default, remembered per user. When off, the right edge shows one muted stub, "+16 modules". When on, a module node shows "3 symbols" collapsed until it is focused or hovered. |
| | Fit shrinks to 6px text | Fit never scales below 0.6. If the graph doesn't fit, fit the entry-point column and let the reader pan. |
| | Legend box always shown | Show only while something is focused, as a one-line strip ("— depends on  — used by  — external import"). The "Entry points on the left…" sentence moves to the first-run hint. |
| Call flow | Plain unit name above the signature line | Remove it at depth 1. Keep it only as the breadcrumb when drilled. |
| | Signature line | Use the typed signature, inline form, with the same tokens. |
| | Stat pills including "0 loops" | Hide zero counts at Glance and Read. Show all at Dig. |
| | "Open ›" on every step | Each drillable step shows a muted trailing `›` at rest. The full "Open ›" button appears on row hover, on row focus and on the current step. |
| | 10-item legend plus a sentence (about 80px) | Collapse to a quiet "Key" button in the flow toolbar. It opens the first time a flow is shown, then stays collapsed (remembered). Every frame has a keyword tag and every step has a words meta line, so the legend is redundant to the "never color alone" cues. |
| | "Replay", "Built-ins" | Keep "Replay". Move "Built-ins" into the flow's "More ▾". |
| Repo overview | 6 stat pills | Merge into the caption, like the file header. |
| | "Rename" on every module row | Show it on row hover and row focus only. |
| | Support tiers panel | One line, `42 full · 1 outline · 7 text`, with a "What the tiers mean ›" reveal. |
| | Languages table | Top 5 rows, then "+ 3 more". |
| Outline and text tier | "Why there is no call flow" box | Retitle the eyebrow "Outline tier" or "Text tier". Keep one sentence. The rest goes behind "Why ›". The file summary moves above it. |
| Start screen | Underlined recent repos | Mono `ink` rows with no underline, a pointer, and a `bg` hover fill. |

---

## 5. Progressive disclosure levels

A single control, `Detail: Glance · Read · Dig` (`iso-seg`, 3 options), replaces "Summary / + Algorithm".

| | Glance | Read (default) | Dig |
|---|---|---|---|
| File header | Name and caption line | same | + stat pills |
| Overview | AI text clamped to 3 lines, "Start at" | Full text, "Start at", "Imported by" line | + importer list expanded |
| Card | One row: name, export pill, one-line description (ellipsis), `!` count, line range | Description, signature (4 params), algorithm, notes, relations line, "Show code" | + layer chip, all params with defaults and generics, all relation rows open |
| List extras | — | "Local types" and "Imports" collapsed | Both expanded. "Expand all code" shown. |
| Call flow | Non-zero stats, steps | same | + zero stats, "Built-ins" visible |

```
Glance card:  runAgentHarnessLifecycleAttempt  EXPORT  Runs one harness attempt inside…   L357–423
              createAgentHarnessLifecycleDiagnostics   Opens the harness span and…  ! 1   L271–355
```

- **Controls:** keys `1` `2` `3` when not typing. Per card, the relations line and "+ N more params" open that card further; "Show code" is deepest. Per-card overrides are session-only and reset on file change.
- **Default:** Read. No auto-switching by file size, so the view is predictable.
- **Memory:** one object in `localStorage` key `isoline.prefs.v1` (per OS user, never per file): `{ detail, mapExternal, flowKeySeen, hintSeen, aiNoticeHidden }`.

---

## 6. Empty, first-run and AI-off states

- **AI off.** The overview box drops its paragraph. It shows "Start at", "Imported by" and one muted line: "Summaries are off. Cards show signatures and doc comments." A quiet "Turn on AI…" button opens the AI panel, and a quiet "Hide" stores `aiNoticeHidden` for all files. Cards show nothing in place of the summary; the doc comment and signature carry them. Glance shows the doc comment's first sentence, or the signature in inline form.
- **AI on, file not summarized.** Same line, but with "Summarize this file · about $0.02" (from the estimate). While running: "Summarizing 3 / 12".
- **First file ever opened.** A one-line hint strip under the toolbar, dismissed for good with "Got it" (`hintSeen`): "Hover a type to see its definition. Click to go there, ⌘-click anything to open its definition, ⌘[ to come back. Hold ⌘ to see every link." The call-flow "Key" is open the first time.
- **No functions.** Keep "No functions in this file. Its imports and types are listed below." Open "Local types" and "Imports" by default here.
- **Filter has no match.** "No function matches “x”." plus a "Clear filter (Esc)" button.
- **Call flow tab with no selection.** The tab is enabled. The pane says "Select a function to see its calls in order." with the "Start at" chips.
- **Repo indexing.** The module map area shows "Building the module map · parsing 19 / 50". There is no empty frame.
- **Start screen.** Wordmark, search, and the card with recent repos. No AI or theme controls.

---

## 7. Implementation checklist (by impact ÷ effort)

| # | Item (effort) | Acceptance: what the screenshot shows |
|---|---|---|
| 1 | Card relations line replaces the three `iso-deps` rows; drop the `L3` chip and "No summary yet." (S) | In `openclaw-lifecycle-map-light`, card 1 is name → doc → `7 calls · 6 imports · …` → "Show code", with no chips; card 2's header is visible at 900px height. |
| 2 | Quiet links: no underline at rest on `.link`, `.filelink`, `.mx-*`; `mx-type` takes `accent`; hover underline and pointer; ⌘-hold class on `body`; Monaco jump on mouse-up with a 4px drag threshold (S) | No underlines in the overview or code. Hovering `AgentHarness` in code shows an underline and pointer; holding ⌘ underlines every link in that pane. |
| 3 | Typed signature `iso-sig` with the §1 layouts and tokens (M; needs `sig` and `targets`) | Lifecycle card shows 3 stacked params: `AgentHarness` on an `ext-soft` fill, `Promise` muted, `= …` after `execute`. Local types are bold `accent`. Still readable in grayscale. |
| 4 | Detail control `Glance / Read / Dig`, keys 1–3, persisted (M) | Glance shows all 12 one-row cards above the fold; the level survives a reload and a file change. |
| 5 | Header, toolbar and focus-bar declutter; "Imported by" as a count with 2 names (S) | The first card's header is within 260px of the file header's bottom edge (about 440px today). |
| 6 | Definition peek `iso-peek`: `get_source`, pinning, in-place nesting, replaces Monaco's hover (M–L) | After a 400ms hover, `AgentHarnessAttemptParamsV2` shows kind, path · range, doc and 14 lines with real line numbers; `string` shows "No source in this repo." |
| 7 | Go to definition per §2; location stack; Back/Forward, ⌘[ and ⌘] (M) | Clicking `AgentHarness` in a signature opens `types.ts` focused on it. ⌘[ returns to lifecycle.ts with the same card focused and the same scroll position. |
| 8 | Map: external imports off by default with a "+N modules" stub; "More ▾"; minimum fit scale 0.6; legend only while focused (M) | Lifecycle map captions are legible at default zoom; no legend box until a node is clicked. |
| 9 | Call flow: no duplicate title, no zero pills, `›` at rest with "Open ›" on hover, "Key" toggle, Built-ins in "More ▾" (S) | In `openclaw-lifecycle-flow-light`, step 1 is at least 60px higher and the bottom legend is gone. |
| 10 | Shell: repo-name menu, theme to the menu, no idle jobs text, no sidebar legend or "FULL" badges (S) | The idle top bar shows only Isoline, `openclaw ▾`, Files, search and the AI button. |
| 11 | First-run hint and the AI-off line (S) | On a fresh profile one hint strip appears and is gone after "Got it" and a reload. With AI off the overview is one line plus "Start at". |
| 12 | Repo overview: caption stats, "Rename" on hover, one-line tiers, top 5 languages (S) | In `overview-light`, the panels below the map are 220px tall or less. |

---

## Design-system changes this spec makes

These belong in a separate design-system change:

- **CodeBlock README:** links are not underlined at rest; they underline on hover or while ⌘ is held. Color carries origin (`accent` for this file, `ext` for imported). This replaces "solid accent link / dotted ext link".
- **New components:** Peek (`iso-peek`, §3) and Signature (`iso-sig`, §1).
- **SegmentedControl:** add `Detail: Glance / Read / Dig` as the standard disclosure control.
