// The developer panel, docked at the bottom: model requests with their prompts and answers,
// backend calls with their times, backend events, and app state. ⌘⇧D (Ctrl+Shift+D).
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import type { AiRequestDetail } from "../ipc/bindings/AiRequestDetail";
import type { AiRequestRow } from "../ipc/bindings/AiRequestRow";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import type { RepoSummary } from "../ipc/bindings/RepoSummary";
import { api, backend } from "../ipc/api";
import { callStats, clearDevLog, devCalls, devEvents, devLogVersion, subscribeDevLog } from "../lib/devlog";
import { fmtInt, fmtUsd } from "../lib/format";
import { Seg } from "./ui";
import { TASK_TEXT } from "./SettingsDialog";

type Tab = "ai" | "calls" | "events" | "state";

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour12: false }) + "." + String(ms % 1000).padStart(3, "0");
const dur = (ms: number | null) => (ms === null ? "…" : ms < 10 ? `${ms.toFixed(1)} ms` : ms < 10_000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`);

/** JSON for display: bigints as numbers, long output cut. */
function pretty(v: unknown, max = 400_000): string {
  let s: string;
  try {
    s = JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? Number(x) : x), 2) ?? String(v);
  } catch {
    s = String(v);
  }
  return s.length > max ? s.slice(0, max) + `\n… ${fmtInt(s.length - max)} more characters` : s;
}

/** An answer as JSON when it is JSON, else as it came. */
function prettyAnswer(text: string): string {
  const t = text.trim().replace(/^```(?:json)?\s*|\s*```$/g, "");
  try {
    return JSON.stringify(JSON.parse(t), null, 2);
  } catch {
    return text;
  }
}

function copy(text: string) {
  void navigator.clipboard?.writeText(text).catch(() => {});
}

function useDevLog() {
  return useSyncExternalStore(subscribeDevLog, devLogVersion);
}

export function DevPanel({ onClose, ai, repo }: { onClose(): void; ai: AiStatus | null; repo: RepoSummary | null }) {
  const [tab, setTab] = useState<Tab>("ai");
  const [tall, setTall] = useState(false);
  return (
    <section className={`devpanel${tall ? " tall" : ""}`} aria-label="Developer panel">
      <div className="dev-bar">
        <b>Developer</b>
        <Seg
          label="Developer panel view"
          value={tab}
          onChange={setTab}
          options={[
            ["ai", "AI requests"],
            ["calls", "Backend calls"],
            ["events", "Events"],
            ["state", "State"],
          ]}
        />
        <span className="dev-sp" />
        <button type="button" className="iso-btn quiet" onClick={() => setTall(!tall)} aria-pressed={tall}>
          {tall ? "Smaller" : "Taller"}
        </button>
        <button type="button" className="iso-btn quiet" onClick={onClose} aria-label="Close the developer panel">
          ×
        </button>
      </div>
      <div className="dev-body">
        {tab === "ai" && <AiRequests />}
        {tab === "calls" && <Calls />}
        {tab === "events" && <Events />}
        {tab === "state" && <State ai={ai} repo={repo} />}
      </div>
    </section>
  );
}

function AiRequests() {
  const [rows, setRows] = useState<AiRequestRow[]>([]);
  const [sel, setSel] = useState<number | null>(null);
  const [detail, setDetail] = useState<AiRequestDetail | null>(null);
  const [part, setPart] = useState<"answer" | "prompt" | "system">("answer");
  const [raw, setRaw] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Poll while open: requests run in the backend and send no event of their own.
  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .aiRequests()
        .then((r) => {
          if (!alive) return;
          setRows(r);
          setErr(null);
        })
        .catch((e: unknown) => alive && setErr(e instanceof Error ? e.message : String(e)));
    void load();
    const t = setInterval(load, 1000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);

  const row = rows.find((r) => r.id === sel) ?? null;
  const rowKey = row ? `${row.id}:${row.state}` : "";
  useEffect(() => {
    if (sel === null) return setDetail(null);
    let alive = true;
    void api.aiRequest(sel).then((d) => alive && setDetail(d));
    return () => {
      alive = false;
    };
  }, [sel, rowKey]);

  const total = useMemo(() => rows.reduce((a, r) => ({ cost: a.cost + r.costUsd, tin: a.tin + r.tokensIn, tout: a.tout + r.tokensOut, failed: a.failed + (r.state === "failed" ? 1 : 0), running: a.running + (r.state === "running" ? 1 : 0) }), { cost: 0, tin: 0, tout: 0, failed: 0, running: 0 }), [rows]);
  const text = detail ? (part === "answer" ? (raw ? detail.answer : prettyAnswer(detail.answer)) : part === "prompt" ? detail.prompt : detail.system) : "";

  return (
    <div className="dev-split">
      <div className="dev-list">
        <div className="dev-tools">
          <span className="caption">
            {fmtInt(rows.length)} requests this session{total.running ? ` · ${total.running} running` : ""}
            {total.failed ? ` · ${total.failed} failed` : ""} · {fmtInt(total.tin)} tokens in, {fmtInt(total.tout)} out · {fmtUsd(total.cost)}
          </span>
          <button
            type="button"
            className="link-quiet"
            disabled={!rows.length}
            onClick={() =>
              void api.aiRequestsClear().then(() => {
                setRows([]);
                setSel(null);
              })
            }
          >
            Clear
          </button>
        </div>
        {err && <p className="err">{err}</p>}
        {!err && rows.length === 0 && <p className="muted dev-empty">No model requests yet. Requests that the cache answers do not show here, because they send nothing.</p>}
        {rows.length > 0 && (
          <div className="dev-table-wrap">
            <table className="dev-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Time</th>
                  <th>Task</th>
                  <th>Model</th>
                  <th className="n">Took</th>
                  <th className="n">In</th>
                  <th className="n">Out</th>
                  <th className="n">Cost</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.id} className={`${r.id === sel ? "on" : ""} st-${r.state}`} onClick={() => setSel(r.id === sel ? null : r.id)} tabIndex={0} onKeyDown={(e) => e.key === "Enter" && setSel(r.id)} aria-selected={r.id === sel}>
                    <td>{r.id}</td>
                    <td>{time(r.startedMs)}</td>
                    <td>{TASK_TEXT[r.task].name}</td>
                    <td className="mono">{r.model}</td>
                    <td className="n">{r.state === "running" ? <span className="spinner" aria-label="Running" /> : dur(r.durationMs)}</td>
                    <td className="n">{fmtInt(r.tokensIn)}</td>
                    <td className="n">{fmtInt(r.tokensOut)}</td>
                    <td className="n">{r.state === "failed" ? <span className="err">failed</span> : fmtUsd(r.costUsd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
      <div className="dev-detail">
        {!row && <p className="muted dev-empty">Select a request to see its system prompt, prompt and answer.</p>}
        {row && (
          <>
            <dl className="dev-facts">
              <dt>Task</dt>
              <dd>{TASK_TEXT[row.task].name}</dd>
              <dt>Model</dt>
              <dd className="mono">{row.model}</dd>
              <dt>State</dt>
              <dd>{row.state}</dd>
              <dt>Took</dt>
              <dd>{dur(row.durationMs)}</dd>
              <dt>Tokens</dt>
              <dd>
                {fmtInt(row.tokensIn)} in ({fmtInt(row.cachedTokensIn)} cached), {fmtInt(row.tokensOut)} out of at most {fmtInt(row.maxTokens)}
              </dd>
              <dt>Cost</dt>
              <dd>{fmtUsd(row.costUsd)}</dd>
            </dl>
            {row.error && <p className="err">{row.error}</p>}
            <div className="dev-tools">
              <Seg
                label="Part of the request"
                value={part}
                onChange={setPart}
                options={[
                  ["answer", `Answer · ${fmtInt(row.answerChars)}`],
                  ["prompt", `Prompt · ${fmtInt(row.promptChars)}`],
                  ["system", `System · ${fmtInt(row.systemChars)}`],
                ]}
              />
              {part === "answer" && (
                <label className="ai-check">
                  <input type="checkbox" checked={raw} onChange={(e) => setRaw(e.target.checked)} /> As sent
                </label>
              )}
              <span className="dev-sp" />
              <button type="button" className="link-quiet" disabled={!detail} onClick={() => copy(text)}>
                Copy
              </button>
              <button type="button" className="link-quiet" disabled={!detail} onClick={() => detail && copy(pretty(detail))}>
                Copy all as JSON
              </button>
            </div>
            <pre className="dev-pre">{detail ? text || (part === "answer" && row.state === "running" ? "Waiting for the answer…" : "(empty)") : "Loading…"}</pre>
          </>
        )}
      </div>
    </div>
  );
}

function Calls() {
  const v = useDevLog();
  const [filter, setFilter] = useState("");
  const [grouped, setGrouped] = useState(false);
  const [sel, setSel] = useState<number | null>(null);
  const all = devCalls();
  const list = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return [...all].reverse().filter((c) => !q || c.cmd.toLowerCase().includes(q));
  }, [all, v, filter]);
  const c = list.find((x) => x.id === sel) ?? null;
  return (
    <div className="dev-split">
      <div className="dev-list">
        <div className="dev-tools">
          <input className="dev-filter" placeholder="Filter by command" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter by command" />
          <label className="ai-check">
            <input type="checkbox" checked={grouped} onChange={(e) => setGrouped(e.target.checked)} /> By command
          </label>
          <span className="dev-sp" />
          <button type="button" className="link-quiet" onClick={() => clearDevLog("calls")}>
            Clear
          </button>
        </div>
        <div className="dev-table-wrap">
          {grouped ? (
            <table className="dev-table">
              <thead>
                <tr>
                  <th>Command</th>
                  <th className="n">Calls</th>
                  <th className="n">Median</th>
                  <th className="n">95%</th>
                  <th className="n">Slowest</th>
                  <th className="n">Failed</th>
                </tr>
              </thead>
              <tbody>
                {callStats(list).map((s) => (
                  <tr key={s.cmd} onClick={() => (setFilter(s.cmd), setGrouped(false))}>
                    <td className="mono">{s.cmd}</td>
                    <td className="n">{fmtInt(s.n)}</td>
                    <td className="n">{dur(s.p50)}</td>
                    <td className="n">{dur(s.p95)}</td>
                    <td className="n">{dur(s.max)}</td>
                    <td className="n">{s.failed || ""}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <table className="dev-table">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Command</th>
                  <th>Arguments</th>
                  <th className="n">Took</th>
                  <th>Result</th>
                </tr>
              </thead>
              <tbody>
                {list.map((x) => (
                  <tr key={x.id} className={x.id === sel ? "on" : ""} onClick={() => setSel(x.id === sel ? null : x.id)}>
                    <td>{time(x.at)}</td>
                    <td className="mono">{x.cmd}</td>
                    <td className="mono dev-clip">{x.args}</td>
                    <td className="n">{dur(x.ms)}</td>
                    <td className={`mono dev-clip${x.ok === false ? " err" : ""}`}>{x.ok === false ? x.error : x.shape}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
      <div className="dev-detail">
        {!c ? (
          <p className="muted dev-empty">Select a call to see its result. Only the newest 40 results are kept whole. Times include the trip through the backend bridge.</p>
        ) : (
          <>
            <dl className="dev-facts">
              <dt>Command</dt>
              <dd className="mono">{c.cmd}</dd>
              <dt>Arguments</dt>
              <dd className="mono">{c.args || "none"}</dd>
              <dt>Took</dt>
              <dd>{dur(c.ms)}</dd>
            </dl>
            {c.error && <p className="err">{c.error}</p>}
            <div className="dev-tools">
              <span className="dev-sp" />
              <button type="button" className="link-quiet" disabled={c.result === undefined} onClick={() => copy(pretty(c.result))}>
                Copy result
              </button>
            </div>
            <pre className="dev-pre">{c.result !== undefined ? pretty(c.result) : c.ok === null ? "Running…" : `${c.shape}\n\n(The whole result is no longer kept.)`}</pre>
          </>
        )}
      </div>
    </div>
  );
}

function Events() {
  useDevLog();
  const [filter, setFilter] = useState("");
  const [sel, setSel] = useState<number | null>(null);
  const all = devEvents();
  const q = filter.trim().toLowerCase();
  const list = [...all].reverse().filter((e) => !q || e.event.includes(q));
  const e = list.find((x) => x.id === sel) ?? null;
  return (
    <div className="dev-split">
      <div className="dev-list">
        <div className="dev-tools">
          <input className="dev-filter" placeholder="Filter by event" value={filter} onChange={(ev) => setFilter(ev.target.value)} aria-label="Filter by event" />
          <span className="caption">{fmtInt(all.length)} events kept</span>
          <span className="dev-sp" />
          <button type="button" className="link-quiet" onClick={() => clearDevLog("events")}>
            Clear
          </button>
        </div>
        <div className="dev-table-wrap">
          <table className="dev-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Event</th>
                <th>Payload</th>
              </tr>
            </thead>
            <tbody>
              {list.map((x) => (
                <tr key={x.id} className={x.id === sel ? "on" : ""} onClick={() => setSel(x.id === sel ? null : x.id)}>
                  <td>{time(x.at)}</td>
                  <td className="mono">{x.event}</td>
                  <td className="mono dev-clip">{x.shape}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
      <div className="dev-detail">
        {!e ? <p className="muted dev-empty">Select an event to see its payload.</p> : <pre className="dev-pre">{e.payload !== undefined ? pretty(e.payload) : `${e.shape}\n\n(The whole payload is no longer kept.)`}</pre>}
      </div>
    </div>
  );
}

function State({ ai, repo }: { ai: AiStatus | null; repo: RepoSummary | null }) {
  const [extra, setExtra] = useState<Record<string, unknown>>({});
  const [n, setN] = useState(0);
  useEffect(() => {
    let alive = true;
    const get = async (k: string, f: () => Promise<unknown>) => {
      try {
        const v = await f();
        if (alive) setExtra((x) => ({ ...x, [k]: v }));
      } catch (e) {
        if (alive) setExtra((x) => ({ ...x, [k]: { error: e instanceof Error ? e.message : String(e) } }));
      }
    };
    void get("backend", async () => (await backend()).kind);
    void get("aiConfig", async () => {
      const c = await api.aiConfig();
      // The prompts are in Settings; here they would hide everything else.
      return { ...c, tasks: c.tasks.map((t) => ({ ...t, prompt: `${t.prompt.length} characters`, defaultPrompt: `${t.defaultPrompt.length} characters` })) };
    });
    if (repo) void get("jobs", () => api.getJobs());
    if (repo) void get("git", () => api.gitStatus());
    return () => {
      alive = false;
    };
  }, [n, repo]);
  const mem = (performance as unknown as { memory?: { usedJSHeapSize: number; totalJSHeapSize: number } }).memory;
  const blocks: [string, unknown][] = [
    ["App", { backend: extra.backend, userAgent: navigator.userAgent, window: `${innerWidth}×${innerHeight} @${devicePixelRatio}x`, jsHeapMb: mem ? Math.round(mem.usedJSHeapSize / 1e6) : "not reported" }],
    ["AI status", ai],
    ["AI settings", extra.aiConfig],
    ["Repo", repo],
    ["Jobs", extra.jobs],
    ["Git", extra.git],
  ];
  return (
    <div className="dev-state">
      <div className="dev-tools">
        <span className="caption">What the app knows now. Copy a block to attach it to a bug report.</span>
        <span className="dev-sp" />
        <button type="button" className="link-quiet" onClick={() => setN(n + 1)}>
          Refresh
        </button>
        <button type="button" className="link-quiet" onClick={() => copy(pretty(Object.fromEntries(blocks)))}>
          Copy all
        </button>
      </div>
      <div className="dev-state-grid">
        {blocks.map(([k, v]) => (
          <details key={k} open={k !== "Git"}>
            <summary>
              {k}
              <button type="button" className="link-quiet" onClick={(e) => (e.preventDefault(), copy(pretty(v)))}>
                Copy
              </button>
            </summary>
            <pre className="dev-pre">{v === undefined ? "Loading…" : pretty(v)}</pre>
          </details>
        ))}
      </div>
    </div>
  );
}
