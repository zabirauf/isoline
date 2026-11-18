// Typed wrappers for every IPC command and event in CONTRACTS.md section 4.
// Inside Tauri (window.__TAURI_INTERNALS__ present) calls go to the Rust backend through
// `invoke`/`listen`; in a plain browser they go to the mock backend in ./mock, which serves the
// prototype's data. The mock is loaded with a dynamic import so it never ships in the hot path.
import type { AiEstimate } from "./bindings/AiEstimate";
import type { AiProgress } from "./bindings/AiProgress";
import type { AiRunHandle } from "./bindings/AiRunHandle";
import type { AiScope } from "./bindings/AiScope";
import type { AiStatus } from "./bindings/AiStatus";
import type { FileView } from "./bindings/FileView";
import type { FlowUnit } from "./bindings/FlowUnit";
import type { Impact } from "./bindings/Impact";
import type { IndexProgress } from "./bindings/IndexProgress";
import type { JobsState } from "./bindings/JobsState";
import type { ModuleInfo } from "./bindings/ModuleInfo";
import type { ModuleMap } from "./bindings/ModuleMap";
import type { RepoSummary } from "./bindings/RepoSummary";
import type { SearchHit } from "./bindings/SearchHit";
import type { SourceSlice } from "./bindings/SourceSlice";
import type { TreeNode } from "./bindings/TreeNode";
import type { Comparison } from "./bindings/Comparison";
import type { FileDiffText } from "./bindings/FileDiffText";
import type { GitRefs } from "./bindings/GitRefs";
import type { GitStatus } from "./bindings/GitStatus";
import type { PullRequests } from "./bindings/PullRequests";

export interface AiJobDone {
  jobId: number;
  stored: number;
  failed: number;
  costUsd: number;
  error: string | null;
}

/** Payloads of the events the backend emits. */
export interface BackendEvents {
  index_progress: IndexProgress;
  index_done: RepoSummary;
  ai_progress: AiProgress;
  /** An AI run ended, after all of its passes. `error` says why it stopped or failed. */
  ai_job_done: AiJobDone;
  summary_ready: { path: string };
  job_progress: JobsState;
  /** Repo-relative paths whose facts changed after a save (watcher). */
  files_changed: string[];
  /** The file set changed: right after the first walk lists the files, and on adds and removes. */
  tree_changed: null;
  /** HEAD or the branch changed (switch, fetch, or outside the app). */
  git_changed: GitStatus;
  /** A device-flow sign-in finished: signed in, or stopped (the status says why). */
  ai_status: AiStatus;
}
export type EventName = keyof BackendEvents;
export type Unlisten = () => void;
export interface LaunchArgs {
  repo: string | null;
  open: string | null;
}

function urlLaunchArgs(): LaunchArgs {
  const q = new URLSearchParams(typeof window === "undefined" ? "" : window.location.search);
  return { repo: q.get("repo"), open: q.get("open") };
}

export interface Backend {
  readonly kind: "tauri" | "mock" | "http";
  openRepo(path: string): Promise<RepoSummary>;
  getTree(): Promise<TreeNode>;
  getFileView(path: string): Promise<FileView>;
  getFlow(path: string, unit: string): Promise<FlowUnit>;
  getSource(path: string, start?: number, end?: number): Promise<SourceSlice>;
  getImpact(symbol: string, transitive: boolean): Promise<Impact>;
  getModuleMap(): Promise<ModuleMap>;
  search(query: string, limit?: number): Promise<SearchHit[]>;
  aiStatus(): Promise<AiStatus>;
  aiSignIn(): Promise<AiStatus>;
  aiSignInCancel(): Promise<AiStatus>;
  aiSignOut(): Promise<AiStatus>;
  aiEstimate(scope: AiScope, path?: string): Promise<AiEstimate>;
  aiRun(scope: AiScope, path: string | undefined, budgetUsd: number): Promise<AiRunHandle>;
  aiEnable(enabled: boolean): Promise<AiStatus>;
  getJobs(): Promise<JobsState>;
  renameModule(id: string, name?: string, color?: string): Promise<ModuleInfo>;
  recentRepos(): Promise<string[]>;
  /** Repo and file to open at launch: command-line arguments in Tauri, `?repo=`/`?open=` elsewhere. */
  launchArgs(): Promise<LaunchArgs>;
  gitStatus(): Promise<GitStatus>;
  gitRefs(): Promise<GitRefs>;
  /** Switch to a branch; `origin/foo` makes or reuses a local tracking branch `foo`. */
  gitSwitch(name: string, create: boolean): Promise<GitStatus>;
  gitFetch(): Promise<GitStatus>;
  gitPrs(): Promise<PullRequests>;
  /** `head` may be "worktree"; with `pr`, base and head are ignored. */
  compare(base: string, head: string, threeDot: boolean, pr?: number): Promise<Comparison>;
  compareFile(base: string, head: string, path: string, oldPath?: string): Promise<FileDiffText>;
  /** Folder picker: the dialog plugin in Tauri, a fixed answer in the mock. */
  pickFolder(): Promise<string | null>;
  listen<E extends EventName>(event: E, cb: (payload: BackendEvents[E]) => void): Promise<Unlisten>;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function createTauriBackend(): Promise<Backend> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  // Tauri maps camelCase argument names to the Rust command's snake_case parameters.
  const call = <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args);
  return {
    kind: "tauri",
    openRepo: (path) => call("open_repo", { path }),
    getTree: () => call("get_tree"),
    getFileView: (path) => call("get_file_view", { path }),
    getFlow: (path, unit) => call("get_flow", { path, unit }),
    getSource: (path, start, end) => call("get_source", { path, start: start ?? null, end: end ?? null }),
    getImpact: (symbol, transitive) => call("get_impact", { symbol, transitive }),
    getModuleMap: () => call("get_module_map"),
    search: (query, limit) => call("search", { query, limit: limit ?? null }),
    aiStatus: () => call("ai_status"),
    aiSignIn: () => call("ai_sign_in"),
    aiSignInCancel: () => call("ai_sign_in_cancel"),
    aiSignOut: () => call("ai_sign_out"),
    aiEstimate: (scope, path) => call("ai_estimate", { scope, path: path ?? null }),
    aiRun: (scope, path, budgetUsd) => call("ai_run", { scope, path: path ?? null, budgetUsd }),
    aiEnable: (enabled) => call("ai_enable", { enabled }),
    getJobs: () => call("get_jobs"),
    renameModule: (id, name, color) => call("rename_module", { id, name: name ?? null, color: color ?? null }),
    recentRepos: () => call("recent_repos"),
    gitStatus: () => call("git_status"),
    gitRefs: () => call("git_refs"),
    gitSwitch: (name, create) => call("git_switch", { name, create }),
    gitFetch: () => call("git_fetch"),
    gitPrs: () => call("git_prs"),
    compare: (base, head, threeDot, pr) => call("compare", { base, head, threeDot, pr: pr ?? null }),
    compareFile: (base, head, path, oldPath) => call("compare_file", { base, head, path, oldPath: oldPath ?? null }),
    launchArgs: () => call("launch_args"),
    pickFolder: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, title: "Open repo" });
      return typeof picked === "string" ? picked : null;
    },
    listen: async (event, cb) => listen(event, (e) => cb(e.payload as BackendEvents[typeof event])),
  };
}

/**
 * Development and test bridge: the same commands over HTTP to `iso-cli serve`, which binds to
 * 127.0.0.1 only and serves this UI from the same origin. Selected with `?backend=http`. Lets a
 * plain browser (and Playwright) drive the real engine; the desktop app never uses it.
 */
function createHttpBackend(): Backend {
  const call = async <T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> => {
    const res = await fetch(`./invoke/${cmd}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(args) });
    const body = await res.json();
    if (!res.ok) throw new Error(typeof body?.error === "string" ? body.error : `${cmd} failed`);
    return body as T;
  };
  let source: EventSource | null = null;
  const handlers = new Map<string, Set<(p: unknown) => void>>();
  const ensureSource = () => {
    if (source) return;
    source = new EventSource("./events");
    source.onmessage = (m) => {
      const { event, payload } = JSON.parse(m.data) as { event: string; payload: unknown };
      handlers.get(event)?.forEach((h) => h(payload));
    };
  };
  return {
    kind: "http",
    openRepo: (path) => call("open_repo", { path }),
    getTree: () => call("get_tree"),
    getFileView: (path) => call("get_file_view", { path }),
    getFlow: (path, unit) => call("get_flow", { path, unit }),
    getSource: (path, start, end) => call("get_source", { path, start: start ?? null, end: end ?? null }),
    getImpact: (symbol, transitive) => call("get_impact", { symbol, transitive }),
    getModuleMap: () => call("get_module_map"),
    search: (query, limit) => call("search", { query, limit: limit ?? null }),
    aiStatus: () => call("ai_status"),
    aiSignIn: () => call("ai_sign_in"),
    aiSignInCancel: () => call("ai_sign_in_cancel"),
    aiSignOut: () => call("ai_sign_out"),
    aiEstimate: (scope, path) => call("ai_estimate", { scope, path: path ?? null }),
    aiRun: (scope, path, budgetUsd) => call("ai_run", { scope, path: path ?? null, budgetUsd }),
    aiEnable: (enabled) => call("ai_enable", { enabled }),
    getJobs: () => call("get_jobs"),
    renameModule: (id, name, color) => call("rename_module", { id, name: name ?? null, color: color ?? null }),
    recentRepos: () => call("recent_repos"),
    gitStatus: () => call("git_status"),
    gitRefs: () => call("git_refs"),
    gitSwitch: (name, create) => call("git_switch", { name, create }),
    gitFetch: () => call("git_fetch"),
    gitPrs: () => call("git_prs"),
    compare: (base, head, threeDot, pr) => call("compare", { base, head, threeDot, pr: pr ?? null }),
    compareFile: (base, head, path, oldPath) => call("compare_file", { base, head, path, oldPath: oldPath ?? null }),
    launchArgs: async () => urlLaunchArgs(),
    pickFolder: async () => window.prompt("Repo folder") || null,
    listen: async (event, cb) => {
      ensureSource();
      const set = handlers.get(event) ?? new Set();
      const h = cb as (p: unknown) => void;
      set.add(h);
      handlers.set(event, set);
      return () => set.delete(h);
    },
  };
}

function wantsHttp(): boolean {
  return typeof window !== "undefined" && new URLSearchParams(window.location.search).get("backend") === "http";
}

let backendPromise: Promise<Backend> | null = null;

/**
 * Opens a GitHub page in the default browser. The desktop app's webview does not follow
 * target="_blank" links, so it goes through the `open_url` command (github.com only).
 */
export async function openExternal(url: string): Promise<void> {
  const b = await backend();
  if (b.kind === "tauri") {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("open_url", { url });
  } else {
    window.open(url, "_blank", "noopener");
  }
}

export function backend(): Promise<Backend> {
  if (!backendPromise)
    backendPromise = isTauri()
      ? createTauriBackend()
      : wantsHttp()
        ? Promise.resolve(createHttpBackend())
        : import("./mock").then((m) => m.createMockBackend());
  return backendPromise;
}

/** Same commands as `Backend`, resolving the backend on first use. */
export const api = {
  openRepo: async (path: string) => (await backend()).openRepo(path),
  getTree: async () => (await backend()).getTree(),
  getFileView: async (path: string) => (await backend()).getFileView(path),
  getFlow: async (path: string, unit: string) => (await backend()).getFlow(path, unit),
  getSource: async (path: string, start?: number, end?: number) => (await backend()).getSource(path, start, end),
  getImpact: async (symbol: string, transitive: boolean) => (await backend()).getImpact(symbol, transitive),
  getModuleMap: async () => (await backend()).getModuleMap(),
  search: async (query: string, limit?: number) => (await backend()).search(query, limit),
  aiStatus: async () => (await backend()).aiStatus(),
  aiSignIn: async () => (await backend()).aiSignIn(),
  aiSignInCancel: async () => (await backend()).aiSignInCancel(),
  aiSignOut: async () => (await backend()).aiSignOut(),
  aiEstimate: async (scope: AiScope, path?: string) => (await backend()).aiEstimate(scope, path),
  aiRun: async (scope: AiScope, path: string | undefined, budgetUsd: number) => (await backend()).aiRun(scope, path, budgetUsd),
  aiEnable: async (enabled: boolean) => (await backend()).aiEnable(enabled),
  getJobs: async () => (await backend()).getJobs(),
  renameModule: async (id: string, name?: string, color?: string) => (await backend()).renameModule(id, name, color),
  recentRepos: async () => (await backend()).recentRepos(),
  pickFolder: async () => (await backend()).pickFolder(),
  gitStatus: async () => (await backend()).gitStatus(),
  gitRefs: async () => (await backend()).gitRefs(),
  gitSwitch: async (name: string, create: boolean) => (await backend()).gitSwitch(name, create),
  gitFetch: async () => (await backend()).gitFetch(),
  gitPrs: async () => (await backend()).gitPrs(),
  compare: async (base: string, head: string, threeDot: boolean, pr?: number) => (await backend()).compare(base, head, threeDot, pr),
  compareFile: async (base: string, head: string, path: string, oldPath?: string) => (await backend()).compareFile(base, head, path, oldPath),
  launchArgs: async () => (await backend()).launchArgs(),
  listen: async <E extends EventName>(event: E, cb: (payload: BackendEvents[E]) => void) => (await backend()).listen(event, cb),
};

// ---------- caches for data that does not change while a file's content hash is the same ----------
const sourceCache = new Map<string, Promise<SourceSlice>>();
const flowCache = new Map<string, Promise<FlowUnit>>();

/** get_source, memoized per path and range until `invalidate(path)`. */
export function cachedSource(path: string, start?: number, end?: number): Promise<SourceSlice> {
  const k = `${path}\u0000${start ?? ""}\u0000${end ?? ""}`;
  let p = sourceCache.get(k);
  if (!p) {
    p = api.getSource(path, start, end);
    p.catch(() => sourceCache.delete(k));
    sourceCache.set(k, p);
  }
  return p;
}

export function cachedFlow(path: string, unit: string): Promise<FlowUnit> {
  const k = `${path}\u0000${unit}`;
  let p = flowCache.get(k);
  if (!p) {
    p = api.getFlow(path, unit);
    p.catch(() => flowCache.delete(k));
    flowCache.set(k, p);
  }
  return p;
}

export function invalidate(path?: string) {
  for (const c of [sourceCache, flowCache] as Map<string, unknown>[]) {
    for (const k of [...c.keys()]) if (!path || k.startsWith(path + "\u0000")) c.delete(k);
  }
}
