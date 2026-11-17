# Python call flow

Python uses the same walker as TS/JS (`crates/iso-parse/src/flow/mod.rs`), driven by the
table in `flow/python.rs`. No reference analyzer exists for Python, so the goldens in
`flow-*/expected/flows.json` were written by hand before the walker. This file is their
spec. The JSON shape and the block and exit kinds are the same as for TS (see CONTRACTS.md,
"Call flow").

## Units

| Source | Unit id | `owner` | `method` |
| --- | --- | --- | --- |
| Module-level `def f` | `f` | `f` | `null` |
| A method of a module-level class | `Class.m` | `Class` | `m` |
| A method of a nested class | `Outer.Inner.m` | `Outer.Inner` | `m` |

Unit ids are the extractor's qualified names for those functions, matched by name and end
line. Cards therefore link to their flow even in unusual cases: a property setter that
reuses its getter's name gets `C.x#2`. A decorated function's unit starts at its first
decorator line, as its symbol does. `params` is the extractor's list: `self`, `x = …`,
`*args`, `**kw`. `async` comes from `async def`.

Nested `def`s and `name = lambda …` inside a unit are **locals**. As in TS, a call to a
local inlines its body (`kind: inline`, `kw: body`). A local passed as an argument becomes a
callback block. A local that is only defined becomes a deferred block (`kw: later`,
"name() defined here, runs when invoked"). Classes inside a function are skipped.

## Statements and expressions

| Python | Flow |
| --- | --- |
| `if` / `elif` / `else` | One flat `cond` block per clause: `if`, then `elif` with its condition, then `else`. All are emitted if any of them has content. Labels are the condition text without parentheses. `elif` condition calls go inside the `elif` block. |
| `a if c else b` | Same as a TS ternary: calls in `c` (`argOf: "(condition)"`), then `if` / `unless` / `else` blocks. |
| `a and b`, `a or b` | `cond` blocks `only if` / `unless` around the calls in `b`, labeled with `a`, as for `&&` / `\|\|`. |
| `for x in xs` | Calls in `xs`, then `loop` "for x in xs". |
| `while c` | `loop` "while c", with the condition's calls inside. |
| `for … else`, `while … else` | `cond` block `else` "if the loop ends without break". |
| `try` | `try` block, then one `catch` block per handler (kw `except`, label as written, such as `(OSError, ValueError) as err`), then `else` ("if no exception", kind `cond`), then `finally` ("always runs"). Empty handlers are dropped, as in TS. |
| `with a as x, b:` | The context expressions' calls, then a block of kind `try`, kw `with`, labeled with the items. **Choice:** `with` guarantees cleanup on exit, which makes it closest to `try`/`finally`. The block keeps the body visually grouped without inventing a new block kind. |
| `match` / `case` | Calls in the subject, then one `cond` block per arm (kw `case`, label is the pattern plus its guard). Guard calls go first in the arm, with `argOf: "(condition)"`. |
| List, set and dict comprehensions, generator expressions | A loop. The first `for` clause's iterable is walked outside the loop. Inside the loop (`loop` "for x in xs") come the later `for` iterables, the `if` filters, then the element expression. A generator passed as the only argument (`sum(f(x) for x in xs)`) is walked as that argument. |
| `lambda` | Deferred block (`later`) where it stands. As an argument it becomes a callback block, or a `loop` for iteration functions (below). |
| `return x` | `exit` `return` with label `x`. `early` is true inside any conditional or loop, as in TS. |
| `raise E(...) from e` | Calls in `E(...)`, then `exit` `throw` with label `E(...)`. A bare `raise` has an empty label. |
| `await f()` | The call has `await: true`. `async for` and `async with` are walked like their plain forms. |
| `yield`, `assert`, `del`, f-strings | Walked for the calls they contain. No special item. |
| Decorators, type annotations, `import`, `global`, `pass` | Skipped. |

## Call targets

Python checks these in order:

1. A bare name defined at module level (`def`) gives **`fn`**.
2. A name imported with `from m import f` gives **`ext`**, `id: f`, `module: m` (relative
   modules as written, such as `.util`).
3. A local (see above) gives **`local`**.
4. A parameter of the unit gives **`param`**. A parameter whose default is a lambda shows
   that lambda as an `inline` `default` block, as in TS.
5. A Python built-in function or exception class (`print`, `len`, `isinstance`, `open`,
   `super`, `ValueError`, …) gives **`builtin`**.
6. Anything else is **`other`**, including classes of the file and names that are not
   defined.

Attribute calls `a.b.c()` are **`method`** (`id: c`), with these exceptions:

- If the chain starts at an imported module or name (`import os` → `os.path.join()`,
  `import numpy as np` → `np.mean()`, `from pkg import mod` → `mod.f()`), the call is
  **`ext`**. Its `id` is the whole callee (`os.path.join`) and `module` is the import
  (`os`, `numpy`, `pkg`). **This is a choice that differs from TS:** analyze.js shows
  namespace-import calls as methods. In Python, module-qualified calls are the normal way
  to call library code.
- `self.m()` and `cls.m()` in a method also get `unit: "Class.m"` when that method is a
  unit, so the view can drill into it.
- `str.join(…)`, `dict.fromkeys(…)` and other calls on built-in types are **`builtin`**.

Callback arguments run once per item for `map`, `filter`, `sorted`, `min` and `max` (the
built-ins), and for the `.sort()` method. Those arguments are shown as
`loop` "each item of map()". Other callable arguments, such as `key=` passed to anything
else, are `callback` blocks. Keyword arguments are unwrapped, so `key=score` counts as
passing `score`.

## Known limits

- Unit discovery covers module-level functions and class bodies. It skips functions defined
  inside `if`/`try` blocks at module level and classes created inside functions.
- `match` arms have no "else" notion. A wildcard `case _:` is just another arm.
- Name resolution uses the extractor's import facts and the file's own definitions only.
  There is no type inference, so `obj.m()` is a method whatever `obj` is.
