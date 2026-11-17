// Module-level map layout for repo scale (thousands of modules). Same idea as the file map:
// columns by dependency depth (entry points left), barycentre ordering, but index-based and
// iterative so 5,000 modules lay out in a few tens of milliseconds. Tall columns wrap into
// sub-columns to keep the map roughly as wide as it is tall.
import type { ModuleMap } from "../ipc/bindings/ModuleMap";

export const MOD_W = 176;
export const MOD_H = 44;
const ROW_GAP = 14;
const SUB_GAP = 24;
const COL_GAP = 90;

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
  /** Depth bands: columns, or rows when `vertical`. */
  columns: { x: number; y: number; level: number; label: string }[];
  /**
   * Small, deep maps (a long chain of a few modules per depth) read better top to bottom:
   * depth runs down the rows and edges leave a box's bottom for the next box's top.
   */
  vertical: boolean;
  W: number;
  H: number;
}

/** Longest dependency path to a module with no dependencies; back edges of cycles are ignored. */
export function moduleLevels(n: number, out: number[][]): Int32Array {
  const level = new Int32Array(n).fill(-1);
  const state = new Uint8Array(n); // 0 new, 1 on stack, 2 done
  for (let root = 0; root < n; root++) {
    if (state[root]) continue;
    const stack: [number, number][] = [[root, 0]];
    state[root] = 1;
    while (stack.length) {
      const top = stack[stack.length - 1];
      const [v, i] = top;
      if (i < out[v].length) {
        top[1]++;
        const w = out[v][i];
        if (state[w] === 0) {
          state[w] = 1;
          stack.push([w, 0]);
        }
        continue;
      }
      let l = 0;
      for (const w of out[v]) if (state[w] === 2) l = Math.max(l, level[w] + 1);
      level[v] = l;
      state[v] = 2;
      stack.pop();
    }
  }
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

  const maxRows = Math.max(10, Math.ceil(Math.sqrt(n) * 1.6));
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
  const columns: ModuleLayout["columns"] = [];
  const label = (l: number) => (l === maxL && maxL > 0 ? `Depth ${l} · entry points` : l === 0 ? "Depth 0 · no dependencies" : `Depth ${l}`);
  const widest = Math.max(0, ...cols.map((c) => c.length));
  const bands = cols.filter((c) => c.length).length;
  if (n > 0 && n <= 400 && widest <= maxRows && bands * (MOD_W + COL_GAP) > 2.2 * widest * (MOD_H + ROW_GAP)) {
    const BAND_GAP = 46;
    const rowW = (c: number[]) => c.length * (MOD_W + SUB_GAP) - SUB_GAP;
    const W = Math.max(...cols.map(rowW));
    let cy = 0;
    cols.forEach((c, ci) => {
      if (!c.length) return;
      const off = (W - rowW(c)) / 2;
      columns.push({ x: 0, y: cy, level: maxL - ci, label: label(maxL - ci) });
      c.forEach((v, k) => {
        x[v] = off + k * (MOD_W + SUB_GAP);
        y[v] = cy;
      });
      cy += MOD_H + BAND_GAP;
    });
    return { x, y, level, from, to, weight, index, columns, vertical: true, W, H: Math.max(0, cy - BAND_GAP) };
  }
  let cx = 0;
  let H = 0;
  cols.forEach((c, ci) => {
    if (!c.length) return;
    const rows = Math.min(maxRows, c.length);
    const subs = Math.ceil(c.length / rows);
    const l = maxL - ci;
    columns.push({ x: cx, y: 0, level: l, label: label(l) });
    c.forEach((v, k) => {
      x[v] = cx + Math.floor(k / rows) * (MOD_W + SUB_GAP);
      y[v] = (k % rows) * (MOD_H + ROW_GAP);
    });
    H = Math.max(H, rows * (MOD_H + ROW_GAP) - ROW_GAP);
    cx += subs * (MOD_W + SUB_GAP) - SUB_GAP + COL_GAP;
  });
  // Center each column vertically.
  cols.forEach((c) => {
    if (!c.length) return;
    const rows = Math.min(maxRows, c.length);
    const h = rows * (MOD_H + ROW_GAP) - ROW_GAP;
    const off = (H - h) / 2;
    c.forEach((v) => (y[v] += off));
  });
  return { x, y, level, from, to, weight, index, columns, vertical: false, W: Math.max(0, cx - COL_GAP), H };
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
