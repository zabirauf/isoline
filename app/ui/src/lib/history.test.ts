import { describe, expect, it } from "vitest";
import { describe as label, EMPTY_HISTORY, goBack, goForward, push, type Location } from "./history";
import { DEFAULT_PREFS, detailForKey, loadPrefs, PREFS_KEY, savePrefs, type KeyValueStore } from "./prefs";

const loc = (path: string, selection: string | null = null, scroll = 0): Location => ({ path, symbol: null, line: null, selection, scroll });

describe("location history", () => {
  it("goes back to where the reader was, then forward again", () => {
    let h = push(EMPTY_HISTORY, loc("src/a.ts", "run", 340));
    const here = loc("src/types.ts", "AgentHarness");
    const b = goBack(h, here)!;
    expect(b.to).toEqual(loc("src/a.ts", "run", 340));
    h = b.history;
    expect(h.back).toHaveLength(0);
    expect(h.forward).toEqual([here]);
    const f = goForward(h, b.to)!;
    expect(f.to).toEqual(here);
    expect(f.history.back).toEqual([b.to]);
    expect(goBack(EMPTY_HISTORY, here)).toBe(null);
  });

  it("clears forward on a new navigation and does not record the same spot twice", () => {
    let h = push(EMPTY_HISTORY, loc("a"));
    h = goBack(h, loc("b"))!.history;
    expect(h.forward).toHaveLength(1);
    h = push(h, loc("a"));
    expect(h.forward).toHaveLength(0);
    h = push(h, loc("a"));
    expect(h.back).toHaveLength(1);
  });

  it("titles a destination with the file and the selection", () => {
    expect(label(loc("src/agents/harness/lifecycle.ts", "startAgentRun"))).toBe("lifecycle.ts · startAgentRun");
    expect(label(loc("README.md"))).toBe("README.md");
  });
});

describe("detail-level persistence", () => {
  const mem = (): KeyValueStore & { data: Record<string, string> } => {
    const data: Record<string, string> = {};
    return { data, getItem: (k) => data[k] ?? null, setItem: (k, v) => void (data[k] = v) };
  };

  it("defaults to Read and survives a save and load", () => {
    const s = mem();
    expect(loadPrefs(s)).toEqual(DEFAULT_PREFS);
    savePrefs({ ...DEFAULT_PREFS, detail: "glance", mapExternal: true }, s);
    expect(Object.keys(s.data)).toEqual([PREFS_KEY]);
    expect(loadPrefs(s).detail).toBe("glance");
    expect(loadPrefs(s).mapExternal).toBe(true);
  });

  it("ignores bad values and unknown keys", () => {
    const s = mem();
    s.setItem(PREFS_KEY, JSON.stringify({ detail: "everything", hintSeen: "yes", extra: 1 }));
    expect(loadPrefs(s)).toEqual(DEFAULT_PREFS);
    s.setItem(PREFS_KEY, "{not json");
    expect(loadPrefs(s)).toEqual(DEFAULT_PREFS);
    expect(loadPrefs(null)).toEqual(DEFAULT_PREFS);
  });

  it("maps keys 1 to 3 to the levels", () => {
    expect([detailForKey("1"), detailForKey("2"), detailForKey("3"), detailForKey("4")]).toEqual(["glance", "read", "dig", null]);
  });
});
