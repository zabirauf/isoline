// Location history: every go-to-definition and file open pushes where the reader was; plain
// focus clicks do not. Back and Forward walk the stack (spec section 2, "History").
export interface Location {
  path: string;
  /** Symbol to focus (a card name). */
  symbol: string | null;
  line: number | null;
  /** The card that was selected, restored on Back. */
  selection: string | null;
  /** Scroll offset of the reading list, restored on Back. */
  scroll: number;
}

export interface History {
  back: Location[];
  forward: Location[];
}

export const EMPTY_HISTORY: History = { back: [], forward: [] };
const LIMIT = 100;

/** Records `from` before navigating somewhere new; clears the forward stack. */
export function push(h: History, from: Location): History {
  const last = h.back[h.back.length - 1];
  if (last && sameSpot(last, from)) return { back: h.back, forward: [] };
  return { back: [...h.back, from].slice(-LIMIT), forward: [] };
}

/** Goes back: returns the destination and the new stacks, with `current` saved for Forward. */
export function goBack(h: History, current: Location): { to: Location; history: History } | null {
  const to = h.back[h.back.length - 1];
  if (!to) return null;
  return { to, history: { back: h.back.slice(0, -1), forward: [...h.forward, current] } };
}

export function goForward(h: History, current: Location): { to: Location; history: History } | null {
  const to = h.forward[h.forward.length - 1];
  if (!to) return null;
  return { to, history: { back: [...h.back, current], forward: h.forward.slice(0, -1) } };
}

function sameSpot(a: Location, b: Location) {
  return a.path === b.path && a.selection === b.selection && a.symbol === b.symbol && Math.abs(a.scroll - b.scroll) < 2;
}

/** "lifecycle.ts · startAgentRun": the title of a Back or Forward button. */
export function describe(l: Location): string {
  const file = l.path.slice(l.path.lastIndexOf("/") + 1);
  const what = l.selection ?? l.symbol;
  return what ? `${file} · ${what}` : file;
}
