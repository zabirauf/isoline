The standard disclosure control: how much of each item the reading list shows.

**Class:** an `iso-seg` labeled "Detail" with three options, `Glance`, `Read` and `Dig`; the active one has `aria-pressed="true"`.

**You provide:** nothing per screen; the level is one per-user preference (`isoline.prefs.v1`), never per file.

- Glance: one row per item (name, export pill, a one-line description, a "!" count for possible bugs, the line range).
- Read (the default): description, signature (4 params), algorithm, notes, a relations line of counts, and "Show code".
- Dig: everything open, including the layer chip, all params with defaults and generics, all relation rows, stat pills and "Expand all code".
- Keys `1`, `2` and `3` pick a level when the reader is not typing. No automatic switching by file size, so the view stays predictable.
- Per-item reveals ("+ N more params", a relations count, "Show code") open one item further for the session and reset when another file opens.
