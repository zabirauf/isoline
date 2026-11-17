import { describe, expect, it } from "vitest";
import { computeCalledBy, computeLevels, fileStats, groupByLayer, isRuntimeModule, symbolIndex, topoOrder } from "./cards";
import { lifecycle } from "./testdata";

describe("layers", () => {
  it("matches the prototype's levels for lifecycle.ts (4 layers, entry points at 3)", () => {
    const lv = computeLevels(lifecycle.functions);
    expect(lv.runAgentHarnessLifecycleAttempt).toBe(3);
    expect(lv.runAgentHarnessLifecycleFinalization).toBe(3);
    expect(lv.createAgentHarnessLifecycleDiagnostics).toBe(2);
    expect(lv.emitAgentRunCompleted).toBe(1);
    expect(lv.emitAgentHarnessRunCompleted).toBe(1);
    expect(lv.agentRunMessageContent).toBe(0);
    // The backend's level field agrees with the recomputation.
    for (const c of lifecycle.functions) expect(lv[c.name]).toBe(c.level);
    expect(Math.max(...Object.values(lv)) + 1).toBe(lifecycle.layers);
  });

  it("cuts call cycles instead of recursing forever", () => {
    const lv = computeLevels([
      { name: "a", calls: ["b"] },
      { name: "b", calls: ["c"] },
      { name: "c", calls: ["a"] },
      { name: "d", calls: ["a", "d"] },
    ]);
    expect(Object.values(lv).every((v) => Number.isFinite(v))).toBe(true);
    expect(lv.d).toBeGreaterThan(lv.a);
  });

  it("derives calledBy as the reverse of calls", () => {
    const cb = computeCalledBy(lifecycle.functions);
    for (const c of lifecycle.functions) expect(cb[c.name].slice().sort()).toEqual(c.calledBy.slice().sort());
  });

  it("orders topologically: every card after what it calls", () => {
    const order = topoOrder(lifecycle.functions).map((c) => c.name);
    for (const c of lifecycle.functions) for (const callee of c.calls) expect(order.indexOf(callee)).toBeLessThan(order.indexOf(c.name));
  });

  it("groups entry points first or leaves first", () => {
    const entry = groupByLayer(lifecycle.functions, "entry");
    expect(entry.map((g) => g.level)).toEqual([3, 2, 1, 0]);
    expect(entry[0].subtitle).toBe("entry points and orchestration");
    expect(entry[3].subtitle).toBe("leaf helpers: call nothing else in this file");
    expect(entry[0].cards.map((c) => c.name)).toEqual(["runAgentHarnessLifecycleAttempt", "runAgentHarnessLifecycleFinalization"]);
    expect(groupByLayer(lifecycle.functions, "deps").map((g) => g.level)).toEqual([0, 1, 2, 3]);
  });
});

describe("file stats and imports", () => {
  it("counts like the prototype header", () => {
    expect(fileStats(lifecycle)).toEqual({ functions: 12, exported: 2, layers: 4, types: 4, modules: 16, symbols: 33 });
  });

  it("treats a parameter's members as a runtime module with receiver labels", () => {
    const harness = lifecycle.modules.find(isRuntimeModule)!;
    expect(harness.id).toBe("harness (AgentHarness param)");
    const idx = symbolIndex(lifecycle);
    expect(idx.get("classify")!.label).toBe("harness.classify");
    expect(idx.get("getRuntimeConfigSnapshot")!.label).toBe("getRuntimeConfigSnapshot");
  });
});
