// One function card. Detail levels (spec section 5):
//   Glance: one row, name, export pill, a one-line description, a "!" count, the line range.
//   Read:   description, typed signature, algorithm, notes, a relations line, "Show code".
//   Dig:    plus the layer chip, every param with defaults and generics, all relation rows open.
// Calls, imports and callers sit behind one muted line of counts; each opens only its own row.
import { memo, useEffect, useState } from "react";
import type { FnCard } from "../ipc/bindings/FnCard";
import type { Impact } from "../ipc/bindings/Impact";
import { api } from "../ipc/api";
import type { SymInfo } from "../lib/cards";
import type { CardState, Target } from "../lib/focus";
import { lineRange, plural } from "../lib/format";
import type { Detail } from "../lib/prefs";
import { genericsOf, inlineText, layoutSig } from "../lib/sig";
import { CodeView } from "../editor/CodeView";
import type { CodeContext } from "../editor/EditorHost";
import { AiMark, Note } from "../components/ui";
import { usePeek } from "../components/Peek";
import { Signature, type TypeLinkHandlers } from "./Signature";

export type Rel = "calls" | "imports" | "callers";

export interface FunctionCardProps {
  card: FnCard;
  path: string;
  state: CardState;
  hidden: boolean;
  detail: Detail;
  codeOpen: boolean;
  syms: Map<string, SymInfo>;
  codeContext: CodeContext;
  links: TypeLinkHandlers;
  /** Relation rows the reader opened on this card (session only). */
  relOpen: ReadonlySet<Rel>;
  allParams: boolean;
  /** The call-flow step's line, when this card owns the flow being stepped. */
  stepLine: number | null;
  stepNonce: number;
  onToggleCode(name: string): void;
  onToggleRel(name: string, rel: Rel): void;
  onMoreParams(name: string): void;
  /** Header click: focus; ⌘-click opens the code and scrolls to it. */
  onHeader(name: string, cmd: boolean): void;
  /** Chip click: focus; ⌘-click goes to the definition. */
  onTarget(t: Target, cmd: boolean): void;
  /** Hovering a relation count previews it in the map. */
  onPreview(name: string | null): void;
  onOpenFile(path: string, line?: number | null): void;
}

const isCmd = (e: React.MouseEvent | React.KeyboardEvent) => e.metaKey || e.ctrlKey;

/** A long identifier wraps between its words (runAgent|Harness|…), never mid-word. */
function CamelName({ name }: { name: string }) {
  const parts = name.split(/(?=[A-Z])|(?<=[._])/);
  return (
    <>
      {parts.map((p, i) => (
        <span key={i}>
          {i > 0 && <wbr />}
          {p}
        </span>
      ))}
    </>
  );
}

/** First sentence of a doc comment, for the Glance row. */
function firstSentence(s: string): string {
  const m = s.match(/^(.+?[.!?])(\s|$)/);
  return m ? m[1] : s;
}

function FunctionCardImpl(p: FunctionCardProps) {
  const { card, state, detail, syms } = p;
  const peek = usePeek();
  const s = card.summary;
  const nLines = card.end - card.start + 1;
  const warns = s?.notes.filter((n) => n.kind === "warn") ?? [];
  const infos = s?.notes.filter((n) => n.kind === "info") ?? [];
  const desc = s?.desc ?? card.doc;
  const cls = [
    "iso-card",
    "card",
    `lvl-${detail}`,
    card.exported ? "iso-card-exported" : "",
    state === "sel" ? "iso-card-sel" : state === "dep" ? "iso-card-dep" : state === "user" ? "iso-card-user" : "",
    state === "dim" ? "dim" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const header = (
    <div
      className="card-h iso-card-h"
      role="button"
      tabIndex={0}
      aria-label={`Focus ${card.name} in the graph`}
      aria-pressed={state === "sel"}
      onClick={(e) => p.onHeader(card.name, isCmd(e))}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          p.onHeader(card.name, isCmd(e));
        }
      }}
    >
      {detail === "dig" && (
        <span className="iso-lvl" title={`Layer ${card.level}: longest call path to a leaf`}>
          L{card.level}
        </span>
      )}
      <code className="iso-fname">{detail === "glance" ? card.name : <CamelName name={card.name} />}</code>
      {card.exported && <span className="iso-pill">export</span>}
      {detail === "glance" && (
        <span className="glance-desc">{desc ? firstSentence(desc) : <code>{inlineText(layoutSig(card.sig, "read").params, layoutSig(card.sig, "read").returns)}</code>}</span>
      )}
      {detail === "glance" && warns.length > 0 && (
        <span className="warn-count" {...peek.bind(() => ({ kind: "text", title: plural(warns.length, "possible bug"), body: warns.map((w, i) => <p key={i}>{w.text}</p>) }))}>
          <span className="iso-note-ic" aria-hidden="true">
            !
          </span>
          {warns.length}
          <span className="sr-only"> {plural(warns.length, "possible bug")}</span>
        </span>
      )}
      <span className="iso-lines">{lineRange(card.start, card.end)}</span>
    </div>
  );

  if (detail === "glance")
    return (
      <article className={cls} id={`card-${card.name}`} data-card={card.name} hidden={p.hidden}>
        {header}
      </article>
    );

  const chipFn = (n: string) => (
    <button key={n} type="button" className="iso-chip" onClick={(e) => p.onTarget({ kind: "fn", id: n }, isCmd(e))}>
      {n}
    </button>
  );
  const chipX = (n: string, typeOnly: boolean) => {
    const info = syms.get(n);
    const target = p.links.idx.get(n) ?? null;
    return (
      <button
        key={(typeOnly ? "t:" : "") + n}
        type="button"
        className={`iso-chip iso-chip-ext${typeOnly ? " iso-chip-type" : ""}`}
        {...peek.bind(() => p.links.peekFor(n, target))}
        onClick={(e) => p.onTarget({ kind: "sym", id: n }, isCmd(e))}
      >
        {info?.label ?? n}
      </button>
    );
  };
  const nImports = card.uses.length + card.typeUses.length;
  const nCallers = card.calledBy.length;
  const open = (r: Rel) => detail === "dig" || p.relOpen.has(r);
  const count = (r: Rel, noun: string, target: string | null) => (
    <button
      type="button"
      className="rel-count"
      aria-expanded={open(r)}
      onClick={() => p.onToggleRel(card.name, r)}
      onPointerEnter={() => p.onPreview(target)}
      onPointerLeave={() => p.onPreview(null)}
    >
      {noun}
    </button>
  );
  const rels: React.ReactNode[] = [];
  rels.push(count("calls", plural(card.calls.length, "call"), card.name));
  if (nImports) rels.push(count("imports", plural(nImports, "import"), card.name));
  if (nCallers || card.externalCallers)
    rels.push(
      count(
        "callers",
        [nCallers ? plural(nCallers, "caller") : "", card.externalCallers ? `${plural(card.externalCallers, "caller")} in other files` : ""].filter(Boolean).join(" · "),
        card.name,
      ),
    );

  return (
    <article className={cls} id={`card-${card.name}`} data-card={card.name} hidden={p.hidden}>
      {header}
      {desc && (
        <p className="iso-desc">
          {desc}
          {s?.ai && <AiMark model={s.model} />}
        </p>
      )}
      <Signature
        sig={card.sig}
        detail={detail}
        links={p.links}
        generics={genericsOf(card.signature, card.name)}
        allParams={p.allParams}
        onMoreParams={() => p.onMoreParams(card.name)}
      />
      {s && s.algo.length > 0 && (
        <ol className="iso-algo">
          {s.algo.map((a, i) => (
            <li key={i}>{a}</li>
          ))}
        </ol>
      )}
      {(warns.length > 0 || infos.length > 0) && (
        <ul className="notes">
          {[...warns, ...infos].map((n, i) => (
            <Note key={i} kind={n.kind}>
              {n.text}
            </Note>
          ))}
        </ul>
      )}
      <div className="rel-line">
        {rels.map((r, i) => (
          <span key={i}>
            {i > 0 && <span className="sep"> · </span>}
            {r}
          </span>
        ))}
      </div>
      {(open("calls") || open("imports") || open("callers")) && (
        <dl className="iso-deps">
          {open("calls") && (
            <>
              <dt>Calls</dt>
              <dd>{card.calls.length ? card.calls.map(chipFn) : <span className="none">none</span>}</dd>
            </>
          )}
          {open("imports") && nImports > 0 && (
            <>
              <dt>Imports used</dt>
              <dd>{[...card.uses.map((n) => chipX(n, false)), ...card.typeUses.map((n) => chipX(n, true))]}</dd>
            </>
          )}
          {open("callers") && (nCallers > 0 || card.externalCallers > 0) && (
            <>
              <dt>Called by</dt>
              <dd>
                {card.calledBy.map(chipFn)}
                <ExternalCallers card={card} onOpenFile={p.onOpenFile} />
              </dd>
            </>
          )}
        </dl>
      )}
      <button type="button" className="iso-btn-reveal" aria-expanded={p.codeOpen} onClick={() => p.onToggleCode(card.name)}>
        {p.codeOpen ? "Hide code" : "Show code"} · {plural(nLines, "line")}
      </button>
      {p.codeOpen && <CodeView path={p.path} start={card.start} end={card.end} fit context={p.codeContext} stepLine={p.stepLine} stepNonce={p.stepNonce} ariaLabel={`Code of ${card.name}, lines ${card.start} to ${card.end}`} />}
    </article>
  );
}

/** "3 callers in other files": the list from get_impact, each opening its file. */
function ExternalCallers({ card, onOpenFile }: { card: FnCard; onOpenFile(path: string, line?: number | null): void }) {
  const [impact, setImpact] = useState<Impact | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (!card.externalCallers) return;
    let alive = true;
    api
      .getImpact(card.id, false)
      .then((i) => alive && setImpact(i))
      .catch((e: unknown) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [card.id, card.externalCallers]);
  if (!card.externalCallers) return null;
  return (
    <ul className="impact-list">
      {err && <li className="none">Could not load callers: {err}</li>}
      {!impact && !err && <li className="none">Loading callers in other files…</li>}
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
  );
}

export const FunctionCard = memo(FunctionCardImpl);
