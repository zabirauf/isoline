// The one line about summaries at the top of a file: off, being written (with progress),
// waiting for another run, offered with its cost, or failed.
import type { FileAi } from "../lib/aiRuns";
import { fmtUsd, plural } from "../lib/format";
import { setPrefs } from "../lib/prefs";

const scopeText = { file: "this file", module: "this module", repo: "the repo" } as const;

export function AiLine({ ai, hidden, onOpenAi, onSummarize, id }: { ai: FileAi; hidden: boolean; onOpenAi(): void; onSummarize(): void; id?: string }) {
  if (ai.kind === "none" || (ai.kind === "off" && hidden)) return null;
  return (
    <div className="ai-line" id={id} role={ai.kind === "running" || ai.kind === "checking" ? "status" : undefined}>
      {ai.kind === "off" && (
        <>
          <span className="muted">Summaries are off. Cards show signatures and docs.</span>
          <button type="button" className="link-quiet accentish" onClick={onOpenAi}>
            Turn on AI…
          </button>
          <button type="button" className="link-quiet" onClick={() => setPrefs({ aiNoticeHidden: true })}>
            Hide
          </button>
        </>
      )}
      {ai.kind === "checking" && (
        <span className="muted">
          <span className="spinner" aria-hidden="true" /> Checking what this file needs…
        </span>
      )}
      {ai.kind === "running" && (
        <>
          <span className="muted">
            <span className="spinner" aria-hidden="true" /> Writing summaries for {scopeText[ai.job.scope]}
            {ai.job.total > 0 ? ` · ${ai.job.done} / ${ai.job.total}` : "…"}
            {ai.job.costUsd > 0 ? ` · ${fmtUsd(ai.job.costUsd)} so far` : ""}
          </span>
          {ai.job.total > 0 && (
            <span className="ai-bar" aria-hidden="true">
              <span style={{ width: `${(ai.job.done / Math.max(1, ai.job.total)) * 100}%` }} />
            </span>
          )}
        </>
      )}
      {ai.kind === "waiting" && (
        <>
          <span className="muted">
            No summaries yet. This file is next, after the run for {ai.job.scope === "file" ? ai.job.path : scopeText[ai.job.scope]}.
          </span>
        </>
      )}
      {ai.kind === "offer" && (
        <>
          <span className="muted">
            No summaries yet: {plural(ai.est.symbols - ai.est.cached, "item")} to summarize.
          </span>
          <button type="button" className="link-quiet accentish" onClick={onSummarize}>
            Summarize · about {fmtUsd(ai.est.costUsd)}
          </button>
        </>
      )}
      {ai.kind === "failed" && (
        <>
          <span className="err">Summaries failed: {ai.error}</span>
          <button type="button" className="link-quiet accentish" onClick={onSummarize}>
            Try again
          </button>
        </>
      )}
    </div>
  );
}
