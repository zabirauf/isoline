// Typed signature (iso-sig, spec section 1). Each name inside a type is colored by where it is
// defined (FileView.targets): this file in accent, repo imports in ext on ext-soft, packages in
// ext with an outline, built-ins muted, unresolved names plain. Defined names are buttons: hover
// for the definition peek, click to go there. Nothing is underlined at rest.
import { Fragment, memo } from "react";
import type { SigInfo } from "../ipc/bindings/SigInfo";
import type { SymbolTarget } from "../ipc/bindings/SymbolTarget";
import { cutTokens, layoutSig, tokenizeType, TYPE_MAX, type ParamView, type TargetIndex } from "../lib/sig";
import type { Detail } from "../lib/prefs";
import { usePeek, type PeekSubject } from "../components/Peek";

export interface TypeLinkHandlers {
  idx: TargetIndex;
  /** Build the peek for a name (null for no peek). */
  peekFor(name: string, target: SymbolTarget | null): PeekSubject | null;
  /** Click on a defined type: go to its definition, or pin the peek for packages. */
  onType(name: string, target: SymbolTarget, el: HTMLElement): void;
}

/** Type text with every name colored by origin, cut to `max` characters. */
export function TypeText({ text, links, max = Infinity }: { text: string; links: TypeLinkHandlers; max?: number }) {
  const peek = usePeek();
  const toks = cutTokens(tokenizeType(text, links.idx), max);
  return (
    <>
      {toks.map((t, i) => {
        if (t.kind === "name") {
          const o = t.origin ?? "unknown";
          const bind = peek.bind(() => links.peekFor(t.text, t.target ?? null));
          if (t.target && (o === "local" || o === "imported" || o === "external"))
            return (
              <button key={i} type="button" className={`ty ty-${o}`} {...bind} onClick={(e) => links.onType(t.text, t.target!, e.currentTarget)}>
                {t.text}
              </button>
            );
          return (
            <span key={i} className={`ty ty-${o}`} onPointerEnter={bind.onPointerEnter} onPointerLeave={bind.onPointerLeave}>
              {t.text}
            </span>
          );
        }
        if (t.kind === "punct") return <span key={i} className="ty-p">{t.text}</span>;
        return <Fragment key={i}>{t.text}</Fragment>;
      })}
    </>
  );
}

function ParamName({ p, peekText }: { p: ParamView; peekText: PeekSubject }) {
  const peek = usePeek();
  const bind = peek.bind(() => peekText);
  return (
    <span className="sig-name" onPointerEnter={bind.onPointerEnter} onPointerLeave={bind.onPointerLeave}>
      {p.rest && <span className="ty-p">...</span>}
      <span className={p.destructured ? "muted" : undefined}>{p.name}</span>
      {p.optional && <span className="ty-p">?</span>}
    </span>
  );
}

/** "= …" at Read; the full default is in the param's peek. */
function DefText({ p }: { p: ParamView }) {
  const peek = usePeek();
  const bind = peek.bind(() => paramPeek(p));
  return (
    <span className="sig-def" onPointerEnter={bind.onPointerEnter} onPointerLeave={bind.onPointerLeave}>
      {" "}= {p.def === "…" ? "…" : <code>{p.def}</code>}
    </span>
  );
}

function paramPeek(p: ParamView): PeekSubject {
  const full = p.param;
  return {
    kind: "text",
    title: full.name,
    body: (
      <>
        {full.typeText && <code className="peek-sig">{full.typeText}</code>}
        {full.default !== null && (
          <div>
            Default: <code>{full.default}</code>
          </div>
        )}
        {full.rest && <div className="muted">Rest parameter: collects the remaining arguments.</div>}
        {full.optional && full.default === null && <div className="muted">Optional.</div>}
      </>
    ),
  };
}

export interface SignatureProps {
  sig: SigInfo;
  detail: Detail;
  links: TypeLinkHandlers;
  /** Generic parameters, shown before the parens at Dig. */
  generics?: string | null;
  /** Show every parameter (the reader asked for "+ N more params"). */
  allParams?: boolean;
  onMoreParams?(): void;
  /** Force the inline form (the call-flow header). */
  inline?: boolean;
  /** Text before the parens in inline form (the function name in the flow header). */
  lead?: React.ReactNode;
  /** Inline form with parameter names only; each name's peek shows its type (flow header below Dig). */
  namesOnly?: boolean;
}

export const Signature = memo(function Signature({ sig, detail, links, generics, allParams, onMoreParams, inline, lead, namesOnly }: SignatureProps) {
  const view = layoutSig(sig, detail, { all: allParams });
  const mode = inline ? "inline" : view.mode;
  const gen = detail === "dig" && generics ? <span className="ty-p">{generics}</span> : null;
  const max = detail === "dig" ? Infinity : TYPE_MAX;
  const typeOf = (p: ParamView) =>
    p.typeFull ? (
      <span className="sig-type">
        <TypeText text={p.typeFull} links={links} max={max} />
      </span>
    ) : null;
  const def = (p: ParamView) => (p.def !== null ? <DefText p={p} /> : null);
  if (mode === "inline")
    return (
      <div className="iso-sig iso-sig-inline">
        {lead}
        {gen}
        <span className="ty-p">(</span>
        {view.params.map((p, i) => (
          <Fragment key={i}>
            {i > 0 && <span className="ty-p">, </span>}
            <ParamName p={p} peekText={paramPeek(p)} />
            {!namesOnly && p.type && <span className="ty-p">: </span>}
            {!namesOnly && typeOf(p)}
            {!namesOnly && def(p)}
          </Fragment>
        ))}
        <span className="ty-p">)</span>
        {view.returns && (
          <>
            {" "}
            <span className="ty-p">→</span> <TypeText text={view.returnsFull ?? view.returns} links={links} max={max} />
          </>
        )}
      </div>
    );
  return (
    <div className="iso-sig iso-sig-stacked">
      {gen && <div className="sig-gen">{gen}</div>}
      <div className="sig-grid">
        {view.params.map((p, i) => (
          <Fragment key={i}>
            <ParamName p={p} peekText={paramPeek(p)} />
            <span>
              {typeOf(p)}
              {def(p)}
            </span>
          </Fragment>
        ))}
      </div>
      {view.hidden > 0 && (
        <button type="button" className="iso-btn-reveal sig-more" onClick={onMoreParams}>
          + {view.hidden} more param{view.hidden === 1 ? "" : "s"}
        </button>
      )}
      {view.returns && (
        <div className="sig-ret">
          <span className="ty-p">→</span> <TypeText text={view.returnsFull ?? view.returns} links={links} max={max} />
        </div>
      )}
    </div>
  );
});
