// Outline-tier and text-tier files: the whole file in Monaco, an outline list, and a sentence
// saying why there are no cards, map or call flow.
import { useMemo, useRef, useState } from "react";
import type { FileView } from "../ipc/bindings/FileView";
import { lineRange, plural } from "../lib/format";
import type { CodeContext } from "../editor/EditorHost";
import { CodeView } from "../editor/CodeView";
import { AiMark, Eyebrow, RichText, TierBadge } from "../components/ui";
import { FileHeader } from "./FileHeader";
import { Splitter, initialSplit } from "./Splitter";

export function tierSentence(view: FileView): string {
  if (view.tier === "outline")
    return `${view.langLabel} is in the outline tier: Isoline reads its symbols and imports from the grammar's tags query, so there are no function cards, dependency map or call flow, and edges to other files are heuristic.`;
  return `${view.langLabel} is in the text tier: the file is not parsed, so there is no outline, dependency map or call flow. You get highlighting, search and a file summary.`;
}

export function PlainFile({ view, focusLine, onOpenFile }: { view: FileView; focusLine?: number | null; onOpenFile(path: string): void }) {
  const [reveal, setReveal] = useState<{ line: number; n: number } | null>(focusLine ? { line: focusLine, n: 0 } : null);
  const [split] = useState(initialSplit);
  const mainRef = useRef<HTMLDivElement>(null);
  const context = useMemo<CodeContext>(
    () => ({
      path: view.path,
      cards: new Map(),
      syms: new Map(),
      types: new Map(),
      outline: new Map(view.outline.map((o) => [o.name.split(".").pop()!, { start: o.start, end: o.end, kind: o.kind }])),
    }),
    [view],
  );
  return (
    <div className="workbench plain">
      <FileHeader view={view} />
      <div className="main" ref={mainRef} style={{ ["--left" as string]: Math.min(split, 36) + "%" }}>
        <div className="pane-left">
          <div className="inner">
            <section className="overview tier-note">
              <Eyebrow>Why there is no call flow</Eyebrow>
              <p>
                <TierBadge tier={view.tier} /> {tierSentence(view)}
              </p>
              {view.overview && (
                <p>
                  <RichText text={view.overview.text} />
                  <AiMark model={view.overview.model} />
                </p>
              )}
            </section>
            {view.tier === "outline" && (
              <section className="outline-list" aria-label="Outline">
                <div className="group-h">
                  <h2>Outline</h2>
                  <span>{plural(view.outline.length, "symbol")}</span>
                </div>
                {view.outline.length ? (
                  <ul>
                    {view.outline.map((o) => (
                      <li key={o.name + o.start} style={{ paddingLeft: o.depth * 18 }}>
                        <button type="button" className="outline-item" onClick={() => setReveal((r) => ({ line: o.start, n: (r?.n ?? 0) + 1 }))}>
                          <span className="okind">{o.kind}</span>
                          <code>{o.name}</code>
                          <span className="iso-lines">{lineRange(o.start, o.end)}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="muted">The tags query found no symbols in this file.</p>
                )}
              </section>
            )}
            {view.modules.length > 0 && (
              <section className="outline-list" aria-label="Imports">
                <div className="group-h">
                  <h2>Imports</h2>
                  <span>{plural(view.modules.length, "module")}</span>
                </div>
                <ul className="imports">
                  {view.modules.map((m) => (
                    <li key={m.id}>
                      {m.resolved ? (
                        <button type="button" className="filelink" onClick={() => onOpenFile(m.resolved!)}>
                          {m.id}
                        </button>
                      ) : (
                        <code className="ext">{m.id}</code>
                      )}
                      <span className="caption">{m.external ? "outside the repo" : m.resolved ? `→ ${m.resolved}` : "unresolved"} · L{m.line}</span>
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </div>
        </div>
        <Splitter target={mainRef} />
        <div className="pane-right whole-file">
          <CodeView key={view.path} path={view.path} fit={false} context={context} stepLine={reveal?.line ?? null} stepNonce={reveal?.n ?? 0} ariaLabel={`${view.name}, whole file`} />
        </div>
      </div>
    </div>
  );
}
