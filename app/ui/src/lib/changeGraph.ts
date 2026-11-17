// Layout for the compare view's change graph: the modules a comparison touched, the modules at the
// other end of a dependency that appeared, vanished or changed weight, and those dependencies.
// Layered left to right in dependency direction: a module sits left of what it imports.
import type { ModuleChange } from "../ipc/bindings/ModuleChange";
import type { ModuleEdgeChange } from "../ipc/bindings/ModuleEdgeChange";

export const CG_W = 168;
export const CG_H = 50;
const GAP_X = 72;
const GAP_Y = 18;
const PAD = 16;

export type EdgeKind = "added" | "removed" | "more" | "fewer";

export interface CGNode {
  id: string;
  name: string;
  color: string;
  /** Has changed files (false: shown only as the other end of a dependency). */
  changed: boolean;
  change: ModuleChange | null;
  layer: number;
  x: number;
  y: number;
}

export interface CGEdge {
  from: string;
  to: string;
  before: number;
  after: number;
  kind: EdgeKind;
  /** SVG path from the right side of `from` to the left side of `to` (or around, for back edges). */
  d: string;
  /** Label anchor. */
  lx: number;
  ly: number;
}

export interface ChangeGraph {
  nodes: CGNode[];
  edges: CGEdge[];
  width: number;
  height: number;
  /** Changed modules left out to keep the graph readable. */
  hidden: number;
}

export function edgeKind(e: { before: number; after: number }): EdgeKind {
  if (e.before === 0) return "added";
  if (e.after === 0) return "removed";
  return e.after > e.before ? "more" : "fewer";
}

/** "new", "removed", "+2", "−1": what an edge's label says. */
export function edgeLabel(e: { before: number; after: number }): string {
  const k = edgeKind(e);
  if (k === "added") return "new";
  if (k === "removed") return "removed";
  const d = e.after - e.before;
  return d > 0 ? `+${d}` : `−${-d}`;
}

export function buildChangeGraph(modules: ModuleChange[], edges: ModuleEdgeChange[], info: (id: string) => { name: string; color: string } | null, maxNodes = 40): ChangeGraph {
  const changed = [...modules].sort((a, b) => b.additions + b.deletions - (a.additions + a.deletions));
  const keep = changed.slice(0, maxNodes);
  const ids = new Set(keep.map((m) => m.id));
  const es = edges.filter((e) => e.before !== e.after && e.from !== e.to && (ids.has(e.from) || ids.has(e.to)));
  const nodes = new Map<string, CGNode>();
  for (const m of keep) nodes.set(m.id, { id: m.id, name: m.name, color: m.color, changed: true, change: m, layer: 0, x: 0, y: 0 });
  for (const e of es)
    for (const id of [e.from, e.to])
      if (!nodes.has(id)) {
        const i = info(id);
        nodes.set(id, { id, name: i?.name ?? id, color: i?.color ?? "#87909d", changed: false, change: null, layer: 0, x: 0, y: 0 });
      }

  // Layers: longest path from the importers, ignoring the edges that close a cycle.
  const out = new Map<string, string[]>();
  for (const n of nodes.keys()) out.set(n, []);
  for (const e of es) out.get(e.from)!.push(e.to);
  const state = new Map<string, 0 | 1 | 2>();
  const back = new Set<string>();
  const order: string[] = [];
  const visit = (n: string) => {
    state.set(n, 1);
    for (const m of out.get(n)!) {
      const s = state.get(m) ?? 0;
      if (s === 1) back.add(`${n}\u0000${m}`);
      else if (s === 0) visit(m);
    }
    state.set(n, 2);
    order.push(n);
  };
  for (const n of [...nodes.keys()].sort()) if (!state.get(n)) visit(n);
  order.reverse(); // topological: importers first
  for (const n of order)
    for (const m of out.get(n)!) {
      if (back.has(`${n}\u0000${m}`)) continue;
      const a = nodes.get(n)!;
      const b = nodes.get(m)!;
      b.layer = Math.max(b.layer, a.layer + 1);
    }

  // Rows within a layer: by the mean row of the importers (one sweep), changed modules first.
  const layers: CGNode[][] = [];
  for (const n of nodes.values()) (layers[n.layer] ??= []).push(n);
  const rowOf = new Map<string, number>();
  const preds = new Map<string, string[]>();
  for (const e of es) (preds.get(e.to) ?? preds.set(e.to, []).get(e.to)!).push(e.from);
  layers.forEach((L, li) => {
    const key = (n: CGNode) => {
      const p = (preds.get(n.id) ?? []).map((x) => rowOf.get(x)).filter((x): x is number => x !== undefined);
      return p.length ? p.reduce((s, x) => s + x, 0) / p.length : li === 0 ? 0 : Number.MAX_SAFE_INTEGER;
    };
    L.sort((a, b) => key(a) - key(b) || Number(b.changed) - Number(a.changed) || (b.change ? b.change.additions + b.change.deletions : 0) - (a.change ? a.change.additions + a.change.deletions : 0) || a.name.localeCompare(b.name));
    L.forEach((n, i) => rowOf.set(n.id, i));
  });
  const rows = Math.max(1, ...layers.map((L) => L?.length ?? 0));
  const colH = (n: number) => n * CG_H + (n - 1) * GAP_Y;
  const height = PAD * 2 + colH(rows) + 24; // room for back edges below
  layers.forEach((L, li) => {
    if (!L) return;
    const top = PAD + (colH(rows) - colH(L.length)) / 2;
    L.forEach((n, i) => {
      n.x = PAD + li * (CG_W + GAP_X);
      n.y = top + i * (CG_H + GAP_Y);
    });
  });
  const width = PAD * 2 + Math.max(1, layers.length) * CG_W + Math.max(0, layers.length - 1) * GAP_X;

  const bottom = PAD + colH(rows);
  const cgEdges: CGEdge[] = es.map((e) => {
    const a = nodes.get(e.from)!;
    const b = nodes.get(e.to)!;
    let d: string;
    let lx: number;
    let ly: number;
    if (b.layer > a.layer) {
      const x1 = a.x + CG_W;
      const y1 = a.y + CG_H / 2;
      const x2 = b.x;
      const y2 = b.y + CG_H / 2;
      const mx = (x1 + x2) / 2;
      d = `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`;
      lx = mx;
      ly = (y1 + y2) / 2;
    } else {
      // Same layer or backwards (a cycle): loop under the graph, from the bottom of one to the bottom of the other.
      const x1 = a.x + CG_W / 2;
      const y1 = a.y + CG_H;
      const x2 = b.x + CG_W / 2 + (a.id === b.id ? 20 : 0);
      const y2 = b.y + CG_H;
      const yb = Math.max(y1, y2, bottom) + 20;
      d = `M${x1},${y1} C${x1},${yb} ${x2},${yb} ${x2},${y2}`;
      lx = (x1 + x2) / 2;
      ly = yb - 6;
    }
    return { from: e.from, to: e.to, before: e.before, after: e.after, kind: edgeKind(e), d, lx, ly };
  });
  return { nodes: [...nodes.values()], edges: cgEdges, width, height, hidden: changed.length - keep.length };
}
