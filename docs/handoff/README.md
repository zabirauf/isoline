# Isoline handoff package

Everything needed to implement Isoline, a desktop app for understanding code without reading every line. Start with `plan/implementation-plan.md` and follow it.

## Read in this order

1. `plan/implementation-plan.md`: the build brief. Contracts, workstreams, acceptance tests, the validation loop, and what needs a human. If you can spawn helper agents, act as the lead agent it describes.
2. `plan/analysis-plan.md`: the reasoning behind the plan. What deterministic code does and what needs a model, Copilot sign-in and billing, the cost model and the performance targets. Written before the app was named Isoline, so it says "Code Map Desktop".
3. `design-system/project/README.md`: the rules for how the UI looks and reads. Build the UI from this system; do not invent styles.
4. `prototype/code-map.html`: open it in a browser. It is the reference behavior for the function cards, the dependency map and the call flow.

## What is in each folder

| Folder | Contents |
| --- | --- |
| `plan/` | Both plans as Markdown and as Word files, plus the diagrams as PNG under `plan/diagrams/`. |
| `prototype/` | The working single-file prototype, the call-flow analyzer it was built with, its input file and the analyzer's output. |
| `design-system/` | Tokens, component styles, component previews and guidelines. |
| `images/` | Screenshots of the prototype in light and dark themes. |

## Links inside the plans

The plans link to pages you cannot open. Use these local files instead.

| Link text in the plans | Local file |
| --- | --- |
| "analysis plan" | `plan/analysis-plan.md` |
| "prototype code map" | `prototype/code-map.html` |
| the workstream, architecture and milestone diagrams | `plan/diagrams/` |
| `prototype/analyze.js` | `prototype/analyze.js` |

## Prototype

- `code-map.html`: the built page. Self-contained apart from fonts loaded from Google Fonts.
- `analyze.js`: the call-flow analyzer. It is the reference for workstream H. Run it with Node and `typescript@5.6.3`:
  `npm install typescript@5.6.3 && node analyze.js lifecycle.ts > flows.json`
- `lifecycle.ts`: the TypeScript file the prototype analyzes.
- `flows.json`: the analyzer's output for that file. Use it as the first golden file.
- `source/`: the page template, flow view script, styles and the Python build script that assembled `code-map.html`. `build.py` holds the hand-written descriptions; in the real app those come from the model.

## Design system

- `project/tokens.json`: every color (light and dark), text style, spacing step and radius, each with a usage note. This is the source of truth.
- `tokens.css`: the same tokens compiled to CSS custom properties, for convenience. Regenerate it if `tokens.json` changes.
- `project/components/bundle.css`: the component classes (`iso-*`).
- `project/components/<Name>/README.md` and `preview.html`: guidelines and a small example for each of the 11 components. To view a preview locally, load `tokens.css` and `bundle.css` before it.
- `project/components/Cover/preview.html`: the cover artwork.
- There is no logo. Set the name "Isoline" in the display face.
- Not yet covered: tooltip, pane divider, breadcrumbs, graph legend and the overview panel. Take their styles from `prototype/source/`.

## Known gaps

- The Markdown plans were converted from Word files, so inline code lost its backticks. Code blocks, tables and all wording are intact. The Word files are included if you need to check formatting.
- Performance targets and cost figures in the plans are estimates, not measurements.
- Five items need a human before parts of the work can finish. They are listed at the end of the implementation plan.
