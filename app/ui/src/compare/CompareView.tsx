// Compare: what changed between two revisions (or in a pull request), in the map's terms. Which
// modules and dependencies changed (the change graph), which deleted or re-signatured functions
// unchanged files still call, every changed file with its symbols, and a side-by-side diff.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Comparison } from "../ipc/bindings/Comparison";
import type { GitRefs } from "../ipc/bindings/GitRefs";
import type { GitStatus } from "../ipc/bindings/GitStatus";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import type { PullRequest } from "../ipc/bindings/PullRequest";
import { api } from "../ipc/api";
import { basename, fmtAgo, fmtInt, fmtMs, plural } from "../lib/format";
import { Seg } from "../components/ui";
import { defaultBase, defaultHead, refLabel, WORKTREE } from "../lib/refs";
import { usePeek } from "../components/Peek";
import { Splitter, initialSplit } from "../file/Splitter";
import { PrPicker, RefPicker } from "./Pickers";
import { ChangeGraphLegend, ChangeGraphView } from "./ChangeGraphView";
import { FileList } from "./FileList";
import { DiffPane, type RevealRequest } from "./DiffPane";

/** What the branch menu asks for: compare `base` with `head` (null: the defaults). */
export interface CompareRequest {
  base: string | null;
  head: string | null;
  nonce: number;
}

const CHANGE_WORD = { added: "added", deleted: "deleted", modified: "signature changed", renamed: "renamed" } as const;

export function CompareView({ status, moduleMap, request, onOpenFile }: { status: GitStatus | null; moduleMap: ModuleMap | null; request: CompareRequest; onOpenFile(path: string, line?: number | null): void }) {
  const [refs, setRefs] = useState<GitRefs | null>(null);
  const [base, setBase] = useState<string | null>(request.base);
  const [head, setHead] = useState<string | null>(request.head);
  const [threeDot, setThreeDot] = useState(true);
  const [pr, setPr] = useState<PullRequest | null>(null);
  const [result, setResult] = useState<Comparison | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [reveal, setReveal] = useState<RevealRequest | null>(null);
  const [moduleFilter, setModuleFilter] = useState<string | null>(null);
  /** The right pane shows the change graph or one file's diff. */
  const [pane, setPane] = useState<"graph" | "diff">("graph");
  const [split] = useState(initialSplit);
  const mainRef = useRef<HTMLDivElement>(null);
  const seq = useRef(0);
  const peek = usePeek();

  // Revisions to offer; reloaded when HEAD moves.
  useEffect(() => {
    void api.gitRefs().then(setRefs).catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [status?.branch, status?.head]);

  // A new request from the branch menu replaces the sides; null sides take the defaults.
  useEffect(() => {
    setPr(null);
    setBase(request.base);
    setHead(request.head);
  }, [request.nonce]);
  useEffect(() => {
    if (!refs) return;
    const b = base ?? defaultBase(refs, status);
    if (base === null && b) setBase(b);
    if (head === null) setHead(defaultHead(refs, status, b));
  }, [refs, base, head, status]);

  const run = useCallback(() => {
    if (!pr && (!base || !head)) return;
    const id = ++seq.current;
    setLoading(true);
    setError(null);
    api
      .compare(pr ? "" : base!, pr ? "" : head!, threeDot, pr?.number)
      .then((r) => {
        if (id !== seq.current) return;
        setResult(r);
        setPane("graph");
        setSelected((cur) => (cur && r.files.some((f) => f.path === cur) ? cur : (r.files.find((f) => f.symbols.length > 0) ?? r.files[0])?.path ?? null));
        setModuleFilter((m) => (m && r.modules.some((x) => x.id === m) ? m : null));
      })
      .catch((e: unknown) => id === seq.current && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => id === seq.current && setLoading(false));
  }, [base, head, threeDot, pr]);
  useEffect(run, [run]);

  const info = useMemo(() => {
    const m = new Map((moduleMap?.modules ?? []).map((x) => [x.id, { name: x.name, color: x.color }]));
    return (id: string) => m.get(id) ?? null;
  }, [moduleMap]);
  const file = result?.files.find((f) => f.path === selected) ?? null;
  const modName = moduleFilter ? (result?.modules.find((m) => m.id === moduleFilter)?.name ?? info(moduleFilter)?.name ?? moduleFilter) : null;
  // The reader shows the working tree: offer "Open file" when the head side is what is on disk.
  const canOpen = !!result && (result.head.sha === WORKTREE || (!pr && head === status?.branch));
  const t = result?.totals;
  const sameSide = !pr && base !== null && base === head;

  return (
    <div className="compare-page">
      <header className="top cmp-top">
        <div className="cmp-title">
          <h1>Compare</h1>
          {pr ? (
            <div className="cmp-pr">
              <a className="link" href={pr.url} target="_blank" rel="noreferrer">
                #{pr.number}
              </a>
              <span className="cmp-pr-title">{pr.title}</span>
              <span className="caption">
                {pr.author} · <code>{pr.headRef}</code> into <code>{pr.baseRef}</code> · updated {fmtAgo(pr.updated)}
              </span>
              <button type="button" className="iso-btn quiet" onClick={() => setPr(null)}>
                Back to branches
              </button>
            </div>
          ) : (
            <div className="cmp-refs">
              <RefPicker role="Base" value={base} refs={refs} onPick={setBase} allowWorktree={false} />
              <span className="cmp-arrow" aria-hidden="true">
                ←
              </span>
              <RefPicker role="Head" value={head} refs={refs} onPick={setHead} allowWorktree />
              <button
                type="button"
                className="iso-btn quiet"
                disabled={!base || !head || head === WORKTREE}
                title={head === WORKTREE ? "The working tree can only be the head side" : "Swap base and head"}
                onClick={() => {
                  setBase(head);
                  setHead(base);
                }}
              >
                Swap
              </button>
              <button
                type="button"
                className="iso-btn"
                aria-pressed={threeDot}
                onClick={() => setThreeDot(!threeDot)}
                {...peek.bind(() => ({
                  kind: "text",
                  title: "From merge base",
                  body: threeDot ? "On: only what the head side changed since it branched off the base, as a pull request shows it." : "Off: the two tips compared directly, so changes made on the base side since then show too, reversed.",
                }))}
              >
                From merge base
              </button>
            </div>
          )}
          <span className="cmp-spacer" />
          <PrPicker value={pr} github={!!status?.github} onPick={setPr} />
        </div>
        <div className="file cmp-sum" aria-live="polite">
          {loading ? (
            <span className="cmp-loading">
              Comparing {pr ? `#${pr.number}` : `${refLabel(base ?? "", refs)} and ${refLabel(head ?? "", refs)}`}…<span className="pbar indet"><span /></span>
            </span>
          ) : error ? (
            <span className="err">{error}</span>
          ) : sameSide ? (
            <span>Pick two different revisions.</span>
          ) : result && t ? (
            <Summary result={result} />
          ) : null}
        </div>
      </header>
      {result && !error && (
        <div className={`main cmp-main${loading ? " stale" : ""}`} ref={mainRef} style={{ ["--left" as string]: split + "%" }}>
          <div className="pane-left">
            <div className="inner cmp-inner">
              {result.files.length === 0 ? (
                <div className="muted">
                  <p>No differences{result.mergeBase && threeDot ? " since the merge base" : ""}.</p>
                  {threeDot && result.mergeBase && result.head.sha === result.mergeBase && result.base.sha !== result.mergeBase && (
                    <p>
                      {result.head.label} is behind {result.base.label}: it has no commits of its own.{" "}
                      <button type="button" className="link-quiet" onClick={() => setThreeDot(false)}>
                        Compare the two tips directly
                      </button>
                    </p>
                  )}
                </div>
              ) : (
                <>
                  {result.impact.length > 0 && (
                    <section className="panel impact-panel" aria-label="Still called">
                      <div className="cmp-ph">
                        <div className="display-heading">Still called</div>
                        <span className="caption">deleted or changed functions that unchanged files call</span>
                      </div>
                      <ul className="impact">
                        {result.impact.map((it) => (
                          <li key={it.path + it.symbol}>
                            <div className="impact-h">
                              <button
                                type="button"
                                className="cs-name"
                                onClick={() => {
                                  setSelected(it.path);
                                  setPane("diff");
                                  setReveal({ path: it.path, symbol: it.symbol, nonce: Date.now() });
                                }}
                              >
                                <code>{it.symbol}</code>
                              </button>
                              <span
                                className={`cs-chip c-${it.change}`}
                                tabIndex={0}
                                {...peek.bind(() =>
                                  it.signatureBefore || it.signatureAfter
                                    ? {
                                        kind: "text",
                                        title: it.change === "deleted" ? "Deleted" : "Signature changed",
                                        body: (
                                          <div className="sig-diff">
                                            {it.signatureBefore && <code className="c-del">{it.signatureBefore}</code>}
                                            {it.signatureAfter && <code className="c-add">{it.signatureAfter}</code>}
                                          </div>
                                        ),
                                      }
                                    : null,
                                )}
                              >
                                {CHANGE_WORD[it.change]}
                              </span>
                            </div>
                            <ImpactCallers callers={it.callers} total={it.callerFiles} onOpenFile={onOpenFile} />
                          </li>
                        ))}
                      </ul>
                    </section>
                  )}
                  {result.commits.length > 0 && (
                    <details className="panel commits-panel">
                      <summary>
                        <span className="display-heading">Commits</span> <span className="caption">{plural(result.commits.length, "commit")}{result.commits.length >= 200 ? " (the newest 200)" : ""}</span>
                      </summary>
                      <ul className="commits">
                        {result.commits.map((c) => (
                          <li key={c.sha}>
                            <code className="muted">{c.short}</code>
                            <span className="commit-subj">{c.subject}</span>
                            <span className="caption">
                              {c.author} · {fmtAgo(c.date)}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </details>
                  )}
                  <FileList
                    files={result.files}
                    selected={selected}
                    module={moduleFilter ? { id: moduleFilter, name: modName ?? moduleFilter } : null}
                    onSelect={(p) => {
                      setSelected(p);
                      setPane("diff");
                    }}
                    onReveal={(path, symbol) => {
                      setSelected(path);
                      setPane("diff");
                      setReveal({ path, symbol, nonce: Date.now() });
                    }}
                    onOpenFile={(p) => onOpenFile(p)}
                    onClearModule={() => setModuleFilter(null)}
                  />
                  {result.truncated && <p className="caption">This comparison is large: symbols were compared for the first files only.</p>}
                </>
              )}
            </div>
          </div>
          <Splitter target={mainRef} />
          <div className="pane-right">
            {result.files.length > 0 && (
              <div className="cmp-tabs">
                <Seg
                  label="Right pane"
                  value={pane}
                  onChange={setPane}
                  options={[
                    ["graph", "Change graph"],
                    ["diff", file ? `Diff · ${basename(file.path)}` : "Diff"],
                  ]}
                />
                {pane === "graph" && (
                  <span className="caption">
                    {plural(result.modules.length, "module")} changed · {plural(result.moduleEdges.length, "dependency", "dependencies")} changed{moduleFilter ? " · click the module again to show every file" : " · click a module to show its files"}
                  </span>
                )}
              </div>
            )}
            {pane === "graph" && result.files.length > 0 ? (
              <div className="cg-pane" aria-label="Change graph">
                <ChangeGraphView modules={result.modules} edges={result.moduleEdges} info={info} selected={moduleFilter} onSelect={setModuleFilter} />
                <ChangeGraphLegend />
              </div>
            ) : (
              <DiffPane base={result.mergeBase ?? result.base.sha} head={result.head.sha} file={file} reveal={reveal} onOpenFile={(p, l) => onOpenFile(p, l)} canOpen={canOpen} />
            )}
          </div>
        </div>
      )}
      {!result && !error && !loading && !refs && <div className="loading muted">Reading branches…</div>}
    </div>
  );
}

const CALLERS_SHOWN = 8;

/** The first callers inline; the rest (up to the 50 the backend lists) on request. */
function ImpactCallers({ callers, total, onOpenFile }: { callers: { path: string; line: number | null }[]; total: number; onOpenFile(path: string, line?: number | null): void }) {
  const [all, setAll] = useState(false);
  const shown = all ? callers : callers.slice(0, CALLERS_SHOWN);
  const hidden = callers.length - shown.length;
  const unlisted = Math.max(0, total - callers.length);
  return (
    <div className="impact-callers">
      <span className="caption">{total === 1 ? "1 unchanged file calls it:" : `${fmtInt(total)} unchanged files call it:`}</span>
      {shown.map((c) => (
        <button key={c.path + c.line} type="button" className="filelink" onClick={() => onOpenFile(c.path, c.line)}>
          {c.path}
          {c.line ? `:${c.line}` : ""}
        </button>
      ))}
      {hidden > 0 && (
        <button type="button" className="link-quiet" onClick={() => setAll(true)}>
          + {fmtInt(hidden + unlisted)} more
        </button>
      )}
      {hidden === 0 && unlisted > 0 && <span className="caption">and {plural(unlisted, "more file")}</span>}
    </div>
  );
}

function Summary({ result }: { result: Comparison }) {
  const t = result.totals;
  const kinds = [t.added && `${fmtInt(t.added)} added`, t.modified && `${fmtInt(t.modified)} modified`, t.deleted && `${fmtInt(t.deleted)} deleted`, t.renamed && `${fmtInt(t.renamed)} renamed`].filter(Boolean);
  const syms = [t.symbolsAdded && `${fmtInt(t.symbolsAdded)} added`, t.symbolsDeleted && `${fmtInt(t.symbolsDeleted)} deleted`, t.symbolsModified && `${fmtInt(t.symbolsModified)} modified`].filter(Boolean);
  // Each fact is one unbreakable item, so a narrow window wraps between facts, never inside one.
  const items: [string, ReactNode][] = [
    [
      "sides",
      <>
        <b>{result.base.label}</b> ← <b>{result.head.label}</b>
      </>,
    ],
    ["mb", result.mergeBase ? <span className="muted">from merge base {result.mergeBase.slice(0, 7)}</span> : null],
    [
      "lines",
      <>
        <span className="c-add">+{fmtInt(t.additions)}</span> <span className="c-del">−{fmtInt(t.deletions)}</span>
      </>,
    ],
    ["files", `${plural(t.files, "file")}${kinds.length ? ` (${kinds.join(", ")})` : ""}`],
    ["syms", syms.length ? `symbols ${syms.join(", ")}` : null],
    ["sigs", t.signaturesChanged ? `${plural(t.signaturesChanged, "signature")} changed` : null],
    ["imps", t.importsAdded || t.importsDeleted ? `imports +${fmtInt(t.importsAdded)} −${fmtInt(t.importsDeleted)}` : null],
    ["commits", result.commits.length ? plural(result.commits.length, "commit") : null],
    ["ms", <span className="muted">{fmtMs(result.elapsedMs)}</span>],
  ];
  return (
    <>
      {items
        .filter(([, v]) => v !== null)
        .map(([k, v]) => (
          <span key={k} className="sum-item">
            {v}
          </span>
        ))}
    </>
  );
}
