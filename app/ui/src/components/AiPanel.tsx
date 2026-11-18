// AI status and sign-in. Summaries come from the reader's own Copilot account and are never
// requested until AI is turned on for the repo; the panel says what a run would cost first.
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { AiEstimate } from "../ipc/bindings/AiEstimate";
import type { AiScope } from "../ipc/bindings/AiScope";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import { AUTO_MAX_USD, SCOPE_TEXT as scopeText, budgetFor, type AiJob } from "../lib/aiRuns";
import { getPrefs, setPrefs, subscribePrefs } from "../lib/prefs";
import { api } from "../ipc/api";
import { SignInDialog } from "./SignInDialog";
import { Seg } from "./ui";
import { fmtInt, fmtUsd, num, plural } from "../lib/format";

export function aiLabel(s: AiStatus | null): string {
  if (!s) return "AI summaries";
  if (s.signedIn) return s.enabledForRepo ? "AI summaries on" : "Turn on AI for this repo…";
  if (s.deviceCode || s.sidecar === "starting") return "Signing in to AI…";
  if (s.sidecar === "failed") return "AI unavailable";
  return "Turn on AI…";
}

const SCOPES: { scope: AiScope; label: string; needsFile: boolean }[] = [
  { scope: "file", label: "This file", needsFile: true },
  { scope: "module", label: "Its module", needsFile: true },
  { scope: "architecture", label: "Architecture", needsFile: false },
  { scope: "repo", label: "Whole repo", needsFile: false },
];

export function AiPanel({ status, setStatus, filePath, job, onRun, openSignal = 0 }: { status: AiStatus | null; setStatus(s: AiStatus): void; filePath: string | null; job: AiJob | null; onRun(scope: AiScope, path: string | null, budgetUsd: number): Promise<void>; openSignal?: number }) {
  const prefs = useSyncExternalStore(subscribePrefs, getPrefs);
  const [scope, setScope] = useState<AiScope>("file");
  const [open, setOpen] = useState(false);
  // Signing in happens in a dialog; the popover holds what comes after.
  const [dialog, setDialog] = useState(false);
  const signedIn = !!status?.signedIn;
  useEffect(() => {
    if (!openSignal) return;
    if (signedIn) setOpen(true);
    else setDialog(true);
  }, [openSignal]);
  const [est, setEst] = useState<AiEstimate | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  // Poll while a device-flow sign-in is pending; there is no event for it.
  useEffect(() => {
    if (!status || status.signedIn || !status.deviceCode) return;
    const t = setInterval(() => void api.aiStatus().then(setStatus), 1000);
    return () => clearInterval(t);
  }, [status, setStatus]);

  useEffect(() => setEst(null), [filePath, scope]);
  const scopeShown: AiScope = !filePath && (scope === "file" || scope === "module") ? "architecture" : scope;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const run = async <T,>(f: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setErr(null);
    try {
      return await f();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const s = status;
  const running = !!job && !job.finished;
  return (
    <div className="ai" ref={box}>
      <button type="button" className={`iso-btn ai-btn${s?.signedIn && s.enabledForRepo ? " on" : ""}`} aria-expanded={open} aria-haspopup="dialog" onClick={() => (signedIn ? setOpen(!open) : setDialog(true))}>
        {aiLabel(s)}
        {running && job.total > 0 ? ` · ${job.done} / ${job.total}` : ""}
      </button>
      {open && s && (
        <div className="popover ai-pop" role="dialog" aria-label="AI summaries">
          <div className="eyebrow">AI summaries</div>
          {s.mock && <p className="caption">Mock backend: nothing leaves this machine.</p>}
          {!s.signedIn && (
            <button
              type="button"
              className="iso-btn"
              onClick={() => {
                setOpen(false);
                setDialog(true);
              }}
            >
              Sign in with GitHub…
            </button>
          )}
          {s.signedIn && (
            <>
              <p>
                Signed in as <code>{s.user}</code>. {plural(s.models.length, "model")} available.
              </p>
              <button type="button" className="iso-btn" aria-pressed={s.enabledForRepo} disabled={busy} onClick={() => run(() => api.aiEnable(!s.enabledForRepo).then(setStatus))}>
                Use AI for this repo
              </button>
              {s.enabledForRepo && (
                <div className="ai-file">
                  <label className="ai-check">
                    <input type="checkbox" checked={prefs.aiAuto} onChange={(e) => setPrefs({ aiAuto: e.target.checked })} />
                    Summarize files as I open them (up to {fmtUsd(AUTO_MAX_USD)} each)
                  </label>
                  <div className="caption">Summarize now</div>
                  <Seg label="What to summarize" value={scopeShown} options={SCOPES.filter((x) => !x.needsFile || filePath).map((x) => [x.scope, x.label] as [AiScope, string])} onChange={setScope} />
                  {!est ? (
                    <button type="button" className="iso-btn" disabled={busy} onClick={() => run(() => api.aiEstimate(scopeShown, scopeShown === "repo" || scopeShown === "architecture" ? undefined : (filePath ?? undefined)).then(setEst))}>
                      {busy ? "Estimating…" : "Estimate the cost"}
                    </button>
                  ) : (
                    <>
                      <p className="caption">
                        {plural(est.symbols, "item")}, {fmtInt(est.cached)} cached · {plural(est.requests, "request")} · {fmtInt(num(est.tokensIn))} tokens in, {fmtInt(num(est.tokensOut))} out · about {fmtUsd(est.costUsd)} with {est.model}
                      </p>
                      <button
                        type="button"
                        className="iso-btn primary"
                        disabled={busy || running || est.requests === 0}
                        onClick={() =>
                          run(async () => {
                            await onRun(scopeShown, scopeShown === "repo" || scopeShown === "architecture" ? null : filePath, budgetFor(est));
                            setEst(null);
                          })
                        }
                      >
                        {est.requests === 0 ? "Everything is cached" : `Summarize · up to ${fmtUsd(budgetFor(est))}`}
                      </button>
                    </>
                  )}
                  {job && (
                    <p className="caption" aria-live="polite">
                      {job.finished
                        ? job.error
                          ? `Stopped (${scopeText[job.scope]}): ${job.error}`
                          : `Done (${scopeText[job.scope]}): ${plural(job.stored, "summary", "summaries")}, ${fmtUsd(job.costUsd)}`
                        : `Writing summaries for ${scopeText[job.scope]}${job.total > 0 ? ` · ${job.done} / ${job.total}` : "…"} · ${fmtUsd(job.costUsd)} so far`}
                    </p>
                  )}
                </div>
              )}
              <button type="button" className="iso-btn quiet" disabled={busy} onClick={() => run(() => api.aiSignOut().then(setStatus))}>
                Sign out
              </button>
            </>
          )}
          {err && <p className="err">{err}</p>}
        </div>
      )}
      <SignInDialog
        open={dialog}
        status={status}
        setStatus={setStatus}
        onClose={(then) => {
          setDialog(false);
          if (then === "panel") setOpen(true);
        }}
      />
    </div>
  );
}
