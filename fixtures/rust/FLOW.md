# Rust call flow

Rust uses the same walker as TS/JS (`crates/iso-parse/src/flow/mod.rs`), driven by the
table in `flow/rust.rs`. No reference analyzer exists for Rust, so the goldens in
`flow-*/expected/flows.json` were written by hand before the walker. This file is their
spec. The JSON shape and the block and exit kinds are the same as for TS (see CONTRACTS.md,
"Call flow").

## Units

| Source | Unit id | `owner` | `method` |
| --- | --- | --- | --- |
| A free `fn f` with a body | `f` | `f` | `null` |
| `fn m` in `impl Type` or `impl Trait for Type` | `Type.m` | `Type` | `m` |
| A trait method with a default body | `Trait.m` | `Trait` | `m` |
| An item inside an inline `mod x { … }` | `x.f`, `x.Type.m` | as above | as above |

Unit ids are the extractor's qualified names for those functions, matched by name and end
line. Cards therefore link to their flow even in unusual cases: two `#[cfg]` alternatives
of `mod sync` become `sync.wait` and `sync#2.wait`, and `impl … for *const T` gives
`T.is_null`. A unit spans the `fn` item; attributes such as `#[test]` are separate items.
`params` is the extractor's list: `&self`, `mut n`, `(a, b)`. `async` comes from
`async fn`.

Nested `fn` items and `let f = |…| …` closures inside a unit are **locals**. As in TS, a
call to a local inlines its body (`inline` / `body`). A local passed as an argument becomes
a callback block. A local that is only defined becomes a deferred block (`later`).

## Statements and expressions

| Rust | Flow |
| --- | --- |
| `if c {…} else {…}`, `else if` | As in TS: `cond` `if` / `else`, and an `else if` nests inside the `else` block. Labels have no parentheses. |
| `if let P = e` | `cond` `if` labeled `let P = e`. Calls in `e` come first, with `argOf: "(condition)"`. |
| `match e { P => …, P if g => … }` | Calls in `e`, then one `cond` block per arm that has content: kw `match`, label is the pattern with its guard (`v if v > 10`). Guard calls go first, with `argOf: "(condition)"`. |
| `let P = e else { … }` | Calls in `e`, then a `cond` block `else` "if the pattern does not match" holding the diverging block. |
| `loop {…}` | `loop` "loop". |
| `while c {…}`, `while let P = e {…}` | `loop` "while c" / "while let P = e", with the condition's calls inside. |
| `for p in e {…}` | Calls in `e`, then `loop` "for p in e". |
| `a && b`, `a \|\| b` | `cond` `only if` / `unless`, as in TS. |
| `return e` | `exit` `return` with label `e`. `early` is true inside a conditional or loop. |
| Tail expression | **Choice:** when the function declares a return type and its body ends with a value expression, that expression's calls are followed by `exit` `return` (`early: false`), because it is the function's return value. Tails that are `if`, `match`, loops or blocks (their arms carry the values), `()`, and functions without a return type get no exit. Closure bodies never get one. |
| `e?` | Calls in `e`, then `exit` `throw` labeled `e?`. **Choice:** `?` returns early with the error (or `None`), which makes it Rust's `throw`. It is shown at every use so the early-exit points are visible. |
| `panic!`, `unreachable!`, `todo!`, `unimplemented!`, `bail!` | Calls found in the macro's arguments, then `exit` `throw` labeled with the macro text. **Choice:** these never return normally. `bail!` (anyhow) is `return Err(…)`. `assert!` and friends are calls, because they usually return. |
| Other macros | Calls found in the arguments (each with `argOf` the macro), then a call to the macro itself, such as `println!`. It is `builtin` for standard macros (`println!`, `format!`, `vec!`, `write!`, `assert_eq!`, `dbg!`, …), `ext` for a macro imported with `use`, and `other` otherwise. Macro arguments are token trees, not expressions. Isoline scans them for `f(…)`, `a::b(…)`, `x.m(…)` and `f(…).m(…)` shapes, but not for control flow. |
| Closures `\|x\| …` | Deferred block where they stand. As an argument they become a callback block (`thread::spawn(move \|\| …)`, `unwrap_or_else(\|\| …)`), or a `loop` for iterator adapters (below). |
| `.await` | The awaited call has `await: true`. |
| `unsafe {}`, `async {}`, struct literals, references, casts, indexing | Walked for the calls they contain. |
| Items inside a body (`struct`, `impl`, `use`, …), attributes, types, comments | Skipped. Nested `fn` items are locals (see above). |

## Call targets

Bare names `f(…)` are checked in this order:

1. A function of the same inline module (`tests.helper`) gives **`fn`** with that id.
2. A free function at file level gives **`fn`**.
3. A name imported with `use a::b::f` gives **`ext`**, `id: f`, `module: a::b`.
4. A local gives **`local`**.
5. A parameter gives **`param`**.
6. `Some`, `Ok`, `Err` and `drop` give **`builtin`**.
7. Anything else is **`other`**, including tuple-struct and enum-variant constructors.

Paths `A::B::f(…)` (turbofish removed) are checked in this order:

1. `Self::f` gives **`method`** `f`, with `unit: "Type.f"` when that method is a unit of
   the current type.
2. `Type::f` for a type with an `impl` in this file gives **`method`** `f`, with `unit`.
3. `x::f` for an inline module of this file gives **`fn`** with id `x.f`.
4. A path rooted at a `use` binding gives **`ext`** with the whole path as `id`. The module
   is the binding's full path plus the middle segments: `use std::thread` makes
   `thread::spawn` come from `std::thread`, and `use std::sync::Arc` makes `Arc::new` come
   from `std::sync::Arc`.
5. `crate::…`, `super::…` and `self::…` give **`ext`**, with the module being every segment
   but the last.
6. A path rooted at `std`, `core` or `alloc`, at a prelude type (`String`, `Vec`, `Box`,
   `Option`, `Result`, `Rc`, `Arc`, …) or at a primitive type (`u8::from`) gives
   **`builtin`**.
7. Anything else is **`other`**. That includes paths rooted at a crate that is not imported
   with `use`: from one file the walker cannot tell such a crate from a module declared in
   another file.

Method calls `recv.m(…)` are **`method`** `m`. `self.m(…)` also gets `unit: "Type.m"`
when `Type.m` is a unit.

Iterator adapters whose closure runs once per item are shown as `loop` "each item of
.map()": `map`, `for_each`, `filter`, `filter_map`, `flat_map`, `fold`, `try_fold`, `any`,
`all`, `find`, `find_map`, `position`, `take_while`, `skip_while`, `map_while`, `inspect`,
`try_for_each`, `retain`, `sort_by*`, `sort_unstable_by*`, `max_by*`, `min_by*`,
`partition`, `scan`, `reduce`, `dedup_by_key` and `chunk_by`. `Option::map` therefore also
shows as a loop, the same ambiguity TS has with `.map`.

## Known limits

- Calls inside macro arguments are found by token shape only. Closures and control flow
  inside macros (such as `tokio::select!` arms) are not walked.
- `impl` blocks inside function bodies give no units.
- The facts pass runs again when the flow is computed, to read imports and symbol names.
  Flow is computed on demand and cached, so this costs one extra extraction per opened file.
