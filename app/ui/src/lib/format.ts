// Small formatting helpers. Wording follows the design system: numbers with units, sentence case.

/** u64 fields arrive as numbers over Tauri IPC even though ts-rs types them as bigint. */
export function num(x: number | bigint | null | undefined): number {
  if (x === null || x === undefined) return 0;
  return typeof x === "bigint" ? Number(x) : x;
}

const IRREGULAR: Record<string, string> = { "try block": "try blocks", "local type": "local types" };

/** "1 loop", "2 loops". */
export function plural(n: number, noun: string, pluralForm?: string): string {
  return `${fmtInt(n)} ${n === 1 ? noun : (pluralForm ?? IRREGULAR[noun] ?? pluralNoun(noun))}`;
}

export function pluralNoun(noun: string): string {
  if (/(s|x|ch|sh)$/.test(noun)) return noun + "es";
  if (/[^aeiou]y$/.test(noun)) return noun.slice(0, -1) + "ies";
  return noun + "s";
}

export function fmtInt(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

/** Line range as the cards show it: "L357–423", or "L50" for one line. */
export function lineRange(start: number, end: number): string {
  return end > start ? `L${start}–${end}` : `L${start}`;
}

/** Drops leading "../" runs so module paths stay short in the map. */
export function shortPath(p: string): string {
  return p.replace(/^(\.\.\/)+/, "").replace(/^\.\//, "./");
}

export function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? p : p.slice(i + 1);
}

export function dirname(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? "" : p.slice(0, i);
}

export function fmtUsd(x: number): string {
  if (x === 0) return "$0";
  if (x < 0.01) return `$${x.toFixed(4)}`;
  return `$${x.toFixed(2)}`;
}

export function fmtMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

/**
 * Shortens a name in the middle so its distinguishing tail survives:
 * "native-hook-relay-server.ts" -> "native-ho…y-server.ts". The extension and the last part are
 * kept whole when they fit.
 */
export function middleEllipsis(s: string, max: number): string {
  if (s.length <= max) return s;
  if (max <= 1) return "…";
  const dot = s.lastIndexOf(".");
  const ext = dot > 0 && s.length - dot <= 6 ? s.slice(dot) : "";
  const tail = Math.max(ext.length, Math.ceil((max - 1) * 0.55));
  const head = max - 1 - tail;
  if (head < 1) return "…" + s.slice(s.length - (max - 1));
  return s.slice(0, head) + "…" + s.slice(s.length - tail);
}

/** Longest middle-ellipsized form of `s` whose measured width fits `px`. */
export function fitMiddle(s: string, px: number, measure: (t: string) => number): string {
  if (measure(s) <= px) return s;
  let lo = 1;
  let hi = s.length - 1;
  let best = "…";
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const t = middleEllipsis(s, mid);
    if (measure(t) <= px) {
      best = t;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return best;
}
