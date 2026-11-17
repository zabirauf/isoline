// One function card: layer, name, export pill and lines; one sentence; the algorithm; notes;
// calls and imports as chips; code on demand. Follows the FunctionCard guidelines.
import { memo, useState } from "react";
import type { FnCard } from "../ipc/bindings/FnCard";
import type { Impact } from "../ipc/bindings/Impact";
import { api } from "../ipc/api";
import type { SymInfo } from "../lib/cards";
import type { CardState, Target } from "../lib/focus";
import { lineRange, plural } from "../lib/format";
import { CodeView } from "../editor/CodeView";
import type { CodeContext } from "../editor/EditorHost";
import { AiMark, Note } from "../components/ui";

export interface FunctionCardProps {
  card: FnCard;
  path: string;
  state: CardState;
  hidden: boolean;
  brief: boolean;
  codeOpen: boolean;
  syms: Map<string, SymInfo>;
  codeContext: CodeContext;
  /** The call-flow step's line, when this card owns the flow being stepped. */
  stepLine: number | null;
  stepNonce: number;
  onToggleCode(name: string): void;
  onHeader(name: string): void;
  onTarget(t: Target): void;
  onOpenFile(path: string, line?: number | null): void;
}

function FunctionCardImpl(p: FunctionCardProps) {
  const { card, state, brief, syms } = p;
  const s = card.summary;
  const nLines = card.end - card.start + 1;
  const cls = [
    "iso-card",
    "card",
    card.exported ? "iso-card-exported" : "",
    state === "sel" ? "iso-card-sel" : state === "dep" ? "iso-card-dep" : state === "user" ? "iso-card-user" : "",
    state === "dim" ? "dim" : "",
  ]
    .filter(Boolean)
    .join(" ");
  const chipFn = (n: string) => (
    <button key={n} type="button" className="iso-chip" onClick={() => p.onTarget({ kind: "fn", id: n })}>
      {n}
    </button>
  );
  const chipX = (n: string, typeOnly: boolean) => {
    const info = syms.get(n);
    return (
      <button key={(typeOnly ? "t:" : "") + n} type="button" className={`iso-chip iso-chip-ext${typeOnly ? " iso-chip-type" : ""}`} title={info ? (info.sym.member ? `reached through the ${info.label.split(".")[0]} parameter` : `from ${info.module.id}`) : undefined} onClick={() => p.onTarget({ kind: "sym", id: n })}>
        {info?.label ?? n}
      </button>
    );
  };
  return (
    <article className={cls} id={`card-${card.name}`} data-card={card.name} hidden={p.hidden}>
      <div
        className="card-h iso-card-h"
        role="button"
        tabIndex={0}
        aria-label={`Focus ${card.name} in the graph`}
        aria-pressed={state === "sel"}
        onClick={() => p.onHeader(card.name)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            p.onHeader(card.name);
          }
        }}
      >
        <span className="iso-lvl" title={`Layer ${card.level}: longest call path to a leaf`}>
          L{card.level}
        </span>
        <code className="iso-fname">{card.name}</code>
        {card.exported && <span className="iso-pill">export</span>}
        <span className="iso-lines">{lineRange(card.start, card.end)}</span>
      </div>
      {s ? (
        <p className="iso-desc">
          {s.desc}
          {s.ai && <AiMark model={s.model} />}
        </p>
      ) : card.doc ? (
        <p className="iso-desc">{card.doc}</p>
      ) : (
        <p className="iso-desc no-summary">
          <span className="muted">No summary yet.</span>
        </p>
      )}
      {!s && (
        <code className="card-sig" title={card.signature}>
          {card.signature}
        </code>
      )}
      {!brief && s && s.algo.length > 0 && (
        <ol className="iso-algo">
          {s.algo.map((a, i) => (
            <li key={i}>{a}</li>
          ))}
        </ol>
      )}
      {!brief && s && s.notes.length > 0 && (
        <ul className="notes">
          {s.notes.map((n, i) => (
            <Note key={i} kind={n.kind}>
              {n.text}
            </Note>
          ))}
        </ul>
      )}
      <dl className="iso-deps">
        <dt>Calls</dt>
        <dd>{card.calls.length ? card.calls.map(chipFn) : <span className="none">none</span>}</dd>
        <dt>Imports used</dt>
        <dd>{card.uses.length || card.typeUses.length ? [...card.uses.map((n) => chipX(n, false)), ...card.typeUses.map((n) => chipX(n, true))] : <span className="none">none</span>}</dd>
        <dt>Called by</dt>
        <dd>
          {card.calledBy.map(chipFn)}
          <ExternalCallers card={card} onOpenFile={p.onOpenFile} />
          {!card.calledBy.length && !card.externalCallers && <span className="none">{card.exported ? "external callers (exported)" : "none"}</span>}
        </dd>
      </dl>
      <button type="button" className="iso-btn-reveal" aria-expanded={p.codeOpen} onClick={() => p.onToggleCode(card.name)}>
        {p.codeOpen ? "Hide code" : "Show code"} · {plural(nLines, "line")}
      </button>
      {p.codeOpen && <CodeView path={p.path} start={card.start} end={card.end} fit context={p.codeContext} stepLine={p.stepLine} stepNonce={p.stepNonce} ariaLabel={`Code of ${card.name}, lines ${card.start} to ${card.end}`} />}
    </article>
  );
}

/** "3 callers in other files": expands to the list from get_impact, each opening its file. */
function ExternalCallers({ card, onOpenFile }: { card: FnCard; onOpenFile(path: string, line?: number | null): void }) {
  const [open, setOpen] = useState(false);
  const [impact, setImpact] = useState<Impact | null>(null);
  const [err, setErr] = useState<string | null>(null);
  if (!card.externalCallers) return null;
  const toggle = () => {
    setOpen(!open);
    if (!impact && !open)
      api
        .getImpact(card.id, false)
        .then(setImpact)
        .catch((e: unknown) => setErr(String(e)));
  };
  return (
    <>
      <button type="button" className="iso-btn-drill ext-callers" aria-expanded={open} onClick={toggle} title="Callers in other files, found by the index (heuristic or better)">
        {plural(card.externalCallers, "caller")} in other files
      </button>
      {open && (
        <ul className="impact-list">
          {err && <li className="none">Could not load callers: {err}</li>}
          {!impact && !err && <li className="none">Loading…</li>}
          {impact?.dependents.map((d) => (
            <li key={d.id + d.path + d.line}>
              <button type="button" className="link" onClick={() => onOpenFile(d.path, d.line)}>
                <code>{d.name}</code>
              </button>{" "}
              <span className="caption">
                {d.path}:{d.line} · {d.confidence}
              </span>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

export const FunctionCard = memo(FunctionCardImpl);
