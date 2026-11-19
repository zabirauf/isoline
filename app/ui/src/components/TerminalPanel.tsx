// The terminal panel: shells in tabs, docked at the bottom or on the right side. Each tab is
// a shell on a pseudo-terminal in the backend, started in the repo folder; ghostty-web
// (Ghostty's VT parser as WebAssembly) draws it. Tabs stay alive while the panel is hidden.
import { useCallback, useEffect, useRef, useState } from "react";
import type { FitAddon, ITheme, Terminal } from "ghostty-web";
import { NativeTermView, type TermHandle, type TermViewProps } from "./NativeTermView";
import { api } from "../ipc/api";
import { shortcutText, terminalAction, type TermAction } from "../lib/terminalKeys";

const MAC = typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");
const FONT = "'IBM Plex Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";

type Ghostty = typeof import("ghostty-web");
let ready: Promise<Ghostty> | null = null;
/**
 * Loads ghostty-web (with its WebAssembly parser) and the font once, on the first terminal:
 * the app starts without it. The canvas measures the font when it opens.
 */
function ghosttyReady(): Promise<Ghostty> {
  ready ??= import("ghostty-web").then(async (g) => {
    await Promise.all([g.init(), document.fonts?.load(`13px ${FONT}`).catch(() => [])]);
    return g;
  });
  return ready;
}

/** Terminal colors from the app's theme tokens, with an ANSI palette for light or dark. */
function termTheme(): ITheme {
  const css = getComputedStyle(document.documentElement);
  const v = (n: string, d: string) => css.getPropertyValue(n).trim() || d;
  const bg = v("--surface", "#ffffff");
  const dark = (() => {
    const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(bg);
    if (!m) return false;
    const [r, g, b] = [m[1], m[2], m[3]].map((x) => parseInt(x, 16));
    return 0.2126 * r + 0.7152 * g + 0.0722 * b < 128;
  })();
  const base: ITheme = dark
    ? { black: "#1d232c", red: "#f07178", green: "#9ccc65", yellow: "#e6c07b", blue: "#7f9ff2", magenta: "#c792ea", cyan: "#7fd1c7", white: "#c9d1db", brightBlack: "#6b7685", brightRed: "#ff8a8f", brightGreen: "#b5e07f", brightYellow: "#f2d48f", brightBlue: "#a3bcff", brightMagenta: "#dcb0f5", brightCyan: "#9be4da", brightWhite: "#f4f6f9" }
    : { black: "#17202c", red: "#c62f3a", green: "#2f7d32", yellow: "#946200", blue: "#2753c4", magenta: "#8a3fb5", cyan: "#1b7f86", white: "#c4cad3", brightBlack: "#5b6676", brightRed: "#e0444f", brightGreen: "#3c9a40", brightYellow: "#b07a00", brightBlue: "#3d6be0", brightMagenta: "#a552d1", brightCyan: "#22979f", brightWhite: "#ffffff" };
  return { ...base, background: bg, foreground: v("--ink", dark ? "#e3e7ed" : "#17202c"), cursor: v("--accent", "#2753c4"), cursorAccent: bg, selectionBackground: v("--accent-soft", "#e5ebfa"), selectionForeground: v("--ink", "#17202c") };
}

/** The terminal theme, kept in step with the app theme (a data-theme change or the system's). */
function useTermTheme(): ITheme {
  const [theme, setTheme] = useState<ITheme>(() => termTheme());
  useEffect(() => {
    const update = () => setTheme(termTheme());
    const mo = new MutationObserver(update);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "class", "style"] });
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", update);
    return () => {
      mo.disconnect();
      mq.removeEventListener("change", update);
    };
  }, []);
  return theme;
}

interface Tab {
  key: number;
  /** Shown in the default name ("zsh 2"); never reused while the panel lives. */
  n: number;
  /** The backend session, once the shell has started. */
  session: number | null;
  shell: string;
  title: string | null;
  cwd: string | null;
  /** Exit code of a shell that ended on its own with an error; the tab waits for a key. */
  exited: number | null;
  error: string | null;
}

export function TerminalPanel({ open, cwd, dock, size, focusSignal, onHide, onDock, onResize }: { open: boolean; cwd: string | null; dock: "bottom" | "side"; size: number; focusSignal: number; onHide(): void; onDock(d: "bottom" | "side"): void; onResize(px: number): void }) {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState<number | null>(null);
  // The size while the splitter is dragged; saved when the drag ends.
  const [live, setLive] = useState<number | null>(null);
  const nextKey = useRef(0);
  const theme = useTermTheme();
  const keys = shortcutText(MAC);
  // Native frames from libghostty-vt in the backend (macOS, Windows), or raw bytes for
  // ghostty-web (Linux, the mock): asked once.
  const [native, setNative] = useState<boolean | null>(null);
  useEffect(() => {
    void api
      .termNative()
      .then(setNative)
      .catch(() => setNative(false));
  }, []);
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;

  const newTab = useCallback(() => {
    const key = ++nextKey.current;
    setTabs((t) => [...t, { key, n: t.length === 0 ? 1 : Math.max(...t.map((x) => x.n)) + 1, session: null, shell: "", title: null, cwd, exited: null, error: null }]);
    setActive(key);
  }, [cwd]);

  const closeTab = useCallback(
    (key: number) => {
      const list = tabsRef.current;
      const i = list.findIndex((t) => t.key === key);
      if (i < 0) return;
      const rest = list.filter((t) => t.key !== key);
      setTabs(rest);
      setActive((a) => (a === key ? (rest[Math.min(i, rest.length - 1)]?.key ?? null) : a));
      // The last tab closed: hide the panel, like closing the last terminal in an editor.
      if (rest.length === 0) onHide();
    },
    [onHide],
  );

  // Opening the panel with no tab starts one.
  useEffect(() => {
    if (open && tabsRef.current.length === 0) newTab();
  }, [open, newTab]);

  // A shell that exits closes its tab; one that failed keeps it, to show the code.
  useEffect(() => {
    const off = api.listen("term_exit", ({ id, code }) => {
      const t = tabsRef.current.find((x) => x.session === id);
      if (!t) return;
      if (!code) closeTab(t.key);
      else setTabs((all) => all.map((x) => (x.key === t.key ? { ...x, exited: code } : x)));
    });
    return () => void off.then((f) => f());
  }, [closeTab]);

  const act = useCallback(
    (a: TermAction, h: TermHandle): boolean => {
      const list = tabsRef.current;
      const i = list.findIndex((t) => t.key === active);
      switch (a.kind) {
        case "toggle":
          onHide();
          return true;
        case "new":
          newTab();
          return true;
        case "close":
          if (active !== null) closeTab(active);
          return true;
        case "next":
        case "prev":
          if (list.length > 1) setActive(list[(i + (a.kind === "next" ? 1 : list.length - 1)) % list.length].key);
          return true;
        case "goto":
          if (list[a.index]) setActive(list[a.index].key);
          return true;
        case "clear":
          h.clear();
          return true;
        case "copy":
          h.copy();
          return true;
        case "paste":
          h.paste();
          return true;
        case "selectAll":
          h.selectAll();
          return true;
      }
    },
    [active, closeTab, newTab, onHide],
  );

  // The splitter: drag the panel's inner edge to resize it.
  const startDrag = (e: React.PointerEvent) => {
    e.preventDefault();
    const start = dock === "bottom" ? e.clientY : e.clientX;
    const from = size;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    let last = from;
    const move = (ev: PointerEvent) => {
      const d = start - (dock === "bottom" ? ev.clientY : ev.clientX);
      const max = (dock === "bottom" ? window.innerHeight - 160 : window.innerWidth - 320) || 400;
      last = Math.round(Math.max(dock === "bottom" ? 120 : 260, Math.min(max, from + d)));
      setLive(last);
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      setLive(null);
      onResize(last);
    };
    el.addEventListener("pointercancel", up);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  return (
    <section className={`term-panel dock-${dock}`} hidden={!open} style={dock === "bottom" ? { height: live ?? size } : { width: live ?? size }} aria-label="Terminal">
      <div className={`term-grip ${dock}`} role="separator" aria-orientation={dock === "bottom" ? "horizontal" : "vertical"} aria-label="Resize the terminal" onPointerDown={startDrag} />
      <div className="term-bar">
        <div className="term-tabs" role="tablist" aria-label="Terminals">
          {tabs.map((t, i) => (
            <div key={t.key} className={`term-tab${t.key === active ? " on" : ""}`} role="tab" aria-selected={t.key === active} tabIndex={t.key === active ? 0 : -1} title={`${t.cwd ?? ""}${i < 9 ? ` · ${keys.goto.replace("1…9", String(i + 1))}` : ""}`} onClick={() => setActive(t.key)} onAuxClick={(e) => e.button === 1 && closeTab(t.key)}>
              <span className="term-tab-name">{t.title || `${t.shell || "shell"}${t.n > 1 ? ` ${t.n}` : ""}`}</span>
              <button
                type="button"
                className="term-x"
                aria-label={`Close ${t.title || t.shell || "terminal"}`}
                title={`Close (${keys.close})`}
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.key);
                }}
              >
                ×
              </button>
            </div>
          ))}
          <button type="button" className="term-new" aria-label="New terminal" title={`New terminal (${keys.new})`} onClick={newTab}>
            +
          </button>
        </div>
        <span className="term-sp" />
        <span className="caption term-keys">
          {keys.next} / {keys.prev} switch · {keys.toggle} hide
        </span>
        <button type="button" className="iso-btn quiet" onClick={() => onDock(dock === "bottom" ? "side" : "bottom")} title="Dock the terminal on the other edge">
          {dock === "bottom" ? "To the side" : "To the bottom"}
        </button>
        <button type="button" className="iso-btn quiet" aria-label="Hide the terminal" title={`Hide (${keys.toggle})`} onClick={onHide}>
          ×
        </button>
      </div>
      <div className="term-body">
        {native !== null &&
          tabs.map((t) => {
            const View = native ? NativeTermView : TermView;
            return (
          <View
            key={t.key}
            exited={t.exited}
            cwd={t.cwd}
            shown={open && t.key === active}
            focusSignal={focusSignal}
            theme={theme}
            onAction={act}
            onStarted={(session, shell, dir) => setTabs((all) => all.map((x) => (x.key === t.key ? { ...x, session, shell, cwd: dir } : x)))}
            onTitle={(title) => setTabs((all) => all.map((x) => (x.key === t.key ? { ...x, title } : x)))}
            onError={(error) => setTabs((all) => all.map((x) => (x.key === t.key ? { ...x, error } : x)))}
            onCloseExited={() => closeTab(t.key)}
          />
            );
          })}
      </div>
    </section>
  );
}

/** Output kept per tab to redraw it in new colors: Ghostty's terminal fixes its colors when made. */
const REPLAY_BYTES = 1 << 20;

/** A terminal drawn by ghostty-web from raw output (Linux, the mock). */
function TermView({ exited, cwd, shown, focusSignal, theme, onAction, onStarted, onTitle, onError, onCloseExited }: TermViewProps) {
  const tab = { exited };
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const session = useRef<number | null>(null);
  const log = useRef<{ chunks: Uint8Array[]; size: number }>({ chunks: [], size: 0 });
  const themeRef = useRef(theme);
  // Callbacks change every render; the terminal reads the latest through refs.
  const cb = useRef({ onAction, onStarted, onTitle, onError, onCloseExited, exited: tab.exited, shown });
  cb.current = { onAction, onStarted, onTitle, onError, onCloseExited, exited: tab.exited, shown };

  /** Makes a terminal in the host with these colors, wired to the tab's shell. */
  const ghostty = useRef<Ghostty | null>(null);
  const build = (colors: ITheme): Terminal | null => {
    const el = host.current;
    const g = ghostty.current;
    if (!el || !g) return null;
    el.replaceChildren();
    const t = new g.Terminal({ fontSize: 13, fontFamily: FONT, theme: colors, cursorBlink: true, scrollback: 10000 });
    t.open(el);
    const f = new g.FitAddon();
    t.loadAddon(f);
    f.fit();
    f.observeResize();
    t.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return false;
      // A shell that ended with an error: any key closes its tab.
      if (cb.current.exited !== null) {
        cb.current.onCloseExited();
        return true;
      }
      const a = terminalAction(e, MAC);
      // ghostty-web pastes from the paste event itself.
      if (!a || a.kind === "paste") return false;
      return cb.current.onAction(a, {
        clear: () => t.clear(),
        copy: () => {
          const s = t.getSelection();
          if (s) void navigator.clipboard?.writeText(s).catch(() => {});
        },
        paste: () => {},
        selectAll: () => t.selectAll(),
      });
    });
    t.onTitleChange((s) => s && cb.current.onTitle(s));
    t.onData((d) => session.current !== null && void api.termWrite(session.current, d).catch(() => {}));
    t.onResize(({ cols, rows }) => session.current !== null && void api.termResize(session.current, cols, rows).catch(() => {}));
    term.current = t;
    fit.current = f;
    if (cb.current.shown) t.focus();
    return t;
  };

  useEffect(() => {
    let alive = true;
    void (async () => {
      ghostty.current = await ghosttyReady();
      if (!alive) return;
      const t = build(themeRef.current);
      if (!t) return;
      try {
        const info = await api.termOpen({ cwd, cols: t.cols, rows: t.rows }, (d) => {
          if (!(d instanceof Uint8Array)) return;
          const bytes = d;
          const l = log.current;
          l.chunks.push(bytes);
          l.size += bytes.length;
          while (l.size > REPLAY_BYTES && l.chunks.length > 1) l.size -= l.chunks.shift()!.length;
          term.current?.write(bytes);
        });
        if (!alive) {
          void api.termClose(info.id);
          return;
        }
        session.current = info.id;
        cb.current.onStarted(info.id, info.shell, info.cwd);
        const now = term.current;
        if (now) void api.termResize(info.id, now.cols, now.rows).catch(() => {});
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        cb.current.onError(msg);
        term.current?.write(`\x1b[31m${msg}\x1b[0m\r\n`);
      }
    })();
    return () => {
      alive = false;
      if (session.current !== null) void api.termClose(session.current);
      session.current = null;
      term.current?.dispose();
      term.current = null;
      fit.current = null;
    };
  }, []);

  // New colors: make the terminal again and replay its output into it.
  useEffect(() => {
    if (themeRef.current === theme) return;
    themeRef.current = theme;
    const old = term.current;
    if (!old) return;
    old.dispose();
    const t = build(theme);
    if (!t) return;
    for (const c of log.current.chunks) t.write(c);
    if (cb.current.shown) t.focus();
  }, [theme]);

  useEffect(() => {
    if (tab.exited !== null) term.current?.write(`\r\n\x1b[2m[The shell exited with code ${tab.exited}. Press any key to close this tab.]\x1b[0m`);
  }, [tab.exited]);

  // Shown: fit to the panel (it had no size while hidden) and take the keyboard.
  useEffect(() => {
    if (!shown) return;
    const id = requestAnimationFrame(() => {
      fit.current?.fit();
      term.current?.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [shown, focusSignal]);

  return (
    <div
      className="term-view"
      hidden={!shown}
      ref={host}
      // Keys typed in the terminal belong to the shell, not to the app's shortcuts. On macOS
      // the app's Command shortcuts (⌘, for Settings) still work, since no shell gets them.
      onKeyDown={(e) => {
        if (MAC && e.metaKey && !terminalAction(e.nativeEvent, MAC)) return;
        e.stopPropagation();
      }}
    />
  );
}
