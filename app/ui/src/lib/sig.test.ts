import { describe, expect, it } from "vitest";
import type { SigInfo } from "../ipc/bindings/SigInfo";
import type { SymbolTarget } from "../ipc/bindings/SymbolTarget";
import { cutGenerics, cutTokens, displayType, functionType, genericsOf, indexTargets, inlineText, layoutSig, originOf, tokenizeType } from "./sig";

const t = (name: string, origin: SymbolTarget["origin"], path: string | null = null): SymbolTarget => ({ name, origin, kind: "type", path, start: 1, end: 2, module: null, doc: null, signature: null });
const idx = indexTargets([
  t("AgentHarness", "imported", "src/agents/harness/types.ts"),
  t("AgentHarnessAttemptParamsV2", "imported", "src/agents/harness/types.ts"),
  t("Promise", "builtin"),
  t("string", "builtin"),
  t("AgentRunCompletion", "local", "src/agents/harness/lifecycle.ts"),
  t("ZodType", "external"),
  // A local definition wins over an import of the same name.
  t("AgentRunCompletion", "imported", "src/other.ts"),
]);

const lifecycle: SigInfo = {
  params: [
    { name: "harness", typeText: "AgentHarness", default: null, rest: false, optional: false },
    { name: "params", typeText: "AgentHarnessAttemptParamsV2", default: null, rest: false, optional: false },
    { name: "execute", typeText: "(params: AgentHarnessAttemptParamsV2) => Promise<AgentHarnessAttemptResult>", default: "( attemptParams, ) => harness.runAttempt(attemptParams)", rest: false, optional: true },
  ],
  returns: "Promise<EmbeddedRunAttemptResult>",
};

describe("type tokens and origins", () => {
  it("looks each name up in targets; anything else is unknown", () => {
    const toks = tokenizeType("Promise<AgentHarness | Foo>[]", idx);
    const names = toks.filter((x) => x.kind === "name").map((x) => [x.text, x.origin]);
    expect(names).toEqual([
      ["Promise", "builtin"],
      ["AgentHarness", "imported"],
      ["Foo", "unknown"],
    ]);
    expect(toks.filter((x) => x.kind === "punct").map((x) => x.text)).toEqual(["<", "|", ">[]"]);
    expect(toks.map((x) => x.text).join("")).toBe("Promise<AgentHarness | Foo>[]");
  });

  it("prefers a local target and does not look up members after a dot", () => {
    expect(originOf("AgentRunCompletion", idx)).toBe("local");
    expect(originOf("ZodType", idx)).toBe("external");
    const toks = tokenizeType("z.ZodType", idx);
    expect(toks.find((x) => x.text === "ZodType")!.origin).toBe("unknown");
    expect(tokenizeType('Event["phase"]', idx).find((x) => x.text === '"phase"')!.kind).toBe("text");
  });
});

describe("type shortening", () => {
  it("shows function-typed params by name and cuts generics one level deep", () => {
    expect(functionType("(params: A, b?: B) => R")).toEqual({ params: ["params", "b"], ret: "R" });
    expect(functionType("Promise<A>")).toBe(null);
    expect(cutGenerics("Promise<Map<A, B>> | X<Y>")).toBe("Promise<…> | X<…>");
    expect(displayType(lifecycle.params[2].typeText!, "read")).toBe("(params) => Promise<…>");
    expect(displayType(lifecycle.params[2].typeText!, "dig")).toBe(lifecycle.params[2].typeText);
  });

  it("cuts long types at 40 characters on a token boundary", () => {
    const long = "Record<string, AgentHarnessSettledTurnFinalizationAttemptParams>";
    const d = displayType(long, "read");
    expect(d.length).toBeLessThanOrEqual(40);
    expect(d.endsWith("…")).toBe(true);
  });
});

describe("cutting type tokens", () => {
  it("keeps the origin of a name cut part-way", () => {
    const toks = cutTokens(tokenizeType("AgentHarnessAttemptParamsV2<AgentHarness>", idx), 20);
    expect(toks.map((t) => t.text).join("")).toBe("AgentHarnessAttempt…");
    expect(toks[0].origin).toBe("imported");
    expect(toks.map((t) => t.text).join("").length).toBeLessThanOrEqual(20);
    expect(cutTokens(tokenizeType("Promise<string>", idx), 40).map((t) => t.text).join("")).toBe("Promise<string>");
  });
});

describe("signature layout rules", () => {
  it("stacks the lifecycle signature with the default shown as = … at Read", () => {
    const v = layoutSig(lifecycle, "read");
    expect(v.mode).toBe("stacked");
    expect(v.params.map((p) => p.name)).toEqual(["harness", "params", "execute"]);
    expect(v.params[2].optional).toBe(true);
    expect(v.params[2].def).toBe("…");
    expect(v.params[2].type).toBe("(params) => Promise<…>");
    expect(v.returns).toBe("Promise<EmbeddedRunAttemptResult>");
    expect(layoutSig(lifecycle, "dig").params[2].def).toBe("( attemptParams, ) => harness.…");
  });

  it("is inline for 3 or fewer params within 56 characters", () => {
    const short: SigInfo = { params: [{ name: "a", typeText: "string", default: null, rest: false, optional: false }], returns: "number" };
    const v = layoutSig(short, "read");
    expect(v.mode).toBe("inline");
    expect(inlineText(v.params, v.returns)).toBe("(a: string) → number");
    const four: SigInfo = { params: ["a", "b", "c", "d"].map((n) => ({ name: n, typeText: null, default: null, rest: false, optional: false })), returns: null };
    expect(layoutSig(four, "read").mode).toBe("stacked");
  });

  it("shows 4 params at Read and all at Dig", () => {
    const seven: SigInfo = { params: Array.from({ length: 7 }, (_, i) => ({ name: `p${i}`, typeText: "string", default: null, rest: false, optional: false })), returns: null };
    expect(layoutSig(seven, "read").params).toHaveLength(4);
    expect(layoutSig(seven, "read").hidden).toBe(3);
    expect(layoutSig(seven, "dig").params).toHaveLength(7);
  });

  it("marks rest and destructured params and drops a void return except at Dig", () => {
    const s: SigInfo = {
      params: [
        { name: "{ a, b }", typeText: "Opts", default: null, rest: false, optional: false },
        { name: "...args", typeText: "string[]", default: null, rest: true, optional: false },
      ],
      returns: "void",
    };
    const v = layoutSig(s, "read");
    expect(v.params.map((p) => [p.name, p.rest, p.destructured])).toEqual([
      ["{…}", false, true],
      ["args", true, false],
    ]);
    expect(v.returns).toBe(null);
    expect(layoutSig(s, "dig").returns).toBe("void");
    expect(layoutSig({ params: [], returns: null }, "dig").returns).toBe(null);
  });

  it("finds generic parameters in the declaration", () => {
    expect(genericsOf("export function run<T extends X>(a: T): T", "run")).toBe("<T extends X>");
    expect(genericsOf("function run(a: T)", "run")).toBe(null);
  });
});
