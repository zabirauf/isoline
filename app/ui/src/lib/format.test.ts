import { describe, expect, it } from "vitest";
import { fitMiddle, middleEllipsis, plural } from "./format";

describe("middle ellipsis", () => {
  it("keeps the distinguishing tail and the extension", () => {
    const names = ["native-hook-relay-server.ts", "native-hook-relay-client.ts", "native-hook-relay-types.ts"];
    const short = names.map((n) => middleEllipsis(n, 20));
    expect(new Set(short).size).toBe(3);
    for (const s of short) {
      expect(s.length).toBe(20);
      expect(s.endsWith(".ts")).toBe(true);
      expect(s.startsWith("native-")).toBe(true);
      expect(s).toContain("…");
    }
    expect(middleEllipsis("index.ts", 20)).toBe("index.ts");
  });

  it("fits a measured width", () => {
    const m = (t: string) => t.length * 7;
    const s = fitMiddle("native-hook-relay-server.ts", 140, m);
    expect(m(s)).toBeLessThanOrEqual(140);
    expect(s.endsWith("server.ts")).toBe(true);
  });

  it("pluralizes", () => {
    expect(plural(1, "loop")).toBe("1 loop");
    expect(plural(2, "try block")).toBe("2 try blocks");
    expect(plural(3, "dependency", "dependencies")).toBe("3 dependencies");
  });
});
