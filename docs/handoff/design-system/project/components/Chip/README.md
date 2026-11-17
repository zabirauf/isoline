A small mono label for a code identifier; clicking it focuses that symbol everywhere.

**Classes:** `iso-chip` (symbol in this file), `iso-chip-ext` (imported symbol, in `ext`), `iso-chip-type` (type-only import, italic), `iso-event` and `iso-event-err` (event names), `iso-phase` (a phase or stage), `iso-lvl` (layer number), `iso-pill` (the "export" marker).

**You provide:** the identifier exactly as written in code.

- Chips are always mono. Never put prose in a chip.
- Color states origin, not importance: neutral for this file, `ext` for anything imported, `warn` only for the error path.
- Chips for symbols are buttons; event and phase chips are not interactive.
