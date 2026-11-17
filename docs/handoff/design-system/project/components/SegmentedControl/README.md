Two to four mutually exclusive choices that change how the same content is shown.

**Class:** `iso-seg` wrapping plain buttons; the active one has `aria-pressed="true"`.

**You provide:** the option labels and a group `aria-label`.

- Use it for view modes and sort orders, never for navigation between unrelated screens.
- Keep labels to one to three words. The active option gets `accent-soft` and weight 600; inactive options stay `muted`.
- The same control is the tab bar above the graph pane.
- `Detail: Glance / Read / Dig` is the standard disclosure control; see DetailControl.
