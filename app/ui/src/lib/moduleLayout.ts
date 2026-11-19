// Module-level map layout for repo scale (thousands of modules). Same idea as the file map:
// columns by dependency depth (entry points left, deeper layers to the right), barycentre
// ordering, but index-based and iterative so 5,000 modules lay out in a few tens of
// milliseconds. Tall columns wrap into sub-columns so the map stays roughly as wide as tall.
import type { ModuleMap } from "../ipc/bindings/ModuleMap";

export const MOD_W = 150;
export const MOD_H = 40;
const ROW_GAP = 12;
const SUB_GAP = 16;
const COL_GAP = 40;

/** Width to height the wrapped layout aims for: about the shape of the map pane. */
const ASPECT = 2.4;

/**
 * How many rows a column may have before it wraps into another sub-column, chosen so the whole
 * map comes out close to `aspect` (width / height). Short maps never wrap.
 */
export function rowsForAspect(counts: number[], aspect: number): number {
  const tallest = Math.max(1, ...counts);
  let best = tallest;
  let bestErr = Infinity;
  for (let rows = Math.min(tallest, 6); rows <= tallest; rows = rows < 40 ? rows + 1 : Math.ceil(rows * 1.05)) {
    let w = 0;
    for (const c of counts) if (c) w += Math.ceil(c / rows) * (MOD_W + SUB_GAP) - SUB_GAP + COL_GAP;
    const h = rows * (MOD_H + ROW_GAP);
    const err = Math.abs(Math.log(w / h / aspect));
    if (err < bestErr) {
      bestErr = err;
      best = rows;
    }
    if (rows === tallest) break;
  }
  return best;
}

export interface ModuleColumn {
  x: number;
  /** Width including wrapped sub-columns. */
  w: number;
  y0: number;
  y1: number;
  count: number;
  level: number;
  label: string;
}
export interface ModuleBundle {
  from: number;
  to: number;
  count: number;
}

export interface ModuleLayout {
  /** Top-left of each module box, in the order of `map.modules`. */
  x: Float64Array;
  y: Float64Array;
  level: Int32Array;
  /** Edge endpoints as module indices. */
  from: Int32Array;
  to: Int32Array;
  weight: Float64Array;
  index: Map<string, number>;
  /** Column of each module (index into `columns`). */
  col: Int32Array;
  /** One column per dependency depth, entry points first (left). */
  columns: ModuleColumn[];
  /** Edges summed per pair of columns, for the zoomed-out view. */
  bundles: ModuleBundle[];
  W: number;
  H: number;
}

/**
 * Strongly connected components (iterative Tarjan): `comp[v]` numbers each module's cycle group.
 * Components come out in reverse topological order: a component's dependencies come first.
 */
export function components(n: number, out: number[][]): { comp: Int32Array; count: number } {
  const comp = new Int32Array(n).fill(-1);
  const idx = new Int32Array(n).fill(-1);
  const low = new Int32Array(n);
  const onStack = new Uint8Array(n);
  const stack: number[] = [];
  let next = 0;
  let count = 0;
  for (let root = 0; root < n; root++) {
    if (idx[root] >= 0) continue;
    const call: [number, number][] = [[root, 0]];
    idx[root] = low[root] = next++;
    stack.push(root);
    onStack[root] = 1;
    while (call.length) {
      const top = call[call.length - 1];
      const v = top[0];
      if (top[1] < out[v].length) {
        const w = out[v][top[1]++];
        if (idx[w] < 0) {
          idx[w] = low[w] = next++;
          stack.push(w);
          onStack[w] = 1;
          call.push([w, 0]);
        } else if (onStack[w]) low[v] = Math.min(low[v], idx[w]);
        continue;
      }
      call.pop();
      if (call.length) {
        const u = call[call.length - 1][0];
        low[u] = Math.min(low[u], low[v]);
      }
      if (low[v] === idx[v]) {
        let w: number;
        do {
          w = stack.pop()!;
          onStack[w] = 0;
          comp[w] = count;
        } while (w !== v);
        count++;
      }
    }
  }
  return { comp, count };
}

/**
 * Longest dependency path to a module with no dependencies. Modules in one import cycle share a
 * depth: without that, every cycle in a large repo adds columns and the map becomes a thin
 * strip hundreds of columns wide.
 */
export function moduleLevels(n: number, out: number[][]): Int32Array {
  const { comp, count } = components(n, out);
  const members: number[][] = Array.from({ length: count }, () => []);
  for (let v = 0; v < n; v++) members[comp[v]].push(v);
  // Reverse topological order: every dependency's component is numbered before its users'.
  const cl = new Int32Array(count);
  for (let c = 0; c < count; c++) {
    let l = 0;
    for (const v of members[c]) for (const w of out[v]) if (comp[w] !== c) l = Math.max(l, cl[comp[w]] + 1);
    cl[c] = l;
  }
  const level = new Int32Array(n);
  for (let v = 0; v < n; v++) level[v] = cl[comp[v]];
  return level;
}

export function layoutModules(map: ModuleMap): ModuleLayout {
  const n = map.modules.length;
  const index = new Map(map.modules.map((m, i) => [m.id, i]));
  const edges = map.edges.filter((e) => index.has(e.from) && index.has(e.to) && e.from !== e.to);
  const from = new Int32Array(edges.map((e) => index.get(e.from)!));
  const to = new Int32Array(edges.map((e) => index.get(e.to)!));
  const weight = new Float64Array(edges.map((e) => e.weight));
  const out: number[][] = Array.from({ length: n }, () => []);
  const nb: number[][] = Array.from({ length: n }, () => []);
  for (let i = 0; i < from.length; i++) {
    out[from[i]].push(to[i]);
    nb[from[i]].push(to[i]);
    nb[to[i]].push(from[i]);
  }
  const level = moduleLevels(n, out);
  const maxL = n ? Math.max(...level) : 0;

  // Columns, entry points (highest level) first; initial order by size then id for determinism.
  const cols: number[][] = [];
  for (let l = maxL; l >= 0; l--) cols.push([]);
  for (let i = 0; i < n; i++) cols[maxL - level[i]].push(i);
  const size = (i: number) => map.modules[i].files;
  cols.forEach((c) => c.sort((a, b) => size(b) - size(a) || (map.modules[a].id < map.modules[b].id ? -1 : 1)));

  const maxRows = rowsForAspect(cols.map((c) => c.length), ASPECT);
  const pos = new Float64Array(n); // vertical position used for barycentres
  const place = () => {
    for (const c of cols) {
      const rows = Math.min(maxRows, c.length);
      c.forEach((v, k) => (pos[v] = (k % rows) - rows / 2));
    }
  };
  place();
  const bary = new Float64Array(n);
  for (let it = 0; it < 6; it++) {
    const seq = it % 2 ? cols.slice().reverse() : cols;
    for (const c of seq) {
      for (const v of c) {
        const ns = nb[v];
        let s = 0;
        for (const w of ns) s += pos[w];
        bary[v] = ns.length ? s / ns.length : pos[v];
      }
      c.sort((a, b) => bary[a] - bary[b]);
      const rows = Math.min(maxRows, c.length);
      c.forEach((v, k) => (pos[v] = (k % rows) - rows / 2));
    }
  }

  const x = new Float64Array(n);
  const y = new Float64Array(n);
  const col = new Int32Array(n);
  const columns: ModuleColumn[] = [];
  const label = (l: number) => (l === maxL && maxL > 0 ? `Depth ${l} · entry points` : l === 0 ? (maxL > 0 ? "Depth 0 · no dependencies" : "Depth 0") : `Depth ${l}`);
  let cx = 0;
  let H = 0;
  cols.forEach((c, ci) => {
    if (!c.length) return;
    const rows = Math.min(maxRows, c.length);
    const subs = Math.ceil(c.length / rows);
    const l = maxL - ci;
    const w = subs * (MOD_W + SUB_GAP) - SUB_GAP;
    const h = rows * (MOD_H + ROW_GAP) - ROW_GAP;
    const index = columns.length;
    columns.push({ x: cx, w, y0: 0, y1: h, count: c.length, level: l, label: label(l) });
    c.forEach((v, k) => {
      x[v] = cx + Math.floor(k / rows) * (MOD_W + SUB_GAP);
      y[v] = (k % rows) * (MOD_H + ROW_GAP);
      col[v] = index;
    });
    H = Math.max(H, h);
    cx += w + COL_GAP;
  });
  // Center each column vertically.
  for (const c of columns) {
    const off = (H - c.y1) / 2;
    c.y0 += off;
    c.y1 += off;
  }
  for (let i = 0; i < n; i++) y[i] += columns[col[i]].y0;

  // Column-to-column bundles: what the zoomed-out view draws instead of thousands of edges.
  const bundleMap = new Map<number, ModuleBundle>();
  for (let i = 0; i < from.length; i++) {
    const a = col[from[i]];
    const b = col[to[i]];
    if (a === b) continue;
    const key = a * 100003 + b;
    const e = bundleMap.get(key);
    if (e) e.count++;
    else bundleMap.set(key, { from: a, to: b, count: 1 });
  }
  const bundles = [...bundleMap.values()].sort((p, q) => p.from - q.from || p.to - q.to);
  return { x, y, level, col, from, to, weight, index, columns, bundles, W: Math.max(0, cx - COL_GAP), H };
}

/** Uniform grid over the layout for hit testing and viewport culling. */
export class SpatialGrid {
  private cells = new Map<number, number[]>();
  constructor(
    private L: ModuleLayout,
    private size = 256,
  ) {
    for (let i = 0; i < L.x.length; i++) {
      const k = this.key(Math.floor(L.x[i] / size), Math.floor(L.y[i] / size));
      let c = this.cells.get(k);
      if (!c) this.cells.set(k, (c = []));
      c.push(i);
    }
  }
  private key(cx: number, cy: number) {
    return (cx + 4096) * 8192 + (cy + 4096);
  }
  /** Module indices whose boxes may intersect the rectangle. */
  query(x0: number, y0: number, x1: number, y1: number): number[] {
    const out: number[] = [];
    const s = this.size;
    for (let cx = Math.floor((x0 - MOD_W) / s); cx <= Math.floor(x1 / s); cx++)
      for (let cy = Math.floor((y0 - MOD_H) / s); cy <= Math.floor(y1 / s); cy++) {
        const c = this.cells.get(this.key(cx, cy));
        if (c) for (const i of c) out.push(i);
      }
    return out;
  }
  hit(px: number, py: number): number {
    for (const i of this.query(px, py, px, py)) {
      if (px >= this.L.x[i] && px <= this.L.x[i] + MOD_W && py >= this.L.y[i] && py <= this.L.y[i] + MOD_H) return i;
    }
    return -1;
  }
}

export type MapDetail = "aggregate" | "boxes" | "labels";

/**
 * Level of detail for a zoom factor. Far out, a box would be a few pixels tall, so the map
 * draws one block per depth column with counts and bundled edges. In the middle it draws boxes;
 * close in, boxes get a color dot, a name and, when there is room, a caption, always at a
 * readable screen size.
 */
export function mapDetail(k: number): MapDetail {
  if (MOD_H * k < 10) return "aggregate";
  if (MOD_W * k < 84) return "boxes";
  return "labels";
}
