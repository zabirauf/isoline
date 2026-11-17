A frame around the calls that run only under some condition, in a loop, inside a callback, or on the error path.

**Classes:** `iso-blk` with one kind: `iso-b-cond` (if, else, ternary, short-circuit), `iso-b-try`, `iso-b-catch`, `iso-b-finally` (dashed), `iso-b-loop`, `iso-b-callback` (dashed, runs inside the call above), `iso-b-deferred` (dotted, defined here but run later). Exits use `iso-exit`, `iso-exit-early` and `iso-exit-throw`.

**You provide:** the keyword for the tag, the condition or loop head as written in code, and the steps inside. Frames nest.

- Every frame starts with an uppercase keyword tag, so its kind is readable without color.
- Solid frames change whether or how often code runs. Dashed and dotted frames change when or where it runs.
- Show both arms of a branch when either has calls, and write "no calls" in the empty one.
- `warn` is reserved for catch and throw.
