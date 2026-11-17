Source code with real line numbers, shown only when the reader asks for it.

**Classes:** `iso-code` wrapping a `pre` of `iso-line` rows; `iso-line-hl` marks the line the current call-flow step points at; token classes `tk-kw`, `tk-str`, `tk-num`, `tk-com`, `tk-type`, `tk-def`, `tk-fn` and `tk-x`.

**You provide:** the source lines and their original line numbers. In the app this is rendered by the Monaco editor themed with the same tokens.

- Line numbers are the file's, never restarted at 1.
- Links are not underlined at rest. Color carries origin: a name defined in this file is `accent` (a local type also at weight 600), an imported name is `ext`. A link shows a 1px underline in its own color on hover, or while ⌘ (Ctrl elsewhere) is held over its pane, and the pointer cursor; non-link code keeps the text cursor.
- A click on a link goes to its definition on mouse-up, and only when the pointer moved 4px or less, so drag-select still works. F12 does the same from the caret.
- Hovering a link for 400ms opens the definition Peek; the editor's own hover is off.
- Brackets stay in `ink`: no bracket-pair colors.
- Exactly one line is highlighted at a time, with `user-soft` and a 3px `user` bar.
- The block scrolls sideways inside itself; lines never wrap.
