// Rebuilds the tree from `get_tree_packed`: parallel arrays, parents before children. A tenth
// of the bytes of the nested form for a big repo, so the webview parses far less JSON.
import type { FileStatus } from "../ipc/bindings/FileStatus";
import type { Tier } from "../ipc/bindings/Tier";
import type { TreeNode } from "../ipc/bindings/TreeNode";
import type { TreePacked } from "../ipc/bindings/TreePacked";

const STATUS: FileStatus[] = ["indexed", "pending", "failed", "skipped"];
const TIER: (Tier | null)[] = [null, "full", "outline", "text"];

export function unpackTree(p: TreePacked): TreeNode {
  const n = p.parent.length;
  const nodes: TreeNode[] = new Array(n);
  for (let i = 0; i < n; i++) {
    const f = p.flags[i];
    const m = p.module[i];
    const l = p.lang[i];
    const parent = i === 0 ? null : nodes[p.parent[i]];
    const name = p.names[i];
    const node: TreeNode = {
      name,
      path: i === 0 ? "" : parent!.path ? `${parent!.path}/${name}` : name,
      isDir: (f & 1) === 1,
      lang: l ? p.langs[l - 1] : null,
      tier: TIER[(f >> 3) & 3],
      module: m ? p.modules[m - 1] : null,
      color: m ? p.colors[m - 1] || null : null,
      status: STATUS[(f >> 1) & 3],
      lines: p.lines[i],
      children: [],
    };
    nodes[i] = node;
    if (parent) parent.children.push(node);
  }
  return nodes[0];
}
