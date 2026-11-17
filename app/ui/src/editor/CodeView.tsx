// React face of EditorHost. Fetches the code lazily with get_source, creates the editor once the
// text is there, and keeps the step-line decoration in sync. Monaco itself is loaded on first use.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { cachedSource } from "../ipc/api";
import type { CodeContext, EditorHost } from "./EditorHost";
import { prefersReducedMotion } from "../lib/theme";

export interface CodeViewProps {
  path: string;
  /** Slice to show; omit both for the whole file. */
  start?: number;
  end?: number;
  /** Grow to the content (card code) instead of filling the parent (whole-file view). */
  fit: boolean;
  context: CodeContext;
  /** File line to mark as the current call-flow step. */
  stepLine?: number | null;
  /** Bumps whenever the reader asks to see the step line again, so it scrolls into view. */
  stepNonce?: number;
  /** Line to scroll to in a whole-file view. */
  revealLine?: number | null;
  ariaLabel: string;
  className?: string;
}

let hostModule: Promise<typeof import("./EditorHost")> | null = null;
const loadHost = () => (hostModule ??= import("./EditorHost"));

/** Nearest ancestor that scrolls vertically. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const s = getComputedStyle(p).overflowY;
    if ((s === "auto" || s === "scroll") && p.scrollHeight > p.clientHeight) return p;
  }
  return document.scrollingElement as HTMLElement | null;
}

export function CodeView(props: CodeViewProps) {
  const { path, start, end, fit, context, stepLine = null, stepNonce = 0, revealLine = null, ariaLabel, className } = props;
  const box = useRef<HTMLDivElement>(null);
  const host = useRef<EditorHost | null>(null);
  const [height, setHeight] = useState<number>(fit && start && end ? (end - start + 1) * 19 + 16 : 0);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ctxRef = useRef(context);
  ctxRef.current = context;

  useEffect(() => {
    let alive = true;
    let off: (() => void) | undefined;
    setReady(false);
    setError(null);
    Promise.all([cachedSource(path, start, end), loadHost().then(async (m) => (await m.EditorHost.ready(), m))])
      .then(([slice, m]) => {
        if (!alive || !box.current) return;
        const h = new m.EditorHost({ container: box.current, text: slice.text, language: slice.monacoLang || "plaintext", firstLine: slice.start, fit, context: ctxRef.current, ariaLabel });
        host.current = h;
        if (fit) off = h.onHeight((px) => setHeight(px));
        setReady(true);
      })
      .catch((e: unknown) => alive && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      alive = false;
      off?.();
      host.current?.dispose();
      host.current = null;
    };
    // The editor is rebuilt only when the code it shows changes.
  }, [path, start, end, fit]);

  useEffect(() => {
    if (ready) host.current?.setContext(context);
  }, [ready, context]);

  useLayoutEffect(() => {
    const h = host.current;
    if (!ready || !h) return;
    h.setStepLine(stepLine);
    if (stepLine === null) return;
    if (!fit) {
      h.revealLine(stepLine);
      return;
    }
    // Card code has no scrollbar of its own: scroll the reading list so the line sits mid-pane.
    const el = box.current!;
    const sp = scrollParent(el);
    if (!sp) return;
    requestAnimationFrame(() => {
      const top = el.getBoundingClientRect().top - sp.getBoundingClientRect().top + sp.scrollTop + h.lineTop(stepLine);
      sp.scrollTo({ top: Math.max(0, top - sp.clientHeight / 2), behavior: prefersReducedMotion() ? "auto" : "smooth" });
    });
  }, [ready, stepLine, stepNonce, fit]);

  useEffect(() => {
    if (ready && revealLine !== null) host.current?.revealLine(revealLine);
  }, [ready, revealLine]);

  if (error) return <div className={`code-error ${className ?? ""}`}>Could not load the code: {error}</div>;
  return (
    <div className={`code-host ${fit ? "code-fit" : "code-fill"} ${ready ? "" : "code-loading"} ${className ?? ""}`} style={fit ? { height } : undefined}>
      <div ref={box} className="code-box" />
    </div>
  );
}
