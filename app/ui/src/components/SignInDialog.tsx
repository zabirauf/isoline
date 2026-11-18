// GitHub sign-in for AI summaries, as a modal dialog: the device code is the one thing the
// reader has to carry from here to GitHub, so it gets the whole surface. Three steps: what
// signing in does, the code while GitHub waits for approval, and the result.
import { useEffect, useRef, useState } from "react";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import { api, openExternal } from "../ipc/api";
import { plural } from "../lib/format";

export function SignInDialog({ open, status, setStatus, onClose }: { open: boolean; status: AiStatus | null; setStatus(s: AiStatus): void; onClose(then?: "panel"): void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // What starting sign-in did (copied the code, opened the browser). Later status polls carry
  // no message, so it is kept here.
  const [opened, setOpened] = useState(false);

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);

  const s = status;
  const code = s?.deviceCode ?? null;
  const step = s?.signedIn ? "done" : code ? "code" : "intro";

  const start = () =>
    void run(async () => {
      const st = await api.aiSignIn();
      setCopied(!!st.message?.includes("clipboard"));
      setOpened(!!st.message?.includes("browser"));
      return st;
    });

  const run = async (f: () => Promise<AiStatus>): Promise<boolean> => {
    setBusy(true);
    setErr(null);
    try {
      setStatus(await f());
      return true;
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      setBusy(false);
    }
  };

  // Closing while GitHub waits cancels the sign-in, so a stray approval later does nothing.
  const close = () => {
    if (step === "code") void run(() => api.aiSignInCancel());
    onClose();
  };

  const copy = async () => {
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code.userCode);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <dialog
      ref={ref}
      className="signin"
      aria-labelledby="signin-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onClick={(e) => e.target === ref.current && close()}
    >
      {open && s && (
        <div className="signin-body">
          {step === "intro" && (
            <>
              <h2 id="signin-title">Sign in for AI summaries</h2>
              <p>Isoline writes one-line summaries, module names and colors with your own GitHub Copilot plan.</p>
              <ul className="signin-facts">
                <li>Nothing from a repo is sent until you turn AI on for that repo.</li>
                <li>Each run shows its cost first and stops at a budget.</li>
                <li>Your token stays in the system keychain. Revoke it anytime in GitHub settings.</li>
              </ul>
              {s.message && <p className="signin-note">{s.message}</p>}
              <div className="signin-actions">
                <button type="button" className="iso-btn" onClick={close}>
                  Cancel
                </button>
                <button type="button" className="iso-btn primary" disabled={busy} autoFocus onClick={start}>
                  {busy ? "Asking GitHub for a code…" : "Sign in with GitHub"}
                </button>
              </div>
            </>
          )}

          {step === "code" && code && (
            <>
              <h2 id="signin-title">Enter this code on GitHub</h2>
              <button type="button" className="signin-code" onClick={copy} aria-label={`Code ${code.userCode}. Copy it`}>
                {code.userCode}
              </button>
              <p className="signin-copied" aria-live="polite">
                {copied ? "Copied to your clipboard" : "Click the code to copy it"}
              </p>
              <ol className="signin-steps">
                <li>
                  Open <code>{code.verificationUri.replace(/^https:\/\//, "")}</code>
                  {opened ? " (already open in your browser)" : ""}.
                </li>
                <li>Paste the code and choose Continue.</li>
                <li>Authorize Isoline. This window updates by itself.</li>
              </ol>
              <p className="signin-wait" role="status">
                <span className="spinner" aria-hidden="true" />
                Waiting for GitHub…
              </p>
              <div className="signin-actions">
                <button type="button" className="iso-btn" onClick={close}>
                  Cancel
                </button>
                <button type="button" className="iso-btn primary" autoFocus onClick={() => void openExternal(code.verificationUri)}>
                  Open GitHub
                </button>
              </div>
            </>
          )}

          {step === "done" && (
            <>
              <div className="signin-ok" aria-hidden="true">
                ✓
              </div>
              <h2 id="signin-title">Signed in as {s.user}</h2>
              <p>{s.models.length ? `${plural(s.models.length, "model")} available through your Copilot plan.` : "Copilot is ready."}</p>
              {!s.enabledForRepo && <p className="signin-note">AI is still off for this repo. Turn it on to estimate and run summaries.</p>}
              <div className="signin-actions">
                {s.enabledForRepo ? (
                  <button type="button" className="iso-btn primary" autoFocus onClick={() => onClose()}>
                    Done
                  </button>
                ) : (
                  <>
                    <button type="button" className="iso-btn" onClick={() => onClose()}>
                      Not now
                    </button>
                    <button type="button" className="iso-btn primary" disabled={busy} autoFocus onClick={() => void run(() => api.aiEnable(true)).then((ok) => ok && onClose("panel"))}>
                      Use AI for this repo
                    </button>
                  </>
                )}
              </div>
            </>
          )}
          {err && <p className="err">{err}</p>}
        </div>
      )}
    </dialog>
  );
}
