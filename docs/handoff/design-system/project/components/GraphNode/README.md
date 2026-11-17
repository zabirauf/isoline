A function or an imported module in the dependency map, with edges that run from caller to callee.

**Classes (SVG):** `iso-node` with `iso-node-sel` (selected, `ink` 2px), `iso-node-dep` (`accent`) or `iso-node-user` (`user`); `iso-mod` for a module with its imported symbols; `iso-edge` with `iso-edge-dep`, `iso-edge-user` or `iso-edge-ext`; `iso-colhead` for column heads.

**You provide:** the symbol name, a caption of facts ("67 lines · 6 calls · 1 imports"), a layer for the column, and edges.

- Entry points sit in the left column; each column to the right is one layer deeper, and external imports are last.
- An exported function has a 4px `accent` stripe on its left edge.
- When something is selected, what it depends on turns `accent` and what depends on it turns `user`; everything else drops to 25% opacity.
- Edges are curves that leave the right side of the caller and arrive at the left side of the callee. Unfocused edges stay thin and `muted`.
- Nodes are 50px tall with a 7px radius. No shadows, no gradients.
