// What the developer panel shows from the UI side: every backend call with its time, and
// every backend event. Ring buffers, always on and cheap: arguments and results are kept as
// short shapes; only the newest results are kept whole, so they can be inspected.

export interface CallEntry {
  id: number;
  cmd: string;
  /** Wall clock, ms. */
  at: number;
  /** Null while the call runs. */
  ms: number | null;
  args: string;
  ok: boolean | null;
  error: string | null;
  /** A short shape of the result ("array(120)", "{path, tier, …}"). */
  shape: string;
  /** The whole result, for the newest calls only. */
  result?: unknown;
}

export interface EventEntry {
  id: number;
  event: string;
  at: number;
  shape: string;
  payload?: unknown;
}

const MAX = 400;
/** Results and payloads kept whole for this many of the newest entries. */
const KEEP_WHOLE = 40;

const calls: CallEntry[] = [];
const events: EventEntry[] = [];
let nextId = 0;
let version = 0;
const subs = new Set<() => void>();
let notifyQueued = false;

function changed() {
  version++;
  if (notifyQueued || subs.size === 0) return;
  notifyQueued = true;
  // One notification per frame at most: indexing sends many events.
  setTimeout(() => {
    notifyQueued = false;
    subs.forEach((f) => f());
  }, 100);
}

function push<T extends { result?: unknown; payload?: unknown }>(list: T[], e: T) {
  list.push(e);
  if (list.length > MAX) list.splice(0, list.length - MAX);
  const old = list[list.length - 1 - KEEP_WHOLE];
  if (old) {
    delete old.result;
    delete old.payload;
  }
}

/** A short description of a value's shape, without walking into it. */
export function shapeOf(v: unknown, depth = 0): string {
  if (v === null || v === undefined) return String(v);
  if (typeof v === "string") return v.length > 60 ? JSON.stringify(v.slice(0, 57) + "…") : JSON.stringify(v);
  if (typeof v === "number" || typeof v === "boolean" || typeof v === "bigint") return String(v);
  if (Array.isArray(v)) return `array(${v.length})`;
  if (typeof v === "object") {
    const keys = Object.keys(v as object);
    if (depth > 0) return `{${keys.length} keys}`;
    const shown = keys.slice(0, 4).map((k) => {
      const x = (v as Record<string, unknown>)[k];
      return x !== null && typeof x === "object" ? k : `${k}: ${shapeOf(x, 1)}`;
    });
    return `{${shown.join(", ")}${keys.length > 4 ? ", …" : ""}}`;
  }
  return typeof v;
}

/** Times one backend call and records it. */
export function traceCall<T>(cmd: string, args: unknown[], f: () => Promise<T>): Promise<T> {
  const e: CallEntry = { id: ++nextId, cmd, at: Date.now(), ms: null, args: args.map((a) => shapeOf(a)).join(", "), ok: null, error: null, shape: "" };
  const t0 = performance.now();
  push(calls, e);
  changed();
  const p = f();
  p.then(
    (r) => {
      e.ms = performance.now() - t0;
      e.ok = true;
      e.shape = shapeOf(r);
      if (calls.indexOf(e) >= calls.length - KEEP_WHOLE) e.result = r;
      changed();
    },
    (err: unknown) => {
      e.ms = performance.now() - t0;
      e.ok = false;
      e.error = err instanceof Error ? err.message : String(err);
      changed();
    },
  );
  return p;
}

export function recordEvent(event: string, payload: unknown) {
  push(events, { id: ++nextId, event, at: Date.now(), shape: shapeOf(payload), payload });
  changed();
}

export function devCalls(): readonly CallEntry[] {
  return calls;
}

export function devEvents(): readonly EventEntry[] {
  return events;
}

export function clearDevLog(which: "calls" | "events") {
  (which === "calls" ? calls : events).length = 0;
  changed();
}

export function subscribeDevLog(f: () => void): () => void {
  subs.add(f);
  return () => subs.delete(f);
}

export function devLogVersion(): number {
  return version;
}

/** Count, median, 95th percentile and slowest time of each command, slowest first. */
export function callStats(list: readonly CallEntry[]): { cmd: string; n: number; p50: number; p95: number; max: number; failed: number }[] {
  const by = new Map<string, { ms: number[]; failed: number }>();
  for (const c of list) {
    const s = by.get(c.cmd) ?? { ms: [], failed: 0 };
    if (c.ms !== null) s.ms.push(c.ms);
    if (c.ok === false) s.failed++;
    by.set(c.cmd, s);
  }
  const pct = (a: number[], p: number) => (a.length ? a[Math.min(a.length - 1, Math.floor(a.length * p))] : 0);
  return [...by]
    .map(([cmd, s]) => {
      const a = [...s.ms].sort((x, y) => x - y);
      return { cmd, n: a.length, p50: pct(a, 0.5), p95: pct(a, 0.95), max: a.length ? a[a.length - 1] : 0, failed: s.failed };
    })
    .sort((a, b) => b.p95 - a.p95);
}
