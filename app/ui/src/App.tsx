// App shell: top bar, source-tree sidebar and the main area (start screen, repo overview, or a
// file). Holds the repo-level state and listens to the backend's events.
import { useCallback, useEffect, useRef, useState } from "react";
import type { AiProgress } from "./ipc/bindings/AiProgress";
import type { AiStatus } from "./ipc/bindings/AiStatus";
import type { FileView } from "./ipc/bindings/FileView";
import type { IndexProgress } from "./ipc/bindings/IndexProgress";
import type { JobsState } from "./ipc/bindings/JobsState";
import type { ModuleMap } from "./ipc/bindings/ModuleMap";
import type { RepoSummary } from "./ipc/bindings/RepoSummary";
import type { SearchHit } from "./ipc/bindings/SearchHit";
import type { TreeNode } from "./ipc/bindings/TreeNode";
import { api, backend, invalidate } from "./ipc/api";
import { fmtInt, fmtMs, num, plural } from "./lib/format";
import { applyThemeChoice, loadThemeChoice, type ThemeChoice } from "./lib/theme";
import { Sidebar } from "./components/Sidebar";
import { SearchBox } from "./components/SearchBox";
import { AiPanel } from "./components/AiPanel";
import { RepoOverview } from "./components/RepoOverview";
import { StartScreen } from "./components/StartScreen";
import { Seg } from "./components/ui";
import { Workbench } from "./file/Workbench";
import { PlainFile } from "./file/PlainFile";

interface OpenFile {
  path: string;
  symbol: string | null;
  line: number | null;
  nonce: number;
}

const PHASE_TEXT = { walk: "walking", parse: "parsing", resolve: "resolving imports", graph: "building the graph", done: "done" } as const;

export function App() {
  const [repo, setRepo] = useState<RepoSummary | null>(null);
  const [tree, setTree] = useState<TreeNode | null>(null);
  const [moduleMap, setModuleMap] = useState<ModuleMap | null>(null);
  const [recent, setRecent] = useState<string[]>([]);
  const [file, setFile] = useState<OpenFile | null>(null);
  const [view, setView] = useState<FileView | null>(null);
  const [viewError, setViewError] = useState<string | null>(null);
  const [history, setHistory] = useState<OpenFile[]>([]);
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [jobs, setJobs] = useState<JobsState | null>(null);
  const [ai, setAi] = useState<AiStatus | null>(null);
  const [aiProgress, setAiProgress] = useState<AiProgress | null>(null);
  const [sidebar, setSidebar] = useState(true);
  const [theme, setTheme] = useState<ThemeChoice>(loadThemeChoice);
  const [busy, setBusy] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [mode, setMode] = useState<"tauri" | "mock" | null>(null);
  const fileRef = useRef<OpenFile | null>(null);
  fileRef.current = file;

  useEffect(() => applyThemeChoice(theme), [theme]);

  const refreshRepoData = useCallback(() => {
    void api.getTree().then(setTree);
    void api.getModuleMap().then(setModuleMap);
  }, []);

  // Backend events.
  useEffect(() => {
    const offs: Promise<() => void>[] = [
      api.listen("index_progress", setProgress),
      api.listen("index_done", (r) => {
        setRepo(r);
        refreshRepoData();
        setTimeout(() => setProgress((p) => (p?.phase === "done" ? null : p)), 4000);
      }),
      api.listen("job_progress", setJobs),
      api.listen("ai_progress", setAiProgress),
      api.listen("summary_ready", ({ path }) => {
        invalidate(path);
        if (fileRef.current?.path === path) void api.getFileView(path).then(setView);
      }),
    ];
    void backend().then((b) => setMode(b.kind));
    void api.recentRepos().then(setRecent);
    void api.aiStatus().then(setAi);
    return () => offs.forEach((p) => void p.then((off) => off()));
  }, [refreshRepoData]);

  const openRepo = useCallback(
    async (path: string) => {
      setBusy(true);
      setOpenError(null);
      try {
        const r = await api.openRepo(path);
        setRepo(r);
        setFile(null);
        setView(null);
        setHistory([]);
        invalidate();
        refreshRepoData();
        void api.getJobs().then(setJobs);
      } catch (e) {
        setOpenError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [refreshRepoData],
  );

  const pickRepo = useCallback(async () => {
    const p = await api.pickFolder();
    if (p) await openRepo(p);
  }, [openRepo]);

  const openFile = useCallback((path: string, opts?: { line?: number | null; symbol?: string | null }) => {
    setFile((cur) => {
      if (cur && cur.path !== path) setHistory((h) => [...h.slice(-49), cur]);
      return { path, symbol: opts?.symbol ?? null, line: opts?.line ?? null, nonce: (cur?.nonce ?? 0) + 1 };
    });
  }, []);

  const goBack = useCallback(() => {
    setHistory((h) => {
      const prev = h[h.length - 1];
      if (prev) setFile({ ...prev, nonce: prev.nonce + 1 });
      return h.slice(0, -1);
    });
  }, []);

  // Load the file view whenever another file opens.
  useEffect(() => {
    if (!file) return;
    let alive = true;
    setViewError(null);
    api
      .getFileView(file.path)
      .then((v) => alive && setView(v))
      .catch((e: unknown) => alive && setViewError(e instanceof Error ? e.message : String(e)));
    return () => {
      alive = false;
    };
  }, [file?.path]);

  // Deep link for the mock: ?open=<path> opens the first recent repo and that file.
  useEffect(() => {
    if (mode !== "mock") return;
    const p = new URLSearchParams(location.search).get("open");
    if (p === null) return;
    void api.recentRepos().then(async (r) => {
      if (r[0]) await openRepo(r[0]);
      if (p) openFile(p);
    });
  }, [mode]);

  const onSearchPick = useCallback((h: SearchHit) => openFile(h.path, { line: h.line, symbol: h.kind === "symbol" ? h.name : null }), [openFile]);

  const showView = view && file && view.path === file.path ? view : null;
  const indexing = progress && progress.phase !== "done";

  return (
    <div className="app">
      <header className="appbar">
        <button type="button" className="wordmark" onClick={() => setFile(null)} title={repo ? "Repo overview" : undefined} disabled={!repo}>
          Isoline
        </button>
        {repo && (
          <button type="button" className="reponame" onClick={() => setFile(null)} title={`${repo.root}: show the repo overview`}>
            {repo.name}
          </button>
        )}
        <button type="button" className="iso-btn" onClick={() => void pickRepo()} disabled={busy}>
          Open repo
        </button>
        {repo && (
          <button type="button" className="iso-btn" aria-pressed={sidebar} onClick={() => setSidebar(!sidebar)}>
            Files
          </button>
        )}
        {history.length > 0 && file && (
          <button type="button" className="iso-btn" onClick={goBack} title={`Back to ${history[history.length - 1].path}`}>
            Back
          </button>
        )}
        <SearchBox enabled={!!repo} onPick={onSearchPick} />
        <div className="status" aria-live="polite">
          {progress && (
            <span className="indexing" title={progress.current ?? undefined}>
              {indexing ? (
                <>
                  Indexing · {PHASE_TEXT[progress.phase]} {fmtInt(progress.done)} / {fmtInt(progress.total)}
                  <span className="pbar">
                    <span style={{ width: `${(progress.done / Math.max(1, progress.total)) * 100}%` }} />
                  </span>
                </>
              ) : (
                <>
                  Indexed {plural(progress.total, "file")} in {fmtMs(num(progress.elapsedMs))}
                </>
              )}
            </span>
          )}
          {jobs && repo && (
            <span className="jobs" title={jobs.pendingFiles.length ? `Pending: ${jobs.pendingFiles.slice(0, 8).join(", ")}${jobs.pendingFiles.length > 8 ? "…" : ""}` : `Cache: ${fmtInt(num(jobs.cacheHits))} hits, ${fmtInt(num(jobs.cacheMisses))} misses`}>
              {jobs.running || jobs.queued ? `Jobs · ${jobs.running} running, ${jobs.queued} queued` : `Jobs idle · ${Math.round(jobs.hitRate * 100)}% cache hits`}
              {jobs.failed > 0 && ` · ${jobs.failed} failed`}
            </span>
          )}
        </div>
        <AiPanel status={ai} setStatus={setAi} filePath={file?.path ?? null} progress={aiProgress} />
        <Seg
          label="Theme"
          value={theme}
          onChange={setTheme}
          options={[
            ["system", "System"],
            ["light", "Light"],
            ["dark", "Dark"],
          ]}
        />
      </header>
      <div className={`body${repo && sidebar ? " with-side" : ""}`}>
        {repo && sidebar && tree && <Sidebar tree={tree} openPath={file?.path ?? null} onOpen={(p) => openFile(p)} />}
        <main className="content">
          {!repo && <StartScreen recent={recent} onOpen={(p) => void openRepo(p)} onPick={() => void pickRepo()} busy={busy} error={openError} />}
          {repo && !file && <RepoOverview repo={repo} moduleMap={moduleMap} onModuleRenamed={refreshRepoData} />}
          {repo && file && viewError && (
            <div className="start">
              <p className="err">
                Could not open {file.path}: {viewError}
              </p>
            </div>
          )}
          {repo && file && !viewError && !showView && <div className="loading muted">Opening {file.path}…</div>}
          {showView && showView.tier === "full" && <Workbench key={showView.path} view={showView} focusName={file?.symbol} focusLine={file?.line} onOpenFile={openFile} />}
          {showView && showView.tier !== "full" && <PlainFile key={showView.path} view={showView} focusLine={file?.line} onOpenFile={openFile} />}
        </main>
      </div>
      {mode === "mock" && <div className="mock-badge" title="Running in a browser against the mock backend">Mock data</div>}
    </div>
  );
}
