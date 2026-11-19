// The repo's architecture at the top of the repo page: what the system does, the major flows of
// requests and data through the modules, and the layers. A model writes it once from the module
// graph and the module summaries (the repo pass); module chips carry the same colors as the
// map and the file tree, and select the module in the map.
import { useState, useSyncExternalStore } from "react";
import { getPrefs, subscribePrefs } from "../lib/prefs";
import type { AiEstimate } from "../ipc/bindings/AiEstimate";
import type { AiScope } from "../ipc/bindings/AiScope";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import type { Architecture } from "../ipc/bindings/Architecture";
import type { ModuleInfo } from "../ipc/bindings/ModuleInfo";
import { api } from "../ipc/api";
import { asksFirst, budgetFor, type AiJob } from "../lib/aiRuns";
import { fmtInt, fmtUsd, plural } from "../lib/format";
import { AiMark, ModuleDot } from "./ui";

const LAYER_CHIPS = 8;

export function ArchitecturePanel({
  arch,
  modules,
  ai,
  job,
  onRun,
  onOpenAi,
  onSelect,
  selected,
}: {
  arch: Architecture | null;
  modules: ReadonlyMap<string, ModuleInfo>;
  ai: AiStatus | null;
  job: AiJob | null;
  onRun(scope: AiScope, path: string | null, budgetUsd: number): Promise<void>;
  onOpenAi(): void;
  onSelect(id: string): void;
  selected: string | null;
}) {
  const prefs = useSyncExternalStore(subscribePrefs, getPrefs);
  const [est, setEst] = useState<AiEstimate | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [openLayers, setOpenLayers] = useState<Set<number>>(new Set());
  const on = !!ai?.signedIn && !!ai.enabledForRepo;
  const running = !!job && !job.finished && (job.scope === "architecture" || job.scope === "repo");
  const failed = !!job && job.finished && !!job.error && (job.scope === "architecture" || job.scope === "repo") ? job.error : null;

  const estimate = async () => {
    setBusy(true);
    setErr(null);
    try {
      const e = await api.aiEstimate("architecture");
      // Costs not confirmed: start at once, unless it is over the limit.
      if (!asksFirst(e.costUsd, prefs)) await onRun("architecture", null, budgetFor(e));
      else setEst(e);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const write = async () => {
    if (!est) return;
    setBusy(true);
    try {
      await onRun("architecture", null, budgetFor(est));
      setEst(null);
    } finally {
      setBusy(false);
    }
  };

  const chip = (id: string) => {
    const m = modules.get(id);
    if (!m) return null;
    return (
      <button key={id} type="button" className={`mchip${selected === id ? " on" : ""}`} onClick={() => onSelect(id)} title={`${m.dir || "/"} · ${plural(m.files, "file")}${m.role ? ` · ${m.role}` : ""}`}>
        <ModuleDot color={m.color} />
        {m.name}
      </button>
    );
  };

  const actions = (
    <div className="arch-actions">
      {running ? (
        <span className="muted" role="status">
          <span className="spinner" aria-hidden="true" /> {job!.total > 0 ? `Naming modules and writing the overview · ${fmtInt(job!.done)} / ${fmtInt(job!.total)} · ${fmtUsd(job!.costUsd)} so far` : "Starting…"}
        </span>
      ) : !on ? (
        <button type="button" className="link-quiet accentish" onClick={onOpenAi}>
          Turn on AI…
        </button>
      ) : est ? (
        <>
          <span className="caption">
            {!prefs.confirmCost && `Over your limit of ${fmtUsd(prefs.askAboveUsd)} for runs without asking. `}
            {plural(est.symbols, "module")} to name ({fmtInt(est.cached)} cached) and one overview · about {fmtUsd(est.costUsd)}
          </span>
          <button type="button" className="iso-btn primary" disabled={busy} onClick={() => void write()}>
            {arch ? "Rewrite" : "Write it"} · up to {fmtUsd(budgetFor(est))}
          </button>
          <button type="button" className="link-quiet" onClick={() => setEst(null)}>
            Cancel
          </button>
        </>
      ) : (
        <button type="button" className={arch ? "link-quiet" : "iso-btn"} disabled={busy} onClick={() => void estimate()}>
          {busy ? (prefs.confirmCost ? "Estimating…" : "Starting…") : (arch ? "Rewrite" : "Write the architecture overview") + (prefs.confirmCost ? "…" : "")}
        </button>
      )}
      {failed && !running && <span className="err">Stopped: {failed}</span>}
      {err && <span className="err">{err}</span>}
    </div>
  );

  return (
    <section className="panel arch" aria-labelledby="arch-h">
      <div className="rbar arch-bar">
        <div className="display-heading" id="arch-h">
          Architecture
        </div>
        {arch && (
          <span className="caption">
            {plural(arch.modules, "module")} · written {new Date(arch.writtenAt * 1000).toLocaleDateString()}
          </span>
        )}
      </div>
      {!arch ? (
        <p className="muted">No architecture overview yet. A model can write one from the module graph: what the system does, its layers, and the major flows of data between modules. It also names each module; the colors here match the map and the file tree.</p>
      ) : (
        <>
          <p className="arch-text">
            {arch.overview}
            <AiMark model={arch.model} />
          </p>
          {arch.flows.length > 0 && (
            <div className="arch-sec">
              <div className="eyebrow">Major flows</div>
              <ol className="arch-flows">
                {arch.flows.map((f, i) => (
                  <li key={i}>
                    <div className="flow-h">
                      <b>{f.name}</b>
                      <span className="flow-path">
                        {f.modules.map((id, j) => (
                          <span key={id + j} className="flow-step">
                            {j > 0 && (
                              <span className="sep" aria-hidden="true">
                                ›
                              </span>
                            )}
                            {chip(id)}
                          </span>
                        ))}
                      </span>
                    </div>
                    {f.text && <p className="caption">{f.text}</p>}
                  </li>
                ))}
              </ol>
            </div>
          )}
          <div className="arch-sec">
            <div className="eyebrow">Layers, from entry points to foundations</div>
            <ul className="arch-layers">
              {arch.layers.map((l, i) => {
                const all = openLayers.has(i);
                const shown = all ? l.modules : l.modules.slice(0, LAYER_CHIPS);
                return (
                  <li key={i}>
                    <span className="layer-name">{l.name}</span>
                    <span className="chips">
                      {shown.map(chip)}
                      {l.modules.length > LAYER_CHIPS && (
                        <button type="button" className="link-quiet" onClick={() => setOpenLayers((s) => new Set(all ? [...s].filter((x) => x !== i) : [...s, i]))}>
                          {all ? "Show fewer" : `+ ${fmtInt(l.modules.length - LAYER_CHIPS)} more`}
                        </button>
                      )}
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>
        </>
      )}
      {actions}
    </section>
  );
}
