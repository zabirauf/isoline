// Right pane of the compare view: one file's two versions side by side, with its changed symbols
// above. Clicking a symbol scrolls the diff to it (the head side, or the base side when deleted).
import { useEffect, useRef, useState } from "react";
import type { FileChange } from "../ipc/bindings/FileChange";
import type { FileDiffText } from "../ipc/bindings/FileDiffText";
import type { SymbolChange } from "../ipc/bindings/SymbolChange";
import { api } from "../ipc/api";
import { basename, dirname, plural } from "../lib/format";
import { Seg } from "../components/ui";
import type { DiffHost } from "../editor/DiffHost";
import { ChangeBadge, SymbolMark } from "./FileList";

let hostModule: Promise<typeof import("../editor/DiffHost")> | null = null;
const loadHost = () => (hostModule ??= import("../editor/DiffHost"));

export interface RevealRequest {
  path: string;
  symbol: string;
  nonce: number;
}

const VIEW_KEY = "isoline.diffView";
function loadChangesOnly(): boolean {
  try {
    return localStorage.getItem(VIEW_KEY) === "changes";
  } catch {
    return false;
  }
}

export function DiffPane({ base, head, file, reveal, onOpenFile, canOpen }: { base: string; head: string; file: FileChange | null; reveal: RevealRequest | null; onOpenFile(path: string, line?: number): void; canOpen: boolean }) {
  const [text, setText] = useState<FileDiffText | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [changesOnly, setChangesOnly] = useState(loadChangesOnly);
  const [changes, setChanges] = useState<number | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const host = useRef<DiffHost | null>(null);
  const pending = useRef<RevealRequest | null>(null);

  const doReveal = (f: FileChange, name: string) => {
    const s = f.symbols.find((x) => x.name === name);
    const h = host.current;
    if (!s || !h) return;
    if (s.after) h.reveal("head", s.after.start, s.after.end);
    else if (s.before) h.reveal("base", s.before.start, s.before.end);
  };

  useEffect(() => {
    setText(null);
    setError(null);
    setChanges(null);
    if (!file || file.binary) return;
    let alive = true;
    api
      .compareFile(base, head, file.path, file.oldPath ?? undefined)
      .then((t) => alive && setText(t))
      .catch((e: unknown) => alive && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      alive = false;
    };
  }, [base, head, file?.path, file?.oldPath, file?.binary]);

  useEffect(() => {
    const el = box.current;
    if (!text || !el || !file) return;
    let alive = true;
    let off = () => {};
    void loadHost().then(({ DiffHost }) => {
      if (!alive) return;
      const h = new DiffHost({ container: el, before: text.before ?? "", after: text.after ?? "", language: text.monacoLang, ariaLabel: `Changes in ${file.path}`, changesOnly });
      host.current = h;
      off = h.onReady(() => {
        setChanges(h.changeCount());
        const r = pending.current;
        if (r && r.path === file.path) {
          pending.current = null;
          doReveal(file, r.symbol);
        }
      });
    });
    return () => {
      alive = false;
      off();
      host.current?.dispose();
      host.current = null;
    };
    // changesOnly is applied live below; recreating the editor for it would lose the scroll.
  }, [text]);

  useEffect(() => host.current?.setChangesOnly(changesOnly), [changesOnly]);

  useEffect(() => {
    if (!reveal || !file || reveal.path !== file.path) return;
    if (host.current && changes !== null) doReveal(file, reveal.symbol);
    else pending.current = reveal;
  }, [reveal?.nonce]);

  if (!file) return <div className="cmp-empty muted">Pick a file to see what changed in it.</div>;
  const dir = dirname(file.path);
  return (
    <div className="diff-pane">
      <div className="diff-h">
        <ChangeBadge change={file.change} />
        <span className="diff-path" title={file.path}>
          {dir && <span className="cf-dir">{dir}/</span>}
          <b>{basename(file.path)}</b>
        </span>
        {file.oldPath && (
          <span className="caption diff-from" title={file.oldPath}>
            from <code>{file.oldPath}</code>
          </span>
        )}
        <span className="diff-tools">
          {changes !== null && <span className="caption">{changes === 0 ? "no line changes" : plural(changes, "change")}</span>}
          <Seg
            label="Diff view"
            value={changesOnly ? "changes" : "whole"}
            onChange={(v) => {
              setChangesOnly(v === "changes");
              try {
                localStorage.setItem(VIEW_KEY, v);
              } catch {
                /* storage unavailable */
              }
            }}
            options={[
              ["whole", "Whole file"],
              ["changes", "Changes only"],
            ]}
          />
          {canOpen && file.change !== "deleted" && (
            <button type="button" className="iso-btn" onClick={() => onOpenFile(file.path)} title="Open the working-tree version in the reader">
              Open file
            </button>
          )}
        </span>
      </div>
      {file.symbols.length > 0 && (
        <div className="diff-syms" aria-label="Changed symbols">
          {file.symbols.map((s: SymbolChange) => (
            <button
              key={s.change + s.name}
              type="button"
              className={`diff-sym c-${s.change}`}
              onClick={() => {
                if (host.current && changes !== null) doReveal(file, s.name);
                else pending.current = { path: file.path, symbol: s.name, nonce: 0 };
              }}
              title={`${s.change} ${s.kind} ${s.name}`}
            >
              <SymbolMark change={s.change} />
              <code>{s.name}</code>
            </button>
          ))}
        </div>
      )}
      {file.binary ? (
        <div className="cmp-empty muted">Binary file: no text to compare.</div>
      ) : error ? (
        <div className="cmp-empty err">{error}</div>
      ) : (
        <div className="diff-wrap">
          <div className="diff-host" ref={box} />
          {!text && <div className="cmp-empty diff-loading muted">Loading both versions…</div>}
        </div>
      )}
    </div>
  );
}
