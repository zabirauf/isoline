// A terminal whose output the backend parses (libghostty-vt, native): this view only draws
// the frames it gets and sends input back. Frames are drawn in an animation frame, as many
// as arrived since the last one merged into one paint, and each paint is acknowledged, so the
// backend never sends faster than this view draws. Output floods stay off the UI thread.
import { useEffect, useRef, useState } from "react";
import type { ITheme } from "ghostty-web";
import { api, type TermFrame, type TermInput } from "../ipc/api";
import { TermRenderer, measure } from "../lib/termRender";
import { terminalAction, type TermAction } from "../lib/terminalKeys";

const MAC = typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");
export const TERM_FONT = "'IBM Plex Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";
const SIZE = 13;
const PAD = { left: 10, top: 6 };

/** What the panel's shortcuts do to a terminal. */
export interface TermHandle {
  clear(): void;
  copy(): void;
  paste(): void;
  selectAll(): void;
}

export interface TermViewProps {
  cwd: string | null;
  exited: number | null;
  shown: boolean;
  focusSignal: number;
  theme: ITheme;
  onAction(a: TermAction, h: TermHandle): boolean;
  onStarted(session: number, shell: string, cwd: string): void;
  onTitle(t: string): void;
  onError(e: string): void;
  onCloseExited(): void;
}

/** Any CSS color as `#rrggbb` (a canvas normalizes what it is given). */
let probe: CanvasRenderingContext2D | null = null;
function hex6(c: string | undefined, d: string): string {
  probe ??= document.createElement("canvas").getContext("2d");
  if (!probe || !c) return d;
  probe.fillStyle = d;
  probe.fillStyle = c;
  const v = String(probe.fillStyle);
  return /^#[0-9a-f]{6}$/i.test(v) ? v : d;
}

const themeInput = (t: ITheme): Extract<TermInput, { t: "theme" }> => ({
  t: "theme",
  foreground: hex6(t.foreground, "#17202c"),
  background: hex6(t.background, "#ffffff"),
  cursor: hex6(t.cursor ?? t.foreground, "#17202c"),
  palette: [t.black, t.red, t.green, t.yellow, t.blue, t.magenta, t.cyan, t.white, t.brightBlack, t.brightRed, t.brightGreen, t.brightYellow, t.brightBlue, t.brightMagenta, t.brightCyan, t.brightWhite].map((c) => hex6(c, "")),
});

const modBits = (e: { shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean }) => (e.shiftKey ? 1 : 0) | (e.altKey ? 2 : 0) | (e.ctrlKey ? 4 : 0) | (e.metaKey ? 8 : 0);
const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "CapsLock", "NumLock", "Fn", "Hyper", "Super", "OS", "AltGraph"]);

export function NativeTermView({ cwd, exited, shown, focusSignal, theme, onAction, onStarted, onTitle, onError, onCloseExited }: TermViewProps) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const r = useRef<TermRenderer | null>(null);
  const session = useRef<number | null>(null);
  const [scroll, setScroll] = useState<[number, number, number]>([0, 0, 0]);
  const cb = useRef({ onAction, onStarted, onTitle, onError, onCloseExited, exited, shown, theme });
  cb.current = { onAction, onStarted, onTitle, onError, onCloseExited, exited, shown, theme };

  const send = (i: TermInput) => {
    if (session.current !== null) void api.termInput(session.current, i).catch(() => {});
  };

  useEffect(() => {
    const cv = canvas.current!;
    const ctx = cv.getContext("2d", { alpha: false })!;
    let alive = true;
    let queue: TermFrame[] = [];
    let raf = 0;
    let size = { cols: 0, rows: 0 };
    let blink = 0;

    const metrics = () => measure(ctx, TERM_FONT, SIZE);
    const dpr = () => window.devicePixelRatio || 1;
    const rend = new TermRenderer(ctx, metrics(), TERM_FONT, SIZE, dpr());
    r.current = rend;
    rend.selColor = getComputedStyle(document.documentElement).getPropertyValue("--accent-soft").trim() || rend.selColor;

    /** Sizes the canvas to the host and returns the grid that fits. */
    const fit = () => {
      const el = host.current!;
      const w = Math.max(0, el.clientWidth - PAD.left);
      const h = Math.max(0, el.clientHeight - PAD.top);
      if (!w || !h) return null;
      rend.dpr = dpr();
      rend.m = metrics();
      rend.resetContext();
      const cols = Math.max(10, Math.floor(w / rend.m.cw));
      const rows = Math.max(2, Math.floor(h / rend.m.ch));
      const pw = Math.round(w * rend.dpr);
      const ph = Math.round(h * rend.dpr);
      if (cv.width !== pw || cv.height !== ph) {
        cv.width = pw;
        cv.height = ph;
        cv.style.width = `${w}px`;
        cv.style.height = `${h}px`;
        rend.resetContext();
        rend.drawAll();
      }
      return { cols, rows };
    };

    const paint = () => {
      raf = 0;
      if (!alive || !queue.length) return;
      const frames = queue;
      queue = [];
      const dirty = new Set<number>();
      let full = false;
      for (const f of frames) {
        for (const y of rend.apply(f)) dirty.add(y);
        full ||= f.full;
        if (f.title) cb.current.onTitle(f.title);
        if (f.clipboard) void navigator.clipboard?.writeText(f.clipboard).catch(() => {});
      }
      if (cv.width) {
        if (full) rend.drawAll();
        else rend.draw(dirty);
      }
      setScroll((s) => (s[0] === rend.scroll[0] && s[1] === rend.scroll[1] && s[2] === rend.scroll[2] ? s : rend.scroll));
      placeInput();
      send({ t: "ack", seq: frames[frames.length - 1].seq });
    };

    /** The input method's window opens at the cursor. */
    const placeInput = () => {
      const c = rend.cursor;
      const ta = input.current;
      if (!c || !ta) return;
      ta.style.left = `${PAD.left + c.x * rend.m.cw}px`;
      ta.style.top = `${PAD.top + c.y * rend.m.ch}px`;
    };

    const onData = (d: TermFrame | Uint8Array) => {
      if (d instanceof Uint8Array) return;
      queue.push(d);
      raf ||= requestAnimationFrame(paint);
    };

    const ro = new ResizeObserver(() => {
      const g = fit();
      if (!g || (g.cols === size.cols && g.rows === size.rows)) return;
      size = g;
      send({ t: "resize", cols: g.cols, rows: g.rows, cellW: Math.round(rend.m.cw * rend.dpr), cellH: Math.round(rend.m.ch * rend.dpr) });
    });

    void (async () => {
      await document.fonts?.load(`${SIZE}px ${TERM_FONT}`).catch(() => []);
      if (!alive) return;
      const g = fit() ?? { cols: 80, rows: 24 };
      size = g;
      try {
        const info = await api.termOpen({ cwd, cols: g.cols, rows: g.rows, cellW: Math.round(rend.m.cw * rend.dpr), cellH: Math.round(rend.m.ch * rend.dpr), theme: themeInput(cb.current.theme) }, onData);
        if (!alive) {
          void api.termClose(info.id);
          return;
        }
        session.current = info.id;
        cb.current.onStarted(info.id, info.shell, info.cwd);
        send({ t: "refresh" });
        ro.observe(host.current!);
      } catch (e) {
        cb.current.onError(e instanceof Error ? e.message : String(e));
      }
    })();

    // The cursor blinks while the terminal has the keyboard.
    blink = window.setInterval(() => {
      if (!rend.focused || !rend.cursor?.blink) return;
      rend.blinkOn = !rend.blinkOn;
      rend.draw([rend.cursor.y]);
    }, 530);

    return () => {
      alive = false;
      cancelAnimationFrame(raf);
      clearInterval(blink);
      ro.disconnect();
      if (session.current !== null) void api.termClose(session.current);
      session.current = null;
      r.current = null;
    };
  }, []);

  useEffect(() => {
    send({ ...themeInput(theme) });
    const rend = r.current;
    if (rend) rend.selColor = getComputedStyle(document.documentElement).getPropertyValue("--accent-soft").trim() || rend.selColor;
  }, [theme]);

  useEffect(() => {
    if (exited === null) return;
    // Shown under the last line: the shell has ended, any key closes the tab.
    const rend = r.current;
    if (!rend) return;
    const y = Math.min(rend.rows - 1, (rend.cursor?.y ?? rend.rows - 2) + 1);
    rend.lines[y] = { runs: [[0, `[The shell exited with code ${exited}. Press any key to close this tab.]`, rend.fg, -1, 4, 70]], sel: null };
    rend.cursor = null;
    rend.draw([y]);
  }, [exited]);

  // Shown: take the keyboard, and fit (the panel had no size while hidden).
  useEffect(() => {
    if (!shown) return;
    const id = requestAnimationFrame(() => {
      input.current?.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [shown, focusSignal]);

  const handle: TermHandle = {
    clear: () => send({ t: "clear" }),
    copy: () => {
      if (session.current === null) return;
      void api.termCopy(session.current).then((s) => {
        if (s) void navigator.clipboard?.writeText(s).catch(() => {});
      });
    },
    paste: () => {
      void navigator.clipboard
        ?.readText()
        .then((s) => s && send({ t: "paste", text: s }))
        .catch(() => {});
    },
    selectAll: () => send({ t: "selectAll" }),
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const n = e.nativeEvent;
    // Keys typed here belong to the shell, not to the app's shortcuts, except Command
    // shortcuts on macOS that the terminal does not use (⌘, for Settings).
    const a = terminalAction(n, MAC);
    if (!(MAC && n.metaKey && !a)) e.stopPropagation();
    if (cb.current.exited !== null) {
      e.preventDefault();
      cb.current.onCloseExited();
      return;
    }
    if (a) {
      if (a.kind === "paste") return; // the paste event brings the text
      if (cb.current.onAction(a, handle)) e.preventDefault();
      return;
    }
    if (n.isComposing || n.key === "Process" || n.keyCode === 229) return;
    if (MODIFIER_KEYS.has(n.key) || n.key === "Dead") return;
    // App shortcuts with Command on macOS; Ctrl+V pastes everywhere else.
    if (MAC && n.metaKey) return;
    if (!MAC && n.ctrlKey && !n.shiftKey && n.code === "KeyV") return;
    e.preventDefault();
    r.current && (r.current.blinkOn = true);
    // Option on macOS types characters (å, ∑): send them as text.
    if (MAC && n.altKey && !n.ctrlKey && [...n.key].length === 1) {
      send({ t: "text", text: n.key });
      return;
    }
    send({ t: "key", code: n.code, key: n.key, mods: modBits(n), repeat: n.repeat });
  };

  // Pointer: selection, or the program's own mouse handling when it tracks the mouse.
  const drag = useRef<{ mode: "select" | "mouse"; raf: number; last: { x: number; y: number } | null } | null>(null);
  const cellAt = (e: { clientX: number; clientY: number }) => {
    const rend = r.current!;
    const b = canvas.current!.getBoundingClientRect();
    return { x: (e.clientX - b.left) / rend.m.cw, y: (e.clientY - b.top) / rend.m.ch };
  };
  const onPointerDown = (e: React.PointerEvent) => {
    if (!r.current || e.button > 2) return;
    input.current?.focus();
    e.preventDefault();
    const p = cellAt(e);
    const mode = r.current.mouse && !e.shiftKey ? "mouse" : "select";
    if (mode === "select" && e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag.current = { mode, raf: 0, last: null };
    if (mode === "mouse") send({ t: "mouse", action: "press", button: e.button, x: p.x, y: p.y, mods: modBits(e) });
    else send({ t: "select", phase: "press", x: p.x, y: p.y, ms: Math.round(e.timeStamp) });
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    d.last = cellAt(e);
    // One move per animation frame, the newest.
    d.raf ||= requestAnimationFrame(() => {
      d.raf = 0;
      const p = d.last;
      if (!p || drag.current !== d) return;
      if (d.mode === "mouse") send({ t: "mouse", action: "move", button: 0, x: p.x, y: p.y, mods: 0 });
      else send({ t: "select", phase: "drag", x: p.x, y: p.y, ms: Math.round(performance.now()) });
    });
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    cancelAnimationFrame(d.raf);
    const p = cellAt(e);
    if (d.mode === "mouse") send({ t: "mouse", action: "release", button: e.button, x: p.x, y: p.y, mods: modBits(e) });
    else send({ t: "select", phase: "release", x: p.x, y: p.y, ms: Math.round(e.timeStamp) });
  };
  const wheelRest = useRef(0);
  const onWheel = (e: React.WheelEvent) => {
    const rend = r.current;
    if (!rend) return;
    const px = e.deltaMode === 1 ? e.deltaY * rend.m.ch : e.deltaMode === 2 ? e.deltaY * rend.m.ch * rend.rows : e.deltaY;
    wheelRest.current += px / rend.m.ch;
    const lines = Math.trunc(wheelRest.current);
    if (!lines) return;
    wheelRest.current -= lines;
    const p = cellAt(e);
    send({ t: "wheel", x: p.x, y: p.y, lines, mods: modBits(e) });
  };

  // The scrollbar: shown while the history is scrolled back, or on hover.
  const [total, offset, len] = scroll;
  const scrolled = total > len && offset + len < total;
  const sbDrag = (e: React.PointerEvent) => {
    const track = e.currentTarget as HTMLElement;
    track.setPointerCapture(e.pointerId);
    const to = (ev: { clientY: number }) => {
      const b = track.getBoundingClientRect();
      const f = Math.min(1, Math.max(0, (ev.clientY - b.top) / b.height));
      send({ t: "scroll", row: Math.round(f * Math.max(0, total - len)) });
    };
    to(e);
    const move = (ev: PointerEvent) => to(ev);
    const up = () => {
      track.removeEventListener("pointermove", move);
      track.removeEventListener("pointerup", up);
    };
    track.addEventListener("pointermove", move);
    track.addEventListener("pointerup", up);
  };

  return (
    <div className="term-view native" hidden={!shown} ref={host}>
      <canvas ref={canvas} className="term-canvas" onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp} onWheel={onWheel} />
      <textarea
        ref={input}
        className="term-input"
        aria-label="Terminal input"
        autoCapitalize="off"
        autoComplete="off"
        autoCorrect="off"
        spellCheck={false}
        onKeyDown={onKeyDown}
        onFocus={() => {
          if (r.current) {
            r.current.focused = true;
            r.current.blinkOn = true;
            if (r.current.cursor) r.current.draw([r.current.cursor.y]);
          }
          send({ t: "focus", on: true });
        }}
        onBlur={() => {
          if (r.current) {
            r.current.focused = false;
            if (r.current.cursor) r.current.draw([r.current.cursor.y]);
          }
          send({ t: "focus", on: false });
        }}
        onPaste={(e) => {
          e.preventDefault();
          e.stopPropagation();
          const text = e.clipboardData.getData("text");
          if (text) send({ t: "paste", text });
        }}
        onCompositionEnd={(e) => {
          if (e.data) send({ t: "text", text: e.data });
          e.currentTarget.value = "";
        }}
        onInput={(e) => {
          // Text that came without a key event we sent (dead keys, dictation).
          const ta = e.currentTarget;
          const ne = e.nativeEvent as InputEvent;
          if (!ne.isComposing && ta.value) {
            send({ t: "text", text: ta.value });
            ta.value = "";
          }
        }}
      />
      {total > len && (
        <div className={`term-sb${scrolled ? " on" : ""}`} onPointerDown={sbDrag} aria-hidden="true">
          <div className="term-sb-thumb" style={{ top: `${(offset / total) * 100}%`, height: `${Math.max(4, (len / total) * 100)}%` }} />
        </div>
      )}
    </div>
  );
}
