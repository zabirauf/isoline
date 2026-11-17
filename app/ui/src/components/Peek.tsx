// Definition peek (iso-peek, spec section 3): one floating panel for the whole app. Hovering or
// focusing a target for 400ms opens it; once open it is warm for 800ms, so the next target swaps
// in at once. Clicking inside pins it. Names inside the peek replace its content in place, with a
// breadcrumb; "Open ›" does a real go to definition.
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { SymbolTarget } from "../ipc/bindings/SymbolTarget";
import { cachedSource } from "../ipc/api";
import { highlightLines } from "../lib/hl";
import { lineRange, plural } from "../lib/format";
import type { TargetIndex } from "../lib/sig";

export interface SymbolPeek {
  kind: "symbol";
  name: string;
  target: SymbolTarget | null;
  /** Targets of the file the name appears in: names inside the peek are looked up here. */
  idx: TargetIndex;
  /** Language label for built-ins ("built-in · TypeScript"). */
  langLabel: string;
  /** The file being viewed: a target in it reads "this file". */
  viewPath: string;
  /** For a function in this file: its card's description, shown instead of code. */
  card?: { desc: string | null; start: number; end: number } | null;
  /** Real go to definition (Open ›, ⌘-click). */
  onGoTo?(name: string, target: SymbolTarget | null): void;
  /** Card info for nested names that are functions in this file. */
  cardOf?(name: string): { desc: string | null; start: number; end: number } | null;
}

export interface TextPeek {
  kind: "text";
  title?: string;
  body: ReactNode;
}

export type PeekSubject = SymbolPeek | TextPeek;

interface PeekState {
  subject: PeekSubject;
  rect: DOMRect;
  pinned: boolean;
  crumbs: SymbolPeek[];
}

interface PeekApi {
  /** Event props for an element that shows a peek on hover and on keyboard focus. */
  bind(get: () => PeekSubject | null): {
    onPointerEnter(e: React.PointerEvent): void;
    onPointerLeave(): void;
    onFocus(e: React.FocusEvent): void;
    onBlur(): void;
    onKeyDown(e: React.KeyboardEvent): void;
  };
  /** Open now (no delay) anchored to a rectangle; used by Monaco and by "pin" clicks. */
  show(subject: PeekSubject, rect: DOMRect, pinned?: boolean): void;
  /** Hover moved over a target that Monaco reports; opens after the usual delay. */
  hoverAt(subject: PeekSubject | null, rect: DOMRect | null): void;
  close(): void;
}

const Ctx = createContext<PeekApi | null>(null);

export function usePeek(): PeekApi {
  const v = useContext(Ctx);
  if (!v) throw new Error("usePeek outside PeekProvider");
  return v;
}

const OPEN_MS = 400;
const WARM_MS = 800;
const CLOSE_MS = 200;

function sameSubject(a: PeekSubject | null, b: PeekSubject | null) {
  if (!a || !b || a.kind !== b.kind) return false;
  if (a.kind === "symbol" && b.kind === "symbol") return a.name === b.name && a.target?.path === b.target?.path && a.target?.start === b.target?.start;
  return a === b;
}

export function PeekProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<PeekState | null>(null);
  const stateRef = useRef(state);
  stateRef.current = state;
  const openTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const closeTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const lastOpen = useRef(0);
  const pending = useRef<PeekSubject | null>(null);

  const warm = () => !!stateRef.current || performance.now() - lastOpen.current < WARM_MS;
  const clearTimers = () => {
    clearTimeout(openTimer.current);
    clearTimeout(closeTimer.current);
  };

  const close = useCallback(() => {
    clearTimers();
    if (stateRef.current) lastOpen.current = performance.now();
    pending.current = null;
    setState(null);
  }, []);

  const closeSoon = useCallback(() => {
    clearTimeout(openTimer.current);
    pending.current = null;
    if (stateRef.current?.pinned) return;
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => {
      if (!stateRef.current?.pinned) close();
    }, CLOSE_MS);
  }, [close]);

  const show = useCallback((subject: PeekSubject, rect: DOMRect, pinned = false) => {
    clearTimers();
    lastOpen.current = performance.now();
    setState((s) => {
      if (s?.pinned && !pinned) return s;
      return { subject, rect, pinned, crumbs: subject.kind === "symbol" ? [subject] : [] };
    });
  }, []);

  const openLater = useCallback(
    (subject: PeekSubject, rect: DOMRect) => {
      if (stateRef.current?.pinned) return;
      clearTimeout(closeTimer.current);
      if (sameSubject(stateRef.current?.subject ?? null, subject)) return;
      clearTimeout(openTimer.current);
      pending.current = subject;
      if (warm()) show(subject, rect);
      else
        openTimer.current = setTimeout(() => {
          if (pending.current === subject) show(subject, rect);
        }, OPEN_MS);
    },
    [show],
  );

  const api = useMemo<PeekApi>(
    () => ({
      bind: (get) => ({
        onPointerEnter: (e) => {
          const s = get();
          if (s) openLater(s, (e.currentTarget as HTMLElement).getBoundingClientRect());
        },
        onPointerLeave: () => closeSoon(),
        onFocus: (e) => {
          const s = get();
          if (s) openLater(s, (e.currentTarget as HTMLElement).getBoundingClientRect());
        },
        onBlur: () => closeSoon(),
        onKeyDown: (e) => {
          // Shift+Enter pins the peek of the focused target.
          if (e.key === "Enter" && e.shiftKey) {
            const s = get();
            if (s) {
              e.preventDefault();
              show(s, (e.currentTarget as HTMLElement).getBoundingClientRect(), true);
            }
          }
        },
      }),
      show,
      hoverAt: (subject, rect) => (subject && rect ? openLater(subject, rect) : closeSoon()),
      close,
    }),
    [openLater, closeSoon, show, close],
  );

  // Esc closes; a click outside closes a pinned peek; scrolling the page closes an unpinned one.
  useEffect(() => {
    if (!state) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    const onDown = (e: PointerEvent) => {
      if (!(e.target as Element).closest(".iso-peek")) close();
    };
    const onScroll = (e: Event) => {
      if (stateRef.current?.pinned) return;
      if ((e.target as Element | null)?.closest?.(".iso-peek")) return;
      close();
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onDown, true);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onDown, true);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [state, close]);

  const pin = () => setState((s) => (s && !s.pinned ? { ...s, pinned: true } : s));
  const nest = (sub: SymbolPeek) => setState((s) => (s ? { ...s, subject: sub, pinned: true, crumbs: [...s.crumbs, sub] } : s));
  const toCrumb = (i: number) => setState((s) => (s ? { ...s, subject: s.crumbs[i], crumbs: s.crumbs.slice(0, i + 1) } : s));

  return (
    <Ctx.Provider value={api}>
      {children}
      {state && (
        <PeekPanel
          state={state}
          onEnter={() => clearTimeout(closeTimer.current)}
          onLeave={closeSoon}
          onPin={pin}
          onNest={nest}
          onCrumb={toCrumb}
          onClose={close}
        />
      )}
    </Ctx.Provider>
  );
}

function PeekPanel({ state, onEnter, onLeave, onPin, onNest, onCrumb, onClose }: { state: PeekState; onEnter(): void; onLeave(): void; onPin(): void; onNest(s: SymbolPeek): void; onCrumb(i: number): void; onClose(): void }) {
  const { rect, subject, pinned, crumbs } = state;
  const vh = window.innerHeight;
  const vw = window.innerWidth;
  const width = subject.kind === "text" ? 320 : 480;
  const below = vh - rect.bottom >= 220 || vh - rect.bottom >= rect.top;
  const left = Math.max(8, Math.min(rect.left, vw - width - 8));
  const style: React.CSSProperties = below ? { left, top: rect.bottom + 6, width, maxHeight: Math.min(360, vh - rect.bottom - 14) } : { left, bottom: vh - rect.top + 6, width, maxHeight: Math.min(360, rect.top - 14) };
  return (
    <div className={`iso-peek${pinned ? " pinned" : ""}${subject.kind === "text" ? " iso-peek-text" : ""}`} role="dialog" aria-label="Definition" style={style} onPointerEnter={onEnter} onPointerLeave={onLeave} onPointerDown={onPin}>
      {crumbs.length > 1 && (
        <nav className="peek-crumbs" aria-label="Peek path">
          {crumbs.map((c, i) => (
            <span key={i}>
              {i > 0 && <span className="sep">›</span>}
              {i === crumbs.length - 1 ? (
                <span className="cur">{c.name}</span>
              ) : (
                <button type="button" className="crumb" onClick={() => onCrumb(i)}>
                  {c.name}
                </button>
              )}
            </span>
          ))}
        </nav>
      )}
      {subject.kind === "text" ? (
        <div className="peek-text">
          {subject.title && <b>{subject.title}</b>}
          <div>{subject.body}</div>
        </div>
      ) : (
        <SymbolBody s={subject} onNest={onNest} onPin={onPin} />
      )}
      {pinned && (
        <div className="peek-foot">
          <button type="button" className="iso-btn quiet" onClick={onClose}>
            Close (Esc)
          </button>
        </div>
      )}
    </div>
  );
}

const KIND_WORD: Record<string, string> = { function: "function", method: "method", class: "class", interface: "interface", type: "type", enum: "enum", struct: "struct", trait: "trait", constant: "constant", variable: "variable", module: "module" };

function originLine(s: SymbolPeek): ReactNode {
  const t = s.target;
  if (!t) return null;
  const range = t.path && t.start ? lineRange(t.start, t.end) : null;
  if (t.origin === "builtin") return <>built-in · {s.langLabel}</>;
  if (t.origin === "external")
    return (
      <>
        <code>{t.module ?? "package"}</code> · package{t.path ? <> · {t.path}</> : null}
        {range ? <> · {range}</> : null}
      </>
    );
  if (t.path === s.viewPath) return <>this file{range ? <> · {range}</> : null}</>;
  return (
    <>
      {t.path ? (
        <button type="button" className="filelink" onClick={() => s.onGoTo?.(s.name, t)}>
          {t.path}
        </button>
      ) : (
        <code>{t.module}</code>
      )}
      {range ? <> · {range}</> : null}
    </>
  );
}

function SymbolBody({ s, onNest, onPin }: { s: SymbolPeek; onNest(s: SymbolPeek): void; onPin(): void }) {
  const t = s.target;
  const [moreDoc, setMoreDoc] = useState(false);
  const [all, setAll] = useState(false);
  const [lines, setLines] = useState<string[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const card = s.card ?? (t?.origin === "local" && t.kind === "function" ? s.cardOf?.(s.name) : null);
  const hasSource = !!t && !!t.path && t.start > 0 && t.origin !== "builtin" && !card;
  useEffect(() => {
    setLines(null);
    setErr(null);
    setMoreDoc(false);
    setAll(false);
    if (!hasSource) return;
    let alive = true;
    cachedSource(t!.path!, t!.start, t!.end)
      .then((sl) => alive && setLines(sl.text.replace(/\n$/, "").split("\n")))
      .catch((e: unknown) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [hasSource, t]);

  if (!t)
    return (
      <div className="peek-body">
        <div className="peek-h">
          <span className="ty ty-unknown">{s.name}</span>
        </div>
        <p className="muted">
          Not resolved. Isoline found no definition for <code>{s.name}</code> in this file's imports.
        </p>
      </div>
    );

  const kind = t.kind ? (KIND_WORD[t.kind] ?? t.kind) : t.origin === "builtin" ? "built-in" : "symbol";
  const toks = lines ? highlightLines(lines) : null;
  const nestable = (name: string) => {
    const tt = s.idx.get(name);
    return tt && tt.origin !== "builtin" && name !== s.name ? tt : null;
  };
  const shown = toks ? (all ? toks : toks.slice(0, 14)) : null;
  return (
    <div className="peek-body">
      <div className="peek-h">
        <span className="iso-mono-tag">{kind.toUpperCase()}</span>
        <span className={`ty ty-${t.origin}${t.origin === "local" ? " ty-strong" : ""}`}>{s.name}</span>
      </div>
      <div className="peek-origin caption">{originLine(s)}</div>
      {t.doc && (
        <p className={`peek-doc${moreDoc ? "" : " clamp"}`}>
          {t.doc}
          {!moreDoc && t.doc.length > 160 && (
            <button type="button" className="link-quiet" onClick={() => setMoreDoc(true)}>
              More
            </button>
          )}
        </p>
      )}
      {card && (
        <p className="peek-doc">
          {card.desc ?? <span className="muted">No description yet.</span>} <span className="caption">{lineRange(card.start, card.end)}</span>
        </p>
      )}
      {t.origin === "external" && hasSource && <div className="caption">Declaration (package)</div>}
      {hasSource && (
        <div className="peek-code" onScroll={onPin}>
          {err ? (
            <div className="err">Could not load the source: {err}</div>
          ) : !shown ? (
            <div className="muted">Loading lines {lineRange(t.start, t.end).slice(1)}…</div>
          ) : (
            <pre>
              {shown.map((row, i) => (
                <div className="row" key={i}>
                  <span className="n">{t.start + i}</span>
                  <span>
                    {row.length === 0
                      ? " "
                      : row.map((tok, j) => {
                          const nt = tok.kind === "name" || tok.kind === "type" ? nestable(tok.text) : null;
                          if (nt)
                            return (
                              <button
                                key={j}
                                type="button"
                                className={`pk-link ty-${nt.origin}`}
                                onClick={(e) => {
                                  if (e.metaKey || e.ctrlKey) s.onGoTo?.(tok.text, nt);
                                  else onNest({ ...s, name: tok.text, target: nt, card: s.cardOf?.(tok.text) ?? null });
                                }}
                              >
                                {tok.text}
                              </button>
                            );
                          return (
                            <span key={j} className={tok.kind === "plain" || tok.kind === "name" ? undefined : `tk-${tok.kind}`}>
                              {tok.text}
                            </span>
                          );
                        })}
                  </span>
                </div>
              ))}
            </pre>
          )}
        </div>
      )}
      {!hasSource && !card && (
        <>
          {t.signature && <code className="peek-sig">{t.signature}</code>}
          <p className="muted">No source in this repo.</p>
        </>
      )}
      <div className="peek-actions">
        {toks && toks.length > 14 && (
          <button type="button" className="iso-btn-reveal" onClick={() => setAll(!all)}>
            {all ? "Show less" : `Show all · ${plural(toks.length, "line")}`}
          </button>
        )}
        {t.origin !== "builtin" && t.origin !== "external" && s.onGoTo && (
          <button type="button" className="iso-btn-drill" onClick={() => s.onGoTo!(s.name, t)}>
            Open ›
          </button>
        )}
      </div>
    </div>
  );
}
