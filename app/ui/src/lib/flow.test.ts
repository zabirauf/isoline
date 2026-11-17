import { describe, expect, it } from "vitest";
import type { FlowItem } from "../ipc/bindings/FlowItem";
import { drillOf, exitKeyword, flowStats, hasLoops, keyOf, metaFor, numberFlow, staggerMs, unitTitle, type CallItem } from "./flow";
import { lifecycleFlows } from "./testdata";

const attempt = lifecycleFlows.runAgentHarnessLifecycleAttempt;
const startAgentRun = lifecycleFlows["createAgentHarnessLifecycleDiagnostics.startAgentRun"];
const units = new Set(Object.keys(lifecycleFlows));

describe("call flow numbering", () => {
  it("numbers runAgentHarnessLifecycleAttempt like the prototype: 18 steps, step 3 at L368", () => {
    const { steps } = numberFlow(attempt.tree, false);
    expect(steps).toHaveLength(18);
    expect(steps[0].label).toBe("createAgentHarnessLifecycleDiagnostics");
    expect(steps[2].label).toBe("diagnostics.startAgentRun");
    expect(steps[2].line).toBe(368);
    expect(steps[3].label).toBe("diagnostics.run");
    // The callback passed to diagnostics.run runs after it: its calls come next.
    expect(steps[4].label).toBe("diagnostics.setPhase");
    expect(steps[5].label).toBe("execute");
  });

  it("numbers depth-first so nested steps are consecutive", () => {
    const { nodes, steps } = numberFlow(attempt.tree, false);
    const seen: number[] = [];
    const walk = (ns: typeof nodes) =>
      ns.forEach((n) => {
        if (n.t === "step") {
          seen.push(n.n);
          walk(n.inner);
        } else if (n.t === "hidden") walk(n.inner);
        else if (n.t === "block") walk(n.children);
      });
    walk(nodes);
    expect(seen).toEqual(steps.map((_, i) => i + 1));
  });

  it("hides built-ins unless asked, keeping their inner calls", () => {
    expect(numberFlow(startAgentRun.tree, false).steps).toHaveLength(5);
    const all = numberFlow(startAgentRun.tree, true).steps;
    expect(all).toHaveLength(6);
    expect(all.some((s) => s.target.kind === "builtin")).toBe(true);
    const inner: FlowItem[] = [{ t: "call", label: "Promise.all", target: { kind: "builtin" }, line: 1, await: true, optional: false, argOf: null, inner: [{ t: "call", label: "load", target: { kind: "fn", id: "load" }, line: 2, await: false, optional: false, argOf: null, inner: [] }] }];
    const hidden = numberFlow(inner, false);
    expect(hidden.steps.map((s) => s.label)).toEqual(["load"]);
    expect(hidden.nodes[0].t).toBe("hidden");
  });
});

describe("call flow stats and wording", () => {
  it("counts like the prototype header", () => {
    expect(flowStats(attempt.tree, false)).toEqual({ calls: 18, branches: 1, tries: 1, loops: 0, callbacks: 1, awaits: 2 });
    expect(flowStats(startAgentRun.tree, false).calls).toBe(5);
    expect(flowStats(startAgentRun.tree, true).calls).toBe(6);
    expect(hasLoops(Object.values(lifecycleFlows))).toBe(false);
  });

  it("describes steps and drill targets", () => {
    const steps = numberFlow(attempt.tree, false).steps;
    expect(metaFor(steps[0])).toBe("this file · L365");
    expect(metaFor(steps[2])).toBe("controller method · L368");
    expect(metaFor(steps[5])).toBe("parameter · L371");
    expect(drillOf(steps[2], units)).toBe("createAgentHarnessLifecycleDiagnostics.startAgentRun");
    expect(drillOf(steps[5], units)).toBe(null);
    expect(keyOf(steps[2].target)).toBe("f:createAgentHarnessLifecycleDiagnostics");
    const ext: CallItem = { t: "call", label: "x", target: { kind: "ext", id: "x", module: "../../infra/a.js" }, line: 9, await: false, optional: true, argOf: "f", inner: [] };
    expect(metaFor(ext)).toBe("infra/a.js · argument to f() · optional call · L9");
    expect(exitKeyword({ t: "exit", kind: "return", label: "", line: 1, early: true })).toBe("early return");
    expect(exitKeyword({ t: "exit", kind: "throw", label: "", line: 1 })).toBe("throw");
    expect(unitTitle(startAgentRun)).toBe("startAgentRun()");
    expect(unitTitle(attempt)).toBe("runAgentHarnessLifecycleAttempt");
  });

  it("lands a whole flow in about one second", () => {
    expect(staggerMs(18) * 18).toBeLessThanOrEqual(1000);
    expect(staggerMs(1)).toBe(40);
    expect(staggerMs(500)).toBe(12);
  });
});
