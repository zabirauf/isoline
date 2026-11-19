// Per-user preferences, one object under one localStorage key (never per file).
// Spec: docs/ux/progressive-disclosure.md section 5.
export type Detail = "glance" | "read" | "dig";

export interface Prefs {
  detail: Detail;
  /** Show the external-imports column in the dependency map. */
  mapExternal: boolean;
  /** The call-flow "Key" has been shown once. */
  flowKeySeen: boolean;
  /** The first-run hint was dismissed. */
  hintSeen: boolean;
  /** "Summaries are off" line hidden for all files. */
  aiNoticeHidden: boolean;
  /** Summarize a file when it opens, once AI is on for the repo (small runs only). */
  aiAuto: boolean;
  /** The developer panel is open. */
  devPanel: boolean;
}

export const PREFS_KEY = "isoline.prefs.v1";
export const DEFAULT_PREFS: Prefs = { detail: "read", mapExternal: false, flowKeySeen: false, hintSeen: false, aiNoticeHidden: false, aiAuto: true, devPanel: false };

/** The subset of Storage the prefs need, so tests can pass a plain object. */
export interface KeyValueStore {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

function store(): KeyValueStore | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** Reads the prefs, keeping only known keys with valid values. */
export function loadPrefs(s: KeyValueStore | null = store()): Prefs {
  let raw: unknown = null;
  try {
    raw = JSON.parse(s?.getItem(PREFS_KEY) ?? "null");
  } catch {
    raw = null;
  }
  const p = { ...DEFAULT_PREFS };
  if (raw && typeof raw === "object") {
    const r = raw as Record<string, unknown>;
    if (r.detail === "glance" || r.detail === "read" || r.detail === "dig") p.detail = r.detail;
    for (const k of ["mapExternal", "flowKeySeen", "hintSeen", "aiNoticeHidden", "aiAuto", "devPanel"] as const) if (typeof r[k] === "boolean") p[k] = r[k] as boolean;
  }
  return p;
}

export function savePrefs(p: Prefs, s: KeyValueStore | null = store()) {
  try {
    s?.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: prefs last for the session */
  }
}

type Listener = (p: Prefs) => void;
let current: Prefs | null = null;
const listeners = new Set<Listener>();

export function getPrefs(): Prefs {
  return (current ??= loadPrefs());
}

/** Updates and persists prefs, notifying every subscriber (all panes share one object). */
export function setPrefs(patch: Partial<Prefs>) {
  current = { ...getPrefs(), ...patch };
  savePrefs(current);
  listeners.forEach((f) => f(current!));
}

export function subscribePrefs(f: Listener): () => void {
  listeners.add(f);
  return () => listeners.delete(f);
}

/** Keys 1, 2 and 3 pick a level when the reader is not typing. */
export function detailForKey(key: string): Detail | null {
  return key === "1" ? "glance" : key === "2" ? "read" : key === "3" ? "dig" : null;
}
