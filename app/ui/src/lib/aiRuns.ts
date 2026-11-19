// AI runs as the UI sees them: one run at a time, its progress, and what the open file's
// summary line says. Once AI is on for a repo, a file that opens without summaries is
// summarized when the run is small (AUTO_MAX_USD); a bigger one is offered with its estimate.
import { useCallback, useEffect, useRef, useState } from "react";
import type { AiEstimate } from "../ipc/bindings/AiEstimate";
import type { AiScope } from "../ipc/bindings/AiScope";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import type { FileView } from "../ipc/bindings/FileView";
import { api, type AiJobDone } from "../ipc/api";
import { getPrefs, type Prefs } from "./prefs";

/** What a run covers, in words. */
export const SCOPE_TEXT: Record<AiScope, string> = { file: "this file", module: "this module", repo: "the repo", architecture: "the architecture" };

/** The largest estimate a file is summarized at without asking. */
export const AUTO_MAX_USD = 0.05;

/** A run of this estimate asks first: always when costs are confirmed, else over the limit. */
export const asksFirst = (costUsd: number, p: Prefs = getPrefs()) => p.confirmCost || costUsd > p.askAboveUsd;

/** The largest estimate a file is summarized at when it opens, without asking. */
export const autoMaxUsd = (p: Prefs = getPrefs()) => (p.confirmCost ? AUTO_MAX_USD : Math.max(AUTO_MAX_USD, p.askAboveUsd));

/** The budget a run gets: twice the estimate, and at least five cents. */
export const budgetFor = (est: AiEstimate) => Math.max(0.05, est.costUsd * 2);

export interface AiJob {
  jobId: number | null;
  scope: AiScope;
  path: string | null;
  done: number;
  total: number;
  costUsd: number;
  finished: boolean;
  error: string | null;
  stored: number;
}

/** What the open file's summary line shows. */
export type FileAi =
  | { kind: "none" }
  | { kind: "off" }
  | { kind: "checking" }
  | { kind: "running"; job: AiJob }
  | { kind: "waiting"; job: AiJob }
  | { kind: "offer"; est: AiEstimate }
  | { kind: "failed"; error: string };

type Check = { path: string } & ({ kind: "checking" } | { kind: "none" } | { kind: "offer"; est: AiEstimate } | { kind: "failed"; error: string });

export function fileAiState(view: FileView | null, status: AiStatus | null, job: AiJob | null, check: Check | null): FileAi {
  if (!view || view.tier === "text") return { kind: "none" };
  if (!status?.signedIn || !status.enabledForRepo) return view.overview ? { kind: "none" } : { kind: "off" };
  // Module and repo runs summarize this file too; an architecture run does not.
  if (job && !job.finished) return (job.scope === "file" && job.path === view.path) || job.scope === "module" || job.scope === "repo" ? { kind: "running", job } : { kind: "waiting", job };
  if (job?.finished && job.error && job.scope === "file" && job.path === view.path) return { kind: "failed", error: job.error };
  if (check?.path === view.path) {
    if (check.kind === "checking") return { kind: "checking" };
    if (check.kind === "offer") return { kind: "offer", est: check.est };
    if (check.kind === "failed") return { kind: "failed", error: check.error };
  }
  return { kind: "none" };
}

export function useAiRuns({ status, view, repoRoot, auto }: { status: AiStatus | null; view: FileView | null; repoRoot: string | null; auto: boolean }) {
  const [job, setJob] = useState<AiJob | null>(null);
  const [check, setCheck] = useState<Check | null>(null);
  const jobRef = useRef<AiJob | null>(null);
  jobRef.current = job;
  // Events can arrive before aiRun resolves with the job id, so the latest of each is kept.
  const progressOf = useRef(new Map<number, { done: number; total: number; costUsd: number }>());
  const doneOf = useRef(new Map<number, AiJobDone>());
  const checked = useRef(new Set<string>());

  const apply = useCallback((j: AiJob): AiJob => {
    if (j.jobId == null) return j;
    const p = progressOf.current.get(j.jobId);
    const d = doneOf.current.get(j.jobId);
    return { ...j, ...(p ?? {}), ...(d ? { finished: true, error: d.error, stored: d.stored, costUsd: d.costUsd } : {}) };
  }, []);

  useEffect(() => {
    const offs = [
      api.listen("ai_progress", (p) => {
        progressOf.current.set(p.jobId, { done: p.done, total: p.total, costUsd: p.costUsd });
        setJob((j) => (j && j.jobId === p.jobId && !j.finished ? apply(j) : j));
      }),
      api.listen("ai_job_done", (d) => {
        doneOf.current.set(d.jobId, d);
        setJob((j) => (j && j.jobId === d.jobId ? apply(j) : j));
      }),
    ];
    return () => offs.forEach((p) => void p.then((off) => off()));
  }, [apply]);

  const on = !!status?.signedIn && !!status.enabledForRepo;
  useEffect(() => {
    checked.current.clear();
    setCheck(null);
  }, [repoRoot, on]);

  const run = useCallback(
    async (scope: AiScope, path: string | null, budgetUsd: number) => {
      if (jobRef.current && !jobRef.current.finished) throw new Error("Another AI run is still going.");
      const pending: AiJob = { jobId: null, scope, path, done: 0, total: 0, costUsd: 0, finished: false, error: null, stored: 0 };
      setJob(pending);
      try {
        const h = await api.aiRun(scope, path ?? undefined, budgetUsd);
        setJob(apply({ ...pending, jobId: h.jobId, total: h.estimate.symbols - h.estimate.cached }));
      } catch (e) {
        setJob({ ...pending, finished: true, error: e instanceof Error ? e.message : String(e) });
      }
    },
    [apply],
  );

  /** Estimate the open file and summarize it when that is cheap, or offer it. */
  const consider = useCallback(
    async (path: string, force: boolean) => {
      setCheck({ path, kind: "checking" });
      try {
        const est = await api.aiEstimate("file", path);
        if (est.requests === 0) setCheck({ path, kind: "none" });
        else if (force || (auto && est.costUsd <= autoMaxUsd())) {
          setCheck({ path, kind: "none" });
          await run("file", path, budgetFor(est));
        } else setCheck({ path, kind: "offer", est });
      } catch (e) {
        setCheck({ path, kind: "failed", error: e instanceof Error ? e.message : String(e) });
      }
    },
    [auto, run],
  );

  const busy = !!job && !job.finished;
  const path = view && view.tier !== "text" ? view.path : null;
  useEffect(() => {
    if (!on || !path || busy || checked.current.has(path)) return;
    checked.current.add(path);
    void consider(path, false);
  }, [on, path, busy, consider]);

  return {
    job,
    fileAi: fileAiState(view, status, job, check),
    run,
    /** Summarize the open file now (the line's button, or Try again). */
    summarizeFile: () => path && !busy && void consider(path, true),
  };
}
