// AI status and sign-in. Summaries come from the reader's own Copilot account and are never
// requested until AI is turned on for the repo; the panel says what a run would cost first.
import { useEffect, useRef, useState } from "react";
import type { AiEstimate } from "../ipc/bindings/AiEstimate";
import type { AiProgress } from "../ipc/bindings/AiProgress";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import { api } from "../ipc/api";
import { fmtInt, fmtUsd, num, plural } from "../lib/format";

export function aiLabel(s: AiStatus | null): string {
  if (!s) return "AI";
  if (s.signedIn) return s.enabledForRepo ? "AI on" : "AI off for this repo";
  if (s.deviceCode || s.sidecar === "starting") return "AI: signing in";
  if (s.sidecar === "failed") return "AI unavailable";
  return "AI off";
}

export function AiPanel({ status, setStatus, filePath, progress }: { status: AiStatus | null; setStatus(s: AiStatus): void; filePath: string | null; progress: AiProgress | null }) {
  const [open, setOpen] = useState(false);
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

  useEffect(() => setEst(null), [filePath]);

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
  const running = progress && !progress.finished;
  return (
    <div className="ai" ref={box}>
      <button type="button" className={`iso-btn ai-btn${s?.signedIn && s.enabledForRepo ? " on" : ""}`} aria-expanded={open} aria-haspopup="dialog" onClick={() => setOpen(!open)}>
        {aiLabel(s)}
        {running ? ` · ${progress.done} / ${progress.total}` : ""}
      </button>
      {open && s && (
        <div className="popover ai-pop" role="dialog" aria-label="AI summaries">
          <div className="eyebrow">AI summaries</div>
          {s.mock && <p className="caption">Mock backend: nothing leaves this machine.</p>}
          {!s.signedIn && !s.deviceCode && (
            <>
              <p>Summaries come from your own GitHub Copilot account. Nothing is sent until you turn AI on for this repo.</p>
              <button type="button" className="iso-btn" disabled={busy} onClick={() => run(() => api.aiSignIn().then(setStatus))}>
                Sign in with GitHub
              </button>
            </>
          )}
          {!s.signedIn && s.deviceCode && (
            <>
              <p>
                Enter <code className="devcode">{s.deviceCode.userCode}</code> at <code>{s.deviceCode.verificationUri}</code>.
              </p>
              {s.message && <p className="caption">{s.message}</p>}
            </>
          )}
          {s.signedIn && (
            <>
              <p>
                Signed in as <code>{s.user}</code>. {plural(s.models.length, "model")} available.
              </p>
              <button type="button" className="iso-btn" aria-pressed={s.enabledForRepo} disabled={busy} onClick={() => run(() => api.aiEnable(!s.enabledForRepo).then(setStatus))}>
                Use AI for this repo
              </button>
              {s.enabledForRepo && filePath && (
                <div className="ai-file">
                  <div className="caption">This file</div>
                  {!est ? (
                    <button type="button" className="iso-btn" disabled={busy} onClick={() => run(() => api.aiEstimate("file", filePath).then(setEst))}>
                      Estimate the cost
                    </button>
                  ) : (
                    <>
                      <p className="caption">
                        {plural(est.symbols, "symbol")}, {fmtInt(est.cached)} cached · {plural(est.requests, "request")} · {fmtInt(num(est.tokensIn))} tokens in, {fmtInt(num(est.tokensOut))} out · about {fmtUsd(est.costUsd)} with {est.model}
                      </p>
                      <button type="button" className="iso-btn" disabled={busy || !!running || est.requests === 0} onClick={() => run(() => api.aiRun("file", filePath, Math.max(0.05, est.costUsd * 2)))}>
                        {est.requests === 0 ? "Everything is cached" : `Summarize · up to ${fmtUsd(Math.max(0.05, est.costUsd * 2))}`}
                      </button>
                    </>
                  )}
                  {progress && (
                    <p className="caption" aria-live="polite">
                      {progress.error ? `Stopped: ${progress.error}` : progress.finished ? `Done: ${plural(progress.total, "summary", "summaries")}, ${fmtUsd(progress.costUsd)}` : `Summarizing ${progress.done} / ${progress.total} · ${fmtUsd(progress.costUsd)} so far`}
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
    </div>
  );
}
