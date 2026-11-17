// Deterministic generators for scale testing: a repo tree with thousands of files and a module
// map with thousands of modules. Used by the mock backend when the URL asks for them.
import type { TreeNode } from "../bindings/TreeNode";
import type { FileStatus } from "../bindings/FileStatus";
import type { Language } from "../bindings/Language";
import type { Tier } from "../bindings/Tier";
import type { RawModuleMap } from "./index";

/** Small seeded PRNG (mulberry32) so generated data is the same on every run. */
export function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const PALETTE = ["#2f6fb0", "#7a5bb5", "#3d8a7a", "#b5651d", "#5d7a2a", "#a8467a", "#1f7f99", "#9a5b2e", "#6b6f2a", "#80539a", "#46708f", "#7d7466"];

const EXT: [string, Language, Tier][] = [
  ["ts", "typescript", "full"],
  ["ts", "typescript", "full"],
  ["tsx", "tsx", "full"],
  ["py", "python", "full"],
  ["rs", "rust", "full"],
  ["go", "go", "outline"],
  ["java", "java", "outline"],
  ["md", "markdown", "text"],
  ["json", "json", "text"],
];
const WORDS = ["auth", "billing", "cache", "core", "events", "gateway", "ingest", "jobs", "ledger", "mail", "orders", "search", "shared", "sync", "ui", "users"];

export function generateTree(count: number): TreeNode {
  const r = rng(count);
  const root: TreeNode = { name: "monorepo", path: "", isDir: true, lang: null, tier: null, module: null, color: null, status: "indexed", lines: 0, children: [] };
  const dirs = new Map<string, TreeNode>([["", root]]);
  const dir = (path: string): TreeNode => {
    const hit = dirs.get(path);
    if (hit) return hit;
    const i = path.lastIndexOf("/");
    const parent = dir(i < 0 ? "" : path.slice(0, i));
    const top = path.split("/")[1] ?? path.split("/")[0];
    const mi = WORDS.indexOf(top);
    const n: TreeNode = { name: path.slice(i + 1), path, isDir: true, lang: null, tier: null, module: mi >= 0 ? top : null, color: mi >= 0 ? PALETTE[mi % PALETTE.length] : null, status: "indexed", lines: 0, children: [] };
    parent.children.push(n);
    dirs.set(path, n);
    return n;
  };
  for (let i = 0; i < count; i++) {
    const w = WORDS[Math.floor(r() * WORDS.length)];
    const sub = `part${Math.floor(r() * 12)}`;
    const leafDir = `pkg/${w}/${sub}/mod${Math.floor(r() * 9)}`;
    const [ext, lang, tier] = EXT[Math.floor(r() * EXT.length)];
    const d = dir(leafDir);
    const name = `${w}-${i.toString(36)}.${ext}`;
    const status: FileStatus = r() < 0.02 ? "pending" : r() < 0.003 ? "failed" : "indexed";
    const lines = 20 + Math.floor(r() * 600);
    d.children.push({ name, path: `${leafDir}/${name}`, isDir: false, lang, tier, module: w, color: PALETTE[WORDS.indexOf(w) % PALETTE.length], status, lines, children: [] });
  }
  const finish = (n: TreeNode) => {
    n.children.sort((a, b) => (a.isDir === b.isDir ? a.name.localeCompare(b.name) : a.isDir ? -1 : 1));
    n.children.forEach(finish);
    if (n.isDir) {
      n.lines = n.children.reduce((s, c) => s + c.lines, 0);
      n.status = n.children.some((c) => c.status === "pending") ? "pending" : "indexed";
    }
  };
  finish(root);
  return root;
}

export function generateModuleMap(n: number): RawModuleMap {
  const r = rng(n * 7 + 1);
  const modules = Array.from({ length: n }, (_, i) => {
    const w = WORDS[i % WORDS.length];
    return {
      id: `m${i}`,
      name: `${w}-${Math.floor(i / WORDS.length)}`,
      role: null,
      color: PALETTE[i % PALETTE.length],
      source: "cluster" as const,
      dir: `pkg/${w}/part${Math.floor(i / WORDS.length)}`,
      files: 1 + Math.floor(r() * 40),
      lines: 100 + Math.floor(r() * 8000),
    };
  });
  const edges: { from: string; to: string; weight: number }[] = [];
  for (let i = 0; i < n; i++) {
    const k = 1 + Math.floor(r() * 2.4);
    for (let j = 0; j < k; j++) {
      // Mostly forward edges (a DAG with depth), a few back edges make cycles.
      const span = Math.max(1, Math.floor(n / 6));
      const t = r() < 0.03 ? Math.floor(r() * n) : Math.min(n - 1, i + 1 + Math.floor(r() * span));
      if (t !== i) edges.push({ from: `m${i}`, to: `m${t}`, weight: 1 + Math.floor(r() * 6) });
    }
  }
  return { modules, edges, cycles: [] };
}
