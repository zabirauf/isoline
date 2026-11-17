import { describe, expect, it } from "vitest";
import type { TreeNode } from "../ipc/bindings/TreeNode";
import { defaultExpanded } from "./Sidebar";

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
