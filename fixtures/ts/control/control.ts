import { load, save, type Store } from "./store.js";
import { log as writeLog } from "../log.js";
import defaultThing from "default-thing";
import * as path from "node:path";

export function classify(kind: string, store: Store): number {
  switch (kind) {
    case "a":
      load(kind);
      break;
    case "b":
    case "c": {
      save(store, kind);
      return 2;
    }
    case "noop":
      break;
    default:
      writeLog(`unknown ${kind}`);
  }
  return kind.length > 3 ? score(kind) : 0;
}

function score(s: string): number {
  let total = 0;
  for (let i = 0; i < s.length; i++) {
    total += s.charCodeAt(i);
  }
  for (const ch of s.split("")) {
    if (ch === "x") continue;
    total += Math.max(1, ch.length);
  }
  for (const key in { a: 1 }) writeLog(key);
  while (total > 1000) {
    total = Math.floor(total / 2);
  }
  do {
    total -= step(total);
  } while (total > 10 && check(total));
  for (;;) {
    break;
  }
  return total;
}

function step(n: number): number {
  return n > 100 ? half(n) : n < 0 ? negate(n) : 1;
}

function half(n: number) {
  return n / 2;
}

function negate(n: number) {
  return -n;
}

function check(n: number): boolean {
  const ok = n > 0 && isEven(n);
  const fallback = lookup(n) ?? defaultValue(n);
  const either = lookup(n) || defaultValue(n);
  return ok && fallback !== either;
}

function isEven(n: number) {
  return n % 2 === 0;
}

function lookup(n: number): number | undefined {
  return n > 5 ? n : undefined;
}

function defaultValue(n: number) {
  return n * 0;
}

export async function guarded(store?: Store) {
  try {
    const data = await load("x");
    store?.flush?.();
    data?.items.forEach((item) => writeLog(item));
    if (!data) throw new Error("no data");
    if (data.ok) {
      save(store!, data);
    } else if (data.retry) {
      await guarded(store);
    } else {
      writeLog("skip");
    }
  } catch (err: unknown) {
    writeLog(String(err));
    throw err;
  } finally {
    writeLog("done");
  }
  const p = new Map<string, number>();
  const q = new path.Parser(defaultThing());
  return (p.size as number) + q.count();
}

export function noBlocks(a: number) {
  if (a) {
    a++;
  } else {
    a--;
  }
  try {
    a = a * 2;
  } catch {
    a = 0;
  }
  return a;
}
