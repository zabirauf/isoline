// The top of the compare view, for a reviewer: what the change does and what to look at (one AI
// request, cached per version of the change), the change module by module, and the new
// dependencies: module links that appear or go, and packages the change starts or stops using.
import { useEffect, useMemo, useRef, useState } from "react";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import type { ChangeReview } from "../ipc/bindings/ChangeReview";
import type { Comparison } from "../ipc/bindings/Comparison";
import type { ReviewKind } from "../ipc/bindings/ReviewKind";
import type { ReviewState } from "../ipc/bindings/ReviewState";
import { api } from "../ipc/api";
import { basename, fmtInt, fmtUsd, plural } from "../lib/format";
import { AiMark, ModuleDot } from "../components/ui";

type ModInfo = (id: string) => { name: string; color: string } | null;

const KIND_TEXT: Record<ReviewKind, string> = { risk: "Risk", check: "Check", info: "Note" };
/** Budget for a review: twice the estimate, and never under five cents. */
const reviewBudget = (cost: number) => Math.max(0.05, cost * 2);

/** What the change does and what the reviewer must look at, written by a model on request. */
export function ReviewPanel({ result, ai, onOpenAi, onReveal, review, setReview }: { result: Comparison; ai: AiStatus | null; onOpenAi(): void; onReveal(path: string, symbol: string | null): void; review: ChangeReview | null; setReview(r: ChangeReview | null): void }) {
  const on = !!ai?.signedIn && !!ai.enabledForRepo;
  const [state, setState] = useState<ReviewState | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const seq = useRef(0);

  // A new comparison: show its cached review, else what one costs. Never calls a model.
  useEffect(() => {
    const id = ++seq.current;
    setState(null);
    setErr(null);
    setReview(null);
    if (!on || result.files.length === 0) return;
    api
      .aiReview(result)
      .then((s) => {
        if (id !== seq.current) return;
        setState(s);
        setReview(s.review);
      })
      .catch((e: unknown) => id === seq.current && setErr(e instanceof Error ? e.message : String(e)));
  }, [result, on]);

  const write = async () => {
    if (!state) return;
    const id = seq.current;
    setBusy(true);
    setErr(null);
    try {
      const r = await api.aiReviewRun(result, reviewBudget(state.costUsd));
      if (id === seq.current) setReview(r);
    } catch (e) {
      if (id === seq.current) setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  if (result.files.length === 0) return null;
  return (
    <section className="panel review-panel" aria-labelledby="review-h" aria-busy={busy}>
      <div className="cmp-ph">
        <div className="display-heading" id="review-h">
          Review
        </div>
        <span className="caption">what the change does and what to look at</span>
      </div>
      {review ? (
        <>
          <p className="review-sum">
            {review.summary}
            <AiMark model={review.model} />
          </p>
          {review.notes.length > 0 && (
            <ul className="review-notes">
              {review.notes.map((n, i) => (
                <li key={i} className={`rn rn-${n.kind}`}>
                  <span className="rn-kind">{KIND_TEXT[n.kind]}</span>
                  <span className="rn-text">
                    {n.text}
                    {n.path && (
                      <>
                        {" "}
                        <button type="button" className="filelink" onClick={() => onReveal(n.path!, n.symbol)} title={`Show ${n.symbol ?? "the change"} in the diff of ${n.path}`}>
                          {basename(n.path)}
                          {n.symbol ? ` · ${n.symbol}` : ""}
                        </button>
                      </>
                    )}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {review.filesWithoutPatch > 0 && <p className="caption">{plural(review.filesWithoutPatch, "file")} had diffs too large to send; the review saw only their changed symbols.</p>}
        </>
      ) : !on ? (
        <div className="review-actions">
          <span className="muted">A model can summarize this change, list the risks and checks to look at, and say what changed in each module.</span>
          <button type="button" className="link-quiet accentish" onClick={onOpenAi}>
            Turn on AI…
          </button>
        </div>
      ) : busy ? (
        <span className="muted" role="status">
          <span className="spinner" aria-hidden="true" /> Reading the change and writing the review…
        </span>
      ) : state ? (
        <div className="review-actions">
          <button type="button" className="iso-btn primary" onClick={() => void write()}>
            Write the review · about {fmtUsd(state.costUsd)}
          </button>
          <span className="caption">
            One request to {state.model}, about {fmtInt(Number(state.tokensIn))} tokens{state.filesWithoutPatch > 0 ? `; ${plural(state.filesWithoutPatch, "file")} too large to send in full` : ""}. Up to {fmtUsd(reviewBudget(state.costUsd))}.
          </span>
        </div>
      ) : (
        !err && (
          <span className="muted">
            <span className="spinner" aria-hidden="true" /> Checking for a review…
          </span>
        )
      )}
      {err && <span className="err">{err}</span>}
    </section>
  );
}

/** The change module by module, with the review's sentence for each when there is one. */
export function ModuleChanges({ result, info, review, selected, onSelect }: { result: Comparison; info: ModInfo; review: ChangeReview | null; selected: string | null; onSelect(id: string | null): void }) {
  const notes = useMemo(() => new Map((review?.modules ?? []).map((m) => [m.id, m.text])), [review]);
  const links = useMemo(() => {
    const m = new Map<string, { added: number; removed: number }>();
    const bump = (id: string, k: "added" | "removed") => {
      const v = m.get(id) ?? { added: 0, removed: 0 };
      v[k]++;
      m.set(id, v);
    };
    for (const e of result.moduleEdges) {
      if (e.before === 0) bump(e.from, "added");
      if (e.after === 0) bump(e.from, "removed");
    }
    return m;
  }, [result]);
  if (result.modules.length === 0) return null;
  const max = Math.max(1, ...result.modules.map((m) => m.additions + m.deletions));
  return (
    <section className="panel" aria-labelledby="bymod-h">
      <div className="cmp-ph">
        <div className="display-heading" id="bymod-h">
          By module
        </div>
        <span className="caption">{selected ? "the file list shows the selected module; click it again for every file" : "click a module to list only its files"}</span>
      </div>
      <ul className="bymod">
        {result.modules.map((m) => {
          const l = links.get(m.id);
          const color = info(m.id)?.color ?? m.color;
          return (
            <li key={m.id} className={m.id === selected ? "on" : ""}>
              <button type="button" className="bymod-h" aria-pressed={m.id === selected} onClick={() => onSelect(m.id === selected ? null : m.id)}>
                <ModuleDot color={color} />
                <b>{m.name}</b>
                <span className="caption">
                  {plural(m.files, "file")}
                  {m.symbols ? ` · ${plural(m.symbols, "symbol")}` : ""}
                  {l?.added ? ` · ${plural(l.added, "new link")}` : ""}
                  {l?.removed ? ` · ${plural(l.removed, "link")} removed` : ""}
                </span>
                <span className="bymod-n">
                  <span className="c-add">+{fmtInt(m.additions)}</span> <span className="c-del">−{fmtInt(m.deletions)}</span>
                </span>
                <span className="bymod-bar" aria-hidden="true">
                  <span className="a" style={{ width: `${(m.additions / max) * 100}%` }} />
                  <span className="d" style={{ width: `${(m.deletions / max) * 100}%` }} />
                </span>
              </button>
              {notes.get(m.id) && <p className="bymod-note">{notes.get(m.id)}</p>}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** Module links that appear, go or change, and packages the change starts or stops using. */
export function DependencyChanges({ result, info, onSelectModule, onOpenFileChange }: { result: Comparison; info: ModInfo; onSelectModule(id: string): void; onOpenFileChange(path: string): void }) {
  const name = (id: string) => result.modules.find((m) => m.id === id)?.name ?? info(id)?.name ?? id;
  const color = (id: string) => result.modules.find((m) => m.id === id)?.color ?? info(id)?.color ?? null;
  const added = result.moduleEdges.filter((e) => e.before === 0);
  const removed = result.moduleEdges.filter((e) => e.after === 0);
  const changed = result.moduleEdges.filter((e) => e.before > 0 && e.after > 0);
  const pkgs = result.packages ?? [];
  if (!result.moduleEdges.length && !pkgs.length) return null;
  const chip = (id: string) => (
    <button type="button" className="mchip" onClick={() => onSelectModule(id)}>
      <ModuleDot color={color(id)} />
      {name(id)}
    </button>
  );
  const pkgTag = (p: (typeof pkgs)[number]) =>
    p.change === "added" ? (p.elsewhere === 0 ? <span className="dep-tag new">new to the repo</span> : <span className="dep-tag">also in {plural(p.elsewhere, "other file")}</span>) : p.elsewhere === 0 ? <span className="dep-tag gone">no longer used</span> : <span className="dep-tag">still in {plural(p.elsewhere, "other file")}</span>;
  return (
    <section className="panel" aria-labelledby="deps-h">
      <div className="cmp-ph">
        <div className="display-heading" id="deps-h">
          Dependencies
        </div>
        <span className="caption">links between modules and packages that the change adds or removes</span>
      </div>
      {added.length + removed.length + changed.length > 0 && (
        <ul className="deplist">
          {[...added.map((e) => ["new", e] as const), ...removed.map((e) => ["gone", e] as const), ...changed.map((e) => ["changed", e] as const)].map(([k, e]) => (
            <li key={e.from + ">" + e.to}>
              <span className={`dep-kind ${k}`}>{k === "new" ? "New link" : k === "gone" ? "Removed" : "Changed"}</span>
              <span className="dep-link">
                {chip(e.from)}
                <span className="sep" aria-hidden="true">
                  →
                </span>
                {chip(e.to)}
              </span>
              <span className="caption">{k === "new" ? plural(e.after, "import") : k === "gone" ? `had ${plural(e.before, "import")}` : `${fmtInt(e.before)} → ${fmtInt(e.after)} imports`}</span>
            </li>
          ))}
        </ul>
      )}
      {pkgs.length > 0 && (
        <ul className="deplist">
          {pkgs.map((p) => (
            <li key={p.name + p.change}>
              <span className={`dep-kind ${p.change === "added" ? "new" : "gone"}`}>{p.change === "added" ? "Package" : "Dropped"}</span>
              <code className="dep-pkg">{p.name}</code>
              {pkgTag(p)}
              <span className="caption dep-files">
                {p.change === "added" ? "imported in " : "removed from "}
                {p.files.slice(0, 3).map((f, i) => (
                  <span key={f}>
                    {i > 0 && ", "}
                    <button type="button" className="filelink" onClick={() => onOpenFileChange(f)}>
                      {basename(f)}
                    </button>
                  </span>
                ))}
                {p.files.length > 3 ? ` and ${fmtInt(p.files.length - 3)} more` : ""}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
