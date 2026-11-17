Isoline is a tool for understanding code without reading every line. The interface shows summaries first and code on demand, so the design is quiet, dense and exact: neutral surfaces, one blue for interaction, and a small set of colors that each mean one thing.

## Principles

- **Summary before detail.** Every view opens with the shortest useful description. Code, long lists and deep graphs are one click away, never the first thing shown.
- **Color is meaning.** A color names an origin or a kind of control flow. If a color does not say something true about the code, remove it.
- **Two views, one selection.** The list and the graph always show the same selection. `accent` marks what the selection depends on and `user` marks what depends on it, everywhere.
- **Fast and calm.** Transitions finish in well under a second and nothing scrolls unless the reader caused it.

## Voice and wording

- Write in plain, short sentences. Descriptions start with a verb: "Builds the identity fields shared by every harness event."
- Use sentence case for every label: "Expand all code", "Entry points first", "Call flow".
- A control says exactly what it does and how much it affects: "Show code · 67 lines", "Clear (Esc)".
- Use the code's own words for code. Identifiers stay in mono and are never translated or prettified.
- State facts with numbers and units: "12 functions", "L357–423", "6 calls".
- Flag uncertainty honestly. A note describes what was observed ("sits outside the try, so...") and never orders the reader to act.
- No emoji, no exclamation marks, no marketing words.

## Color

- Set pages on `bg` and put content on `surface` cards with a 1px `line` border. Text is `ink`; secondary text is `muted`.
- Use `accent` for everything interactive: links, active toggles, the focus ring, exported symbols. Pair it with `accent-soft` as the fill behind an active control.
- In a focused graph or list, `accent` means "the selection depends on this" and `user` means "this depends on the selection". Never swap them and never use `user` for anything else except the current code line.
- Use `ext` for anything that comes from outside the file being viewed.
- In a call flow, each kind of control structure has one color: `cond` for conditionals, `try` for try and finally, `warn` for catch and throw, `loop` for loops, `method` for controller methods and local helpers.
- `warn` means a possible problem. Do not use it for emphasis.
- Every `-soft` color is a fill for its base color and carries `ink` text.
- Never rely on color alone. Each colored element also carries a word, a keyword tag or a distinct shape.
- Both themes are complete. Dark is not an inversion: fills get darker and hues get lighter so the same pairs stay readable.
- `tok-com` is kept from the source at about 3:1 on `code-bg`. Use it only for comments and line numbers.

## Type

- Three IBM Plex faces, loaded from Google Fonts: `display` (IBM Plex Sans Condensed) for titles and uppercase labels, `body` (IBM Plex Sans) for text and controls, `mono` (IBM Plex Mono) for anything that is code.
- Body text is `body` at 14px. Controls use `body-small`, metadata uses `caption`.
- Set every identifier, line number, event name and keyword tag in mono. If it could be pasted into an editor, it is mono.
- Use `display-eyebrow` and `mono-tag` in uppercase with letter spacing; nothing else is uppercase.
- Keep running text under 72 characters wide. Use tabular figures wherever numbers line up.

## Space, shape and depth

- The scale is small and literal: `space-4` between chips, `space-8` inside a card, `space-10` between cards, `space-12` by `space-14` card padding, `space-20` page gutters.
- Radii grow with the size of the thing: `radius-4` for chips, `radius-7` for controls and code, `radius-9` for cards, `radius-10` for frames, `radius-pill` for counters.
- Depth comes from borders and fills, not shadows. The only shadow is `shadow-lift`, on a call-flow step while it moves.
- A 3px left stripe on a card or step states its kind or state. Do not add stripes for decoration.
- Solid borders mean "this changes whether code runs". Dashed and dotted borders mean "this changes when or where it runs", or "optional".

## Layout

- The main window is a split: reading list on the left, graph on the right, with a draggable divider. Below 900px the panes stack with the graph on top.
- The list is grouped by layer, entry points first. The graph uses the same layers as columns, entry points on the left.
- Toolbars stick to the top of their pane and wrap instead of scrolling sideways.
- Only code, wide tables and the graph may scroll sideways, each inside its own container.

## Motion and states

- Transitions are 150 to 380ms with an ease-out curve. A whole call flow lands in about one second, staggered a few milliseconds per step.
- Unrelated items fade (cards to 42%, graph nodes to 25%); they never vanish, so the reader keeps their place.
- Hover changes a border to `accent`. Focus is a solid 2px `accent` outline with a 2px offset on every control.
- Honor reduced-motion: show the final state with no animation.

## Iconography and imagery

- There is no icon set. Controls use words, and the few glyphs are text characters: "›" for drilling in, "◀" and "▶" for stepping, "+" and "−" for zoom, "i" and "!" in notes.
- There is no logo yet. Set the name "Isoline" in `display` at weight 600.
- The only imagery is the product itself: the screenshots in the Reference assets show the prototype these rules were taken from.
