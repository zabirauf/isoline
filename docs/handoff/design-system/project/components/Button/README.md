A quiet bordered button for toolbar actions and toggles.

**Classes:** `iso-btn` (default and toggle), `iso-btn-drill` (opens a deeper view from inside a row), `iso-btn-reveal` (shows or hides content in place).

**You provide:** a label that says exactly what happens, and `aria-pressed` on toggles or `aria-expanded` on reveal buttons.

- Use sentence case and name the result: "Expand all code", not "Expand".
- A reveal button states what it reveals and how much: "Show code · 13 lines".
- Toggles show state with `accent-soft` fill and `accent` text. There is no filled primary button; nothing in the app is a call to action.
- Icon-only buttons need an `aria-label`.
