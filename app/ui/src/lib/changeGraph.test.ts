import { describe, expect, it } from "vitest";
import type { ModuleChange } from "../ipc/bindings/ModuleChange";
import { buildChangeGraph, CG_W, edgeKind, edgeLabel } from "./changeGraph";
import { defaultBase, defaultHead, validBranchName, WORKTREE } from "./refs";
import { fmtAgo } from "./format";
import type { GitRefs } from "../ipc/bindings/GitRefs";
import type { GitStatus } from "../ipc/bindings/GitStatus";

const mod = (id: string, lines = 10): ModuleChange => ({ id, name: id.toUpperCase(), color: "#123456", dir: id, files: 1, additions: lines, deletions: 0, symbols: 1 });
const info = (id: string) => ({ name: `ctx ${id}`, color: "#999999" });

describe("change graph", () => {
  it("names edge kinds", () => {
    expect(edgeKind({ before: 0, after: 2 })).toBe("added");
    expect(edgeKind({ before: 3, after: 0 })).toBe("removed");
    expect(edgeKind({ before: 3, after: 5 })).toBe("more");
    expect(edgeKind({ before: 5, after: 3 })).toBe("fewer");
    expect(edgeLabel({ before: 0, after: 2 })).toBe("new");
    expect(edgeLabel({ before: 5, after: 3 })).toBe("−2");
    expect(edgeLabel({ before: 3, after: 5 })).toBe("+2");
  });

  it("puts importers left of what they import, and adds context modules", () => {
    const g = buildChangeGraph([mod("a"), mod("b")], [
      { from: "a", to: "b", before: 0, after: 1 },
      { from: "b", to: "c", before: 2, after: 0 },
    ], info);
    const at = (id: string) => g.nodes.find((n) => n.id === id)!;
    expect(at("a").layer).toBe(0);
    expect(at("b").layer).toBe(1);
    expect(at("c").layer).toBe(2);
    expect(at("c").changed).toBe(false);
    expect(at("c").name).toBe("ctx c");
    expect(at("a").x + CG_W).toBeLessThan(at("b").x);
    expect(g.edges.map((e) => e.kind)).toEqual(["added", "removed"]);
    expect(g.width).toBeGreaterThan(3 * CG_W);
  });

  it("survives cycles and drops unchanged edges", () => {
    const g = buildChangeGraph([mod("a"), mod("b")], [
      { from: "a", to: "b", before: 1, after: 2 },
      { from: "b", to: "a", before: 0, after: 1 },
      { from: "a", to: "x", before: 1, after: 1 },
    ], info);
    expect(g.nodes.map((n) => n.id).sort()).toEqual(["a", "b"]);
    expect(g.edges).toHaveLength(2);
    for (const e of g.edges) expect(e.d.startsWith("M")).toBe(true);
  });

  it("caps the number of changed modules, largest first", () => {
    const mods = Array.from({ length: 10 }, (_, i) => mod(`m${i}`, i));
    const g = buildChangeGraph(mods, [], info, 4);
    expect(g.hidden).toBe(6);
    expect(g.nodes.map((n) => n.id).sort()).toEqual(["m6", "m7", "m8", "m9"]);
  });
});

const status = (branch: string | null): GitStatus => ({ isRepo: true, branch, head: "abc1234", upstream: null, ahead: 0, behind: 0, changed: 0, untracked: 0, operation: null, github: false });
const refs = (names: [string, string][]): GitRefs => ({
  branches: names.map(([name, sha]) => ({ name, remote: name.startsWith("origin/"), current: false, sha, subject: "", author: "", date: 0, upstream: null })),
  tags: [],
  commits: [],
});

describe("default refs", () => {
  it("prefers the usual default branch as the base", () => {
    expect(defaultBase(refs([["feature", "1"], ["origin/main", "2"], ["main", "3"]]), status("feature"))).toBe("origin/main");
    expect(defaultBase(refs([["feature", "1"], ["master", "3"]]), status("feature"))).toBe("master");
    expect(defaultBase(refs([["feature", "1"], ["other", "3"]]), status("feature"))).toBe("other");
  });
  it("compares the current branch, or the working tree when HEAD is the base", () => {
    const r = refs([["main", "1"], ["origin/main", "1"], ["feature", "2"]]);
    expect(defaultHead(r, status("feature"), "origin/main")).toBe("feature");
    expect(defaultHead(r, status("main"), "main")).toBe(WORKTREE);
    expect(defaultHead(r, status("main"), "origin/main")).toBe(WORKTREE);
  });
  it("checks branch names lightly", () => {
    expect(validBranchName("fix/login")).toBe(true);
    expect(validBranchName("has space")).toBe(false);
    expect(validBranchName("a..b")).toBe(false);
    expect(validBranchName("-x")).toBe(false);
    expect(validBranchName("x.lock")).toBe(false);
  });
});

describe("relative time", () => {
  it("rounds down to the largest unit", () => {
    const now = 1_000_000_000_000;
    expect(fmtAgo(now / 1000 - 30, now)).toBe("just now");
    expect(fmtAgo(now / 1000 - 300, now)).toBe("5 min ago");
    expect(fmtAgo(now / 1000 - 3 * 3600, now)).toBe("3 h ago");
    expect(fmtAgo(now / 1000 - 3 * 86400, now)).toBe("3 d ago");
    expect(fmtAgo(now / 1000 - 90 * 86400, now)).toBe("3 mo ago");
  });
});
