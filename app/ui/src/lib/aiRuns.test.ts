import { describe, expect, it } from "vitest";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import type { FileView } from "../ipc/bindings/FileView";
import { AUTO_MAX_USD, asksFirst, autoMaxUsd, fileAiState, type AiJob } from "./aiRuns";
import { DEFAULT_PREFS, PREFS_KEY, loadPrefs, savePrefs } from "./prefs";

const view = (over: Partial<FileView> = {}) => ({ path: "src/a.ts", tier: "full", overview: null, ...over }) as unknown as FileView;
const status = (over: Partial<AiStatus> = {}) => ({ signedIn: true, enabledForRepo: true, ...over }) as AiStatus;
const job = (over: Partial<AiJob> = {}): AiJob => ({ jobId: 1, scope: "file", path: "src/a.ts", done: 2, total: 5, costUsd: 0.001, finished: false, error: null, stored: 0, ...over });
const est = { scope: "file", symbols: 9, cached: 0, requests: 2, tokensIn: 1, tokensOut: 1, costUsd: 0.3, model: "m" } as const;

describe("fileAiState", () => {
  it("says summaries are off until AI is on for the repo, unless the file already has them", () => {
    expect(fileAiState(view(), null, null, null).kind).toBe("off");
    expect(fileAiState(view(), status({ enabledForRepo: false }), null, null).kind).toBe("off");
    expect(fileAiState(view({ overview: { text: "x", model: "m" } }), status({ signedIn: false }), null, null).kind).toBe("none");
  });

  it("never offers summaries for text-tier files", () => {
    expect(fileAiState(view({ tier: "text" }), null, null, null).kind).toBe("none");
  });

  it("shows a run for this file, a module or the repo as running, and another file's run as waiting", () => {
    expect(fileAiState(view(), status(), job(), null).kind).toBe("running");
    expect(fileAiState(view(), status(), job({ scope: "repo", path: null }), null).kind).toBe("running");
    expect(fileAiState(view(), status(), job({ path: "src/b.ts" }), null).kind).toBe("waiting");
  });

  it("reports this file's failed run, not another file's", () => {
    expect(fileAiState(view(), status(), job({ finished: true, error: "no answer could be used" }), null)).toEqual({ kind: "failed", error: "no answer could be used" });
    expect(fileAiState(view(), status(), job({ path: "src/b.ts", finished: true, error: "x" }), null).kind).toBe("none");
  });

  it("offers an expensive file with its estimate, for the open file only", () => {
    expect(fileAiState(view(), status(), null, { path: "src/a.ts", kind: "offer", est })).toEqual({ kind: "offer", est });
    expect(fileAiState(view(), status(), null, { path: "src/b.ts", kind: "offer", est }).kind).toBe("none");
  });
});

describe("cost confirmation", () => {
  const p = (confirmCost: boolean, askAboveUsd = 1) => ({ ...DEFAULT_PREFS, confirmCost, askAboveUsd });
  it("asks before every run when costs are confirmed", () => {
    expect(asksFirst(0.001, p(true))).toBe(true);
    expect(autoMaxUsd(p(true))).toBe(AUTO_MAX_USD);
  });
  it("asks only over the limit when they are not", () => {
    expect(asksFirst(0.9, p(false))).toBe(false);
    expect(asksFirst(1.2, p(false))).toBe(true);
    expect(asksFirst(0, p(false, 0))).toBe(false);
    expect(autoMaxUsd(p(false, 2))).toBe(2);
    expect(autoMaxUsd(p(false, 0))).toBe(AUTO_MAX_USD);
  });
  it("keeps the setting across loads", () => {
    const s = new Map<string, string>();
    const kv = { getItem: (k: string) => s.get(k) ?? null, setItem: (k: string, v: string) => void s.set(k, v) };
    savePrefs({ ...DEFAULT_PREFS, confirmCost: false, askAboveUsd: 2.5 }, kv);
    expect(loadPrefs(kv)).toMatchObject({ confirmCost: false, askAboveUsd: 2.5 });
    kv.setItem(PREFS_KEY, JSON.stringify({ confirmCost: "no", askAboveUsd: -1 }));
    expect(loadPrefs(kv)).toMatchObject({ confirmCost: true, askAboveUsd: 1 });
  });
});
