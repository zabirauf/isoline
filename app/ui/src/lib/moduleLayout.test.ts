import { describe, expect, it } from "vitest";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import { generateModuleMap } from "../ipc/mock/generate";
import { layoutModules, moduleLevels, MOD_H, MOD_W, SpatialGrid } from "./moduleLayout";
import { mockModuleMap } from "./testdata";

const big = (n: number): ModuleMap => {
  const raw = generateModuleMap(n);
  return { ...raw, modules: raw.modules.map((m) => ({ ...m, lines: BigInt(m.lines) })) };
};

describe("module map layout", () => {
  it("computes depth by longest path and ignores cycle back edges", () => {
    const lv = moduleLevels(4, [[1], [2], [0], [0]]);
    expect([...lv].every((v) => v >= 0)).toBe(true);
    expect(lv[3]).toBeGreaterThan(lv[0]);
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

  it("stacks a small, deep map top to bottom", () => {
    const map = { ...mockModuleMap, modules: mockModuleMap.modules.map((m) => ({ ...m, lines: BigInt(m.lines) })) };
    const L = layoutModules(map);
    expect(L.vertical).toBe(true);
    const cli = L.index.get("cli")!;
    const config = L.index.get("config")!;
    expect(L.y[cli]).toBeLessThan(L.y[config]);
  });
});
