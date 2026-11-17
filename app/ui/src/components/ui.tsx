// Small presentational pieces shared by every view, built from the design system's classes.
import type { ReactNode } from "react";
import type { Tier } from "../ipc/bindings/Tier";
import { plural } from "../lib/format";

export function StatPill({ n, noun, pluralForm }: { n: number; noun: string; pluralForm?: string }) {
  const text = plural(n, noun, pluralForm);
  const sp = text.indexOf(" ");
  return (
    <span className="iso-stat">
      <b>{text.slice(0, sp)}</b>
      {text.slice(sp)}
    </span>
  );
}

export function Seg<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: [T, string, string?][]; onChange: (v: T) => void }) {
  return (
    <div className="iso-seg" role="group" aria-label={label}>
      {options.map(([v, text, title]) => (
        <button key={v} type="button" aria-pressed={value === v} title={title} onClick={() => onChange(v)}>
          {text}
        </button>
      ))}
    </div>
  );
}

/** Marks text written by a model. Model text is a suggestion, so the marker says where it came from. */
export function AiMark({ model }: { model: string }) {
  return (
    <span className="ai-mark" title={`Written by a model (${model}). It can be wrong; the code is the source of truth.`} aria-label={`Written by a model, ${model}`}>
      AI
    </span>
  );
}

/** Plain text with `backticks` set in mono, as model overviews write identifiers. */
export function RichText({ text }: { text: string }) {
  const parts = text.split(/(`[^`]+`)/g);
  return <>{parts.map((p, i) => (p.startsWith("`") && p.endsWith("`") && p.length > 2 ? <code key={i}>{p.slice(1, -1)}</code> : p))}</>;
}

const TIER_WORD: Record<Tier, string> = { full: "full", outline: "outline", text: "text" };
export const TIER_HELP: Record<Tier, string> = {
  full: "Full support: function cards, dependency map, call flow and summaries.",
  outline: "Outline: symbols and imports from the grammar's tags query. No call flow; edges are heuristic.",
  text: "Text: highlighting, search and a file summary. The file is not parsed.",
};

export function TierBadge({ tier }: { tier: Tier }) {
  return (
    <span className={`tier-badge tier-${tier}`} title={TIER_HELP[tier]}>
      {TIER_WORD[tier]}
    </span>
  );
}

export function ModuleDot({ color, title }: { color: string | null; title?: string }) {
  if (!color) return <span className="mdot mdot-none" aria-hidden="true" />;
  return <span className="mdot" style={{ ["--mc" as string]: color }} title={title} aria-hidden={title ? undefined : true} />;
}

/** A repo path that wraps only after a "/" (never mid-name), with the full path on hover. */
export function FilePath({ path }: { path: string }) {
  const parts = path.split("/");
  return (
    <span className="fpath" title={path}>
      {parts.map((p, i) => (
        <span key={i}>
          {p}
          {i < parts.length - 1 && (
            <>
              /<wbr />
            </>
          )}
        </span>
      ))}
    </span>
  );
}

export function Eyebrow({ children, id }: { children: ReactNode; id?: string }) {
  return (
    <div className="eyebrow" id={id}>
      {children}
    </div>
  );
}

export function Note({ kind, children }: { kind: "info" | "warn"; children: ReactNode }) {
  return (
    <li className={`iso-note${kind === "warn" ? " iso-note-warn" : ""}`}>
      <span className="iso-note-ic" aria-hidden="true">
        {kind === "warn" ? "!" : "i"}
      </span>
      <span>{children}</span>
    </li>
  );
}
