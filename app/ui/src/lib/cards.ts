// Layers and ordering for the reading list and the dependency map.
// The backend sends `level`, `calls` and `calledBy` on every card; these helpers order and group
// them the way the prototype does, and recompute levels when the UI needs to check them.
import type { FnCard } from "../ipc/bindings/FnCard";
import type { FileView } from "../ipc/bindings/FileView";
import type { ImportModule } from "../ipc/bindings/ImportModule";
import type { ImportSym } from "../ipc/bindings/ImportSym";

export type Order = "entry" | "deps";

/**
 * Level of each card = longest call path to a leaf inside the file.
 * A call cycle is cut where it is first re-entered, so every card still gets a finite level.
 */
export function computeLevels(cards: Pick<FnCard, "name" | "calls">[]): Record<string, number> {
  const byName = new Map(cards.map((c) => [c.name, c]));
  const level: Record<string, number> = {};
  const visiting = new Set<string>();
  const L = (n: string): number => {
    if (n in level) return level[n];
    if (visiting.has(n)) return 0;
    const c = byName.get(n);
    if (!c) return 0;
    visiting.add(n);
    const callees = c.calls.filter((x) => byName.has(x) && x !== n);
    const v = callees.length ? 1 + Math.max(...callees.map(L)) : 0;
    visiting.delete(n);
    level[n] = v;
    return v;
  };
  cards.forEach((c) => L(c.name));
  return level;
}

/** Reverse of `calls`, restricted to cards in the file. */
export function computeCalledBy(cards: Pick<FnCard, "name" | "calls">[]): Record<string, string[]> {
  const out: Record<string, string[]> = Object.fromEntries(cards.map((c) => [c.name, [] as string[]]));
  cards.forEach((c) => c.calls.forEach((x) => out[x]?.push(c.name)));
  return out;
}

/** Topological order: every card after what it calls (level, then position in the file). */
export function topoOrder<T extends Pick<FnCard, "level" | "start">>(cards: T[]): T[] {
  return cards.slice().sort((a, b) => a.level - b.level || a.start - b.start);
}

export function maxLevel(cards: Pick<FnCard, "level">[]): number {
  return cards.length ? Math.max(...cards.map((c) => c.level)) : 0;
}

export interface LayerGroup<T> {
  level: number;
  title: string;
  subtitle: string;
  cards: T[];
}

export function groupInfo(level: number, max: number): [string, string] {
  if (level === 0) return ["Layer 0", "leaf helpers: call nothing else in this file"];
  if (level === max) return [`Layer ${level}`, "entry points and orchestration"];
  return [`Layer ${level}`, `build on layer ${level - 1} and below`];
}

/** Groups cards by layer. "entry" puts entry points first, "deps" leaves first. Empty layers are skipped. */
export function groupByLayer<T extends Pick<FnCard, "level" | "start">>(cards: T[], order: Order): LayerGroup<T>[] {
  const topo = topoOrder(cards);
  const max = maxLevel(cards);
  const levels = [...Array(max + 1).keys()];
  if (order === "entry") levels.reverse();
  return levels
    .map((l) => {
      const [title, subtitle] = groupInfo(l, max);
      return { level: l, title, subtitle, cards: topo.filter((c) => c.level === l) };
    })
    .filter((g) => g.cards.length > 0);
}

/** A module reached through a parameter (every symbol is a member), not an import. */
export function isRuntimeModule(m: ImportModule): boolean {
  return m.symbols.length > 0 && m.symbols.every((s) => s.member);
}

/** "harness" for "harness (AgentHarness param)": the receiver a member symbol is reached through. */
export function receiverOf(m: ImportModule): string {
  return m.id.split(/[\s(]/)[0] || m.id;
}

export interface SymInfo {
  module: ImportModule;
  sym: ImportSym;
  /** Label as written in code: `harness.classify` for members. */
  label: string;
}

/** Imported name → the module it comes from. */
export function symbolIndex(view: Pick<FileView, "modules">): Map<string, SymInfo> {
  const out = new Map<string, SymInfo>();
  for (const m of view.modules) {
    for (const s of m.symbols) {
      if (!out.has(s.name)) out.set(s.name, { module: m, sym: s, label: s.member ? `${receiverOf(m)}.${s.name}` : s.name });
    }
  }
  return out;
}

export interface FileStats {
  functions: number;
  exported: number;
  layers: number;
  types: number;
  modules: number;
  symbols: number;
}

export function fileStats(view: FileView): FileStats {
  return {
    functions: view.functions.length,
    exported: view.functions.filter((f) => f.exported).length,
    layers: view.layers,
    types: view.types.length,
    modules: view.modules.filter((m) => !isRuntimeModule(m)).length,
    symbols: view.modules.reduce((n, m) => n + m.symbols.length, 0),
  };
}

/** Lower-cased text a card matches against in the filter. */
export function searchText(c: FnCard): string {
  const s = c.summary;
  return [c.name, s?.desc ?? c.doc ?? "", ...(s?.algo ?? []), ...(s?.notes.map((n) => n.text) ?? []), ...c.uses, ...c.typeUses, ...c.calls]
    .join(" ")
    .toLowerCase();
}
