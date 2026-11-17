// Mock git: branches, tags, commits, two pull requests and one canned comparison built from the
// mock repo's files, so the branch menu and the compare view work in a plain browser.
import type { Comparison } from "../bindings/Comparison";
import type { FileChange } from "../bindings/FileChange";
import type { FileDiffText } from "../bindings/FileDiffText";
import type { GitBranch } from "../bindings/GitBranch";
import type { GitCommit } from "../bindings/GitCommit";
import type { GitRefs } from "../bindings/GitRefs";
import type { GitStatus } from "../bindings/GitStatus";
import type { PullRequest } from "../bindings/PullRequest";
import type { PullRequests } from "../bindings/PullRequests";
import type { SymbolChange } from "../bindings/SymbolChange";

const NOW = Math.floor(Date.UTC(2026, 10, 17, 15, 0, 0) / 1000);
const H = 3600;
const D = 24 * H;

const sha = (seed: string) => {
  let h = 0x811c9dc5;
  let out = "";
  for (let k = 0; k < 5; k++) {
    for (const c of seed + k) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193) >>> 0;
    out += h.toString(16).padStart(8, "0");
  }
  return out;
};

interface BranchSeed {
  name: string;
  subject: string;
  author: string;
  age: number;
  upstream: string | null;
}

const LOCAL: BranchSeed[] = [
  { name: "main", subject: "Gateway: close idle sockets after 60 s", author: "Mira Okafor", age: 5 * H, upstream: "origin/main" },
  { name: "fix/harness-retry", subject: "Harness: retry transient attempt failures", author: "Dev Patel", age: 2 * H, upstream: "origin/fix/harness-retry" },
  { name: "config-snapshot-opts", subject: "Config: snapshot takes options", author: "Dev Patel", age: 3 * D, upstream: null },
  { name: "docs/refresh", subject: "Docs: rename diagnostics page", author: "Lena Brandt", age: 9 * D, upstream: null },
];
const REMOTE: BranchSeed[] = [
  { name: "origin/main", subject: "Gateway: close idle sockets after 60 s", author: "Mira Okafor", age: 5 * H, upstream: null },
  { name: "origin/fix/harness-retry", subject: "Harness: retry transient attempt failures", author: "Dev Patel", age: 2 * H, upstream: null },
  { name: "origin/release/2026.11", subject: "Release 2026.11.2", author: "Release bot", age: 6 * D, upstream: null },
  { name: "origin/feat/slack-threads", subject: "Slack: reply in threads", author: "Sam Ito", age: 12 * D, upstream: null },
];

const COMMITS: [string, string, number][] = [
  ["Harness: retry transient attempt failures", "Dev Patel", 2 * H],
  ["Harness: drop legacyAttemptTimeout", "Dev Patel", 3 * H],
  ["Config: snapshot takes options", "Dev Patel", 4 * H],
  ["Channels: remove the terminal adapter", "Mira Okafor", 26 * H],
  ["Docs: rename diagnostics page", "Lena Brandt", 30 * H],
];

const MAIN_LOG: [string, string, number][] = [
  ["Gateway: close idle sockets after 60 s", "Mira Okafor", 5 * H],
  ["CLI: --json for status", "Sam Ito", 20 * H],
  ["Diagnostics: trace ids in every event", "Lena Brandt", 2 * D],
  ["Release 2026.11.2", "Release bot", 6 * D],
  ["Slack adapter: format code blocks", "Sam Ito", 7 * D],
  ["Context engine: host compat for v2 params", "Mira Okafor", 8 * D],
  ["Runner: split helpers out of run.ts", "Dev Patel", 10 * D],
  ["Release 2026.11.1", "Release bot", 13 * D],
];

export interface MockGit {
  status(): GitStatus;
  refs(): GitRefs;
  switchTo(name: string, create: boolean): GitStatus;
  fetch(): GitStatus;
  prs(): PullRequests;
  compare(base: string, head: string, threeDot: boolean, pr: number | undefined, sourceOf: (path: string) => string | null): Comparison;
  compareFile(base: string, head: string, path: string, oldPath: string | undefined, sourceOf: (path: string) => string | null): FileDiffText;
}

const PRS: PullRequest[] = [
  { number: 482, title: "Retry transient harness failures", author: "devpatel", headRef: "fix/harness-retry", baseRef: "main", draft: false, url: "https://github.com/example/openclaw/pull/482", updated: NOW - 2 * H, additions: 61, deletions: 38, changedFiles: 5 },
  { number: 479, title: "Config snapshot options", author: "devpatel", headRef: "config-snapshot-opts", baseRef: "main", draft: true, url: "https://github.com/example/openclaw/pull/479", updated: NOW - 3 * D, additions: 6, deletions: 2, changedFiles: 1 },
];

export function createMockGit(): MockGit {
  let current: string | null = "main";
  const locals = LOCAL.map((b) => ({ ...b }));
  let behind = 2;
  const branch = (b: BranchSeed, remote: boolean): GitBranch => ({ name: b.name, remote, current: !remote && b.name === current, sha: sha(b.name).slice(0, 7), subject: b.subject, author: b.author, date: NOW - b.age, upstream: b.upstream });
  const status = (): GitStatus => {
    const b = locals.find((x) => x.name === current);
    return { isRepo: true, branch: current, head: sha(current ?? "detached").slice(0, 7), upstream: b?.upstream ?? null, ahead: current === "fix/harness-retry" ? 5 : 0, behind: current === "main" ? behind : 0, changed: 3, untracked: 1, operation: null, github: true };
  };
  const commit = ([subject, author, age]: [string, string, number], refs: string[] = []): GitCommit => {
    const s = sha(subject);
    return { sha: s, short: s.slice(0, 7), subject, author, date: NOW - age, refs };
  };

  return {
    status,
    refs: () => ({
      branches: [...locals.map((b) => branch(b, false)), ...REMOTE.map((b) => branch(b, true))],
      tags: [
        { name: "v2026.11.2", sha: sha("Release 2026.11.2").slice(0, 7), date: NOW - 6 * D },
        { name: "v2026.11.1", sha: sha("Release 2026.11.1").slice(0, 7), date: NOW - 13 * D },
        { name: "v2026.10.0", sha: sha("v2026.10.0").slice(0, 7), date: NOW - 40 * D },
      ],
      commits: (current === "fix/harness-retry" ? [...COMMITS, ...MAIN_LOG] : MAIN_LOG).map((c, i) => commit(c, i === 0 && current ? [current] : c[0].startsWith("Release") ? [`v${c[0].slice(8)}`] : [])),
    }),
    switchTo: (name, create) => {
      const local = name.replace(/^origin\//, "");
      if (create) {
        if (locals.some((b) => b.name === local)) throw new Error(`fatal: a branch named '${local}' already exists`);
        locals.unshift({ name: local, subject: locals.find((b) => b.name === current)?.subject ?? "", author: "You", age: 0, upstream: null });
      } else if (!locals.some((b) => b.name === local)) {
        const r = REMOTE.find((b) => b.name === name || b.name === `origin/${local}`);
        if (!r) throw new Error(`fatal: invalid reference: ${name}`);
        locals.push({ ...r, name: local, upstream: r.name });
      }
      if (local === "docs/refresh") throw new Error("error: Your local changes to the following files would be overwritten by checkout:\n\tdocs/diagnostics.md\nPlease commit your changes or stash them before you switch branches.\nAborting");
      current = local;
      return status();
    },
    fetch: () => {
      behind = 3;
      return status();
    },
    prs: () => ({ available: true, message: null, items: PRS }),
    compare: (base, head, threeDot, pr, sourceOf) => comparison(base, head, threeDot, pr, current, sourceOf),
    compareFile: (_base, _head, path, oldPath, sourceOf) => fileText(path, oldPath ?? null, sourceOf),
  };
}

// ---------- the canned comparison ----------

const LIFE = "src/agents/harness/lifecycle.ts";
const RETRY = "src/agents/harness/retry-policy.ts";
const CONFIG = "src/config/config.ts";
const TERMINAL = "src/channels/terminal/adapter.ts";
const DOC_OLD = "docs/diagnostics.md";
const DOC_NEW = "docs/observability.md";

const RETRY_SRC = `// When a harness attempt fails for a reason worth retrying, and how long to wait.
import { emitTrustedDiagnosticEvent } from "../../infra/diagnostic-events.js";
import { deliveryBackoffMs } from "../../channels/delivery.js";
import type { AgentHarnessAttemptResult } from "./types.js";

const TRANSIENT = new Set(["timeout", "rate_limited", "upstream_unavailable"]);

export interface RetryPolicy {
  maxAttempts: number;
  baseDelayMs: number;
}

export const DEFAULT_RETRY_POLICY: RetryPolicy = { maxAttempts: 3, baseDelayMs: 400 };

export function shouldRetryAttempt(result: AgentHarnessAttemptResult, attempt: number, policy = DEFAULT_RETRY_POLICY): boolean {
  if (attempt >= policy.maxAttempts) return false;
  const reason = result.error?.code ?? "";
  const retry = TRANSIENT.has(reason);
  emitTrustedDiagnosticEvent({ kind: "harness.retry_decision", attempt, reason, retry });
  return retry;
}

export function retryDelayMs(attempt: number, policy = DEFAULT_RETRY_POLICY): number {
  const jitter = Math.floor(Math.random() * policy.baseDelayMs);
  return Math.min(deliveryBackoffMs(attempt) + policy.baseDelayMs * 2 ** attempt + jitter, 10_000);
}
`;

const TERMINAL_SRC = `// Terminal channel: prints agent turns to stdout. Replaced by the CLI's own renderer.
import { formatTurn } from "../format.js";

export function createTerminalAdapter(write: (s: string) => void = (s) => process.stdout.write(s)) {
  return {
    name: "terminal",
    send(turn: { text: string }) {
      write(formatTurn(turn.text) + "\\n");
    },
  };
}
`;

const DOC_SRC = `# Diagnostics

Every agent run emits diagnostic events with a trace id. See \`src/infra/diagnostic-events.ts\`.

## Events

- \`agent.run.started\`
- \`agent.run.completed\`
`;

const LEGACY = ["export function legacyAttemptTimeout(ms: number): number {", "  return Math.max(1_000, Math.min(ms, 120_000));", "}", ""];

/** Base side of lifecycle.ts, made from the head side: the retry wrapper did not exist, a removed helper did, and the finalization took one more parameter. */
function lifecycleBefore(after: string): { text: string; lines: Record<string, [number, number]> } {
  const a = after.split("\n");
  const rows: { t: string; from: number | null }[] = a.map((t, i) => ({ t, from: i + 1 }));
  // Base had one more parameter on the finalization.
  rows.splice(426, 0, { t: "  legacy?: boolean,", from: null });
  // The base had legacyAttemptTimeout before runAgentHarnessLifecycleAttempt.
  rows.splice(356, 0, ...LEGACY.map((t) => ({ t, from: null })));
  // A line inside emitAgentRunCompleted differed.
  rows[182] = { t: '    completion.outcome === "error" && completion.error != null;', from: null };
  // withFallbackDiagnosticTrace (216-228) is new on the head side.
  rows.splice(215, 13);
  const at = (afterLine: number) => rows.findIndex((r) => r.from === afterLine) + 1;
  const legacyStart = rows.findIndex((r) => r.t === LEGACY[0]) + 1;
  return {
    text: rows.map((r) => r.t).join("\n"),
    lines: {
      emitAgentRunCompleted: [at(171), at(200)],
      legacyAttemptTimeout: [legacyStart, legacyStart + 2],
      runAgentHarnessLifecycleAttempt: [at(357), at(423)],
      runAgentHarnessLifecycleFinalization: [at(425), at(486)],
    },
  };
}

const CONFIG_AFTER = `// config.ts: mock stub generated from what lifecycle.ts imports.

export interface SnapshotOptions {
  /** Read the file again instead of returning the cached snapshot. */
  fresh?: boolean;
}

export function getRuntimeConfigSnapshot(opts: SnapshotOptions = {}): unknown {
  return opts.fresh ? structuredClone(globalThis) : globalThis;
}
`;

function sym(name: string, change: SymbolChange["change"], o: Partial<SymbolChange> = {}): SymbolChange {
  return { name, kind: "function", change, exported: false, signatureBefore: null, signatureAfter: null, before: null, after: null, callsAdded: [], callsDeleted: [], callers: 0, ...o };
}

function fileChanges(lifeLines: Record<string, [number, number]>): FileChange[] {
  const harness = { id: "agents-harness", name: "Agent harness", color: "#2f6fb0" };
  const r = (k: string) => ({ start: lifeLines[k][0], end: lifeLines[k][1] });
  return [
    {
      path: LIFE,
      oldPath: null,
      change: "modified",
      lang: "typescript",
      tier: "full",
      additions: 19,
      deletions: 9,
      binary: false,
      module: harness,
      symbols: [
        sym("emitAgentRunCompleted", "modified", { signatureBefore: "function emitAgentRunCompleted( params: AgentHarnessAttemptParams, trace: DiagnosticTraceContext, completion: AgentRunCompletion )", signatureAfter: "function emitAgentRunCompleted( params: AgentHarnessAttemptParams, trace: DiagnosticTraceContext, completion: AgentRunCompletion )", before: r("emitAgentRunCompleted"), after: { start: 171, end: 200 }, callsAdded: ["classifyMissingError"] }),
        sym("withFallbackDiagnosticTrace", "added", { signatureAfter: "function withFallbackDiagnosticTrace( result: EmbeddedRunAttemptResult, trace: DiagnosticTraceContext ): EmbeddedRunAttemptResult", after: { start: 216, end: 227 }, callsAdded: ["freezeDiagnosticTraceContext"] }),
        sym("legacyAttemptTimeout", "deleted", { exported: true, signatureBefore: "export function legacyAttemptTimeout(ms: number): number", before: r("legacyAttemptTimeout"), callers: 2 }),
        sym("runAgentHarnessLifecycleAttempt", "modified", { exported: true, signatureBefore: "export async function runAgentHarnessLifecycleAttempt( harness: AgentHarness, params: AgentHarnessAttemptParams )", signatureAfter: "export async function runAgentHarnessLifecycleAttempt( harness: AgentHarness, params: AgentHarnessAttemptParams )", before: r("runAgentHarnessLifecycleAttempt"), after: { start: 357, end: 423 }, callsAdded: ["withFallbackDiagnosticTrace", "shouldRetryAttempt", "retryDelayMs"], callsDeleted: ["legacyAttemptTimeout"], callers: 3 }),
        sym("runAgentHarnessLifecycleFinalization", "modified", { exported: true, signatureBefore: "export async function runAgentHarnessLifecycleFinalization( harness: AgentHarness, params: AgentHarnessSettledTurnFinalizationAttemptParams<AgentHarnessAttemptParamsV2>, legacy?: boolean )", signatureAfter: "export async function runAgentHarnessLifecycleFinalization( harness: AgentHarness, params: AgentHarnessSettledTurnFinalizationAttemptParams<AgentHarnessAttemptParamsV2> )", before: r("runAgentHarnessLifecycleFinalization"), after: { start: 425, end: 486 }, callers: 1 }),
      ],
      imports: [{ change: "added", specifier: "./retry-policy.js", file: RETRY, external: null }],
    },
    {
      path: RETRY,
      oldPath: null,
      change: "added",
      lang: "typescript",
      tier: "full",
      additions: RETRY_SRC.split("\n").length - 1,
      deletions: 0,
      binary: false,
      module: harness,
      symbols: [
        sym("RetryPolicy", "added", { kind: "interface", exported: true, signatureAfter: "export interface RetryPolicy", after: { start: 8, end: 11 } }),
        sym("DEFAULT_RETRY_POLICY", "added", { kind: "constant", exported: true, signatureAfter: "export const DEFAULT_RETRY_POLICY: RetryPolicy", after: { start: 13, end: 13 } }),
        sym("shouldRetryAttempt", "added", { exported: true, signatureAfter: "export function shouldRetryAttempt(result: AgentHarnessAttemptResult, attempt: number, policy = DEFAULT_RETRY_POLICY): boolean", after: { start: 15, end: 21 }, callsAdded: ["emitTrustedDiagnosticEvent"] }),
        sym("retryDelayMs", "added", { exported: true, signatureAfter: "export function retryDelayMs(attempt: number, policy = DEFAULT_RETRY_POLICY): number", after: { start: 23, end: 26 }, callsAdded: ["deliveryBackoffMs"] }),
      ],
      imports: [
        { change: "added", specifier: "../../infra/diagnostic-events.js", file: "src/infra/diagnostic-events.ts", external: null },
        { change: "added", specifier: "../../channels/delivery.js", file: "src/channels/delivery.ts", external: null },
        { change: "added", specifier: "./types.js", file: "src/agents/harness/types.ts", external: null },
      ],
    },
    {
      path: CONFIG,
      oldPath: null,
      change: "modified",
      lang: "typescript",
      tier: "full",
      additions: 8,
      deletions: 2,
      binary: false,
      module: { id: "config", name: "Config", color: "#5d7a2a" },
      symbols: [
        sym("SnapshotOptions", "added", { kind: "interface", exported: true, signatureAfter: "export interface SnapshotOptions", after: { start: 3, end: 6 } }),
        sym("getRuntimeConfigSnapshot", "modified", { exported: true, signatureBefore: "export function getRuntimeConfigSnapshot(...args: unknown[]): unknown", signatureAfter: "export function getRuntimeConfigSnapshot(opts: SnapshotOptions = {}): unknown", before: { start: 3, end: 5 }, after: { start: 8, end: 10 }, callsAdded: ["structuredClone"], callers: 4 }),
      ],
      imports: [],
    },
    {
      path: TERMINAL,
      oldPath: null,
      change: "deleted",
      lang: "typescript",
      tier: "full",
      additions: 0,
      deletions: TERMINAL_SRC.split("\n").length - 1,
      binary: false,
      module: { id: "channels", name: "Channels", color: "#9a5b2e" },
      symbols: [sym("createTerminalAdapter", "deleted", { exported: true, signatureBefore: "export function createTerminalAdapter(write: (s: string) => void = (s) => process.stdout.write(s))", before: { start: 4, end: 11 }, callsDeleted: ["formatTurn"] })],
      imports: [{ change: "deleted", specifier: "../format.js", file: "src/channels/format.ts", external: null }],
    },
    {
      path: DOC_NEW,
      oldPath: DOC_OLD,
      change: "renamed",
      lang: "markdown",
      tier: "text",
      additions: 2,
      deletions: 1,
      binary: false,
      module: { id: "docs", name: "Docs", color: "#7d7466" },
      symbols: [],
      imports: [],
    },
  ];
}

function revLabel(spec: string): string {
  return spec === "worktree" ? "Working tree" : spec;
}

function comparison(base: string, head: string, threeDot: boolean, pr: number | undefined, current: string | null, sourceOf: (p: string) => string | null): Comparison {
  const t0 = performance.now();
  const p = pr !== undefined ? PRS.find((x) => x.number === pr) : undefined;
  if (pr !== undefined && !p) throw new Error(`No pull request #${pr}`);
  const b = p ? p.baseRef : base;
  const h = p ? p.headRef : head;
  const same = b === h || (h === current && b === current);
  let files = same ? [] : fileChanges(lifecycleBefore(sourceOf(LIFE) ?? "").lines);
  if (p?.number === 479) files = files.filter((f) => f.path === CONFIG);
  const sum = (f: (x: FileChange) => number) => files.reduce((s, x) => s + f(x), 0);
  const syms = files.flatMap((f) => f.symbols);
  const imports = files.flatMap((f) => f.imports);
  const touched = new Set(files.map((f) => f.module?.id));
  const modules = [
    { id: "agents-harness", name: "Agent harness", color: "#2f6fb0", dir: "src/agents/harness" },
    { id: "config", name: "Config", color: "#5d7a2a", dir: "src/config" },
    { id: "channels", name: "Channels", color: "#9a5b2e", dir: "src/channels" },
    { id: "docs", name: "Docs", color: "#7d7466", dir: "docs" },
  ]
    .filter((m) => touched.has(m.id))
    .map((m) => {
      const fs = files.filter((f) => f.module?.id === m.id);
      return { ...m, files: fs.length, additions: fs.reduce((s, f) => s + f.additions, 0), deletions: fs.reduce((s, f) => s + f.deletions, 0), symbols: fs.reduce((s, f) => s + f.symbols.length, 0) };
    })
    .sort((x, y) => y.additions + y.deletions - (x.additions + x.deletions));
  const harnessTouched = touched.has("agents-harness");
  const moduleEdges = harnessTouched
    ? [
        { from: "agents-harness", to: "channels", before: 0, after: 1 },
        { from: "agents-harness", to: "context-engine", before: 1, after: 0 },
        { from: "agents-harness", to: "infra-diagnostics", before: 9, after: 10 },
        { from: "channels", to: "agents-core", before: 1, after: 0 },
        { from: "gateway", to: "agents-harness", before: 4, after: 4 },
      ].filter((e) => e.before !== e.after)
    : [];
  const sideRev = (spec: string, label: string) => ({ spec, sha: spec === "worktree" ? "worktree" : sha(spec), label, subject: spec === "worktree" ? null : spec === "main" ? "Gateway: close idle sockets after 60 s" : "Harness: retry transient attempt failures" });
  return {
    base: sideRev(p ? `pr:${p.number}` : b, p ? p.baseRef : revLabel(b)),
    head: sideRev(p ? `pr:${p.number}` : h, p ? `#${p.number} · ${p.headRef}` : revLabel(h)),
    mergeBase: threeDot && !same ? sha("merge-base " + b + h) : null,
    pr: p ?? null,
    files,
    totals: {
      files: files.length,
      added: files.filter((f) => f.change === "added").length,
      modified: files.filter((f) => f.change === "modified").length,
      deleted: files.filter((f) => f.change === "deleted").length,
      renamed: files.filter((f) => f.change === "renamed").length,
      additions: sum((f) => f.additions),
      deletions: sum((f) => f.deletions),
      symbolsAdded: syms.filter((s) => s.change === "added").length,
      symbolsDeleted: syms.filter((s) => s.change === "deleted").length,
      symbolsModified: syms.filter((s) => s.change === "modified").length,
      signaturesChanged: syms.filter((s) => s.change === "modified" && s.signatureBefore !== s.signatureAfter).length,
      importsAdded: imports.filter((i) => i.change === "added").length,
      importsDeleted: imports.filter((i) => i.change === "deleted").length,
    },
    modules,
    moduleEdges,
    impact: [
      ...(harnessTouched
        ? [
            { symbol: "legacyAttemptTimeout", path: LIFE, change: "deleted" as const, signatureBefore: "export function legacyAttemptTimeout(ms: number): number", signatureAfter: null, callers: [{ path: "src/gateway/agent-session.ts", line: 41 }, { path: "src/agents/embedded-agent-runner/run/helpers.ts", line: 12 }], callerFiles: 2 },
            { symbol: "runAgentHarnessLifecycleFinalization", path: LIFE, change: "modified" as const, signatureBefore: "export async function runAgentHarnessLifecycleFinalization( harness, params, legacy?: boolean )", signatureAfter: "export async function runAgentHarnessLifecycleFinalization( harness, params )", callers: [{ path: "src/agents/embedded-agent-runner/runner.ts", line: 88 }], callerFiles: 1 },
          ]
        : []),
      ...(touched.has("config") ? [{ symbol: "getRuntimeConfigSnapshot", path: CONFIG, change: "modified" as const, signatureBefore: "export function getRuntimeConfigSnapshot(...args: unknown[]): unknown", signatureAfter: "export function getRuntimeConfigSnapshot(opts: SnapshotOptions = {}): unknown", callers: [{ path: "src/cli/run.ts", line: 17 }, { path: "src/gateway/server.ts", line: 30 }, { path: "src/context-engine/host-compat.ts", line: 8 }, ...["src/channels/slack/adapter.ts", "src/channels/discord/adapter.ts", "src/plugins/loader.ts", "src/gateway/ws.ts", "src/cli/index.ts", "src/infra/diagnostic-events.ts", "src/agents/harness/types.ts"].map((path, i) => ({ path, line: 10 + i * 7 }))], callerFiles: 214 }] : []),
    ],
    commits: same ? [] : (p?.number === 479 ? COMMITS.slice(2, 3) : COMMITS).map(([subject, author, age]) => {
      const s = sha(subject);
      return { sha: s, short: s.slice(0, 7), subject, author, date: NOW - age, refs: [] };
    }),
    elapsedMs: Math.round(performance.now() - t0) + 38,
    truncated: false,
  };
}

function fileText(path: string, oldPath: string | null, sourceOf: (p: string) => string | null): FileDiffText {
  const lang = path.endsWith(".md") ? "markdown" : "typescript";
  const out = (before: string | null, after: string | null): FileDiffText => ({ path, oldPath, before, after, monacoLang: lang });
  if (path === LIFE) {
    const after = sourceOf(LIFE) ?? "";
    return out(lifecycleBefore(after).text, after);
  }
  if (path === RETRY) return out(null, RETRY_SRC);
  if (path === CONFIG) return out(sourceOf(CONFIG), CONFIG_AFTER);
  if (path === TERMINAL) return out(TERMINAL_SRC, null);
  if (path === DOC_NEW) return out(DOC_SRC, DOC_SRC.replace("# Diagnostics", "# Observability").replace("- `agent.run.completed`", "- `agent.run.completed`\n- `harness.retry_decision`"));
  const s = sourceOf(path);
  return out(s, s);
}

/** Line ranges of the base side of lifecycle.ts, for tests. */
export const _lifecycleBefore = lifecycleBefore;
