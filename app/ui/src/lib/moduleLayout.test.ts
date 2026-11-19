import { describe, expect, it } from "vitest";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import { generateModuleMap } from "../ipc/mock/generate";
import { layoutModules, mapDetail, moduleLevels, MOD_H, MOD_W, SpatialGrid } from "./moduleLayout";
import { mockModuleMap } from "./testdata";

const big = (n: number): ModuleMap => {
  const raw = generateModuleMap(n);
  return { ...raw, modules: raw.modules.map((m) => ({ ...m, lines: Number(m.lines) })) };
};

describe("module map layout", () => {
  it("computes depth by longest path and puts a cycle in one column", () => {
    // 3 -> 0 -> 1 -> 2 -> 0 (cycle of 0, 1, 2); 4 is a leaf that 2 imports.
    const lv = moduleLevels(5, [[1], [2], [0, 4], [0], []]);
    expect([...lv]).toEqual([1, 1, 1, 2, 0]);
  });

  it("keeps a dense graph with many cycles to a few dozen columns", () => {
    const raw = generateModuleMap(1500, 15);
    const map = { ...raw, modules: raw.modules.map((m) => ({ ...m, lines: Number(m.lines) })) };
    const L = layoutModules(map);
    expect(L.columns.length).toBeLessThan(40);
    expect(L.W / L.H).toBeLessThan(8);
  });

  it("lays out 5,000 modules quickly and deterministically, without overlaps", () => {
    const map = big(5000);
    const t0 = performance.now();
    const a = layoutModules(map);
    const ms = performance.now() - t0;
    expect(ms).toBeLessThan(1500);
    const b = layoutModules(map);
    expect([...a.x]).toEqual([...b.x]);
    expect([...a.y]).toEqual([...b.y]);
    const seen = new Set<string>();
    for (let i = 0; i < a.x.length; i++) {
      const k = `${a.x[i]},${a.y[i]}`;
      expect(seen.has(k)).toBe(false);
      seen.add(k);
    }
    // Wrapping keeps the map from becoming a thin strip.
    expect(a.W / a.H).toBeLessThan(20);
  });

  it("finds modules by point and viewport", () => {
    const L = layoutModules(big(800));
    const g = new SpatialGrid(L);
    for (const i of [0, 17, 799]) {
      expect(g.hit(L.x[i] + MOD_W / 2, L.y[i] + MOD_H / 2)).toBe(i);
      expect(g.query(L.x[i], L.y[i], L.x[i] + 1, L.y[i] + 1)).toContain(i);
    }
    expect(g.hit(-1e6, -1e6)).toBe(-1);
  });

  it("puts entry points left and deeper layers to the right, like the file map", () => {
    const map = { ...mockModuleMap, modules: mockModuleMap.modules.map((m) => ({ ...m, lines: Number(m.lines) })) };
    const L = layoutModules(map);
    const cli = L.index.get("cli")!;
    const gateway = L.index.get("gateway")!;
    const config = L.index.get("config")!;
    expect(L.x[cli]).toBeLessThan(L.x[gateway]);
    expect(L.x[gateway]).toBeLessThan(L.x[config]);
    expect(L.columns[0].label).toMatch(/entry points/);
    for (let i = 1; i < L.columns.length; i++) expect(L.columns[i].x).toBeGreaterThan(L.columns[i - 1].x + L.columns[i - 1].w);
  });

  it("bundles edges per column pair for the zoomed-out view", () => {
    const L = layoutModules(big(5000));
    let between = 0;
    for (let i = 0; i < L.from.length; i++) if (L.col[L.from[i]] !== L.col[L.to[i]]) between++;
    expect(L.bundles.reduce((s, b) => s + b.count, 0)).toBe(between);
    expect(L.columns.reduce((s, c) => s + c.count, 0)).toBe(5000);
    // A realistic depth: a dozen or so columns, not hundreds.
    expect(L.columns.length).toBeLessThan(30);
  });

  it("picks a level of detail that keeps text readable", () => {
    expect(mapDetail(0.05)).toBe("aggregate");
    expect(mapDetail(0.4)).toBe("boxes");
    expect(mapDetail(0.7)).toBe("labels");
    expect(mapDetail(1)).toBe("labels");
  });
});
