A function's parameters and return type, with every type name colored by where it is defined.

**Classes:** `iso-sig` with `iso-sig-inline` or `iso-sig-stacked`; type names are `ty` with one origin class: `ty-local` (this file: `accent`, weight 600), `ty-imported` (repo import: `ext` on an `ext-soft` fill, `radius-4`), `ty-external` (package: `ext` with a 1px `line` outline), `ty-builtin` (`muted`) and `ty-unknown` (`ink`). Punctuation is `ty-p` (`muted`).

**You provide:** the parameters as written (name, type text, default, rest, optional), the return type, and the definition targets the type names are looked up in.

- Inline when there are 3 or fewer params and the text is 56 characters or fewer: `(harness: AgentHarness, opts: RunOpts) → Result`. Otherwise stacked: one param per line in a name | type grid, indented `space-8`, the return on its own last line.
- Read shows the first 4 params, then "+ N more params"; Dig shows all, with literal defaults (cut at 32 characters) and generic parameters.
- Types longer than 40 characters are cut with "…"; a name cut part-way keeps its origin color. Function-typed params show parameter names only: `(params) => Promise<…>`.
- A default shows as `= …` in `muted`; the full text is in the param's Peek.
- The return is `→ Type`, omitted when there is none or it is `void` (Dig shows `→ void`).
- Local, imported and external names are buttons: hover for the Peek, click to go to the definition (an external one pins its Peek). Built-in and unknown names are not interactive and keep the default cursor.
- Nothing is underlined at rest. Each origin also has a non-color cue (weight, fill or outline), so the signature reads in grayscale.
