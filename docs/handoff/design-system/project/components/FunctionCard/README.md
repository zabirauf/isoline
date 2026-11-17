The unit of the reading list: one function as a one-line description, its main steps, and its code on demand.

**Classes:** `iso-card`, plus `iso-card-exported` (accent stripe), `iso-card-sel` (selected), `iso-card-dep` (something the selection depends on) and `iso-card-user` (something that depends on the selection).

**You provide, top to bottom:** layer chip, symbol name, optional export pill, line range; one sentence saying what it does; three to seven steps; optional notes; calls and imports as chips; a reveal button for the code.

- The description is one sentence that starts with a verb. The steps are the algorithm, not a restatement of the code.
- Code is collapsed by default. The summary must be enough to decide whether to open it.
- The left stripe carries state: `line` by default, `accent` for exported or depended-on, `user` for dependents. Do not use the stripe for decoration.
- Unrelated cards fade to 42% opacity when something is selected; they never disappear.
