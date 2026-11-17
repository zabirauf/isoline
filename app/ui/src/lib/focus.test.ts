import { describe, expect, it } from "vitest";
import { cardState, edgeFocus, focusSets } from "./focus";
import { lifecycle } from "./testdata";

const cards = lifecycle.functions;
const opt = { transitive: false, showTypes: false };

describe("focus sets", () => {
  it("direct: what a function calls and what calls it", () => {
    const S = focusSets(cards, { kind: "fn", id: "createAgentHarnessLifecycleDiagnostics" }, opt);
    const self = cards.find((c) => c.name === "createAgentHarnessLifecycleDiagnostics")!;
    expect([...S.deps].sort()).toEqual(self.calls.slice().sort());
    expect([...S.users].sort()).toEqual(["runAgentHarnessLifecycleAttempt", "runAgentHarnessLifecycleFinalization"]);
    expect([...S.syms].sort()).toEqual(self.uses.slice().sort());
  });

  it("transitive: follows calls and callers all the way", () => {
    const direct = focusSets(cards, { kind: "fn", id: "runAgentHarnessLifecycleAttempt" }, opt);
    const all = focusSets(cards, { kind: "fn", id: "runAgentHarnessLifecycleAttempt" }, { ...opt, transitive: true });
    expect(all.deps.size).toBeGreaterThan(direct.deps.size);
    for (const d of direct.deps) expect(all.deps.has(d)).toBe(true);
    // createAgentHarnessLifecycleDiagnostics calls emitAgentRunCompleted, which calls agentRunDiagnosticBase.
    expect(all.deps.has("agentRunDiagnosticBase")).toBe(true);
    const leaf = focusSets(cards, { kind: "fn", id: "agentRunDiagnosticBase" }, { ...opt, transitive: true });
    expect(leaf.users.has("runAgentHarnessLifecycleAttempt")).toBe(true);
    expect(leaf.deps.size).toBe(0);
  });

  it("type-only imports count only when shown", () => {
    const withTypes = focusSets(cards, { kind: "fn", id: "runAgentHarnessLifecycleAttempt" }, { ...opt, showTypes: true });
    const self = cards.find((c) => c.name === "runAgentHarnessLifecycleAttempt")!;
    for (const t of self.typeUses) expect(withTypes.syms.has(t)).toBe(true);
  });

  it("an imported symbol is used by the functions that reference it", () => {
    const S = focusSets(cards, { kind: "sym", id: "emitTrustedDiagnosticEvent" }, opt);
    const expected = cards.filter((c) => c.uses.includes("emitTrustedDiagnosticEvent")).map((c) => c.name);
    expect([...S.users].sort()).toEqual(expected.sort());
    expect(S.deps.size).toBe(0);
  });

  it("classifies edges and cards", () => {
    const t = { kind: "fn" as const, id: "createAgentHarnessLifecycleDiagnostics" };
    const S = focusSets(cards, t, opt);
    expect(edgeFocus("createAgentHarnessLifecycleDiagnostics", "emitAgentRunCompleted", t, S, false)).toEqual({ dep: true, user: false });
    expect(edgeFocus("runAgentHarnessLifecycleAttempt", "createAgentHarnessLifecycleDiagnostics", t, S, false)).toEqual({ dep: false, user: true });
    expect(edgeFocus("emitAgentRunCompleted", "agentRunDiagnosticBase", t, S, false)).toEqual({ dep: false, user: false });
    expect(cardState("createAgentHarnessLifecycleDiagnostics", t, S)).toBe("sel");
    expect(cardState("emitAgentRunCompleted", t, S)).toBe("dep");
    expect(cardState("runAgentHarnessLifecycleAttempt", t, S)).toBe("user");
    expect(cardState("normalizeAgentHarnessAttemptResult", t, S)).toBe("dim");
    expect(cardState("normalizeAgentHarnessAttemptResult", null, null)).toBe(null);
  });
});
