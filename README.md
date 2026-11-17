# Isoline

A desktop app for understanding code without reading every line. Open a repo and Isoline
shows, for any file, a reading list of function cards grouped by layer, a dependency map, and
a step-by-step call flow, with the code one click away in a Monaco editor.

Structure comes from deterministic parsing (tree-sitter, oxc_resolver, SQLite). A model is
used only for one-line summaries, module names and colors, through the user's own GitHub
Copilot sign-in, and never without the user turning it on.

- Build brief: `docs/handoff/plan/implementation-plan.md`
- Contracts: `CONTRACTS.md`
- Design system: `docs/handoff/design-system/`

Status: under construction. See `docs/STATUS.md`.
