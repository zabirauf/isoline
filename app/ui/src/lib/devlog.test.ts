import { describe, expect, it } from "vitest";
import { callStats, devCalls, recordEvent, devEvents, shapeOf, traceCall } from "./devlog";

describe("devlog", () => {
  it("describes shapes without walking into values", () => {
    expect(shapeOf([1, 2, 3])).toBe("array(3)");
    expect(shapeOf({ path: "a.ts", tier: "full", fns: [], x: 1, y: 2 })).toBe('{path: "a.ts", tier: "full", fns, x: 1, …}');
    expect(shapeOf("x".repeat(100)).length).toBeLessThan(70);
    expect(shapeOf(null)).toBe("null");
  });

  it("times calls, keeps errors and sums per command", async () => {
    await traceCall("getTree", [], async () => ({ name: "root" }));
    await traceCall("getTree", [], async () => ({ name: "root" }));
    await traceCall("search", ["q", 20], async () => {
      throw new Error("no index");
    }).catch(() => {});
    const calls = devCalls();
    const last = calls[calls.length - 1];
    expect(last.cmd).toBe("search");
    expect(last.args).toBe('"q", 20');
    expect(last.ok).toBe(false);
    expect(last.error).toBe("no index");
    expect(calls[calls.length - 2].result).toEqual({ name: "root" });
    const s = callStats(calls);
    expect(s.find((x) => x.cmd === "getTree")?.n).toBe(2);
    expect(s.find((x) => x.cmd === "search")?.failed).toBe(1);
  });

  it("records events", () => {
    recordEvent("tree_changed", null);
    expect(devEvents().at(-1)?.event).toBe("tree_changed");
  });
});
