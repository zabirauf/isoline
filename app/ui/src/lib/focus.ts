// Focus sets: what a selection depends on (accent) and what depends on it (user).
// Ported from the prototype's focusSets / applyFocus.
import type { FnCard } from "../ipc/bindings/FnCard";

export type Target = { kind: "fn"; id: string } | { kind: "sym"; id: string };

export interface FocusOptions {
  transitive: boolean;
  showTypes: boolean;
}

export interface FocusSets {
  deps: Set<string>;
  users: Set<string>;
  syms: Set<string>;
}

type CardLike = Pick<FnCard, "name" | "calls" | "calledBy" | "uses" | "typeUses">;

export function focusSets(cards: CardLike[], t: Target, opt: FocusOptions): FocusSets {
  const byId = new Map(cards.map((c) => [c.name, c]));
  const deps = new Set<string>();
  const users = new Set<string>();
  const syms = new Set<string>();
  const symsOf = (f: CardLike) => (opt.showTypes ? f.uses.concat(f.typeUses) : f.uses);
  const up = (n: string) =>
    byId.get(n)?.calledBy.forEach((c) => {
      if (!users.has(c)) {
        users.add(c);
        if (opt.transitive) up(c);
      }
    });
  if (t.kind === "fn") {
    const down = (n: string) => {
      const f = byId.get(n);
      if (!f) return;
      symsOf(f).forEach((s) => syms.add(s));
      f.calls.forEach((c) => {
        if (!deps.has(c)) {
          deps.add(c);
          if (opt.transitive) down(c);
        }
      });
    };
    down(t.id);
    up(t.id);
  } else {
    cards.forEach((f) => {
      if (f.uses.includes(t.id) || f.typeUses.includes(t.id)) users.add(f.name);
    });
    if (opt.transitive) [...users].forEach(up);
  }
  return { deps, users, syms };
}

/** Edge classification for a focused map: `to` is a card name, or "s:<symbol>" for an imported symbol. */
export function edgeFocus(from: string, to: string, t: Target, S: FocusSets, transitive: boolean): { dep: boolean; user: boolean } {
  const isSym = to.startsWith("s:");
  const tid = isSym ? to.slice(2) : to;
  if (t.kind === "fn") {
    return {
      dep: (from === t.id || (transitive && S.deps.has(from))) && (isSym ? S.syms.has(tid) : S.deps.has(tid)),
      user: S.users.has(from) && (tid === t.id || (transitive && S.users.has(tid))),
    };
  }
  return { dep: false, user: S.users.has(from) && ((isSym && tid === t.id) || (transitive && !isSym && S.users.has(tid))) };
}

export type CardState = "sel" | "dep" | "user" | "dim" | null;

/** How one card shows in the reading list for the committed selection. */
export function cardState(name: string, sel: Target | null, S: FocusSets | null): CardState {
  if (!sel || !S) return null;
  if (sel.kind === "fn" && sel.id === name) return "sel";
  if (S.deps.has(name)) return "dep";
  if (S.users.has(name)) return "user";
  return "dim";
}

export const sameTarget = (a: Target | null, b: Target | null) => !!a && !!b && a.kind === b.kind && a.id === b.id;
