import { describe, expect, it } from "vitest";
import { terminalAction, type KeyLike } from "./terminalKeys";

const k = (code: string, mods: Partial<KeyLike> = {}): KeyLike => ({ code, key: "", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods });

describe("terminalAction", () => {
  it("uses Command shortcuts on macOS and leaves Ctrl keys to the shell", () => {
    expect(terminalAction(k("KeyT", { metaKey: true }), true)).toEqual({ kind: "new" });
    expect(terminalAction(k("KeyW", { metaKey: true }), true)).toEqual({ kind: "close" });
    expect(terminalAction(k("BracketRight", { metaKey: true, shiftKey: true }), true)).toEqual({ kind: "next" });
    expect(terminalAction(k("BracketLeft", { metaKey: true, shiftKey: true }), true)).toEqual({ kind: "prev" });
    expect(terminalAction(k("Digit3", { metaKey: true }), true)).toEqual({ kind: "goto", index: 2 });
    expect(terminalAction(k("KeyK", { metaKey: true }), true)).toEqual({ kind: "clear" });
    for (const c of ["KeyC", "KeyW", "KeyT", "KeyK", "KeyR", "KeyD"]) expect(terminalAction(k(c, { ctrlKey: true }), true)).toBeNull();
  });

  it("uses Ctrl+Shift shortcuts elsewhere, so Ctrl+C, Ctrl+W and Ctrl+T reach the shell", () => {
    expect(terminalAction(k("KeyT", { ctrlKey: true, shiftKey: true }), false)).toEqual({ kind: "new" });
    expect(terminalAction(k("KeyW", { ctrlKey: true, shiftKey: true }), false)).toEqual({ kind: "close" });
    expect(terminalAction(k("KeyC", { ctrlKey: true, shiftKey: true }), false)).toEqual({ kind: "copy" });
    expect(terminalAction(k("KeyV", { ctrlKey: true, shiftKey: true }), false)).toEqual({ kind: "paste" });
    expect(terminalAction(k("PageDown", { ctrlKey: true }), false)).toEqual({ kind: "next" });
    expect(terminalAction(k("Digit1", { altKey: true }), false)).toEqual({ kind: "goto", index: 0 });
    for (const c of ["KeyC", "KeyW", "KeyT", "KeyK", "KeyV"]) expect(terminalAction(k(c, { ctrlKey: true }), false)).toBeNull();
  });

  it("toggles and cycles the same way everywhere", () => {
    for (const mac of [true, false]) {
      expect(terminalAction(k("Backquote", { ctrlKey: true }), mac)).toEqual({ kind: "toggle" });
      expect(terminalAction(k("Tab", { ctrlKey: true }), mac)).toEqual({ kind: "next" });
      expect(terminalAction(k("Tab", { ctrlKey: true, shiftKey: true }), mac)).toEqual({ kind: "prev" });
      expect(terminalAction(k("KeyA"), mac)).toBeNull();
      expect(terminalAction(k("Tab"), mac)).toBeNull();
    }
  });
});
