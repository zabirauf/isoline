// Keyboard shortcuts of the terminal panel. They follow Ghostty and Terminal on macOS and
// Windows Terminal elsewhere, so the shell keeps every Ctrl key it uses (Ctrl+C, Ctrl+W,
// Ctrl+T, Ctrl+K ...). Matched on `code`, so they work in any keyboard layout.

export type TermAction = { kind: "new" | "close" | "next" | "prev" | "clear" | "copy" | "paste" | "toggle" } | { kind: "goto"; index: number };

export interface KeyLike {
  code: string;
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

export function terminalAction(e: KeyLike, mac: boolean): TermAction | null {
  const digit = /^Digit([1-9])$/.exec(e.code);
  // Everywhere: Ctrl+` shows or hides the panel; Ctrl+Tab and Ctrl+Shift+Tab move between tabs.
  if (e.ctrlKey && !e.metaKey && !e.altKey && e.code === "Backquote") return { kind: "toggle" };
  if (e.ctrlKey && !e.metaKey && !e.altKey && e.code === "Tab") return { kind: e.shiftKey ? "prev" : "next" };
  if (mac) {
    if (!e.metaKey || e.ctrlKey || e.altKey) return null;
    if (e.shiftKey) {
      if (e.code === "BracketRight") return { kind: "next" };
      if (e.code === "BracketLeft") return { kind: "prev" };
      return null;
    }
    if (e.code === "KeyT") return { kind: "new" };
    if (e.code === "KeyW") return { kind: "close" };
    if (e.code === "KeyK") return { kind: "clear" };
    if (digit) return { kind: "goto", index: Number(digit[1]) - 1 };
    return null;
  }
  if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && digit) return { kind: "goto", index: Number(digit[1]) - 1 };
  if (!e.ctrlKey || e.metaKey || e.altKey) return null;
  if (e.code === "PageDown" && !e.shiftKey) return { kind: "next" };
  if (e.code === "PageUp" && !e.shiftKey) return { kind: "prev" };
  if (!e.shiftKey) return null;
  if (e.code === "KeyT") return { kind: "new" };
  if (e.code === "KeyW") return { kind: "close" };
  if (e.code === "KeyC") return { kind: "copy" };
  if (e.code === "KeyV") return { kind: "paste" };
  if (e.code === "KeyK") return { kind: "clear" };
  return null;
}

/** The shortcuts, in words, for the panel's help line and tooltips. */
export function shortcutText(mac: boolean) {
  return mac
    ? { new: "⌘T", close: "⌘W", next: "⌘⇧]", prev: "⌘⇧[", goto: "⌘1…9", clear: "⌘K", toggle: "⌃`" }
    : { new: "Ctrl+Shift+T", close: "Ctrl+Shift+W", next: "Ctrl+Tab", prev: "Ctrl+Shift+Tab", goto: "Alt+1…9", clear: "Ctrl+Shift+K", toggle: "Ctrl+`" };
}
