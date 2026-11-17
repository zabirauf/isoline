Source code with real line numbers, shown only when the reader asks for it.

**Classes:** `iso-code` wrapping a `pre` of `iso-line` rows; `iso-line-hl` marks the line the current call-flow step points at; token classes `tk-kw`, `tk-str`, `tk-num`, `tk-com`, `tk-type`, `tk-def`, `tk-fn` and `tk-x`.

**You provide:** the source lines and their original line numbers. In the app this is rendered by the Monaco editor themed with the same tokens.

- Line numbers are the file's, never restarted at 1.
- A name defined in this file is a solid `accent` link; an imported name is a dotted `ext` link. Both jump to the definition.
- Exactly one line is highlighted at a time, with `user-soft` and a 3px `user` bar.
- The block scrolls sideways inside itself; lines never wrap.
