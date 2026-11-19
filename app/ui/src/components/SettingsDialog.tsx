// Settings: the model each AI task uses and, under Advanced, the system prompt each task
// starts with. Saved for every repo; the next request uses them. Also opens the developer
// panel.
import { useEffect, useRef, useState } from "react";
import type { AiConfig } from "../ipc/bindings/AiConfig";
import type { AiModel } from "../ipc/bindings/AiModel";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import type { AiTask } from "../ipc/bindings/AiTask";
import { api } from "../ipc/api";
import { fmtInt } from "../lib/format";
import { Seg } from "./ui";

export const TASK_TEXT: Record<AiTask, { name: string; help: string }> = {
  summaries: { name: "Summaries", help: "Function, type and file summaries. Many small requests: a fast, low-cost model fits." },
  modules: { name: "Module names", help: "The name, role and color of each module. One request per module." },
  architecture: { name: "Architecture", help: "The architecture overview, layers and major flows. One request." },
  review: { name: "Change review", help: "The review of a change or pull request in Compare. One request per change." },
};

interface Draft {
  models: Record<AiTask, string>;
  prompts: Record<AiTask, string>;
  economy: boolean;
}

const draftOf = (c: AiConfig): Draft => ({
  models: Object.fromEntries(c.tasks.map((t) => [t.task, t.model])) as Record<AiTask, string>,
  prompts: Object.fromEntries(c.tasks.map((t) => [t.task, t.prompt])) as Record<AiTask, string>,
  economy: c.economy,
});

const usd = (x: number) => "$" + (Number.isInteger(x) ? String(x) : x.toFixed(2));
const price = (m: AiModel) => `${usd(m.inputPerM)} in · ${usd(m.outputPerM)} out`;

export function SettingsDialog({ open, onClose, ai, onSignIn, onOpenDev }: { open: boolean; onClose(): void; ai: AiStatus | null; onSignIn: (() => void) | null; onOpenDev(): void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [cfg, setCfg] = useState<AiConfig | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [advanced, setAdvanced] = useState(false);
  const [task, setTask] = useState<AiTask>("summaries");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);

  // Read the settings each time the dialog opens; signing in can change the model list.
  useEffect(() => {
    if (!open) return;
    let alive = true;
    setErr(null);
    setSaved(false);
    api
      .aiConfig()
      .then((c) => {
        if (!alive) return;
        setCfg(c);
        setDraft(draftOf(c));
      })
      .catch((e: unknown) => alive && setErr(e instanceof Error ? e.message : String(e)));
    return () => {
      alive = false;
    };
  }, [open, ai?.signedIn]);

  const base = cfg ? draftOf(cfg) : null;
  const dirty = !!draft && !!base && JSON.stringify(draft) !== JSON.stringify(base);

  const save = async () => {
    if (!draft || !cfg) return;
    setBusy(true);
    setErr(null);
    try {
      const c = await api.aiSetConfig({
        economy: draft.economy,
        tasks: cfg.tasks.map((t) => ({ task: t.task, model: draft.models[t.task], prompt: draft.prompts[t.task] === t.defaultPrompt ? null : draft.prompts[t.task] })),
      });
      setCfg(c);
      setDraft(draftOf(c));
      setSaved(true);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const set = (f: (d: Draft) => Draft) => {
    setSaved(false);
    setDraft((d) => (d ? f(d) : d));
  };
  const close = () => {
    if (dirty && !window.confirm("Close without saving your changes?")) return;
    onClose();
  };

  const tc = cfg?.tasks.find((t) => t.task === task) ?? null;
  return (
    <dialog
      ref={ref}
      className="signin settings"
      aria-labelledby="settings-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onClick={(e) => e.target === ref.current && close()}
    >
      {open && (
        <div className="settings-body">
          <div className="settings-head">
            <h2 id="settings-title">Settings</h2>
            <button type="button" className="iso-btn quiet" aria-label="Close" onClick={close}>
              ×
            </button>
          </div>
          <div className="settings-scroll">
            <section className="settings-sec" aria-labelledby="set-models">
              <div className="eyebrow" id="set-models">
                AI models
              </div>
              <p className="caption">Each task sends its own requests. Prices are in USD per million tokens. The settings apply to every repo.</p>
              {cfg && !cfg.modelsListed && (
                <p className="signin-note">
                  {ai?.signedIn ? "The account's models could not be listed. These are the defaults." : "Sign in to choose from your account's models. These are the defaults."}{" "}
                  {!ai?.signedIn &&
                    (onSignIn ? (
                      <button type="button" className="link-quiet accentish" onClick={onSignIn}>
                        Sign in…
                      </button>
                    ) : (
                      "Open a repo to sign in."
                    ))}
                </p>
              )}
              {!cfg && !err && (
                <p className="muted">
                  <span className="spinner" aria-hidden="true" /> Reading the settings…
                </p>
              )}
              {cfg && draft && (
                <ul className="set-tasks">
                  {cfg.tasks.map((t) => {
                    const locked = t.task === "modules" && draft.economy;
                    const value = locked ? draft.models.summaries : draft.models[t.task];
                    const ids = cfg.models.map((m) => m.id);
                    return (
                      <li key={t.task}>
                        <div className="set-task-h">
                          <label htmlFor={`model-${t.task}`}>{TASK_TEXT[t.task].name}</label>
                          <span className="caption">{TASK_TEXT[t.task].help}</span>
                        </div>
                        <div className="set-task-c">
                          <select id={`model-${t.task}`} className="iso-select" value={value} disabled={locked} onChange={(e) => set((d) => ({ ...d, models: { ...d.models, [t.task]: e.target.value } }))}>
                            {!ids.includes(value) && <option value={value}>{value} (not listed)</option>}
                            {cfg.models.map((m) => (
                              <option key={m.id} value={m.id}>
                                {m.name || m.id} · {price(m)}
                                {m.id === t.defaultModel ? " · default" : ""}
                              </option>
                            ))}
                          </select>
                          {t.task === "modules" && (
                            <label className="ai-check">
                              <input type="checkbox" checked={draft.economy} onChange={(e) => set((d) => ({ ...d, economy: e.target.checked }))} />
                              Use the summaries model (lower cost)
                            </label>
                          )}
                          {!locked && value !== t.defaultModel && ids.includes(t.defaultModel) && (
                            <button type="button" className="link-quiet" onClick={() => set((d) => ({ ...d, models: { ...d.models, [t.task]: t.defaultModel } }))}>
                              Use the default
                            </button>
                          )}
                        </div>
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>

            <section className="settings-sec" aria-label="Prompts">
              <label className="ai-check set-adv">
                <input type="checkbox" checked={advanced} onChange={(e) => setAdvanced(e.target.checked)} />
                <span>
                  <b>Advanced:</b> show and change the prompts
                </span>
              </label>
              {advanced && cfg && draft && tc && (
                <div className="set-prompts">
                  <Seg label="Prompt of task" value={task} onChange={setTask} options={cfg.tasks.map((t) => [t.task, TASK_TEXT[t.task].name + (draft.prompts[t.task] !== t.defaultPrompt ? " •" : "")] as [AiTask, string])} />
                  <p className="caption">
                    The system message that every {TASK_TEXT[task].name.toLowerCase()} request starts with. Isoline adds the items after it (code skeletons, module facts or the diff). The answer must keep the JSON shape that the prompt asks for, or Isoline cannot read it. A changed prompt keeps its own cached answers; reset it to get the earlier answers back.
                  </p>
                  <textarea className="set-prompt" aria-label={`Prompt of ${TASK_TEXT[task].name}`} spellCheck={false} value={draft.prompts[task]} onChange={(e) => set((d) => ({ ...d, prompts: { ...d.prompts, [task]: e.target.value } }))} />
                  <div className="set-prompt-bar">
                    <span className="caption">
                      {fmtInt(draft.prompts[task].length)} characters, about {fmtInt(Math.round(draft.prompts[task].length / 3.8))} tokens
                      {draft.prompts[task] !== tc.defaultPrompt ? " · changed" : " · default"}
                    </span>
                    <button type="button" className="link-quiet" disabled={draft.prompts[task] === tc.defaultPrompt} onClick={() => set((d) => ({ ...d, prompts: { ...d.prompts, [task]: tc.defaultPrompt } }))}>
                      Reset to default
                    </button>
                  </div>
                </div>
              )}
            </section>

            <section className="settings-sec" aria-labelledby="set-dev">
              <div className="eyebrow" id="set-dev">
                Developer
              </div>
              <div className="set-row">
                <span className="caption">Model requests with their prompts and answers, backend calls with their times, events, and app state.</span>
                <button type="button" className="iso-btn" onClick={onOpenDev}>
                  Open the developer panel
                </button>
              </div>
              <span className="caption">Shortcut: {navigator.platform.includes("Mac") ? "⌘⇧D" : "Ctrl+Shift+D"}</span>
            </section>
          </div>
          <div className="signin-actions settings-foot">
            {err && <span className="err">{err}</span>}
            {saved && !dirty && (
              <span className="caption" role="status">
                Saved. The next request uses these settings.
              </span>
            )}
            <button type="button" className="iso-btn" onClick={close}>
              {dirty ? "Cancel" : "Close"}
            </button>
            <button type="button" className="iso-btn primary" disabled={!dirty || busy} onClick={() => void save()}>
              {busy ? "Saving…" : "Save"}
            </button>
          </div>
        </div>
      )}
    </dialog>
  );
}
