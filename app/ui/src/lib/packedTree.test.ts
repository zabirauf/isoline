import { describe, expect, it } from "vitest";
import { unpackTree } from "./packedTree";

describe("packed tree", () => {
  it("rebuilds paths, flags, languages and modules", () => {
    const t = unpackTree({
      parent: [0, 0, 1, 0],
      names: ["repo", "src", "a.ts", "README.md"],
      flags: [1, 1 | (1 << 1), (1 << 1) | (1 << 3), 3 << 3],
      lang: [0, 0, 1, 2],
      langs: ["typescript", "markdown"],
      module: [0, 1, 1, 0],
      modules: ["core"],
      colors: ["#123456"],
      lines: [12, 10, 10, 2],
    });
    expect(t.path).toBe("");
    expect(t.children.map((c) => c.path)).toEqual(["src", "README.md"]);
    const a = t.children[0].children[0];
    expect(a).toMatchObject({ path: "src/a.ts", isDir: false, status: "pending", tier: "full", lang: "typescript", module: "core", color: "#123456", lines: 10 });
    expect(t.children[0]).toMatchObject({ isDir: true, status: "pending", tier: null });
    expect(t.children[1]).toMatchObject({ tier: "text", module: null, color: null });
  });
});
