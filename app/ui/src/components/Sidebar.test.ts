import { describe, expect, it } from "vitest";
import type { TreeNode } from "../ipc/bindings/TreeNode";
import { activeIndex, defaultExpanded, type Row } from "./Sidebar";

const dir = (path: string, children: TreeNode[]): TreeNode => ({ name: path.split("/").pop() || "repo", path, isDir: true, lang: null, tier: null, module: null, color: null, status: "indexed", lines: 0, children });
const file = (path: string): TreeNode => ({ ...dir(path, []), isDir: false });

describe("sidebar default expansion", () => {
  it("starts with every folder closed", () => {
    expect([...defaultExpanded(dir("", [dir("a", [file("a/x.ts")]), dir("b", [file("b/y.ts")]), file("README.md")]))]).toEqual([]);
  });
  it("opens a chain of lone folders at the top", () => {
    const t = dir("", [dir("src", [dir("src/main", [dir("src/main/java", [file("src/main/java/A.java")]), file("src/main/x")])]), file("pom.xml")]);
    expect([...defaultExpanded(t)]).toEqual(["src", "src/main"]);
  });
  it("is empty for an empty tree", () => {
    expect([...defaultExpanded(dir("", []))]).toEqual([]);
  });
});

describe("sidebar focused row", () => {
  const rows = (paths: string[]): Row[] => paths.map((p) => ({ node: file(p), depth: p.split("/").length - 1, open: false }));
  it("follows its path when folders above it open", () => {
    expect(activeIndex(rows(["a", "b", "b/y.ts"]), "b/y.ts")).toBe(2);
    expect(activeIndex(rows(["a", "a/1", "a/2", "b", "b/y.ts"]), "b/y.ts")).toBe(4);
  });
  it("falls back to the nearest shown folder when its own is closed", () => {
    expect(activeIndex(rows(["a", "b", "c"]), "b/deep/y.ts")).toBe(1);
  });
  it("is the first row when nothing matches", () => {
    expect(activeIndex(rows(["a"]), "zzz/y.ts")).toBe(0);
    expect(activeIndex(rows(["a"]), null)).toBe(0);
  });
});
