// Dependency map layout for one file, ported from the prototype:
// columns by layer (entry points left, leaves right, external imports last), barycentre ordering
// over 8 sweeps, module boxes listing the symbols used, and spread edge ports.
// Pure and deterministic: the same input always gives the same coordinates.
import type { FnCard } from "../ipc/bindings/FnCard";
import type { ImportModule } from "../ipc/bindings/ImportModule";
import type { ImportSym } from "../ipc/bindings/ImportSym";
import { maxLevel, topoOrder } from "./cards";
import { shortPath } from "./format";

export interface MapOptions {
  showExt: boolean;
  showTypes: boolean;
  /** Module boxes show one "N symbols" row until focused or hovered; edges meet that row. */
  collapseModules?: boolean;
}

type CardLike = Pick<FnCard, "name" | "start" | "end" | "level" | "exported" | "calls" | "calledBy" | "uses" | "typeUses">;

export interface FnNode {
  kind: "fn";
  key: string;
  id: string;
  f: CardLike;
  cap: string;
  x: number;
  y: number;
  w: number;
  h: number;
  bary: number;
}
export interface ModNode {
  kind: "mod";
  key: string;
  id: string;
  m: ImportModule;
  vis: ImportSym[];
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
  bary: number;
}
export type MapNode = FnNode | ModNode;

export interface Pt {
  x: number;
  y: number;
}
export interface MapEdge {
  from: string;
  /** Card name, or "s:<symbol>" for an imported symbol. */
  to: string;
  kind: "call" | "ext" | "type";
  s: Pt;
  t: Pt;
}
export interface ColMeta {
  x: number;
  w: number;
  kind: "fn" | "mod";
  level: number;
  label: string;
}
export interface MapLayout {
  cols: MapNode[][];
  colMeta: ColMeta[];
  nodes: Record<string, MapNode>;
  edges: MapEdge[];
  symPos: Record<string, Pt>;
  H: number;
  W: number;
}

export const NODE_H = 50;
export const MOD_HEAD = 26;
export const SYM_ROW = 19;
const GAP = 92;

export const nodeCaption = (f: CardLike) =>
  `${f.exported ? "export · " : ""}${f.end - f.start + 1} lines · ${f.calls.length} calls · ${f.uses.length} imports`;

export const symLabel = (s: ImportSym) => `${s.member ? "." : ""}${s.name}${s.typeOnly ? " (type)" : ""}`;

export function layoutMap(cards: CardLike[], modules: ImportModule[], opt: MapOptions): MapLayout {
  const topo = topoOrder(cards);
  const maxL = maxLevel(cards);
  const symMod = new Map<string, ImportModule>();
  modules.forEach((m) => m.symbols.forEach((s) => symMod.has(s.name) || symMod.set(s.name, m)));

  const mods = opt.showExt
    ? modules.map((m) => ({ m, vis: m.symbols.filter((s) => opt.showTypes || !s.typeOnly) })).filter((x) => x.vis.length)
    : [];

  const cols: MapNode[][] = [];
  for (let l = maxL; l >= 0; l--) {
    const col: MapNode[] = topo
      .filter((f) => f.level === l)
      .map((f) => {
        const cap = nodeCaption(f);
        return { kind: "fn", key: "f:" + f.name, id: f.name, f, cap, x: 0, y: 0, w: Math.max(190, f.name.length * 7.3 + 30, cap.length * 5.9 + 28), h: NODE_H, bary: 0 };
      });
    if (col.length) cols.push(col);
  }
  if (mods.length) {
    cols.push(
      mods.map(({ m, vis }) => {
        const label = shortPath(m.id);
        return {
          kind: "mod",
          key: "m:" + m.id,
          id: m.id,
          m,
          vis,
          label,
          x: 0,
          y: 0,
          w: Math.max(160, label.length * 6.5 + 22, ...vis.map((s) => (s.name.length + (s.member ? 8 : 0)) * 6.8 + 34)),
          h: MOD_HEAD + (opt.collapseModules ? 1 : vis.length) * SYM_ROW + 6,
          bary: 0,
        };
      }),
    );
  }

  const nodes: Record<string, MapNode> = {};
  cols.flat().forEach((n) => (nodes[n.key] = n));
  const visSym = new Set(mods.flatMap((x) => x.vis.map((s) => s.name)));
  const symsOf = (f: CardLike) => (opt.showTypes ? f.uses.concat(f.typeUses) : f.uses).filter((s) => visSym.has(s));

  // Neighbours for barycentre ordering, computed once.
  const nbs = new Map<string, MapNode[]>();
  for (const n of cols.flat()) {
    let out: (MapNode | undefined)[];
    if (n.kind === "fn") {
      out = n.f.calls.concat(n.f.calledBy).map((c) => nodes["f:" + c]);
      symsOf(n.f).forEach((s) => out.push(nodes["m:" + symMod.get(s)!.id]));
    } else {
      out = cards.filter((f) => symsOf(f).some((s) => symMod.get(s)!.id === n.id)).map((f) => nodes["f:" + f.name]);
    }
    nbs.set(n.key, out.filter((x): x is MapNode => !!x));
  }

  let x = 0;
  const colMeta: ColMeta[] = cols.map((c) => {
    const w = Math.max(...c.map((n) => n.w));
    const first = c[0];
    const level = first.kind === "fn" ? first.f.level : -1;
    const label =
      first.kind === "mod" ? "External imports" : level === 0 ? (maxL === 0 ? "Layer 0" : "Layer 0 · leaves") : level === maxL ? `Layer ${level} · entry points` : `Layer ${level}`;
    const meta: ColMeta = { x, w, kind: first.kind, level, label };
    c.forEach((n) => (n.x = n.kind === "mod" ? x : x + (w - n.w) / 2));
    x += w + GAP;
    return meta;
  });
  const stack = (c: MapNode[]) => {
    const gap = c[0].kind === "fn" ? 22 : 12;
    let y = 0;
    c.forEach((n) => {
      n.y = y;
      y += n.h + gap;
    });
    return y - gap;
  };
  const recenter = () => {
    const hs = cols.map(stack);
    const H = hs.length ? Math.max(...hs) : 0;
    cols.forEach((c, i) => c.forEach((n) => (n.y += (H - hs[i]) / 2)));
    return H;
  };
  let H = recenter();
  for (let it = 0; it < 8; it++) {
    const seq = it % 2 ? cols.slice().reverse() : cols;
    seq.forEach((c) => {
      c.forEach((n) => {
        const ns = nbs.get(n.key)!;
        n.bary = ns.length ? ns.reduce((a, m) => a + m.y + m.h / 2, 0) / ns.length : n.y + n.h / 2;
      });
      // Array.prototype.sort is stable, so ties keep their previous order and runs are identical.
      c.sort((a, b) => a.bary - b.bary);
    });
    H = recenter();
  }

  const symPos: Record<string, Pt> = {};
  mods.forEach(({ m, vis }) => {
    const n = nodes["m:" + m.id];
    vis.forEach((s, i) => (symPos[s.name] = { x: n.x, y: n.y + MOD_HEAD + (opt.collapseModules ? 0 : i) * SYM_ROW + SYM_ROW / 2 }));
  });

  const edges: MapEdge[] = [];
  const origin = { x: 0, y: 0 };
  cards.forEach((f) => {
    if (!nodes["f:" + f.name]) return;
    f.calls.forEach((c) => nodes["f:" + c] && edges.push({ from: f.name, to: c, kind: "call", s: origin, t: origin }));
    if (opt.showExt) {
      f.uses.forEach((s) => visSym.has(s) && edges.push({ from: f.name, to: "s:" + s, kind: "ext", s: origin, t: origin }));
      if (opt.showTypes) f.typeUses.forEach((s) => visSym.has(s) && edges.push({ from: f.name, to: "s:" + s, kind: "type", s: origin, t: origin }));
    }
  });
  const tgt = (e: MapEdge): Pt => {
    if (e.to.startsWith("s:")) return symPos[e.to.slice(2)];
    const n = nodes["f:" + e.to];
    return { x: n.x, y: n.y + n.h / 2 };
  };
  const byFrom: Record<string, MapEdge[]> = {};
  const byTo: Record<string, MapEdge[]> = {};
  edges.forEach((e) => {
    e.t = tgt(e);
    (byFrom[e.from] ??= []).push(e);
    if (!e.to.startsWith("s:")) (byTo[e.to] ??= []).push(e);
  });
  Object.entries(byFrom).forEach(([k, es]) => {
    const n = nodes["f:" + k];
    es.sort((a, b) => a.t.y - b.t.y);
    const step = Math.min(4, 36 / Math.max(1, es.length));
    es.forEach((e, i) => (e.s = { x: n.x + n.w, y: n.y + n.h / 2 + (i - (es.length - 1) / 2) * step }));
  });
  Object.entries(byTo).forEach(([k, es]) => {
    const n = nodes["f:" + k];
    es.sort((a, b) => a.s.y - b.s.y);
    const step = Math.min(6, 34 / Math.max(1, es.length));
    es.forEach((e, i) => (e.t = { x: n.x, y: n.y + n.h / 2 + (i - (es.length - 1) / 2) * step }));
  });
  return { cols, colMeta, nodes, edges, symPos, H, W: Math.max(0, x - GAP) };
}

/** Cubic curve from the caller's right side to the callee's left side. */
export function edgePath(e: Pick<MapEdge, "s" | "t">): string {
  const dx = Math.max(40, (e.t.x - e.s.x) * 0.45);
  return `M${e.s.x},${e.s.y} C${e.s.x + dx},${e.s.y} ${e.t.x - dx},${e.t.y} ${e.t.x - 2},${e.t.y}`;
}
