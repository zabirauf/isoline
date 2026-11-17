A floating panel that shows where a name is defined, without leaving the current view.

**Class:** `iso-peek` (on the `.popover` surface: `surface`, 1px `line` border, `radius-9`), with `pinned` once the reader interacts with it. Inside: `peek-h` (a `mono-tag` kind and the name in its origin token), `peek-origin` (caption), `peek-doc`, `peek-code` (real line numbers, at most 14 lines, scrolls inside), `peek-actions` ("Show all · N lines", "Open ›") and `peek-crumbs` when nested.

**You provide:** the name and its definition target (origin, kind, path, line range, doc, signature). The source loads lazily by line range.

- It opens after 400ms of pointer rest or keyboard focus on a target. Once open it stays warm for 800ms, so the next target swaps in with no delay.
- It sits below the token, flips above when there is no room, and never covers the token. Width 320–520px, height at most 360px.
- Clicking inside, scrolling its code, or Shift+Enter pins it; a pinned peek shows "Close (Esc)". Only one peek exists at a time.
- An unpinned peek closes 200ms after the pointer leaves both the token and the peek, on Esc, or when the pane under it scrolls. A pinned one closes on Esc, "Close (Esc)" or a click outside.
- Names inside the peek are links. A click replaces the content in place and adds a breadcrumb (`AgentHarness › RunAttempt`); hovering inside never opens a second peek. ⌘-click or "Open ›" goes to the definition for real.
- The origin line says where the name lives: "this file · L23–41", a repo path and range, "`zod` · package", or "built-in · TypeScript". Without source: "No source in this repo." An unresolved name says so plainly.
- A peek replaces a native `title` tooltip; never show both.
