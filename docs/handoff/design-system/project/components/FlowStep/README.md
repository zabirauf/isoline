One call in a function's execution order: a number, the callee, and where it comes from.

**Classes:** `iso-step` with one kind class: `iso-k-fn` (this file, `accent`), `iso-k-ext` (imported, `ext`), `iso-k-method` (method or local helper, `method`), `iso-k-param` (parameter or built-in, `muted`). `iso-step-cur` marks the step being inspected. `iso-inner` indents calls that run inside the call above by `space-34`.

**You provide:** the step number in true execution order, the callee as written, an origin and line for the meta line, an `await` tag when awaited, and an "Open ›" button only when the callee's own flow can be shown.

- Arguments are numbered before the call that receives them, because they run first.
- The number disc and left stripe share one kind color; the meta line repeats the kind in words, so color is never the only signal.
- Clicking a step highlights its line in the code. "Open ›" drills into the callee.
