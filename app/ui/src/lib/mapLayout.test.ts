import { describe, expect, it } from "vitest";
import { edgePath, layoutMap } from "./mapLayout";
import { lifecycle } from "./testdata";

const run = (showExt = true, showTypes = false) => layoutMap(lifecycle.functions, lifecycle.modules, { showExt, showTypes });

describe("dependency map layout", () => {
  it("is deterministic", () => {
    const a = run();
    const b = run();
    const pos = (L: ReturnType<typeof run>) => Object.values(L.nodes).map((n) => [n.key, n.x, n.y, n.w, n.h]);
    expect(pos(a)).toEqual(pos(b));
    expect(a.edges.map((e) => [e.from, e.to, e.s, e.t])).toEqual(b.edges.map((e) => [e.from, e.to, e.s, e.t]));
    expect(edgePath(a.edges[0])).toBe(edgePath(b.edges[0]));
  });

  it("puts entry points left, leaves right, external imports last", () => {
    const L = run();
    expect(L.colMeta.map((c) => c.label)).toEqual(["Layer 3 · entry points", "Layer 2", "Layer 1", "Layer 0 · leaves", "External imports"]);
    for (let i = 1; i < L.colMeta.length; i++) expect(L.colMeta[i].x).toBeGreaterThan(L.colMeta[i - 1].x + L.colMeta[i - 1].w);
    expect(L.cols[0].map((n) => n.id).sort()).toEqual(["runAgentHarnessLifecycleAttempt", "runAgentHarnessLifecycleFinalization"]);
  });

  it("keeps nodes in a column from overlapping and edges finite", () => {
    const L = run(true, true);
    for (const col of L.cols) {
      const ys = col.map((n) => [n.y, n.y + n.h]).sort((a, b) => a[0] - b[0]);
      for (let i = 1; i < ys.length; i++) expect(ys[i][0]).toBeGreaterThanOrEqual(ys[i - 1][1]);
    }
    for (const e of L.edges) for (const v of [e.s.x, e.s.y, e.t.x, e.t.y]) expect(Number.isFinite(v)).toBe(true);
  });

  it("toggles external and type-only imports", () => {
    const noExt = run(false);
    expect(noExt.colMeta.some((c) => c.kind === "mod")).toBe(false);
    expect(noExt.edges.every((e) => e.kind === "call")).toBe(true);
    const plain = run(true, false);
    const typed = run(true, true);
    expect(plain.edges.some((e) => e.kind === "type")).toBe(false);
    expect(typed.edges.some((e) => e.kind === "type")).toBe(true);
    const calls = lifecycle.functions.reduce((n, f) => n + f.calls.length, 0);
    expect(plain.edges.filter((e) => e.kind === "call").length).toBe(calls);
  });
});
