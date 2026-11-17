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
import type { GitStatus } from "./ipc/bindings/GitStatus";
import { api, backend, invalidate } from "./ipc/api";
import { fmtInt, fmtMs, num, plural } from "./lib/format";
import { applyThemeChoice, loadThemeChoice, type ThemeChoice } from "./lib/theme";
import { Sidebar } from "./components/Sidebar";
import { SearchBox } from "./components/SearchBox";
import { AiPanel } from "./components/AiPanel";
import { RepoOverview } from "./components/RepoOverview";
import { StartScreen } from "./components/StartScreen";
import { Seg } from "./components/ui";
import { Menu } from "./components/Menu";
import { BranchMenu } from "./components/BranchMenu";
import { CompareView, type CompareRequest } from "./compare/CompareView";
import { PeekProvider, usePeek } from "./components/Peek";
import { Workbench, type WorkbenchLocation } from "./file/Workbench";
import { PlainFile } from "./file/PlainFile";
import { describe, EMPTY_HISTORY, goBack as historyBack, goForward as historyForward, push, type History, type Location } from "./lib/history";

interface OpenFile {
  path: string;
  symbol: string | null;
  line: number | null;
  nonce: number;
  /** Selection and scroll to restore (Back, Forward). */
  restore?: WorkbenchLocation | null;
}

export function App() {
  return (
    <PeekProvider>
      <Shell />
    </PeekProvider>
  );
}

/** Hold ⌘ (Ctrl elsewhere) to underline every go-to-definition target in the pane under the pointer. */
function useCmdHold() {
  useEffect(() => {
    let under: Element | null = null;
    let lit: Element | null = null;
    const isMac = navigator.platform.toLowerCase().includes("mac");
    const clear = () => {
      lit?.classList.remove("cmd-links");
      lit = null;
    };
    const light = () => {
      clear();
      lit = under?.closest(".pane-left, .pane-right, .iso-peek, .repo-page, .sidebar") ?? null;
      lit?.classList.add("cmd-links");
    };
    const onMove = (e: PointerEvent) => {
      under = e.target as Element;
      if (lit && (e.metaKey || e.ctrlKey)) light();
    };
    const onDown = (e: KeyboardEvent) => {
      if ((isMac && e.key === "Meta") || (!isMac && e.key === "Control")) light();
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.key === "Meta" || e.key === "Control") clear();
    };
    document.addEventListener("pointermove", onMove, true);
    document.addEventListener("keydown", onDown, true);
    document.addEventListener("keyup", onUp, true);
    window.addEventListener("blur", clear);
    return () => {
      document.removeEventListener("pointermove", onMove, true);
      document.removeEventListener("keydown", onDown, true);
      document.removeEventListener("keyup", onUp, true);
      window.removeEventListener("blur", clear);
    };
  }, []);
}

/** Whether a file path is in the tree. */
function treeHas(tree: TreeNode, path: string): boolean {
  let n: TreeNode | undefined = tree;
  for (const part of path.split("/")) {
    n = n.children.find((c) => c.name === part);
    if (!n) return false;
  }
  return !n.isDir;
}

const PHASE_TEXT = { walk: "walking", parse: "parsing", resolve: "resolving imports", graph: "building the graph", done: "done" } as const;

function Shell() {
  const [repo, setRepo] = useState<RepoSummary | null>(null);
  const [tree, setTree] = useState<TreeNode | null>(null);
  const [moduleMap, setModuleMap] = useState<ModuleMap | null>(null);
  const [recent, setRecent] = useState<string[]>([]);
  const [file, setFile] = useState<OpenFile | null>(null);
  const [view, setView] = useState<FileView | null>(null);
  const [viewError, setViewError] = useState<string | null>(null);
  const [history, setHistory] = useState<History>(EMPTY_HISTORY);
  const [aiOpen, setAiOpen] = useState(0);
  const [themeOpen, setThemeOpen] = useState(false);
  const locationOf = useRef<(() => WorkbenchLocation) | null>(null);
  const pickRepoRef = useRef<() => Promise<void>>(async () => {});
  const peek = usePeek();
  useCmdHold();
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [jobs, setJobs] = useState<JobsState | null>(null);
  const [ai, setAi] = useState<AiStatus | null>(null);
  const [aiProgress, setAiProgress] = useState<AiProgress | null>(null);
  const [sidebar, setSidebar] = useState(true);
  const [theme, setTheme] = useState<ThemeChoice>(loadThemeChoice);
  const [busy, setBusy] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [mode, setMode] = useState<"tauri" | "mock" | "http" | null>(null);
  const [git, setGit] = useState<GitStatus | null>(null);
  const [compareReq, setCompareReq] = useState<CompareRequest | null>(null);
  const [compareOn, setCompareOn] = useState(false);
  const fileRef = useRef<OpenFile | null>(null);
  fileRef.current = file;
  const repoRef = useRef<RepoSummary | null>(null);
  repoRef.current = repo;
  /** The first parse progress after opening a repo also refetches the tree (in case tree_changed was missed). */
  const treeWanted = useRef(false);

  useEffect(() => applyThemeChoice(theme), [theme]);

  const refreshTree = useCallback(() => {
    void api.getTree().then((t) => {
      setTree(t);
      // A branch switch (or a delete) took the open file away: back to the overview.
      const open = fileRef.current?.path;
      if (open && t.children.length > 0 && !treeHas(t, open)) {
        setFile(null);
        setView(null);
      }
    });
  }, []);
  const refreshRepoData = useCallback(() => {
    refreshTree();
    void api.getModuleMap().then(setModuleMap);
  }, [refreshTree]);
  const refreshGit = useCallback(() => {
    if (!repoRef.current) return;
    void api
      .gitStatus()
      .then(setGit)
      .catch(() => setGit(null));
  }, []);

  // Backend events.
  useEffect(() => {
    let treeTimer: ReturnType<typeof setTimeout> | undefined;
    let gitTimer: ReturnType<typeof setTimeout> | undefined;
    const offs: Promise<() => void>[] = [
      api.listen("index_progress", (p) => {
        setProgress(p);
        if (p.phase === "parse" && treeWanted.current) {
          treeWanted.current = false;
          refreshTree();
        }
      }),
      // The walk listed the files (long before indexing ends), or files came and went.
      api.listen("tree_changed", () => {
        treeWanted.current = false;
        clearTimeout(treeTimer);
        treeTimer = setTimeout(refreshTree, 150);
      }),
      api.listen("git_changed", setGit),
      api.listen("index_done", (r) => {
        setRepo(r);
        refreshRepoData();
        // A file opened mid-index was shown before its imports resolved; refresh it now.
        const open = fileRef.current?.path;
        if (open) {
          invalidate(open);
          void api.getFileView(open).then(setView).catch(() => {});
        }
        setTimeout(() => setProgress((p) => (p?.phase === "done" ? null : p)), 4000);
      }),
      api.listen("job_progress", setJobs),
      // Files saved in an editor: refresh the open file when it is among them.
      api.listen("files_changed", (paths) => {
        clearTimeout(gitTimer);
        gitTimer = setTimeout(refreshGit, 800);
        const open = fileRef.current?.path;
        if (open && paths.includes(open)) {
          invalidate(open);
          void api.getFileView(open).then(setView).catch(() => {});
        }
      }),
      api.listen("ai_progress", setAiProgress),
      api.listen("summary_ready", ({ path }) => {
        invalidate(path);
        if (fileRef.current?.path === path) void api.getFileView(path).then(setView);
      }),
    ];
    void backend().then((b) => setMode(b.kind));
    void api.recentRepos().then(setRecent);
    void api.aiStatus().then(setAi);
    return () => {
      clearTimeout(treeTimer);
      clearTimeout(gitTimer);
      offs.forEach((p) => void p.then((off) => off()));
    };
  }, [refreshRepoData, refreshTree, refreshGit]);

  // Git status: on focus, and every 10 s while the window is visible.
  useEffect(() => {
    if (!repo) return;
    refreshGit();
    const onFocus = () => refreshGit();
    const t = setInterval(() => document.visibilityState === "visible" && refreshGit(), 10_000);
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, [repo?.root, refreshGit]);

  /** After a branch switch: new tree, module map and status; the open file reloads or closes. */
  const onSwitched = useCallback(
    (s: GitStatus) => {
      setGit(s);
      refreshRepoData();
      const open = fileRef.current?.path;
      if (open) {
        invalidate(open);
        void api
          .getFileView(open)
          .then(setView)
          .catch(() => {
            setFile(null);
            setView(null);
          });
      }
    },
    [refreshRepoData],
  );

  const openCompare = useCallback((base: string | null, head: string | null) => {
    peek.close();
    setCompareReq((r) => ({ base, head, nonce: (r?.nonce ?? 0) + 1 }));
    setCompareOn(true);
  }, [peek]);

  const openRepo = useCallback(
    async (path: string) => {
      setBusy(true);
      setOpenError(null);
      try {
        treeWanted.current = true;
        setTree(null);
        setGit(null);
        setCompareReq(null);
        setCompareOn(false);
        const r = await api.openRepo(path);
        setRepo(r);
        setFile(null);
        setView(null);
        setHistory(EMPTY_HISTORY);
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
  pickRepoRef.current = pickRepo;

  /** Where the reader is now, as a history entry. */
  const here = useCallback((): Location | null => {
    const f = fileRef.current;
    if (!f) return null;
    const l = locationOf.current?.() ?? { selection: null, scroll: 0 };
    return { path: f.path, symbol: f.symbol, line: f.line, selection: l.selection, scroll: l.scroll };
  }, []);

  const pushHere = useCallback(() => {
    const h = here();
    if (h) setHistory((x) => push(x, h));
  }, [here]);

  /** Every file open and every go to definition records where the reader was. */
  const openFile = useCallback(
    (path: string, opts?: { line?: number | null; symbol?: string | null }) => {
      pushHere();
      peek.close();
      setCompareOn(false);
      setFile((cur) => ({ path, symbol: opts?.symbol ?? null, line: opts?.line ?? null, nonce: (cur?.nonce ?? 0) + 1 }));
    },
    [pushHere, peek],
  );

  const travel = useCallback(
    (dir: "back" | "forward") => {
      const cur = here();
      if (!cur) return;
      const r = dir === "back" ? historyBack(history, cur) : historyForward(history, cur);
      if (!r) return;
      setHistory(r.history);
      peek.close();
      setCompareOn(false);
      setFile((f) => ({ path: r.to.path, symbol: null, line: null, nonce: (f?.nonce ?? 0) + 1, restore: { selection: r.to.selection, scroll: r.to.scroll } }));
    },
    [history, here, peek],
  );

  // ⌘[ and ⌘] (Ctrl elsewhere), the mouse's back and forward buttons, ⌘O to open a repo.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      if (e.key === "[") (e.preventDefault(), travel("back"));
      else if (e.key === "]") (e.preventDefault(), travel("forward"));
      else if (e.key === "o") (e.preventDefault(), void pickRepoRef.current());
    };
    const onMouse = (e: MouseEvent) => {
      if (e.button === 3) (e.preventDefault(), travel("back"));
      else if (e.button === 4) (e.preventDefault(), travel("forward"));
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mouseup", onMouse);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mouseup", onMouse);
    };
  }, [travel]);

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

  // Launch arguments: `isoline <repo> [file]` in the app, `?repo=`/`?open=` in a browser.
  useEffect(() => {
    if (mode === null) return;
    void api.launchArgs().then(async ({ repo, open }) => {
      if (repo === null && open === null) return;
      const root = repo ?? (await api.recentRepos())[0];
      if (root) await openRepo(root);
      if (open) openFile(open);
    });
  }, [mode]);

  const onSearchPick = useCallback((h: SearchHit) => openFile(h.path, { line: h.line, symbol: h.kind === "symbol" ? h.name : null }), [openFile]);

  const showView = view && file && view.path === file.path ? view : null;
  const indexing = !!progress && progress.phase !== "done";
  const justIndexed = !!progress && progress.phase === "done";
  const jobsRunning = !!jobs && !!repo && jobs.running > 0;
  const registerLocation = useCallback((f: (() => WorkbenchLocation) | null) => {
    locationOf.current = f;
  }, []);

  return (
    <div className="app">
      <header className="appbar">
        {/* The native window's title bar already says "Isoline"; the wordmark is for the browser. */}
        {mode !== "tauri" && (
          <button
            type="button"
            className="wordmark"
            onClick={() => {
              setFile(null);
              setCompareOn(false);
            }}
            disabled={!repo}
          >
            Isoline
          </button>
        )}
        {repo && (
          <Menu
            className="repo-menu"
            label={<span className="reponame-label">{repo.name}</span>}
            ariaLabel={`Repo ${repo.name}`}
            align="left"
            items={[
              {
                kind: "action",
                label: "Repo overview",
                onSelect: () => {
                  setFile(null);
                  setCompareOn(false);
                },
              },
              { kind: "action", label: "Open repo…", hint: "⌘O", onSelect: () => void pickRepo() },
              ...(recent.filter((r) => r !== repo.root).length ? [{ kind: "sep" as const }, { kind: "label" as const, label: "Recent repos" }] : []),
              ...recent.filter((r) => r !== repo.root).map((r) => ({ kind: "action" as const, label: <code>{r}</code>, onSelect: () => void openRepo(r) })),
            ]}
          />
        )}
        {repo && git?.isRepo && <BranchMenu status={git} onSwitched={onSwitched} onStatus={setGit} onCompare={openCompare} />}
        {repo && (
          <button type="button" className="iso-btn" aria-pressed={sidebar} onClick={() => setSidebar(!sidebar)}>
            Files
          </button>
        )}
        {repo && git?.isRepo && (
          <button
            type="button"
            className="iso-btn"
            aria-pressed={compareOn}
            onClick={() => {
              if (compareOn) setCompareOn(false);
              else if (compareReq) setCompareOn(true);
              else openCompare(null, null);
            }}
          >
            Compare
          </button>
        )}
        {history.back.length > 0 && (
          <button type="button" className="iso-btn quiet" onClick={() => travel("back")} aria-label={`Back to ${describe(history.back[history.back.length - 1])}`} title={`Back to ${describe(history.back[history.back.length - 1])} (⌘[)`}>
            Back
          </button>
        )}
        {history.forward.length > 0 && (
          <button type="button" className="iso-btn quiet" onClick={() => travel("forward")} aria-label={`Forward to ${describe(history.forward[history.forward.length - 1])}`} title={`Forward to ${describe(history.forward[history.forward.length - 1])} (⌘])`}>
            Forward
          </button>
        )}
        <SearchBox enabled={!!repo} onPick={onSearchPick} />
        <div className="status" aria-live="polite">
          {mode === "mock" && (
            <span className="mock-tag" {...peek.bind(() => ({ kind: "text", title: "Mock data", body: "Running in a browser against the mock backend: the data is the prototype's sample repo." }))}>
              mock data
            </span>
          )}
          {(indexing || justIndexed || jobsRunning) && progress !== undefined && (
            <span
              className="indexing"
              {...peek.bind(() =>
                jobs
                  ? {
                      kind: "text",
                      title: "Background work",
                      body: (
                        <>
                          {jobs.running} running, {jobs.queued} queued, {jobs.failed} failed. Cache: {fmtInt(num(jobs.cacheHits))} hits, {fmtInt(num(jobs.cacheMisses))} misses ({Math.round(jobs.hitRate * 100)}%).
                          {progress?.current ? <div className="muted">Now: {progress.current}</div> : null}
                        </>
                      ),
                    }
                  : null,
              )}
            >
              {indexing && progress ? (
                <>
                  Indexing · {PHASE_TEXT[progress.phase]} {fmtInt(progress.done)} / {fmtInt(progress.total)}
                  <span className="pbar">
                    <span style={{ width: `${(progress.done / Math.max(1, progress.total)) * 100}%` }} />
                  </span>
                </>
              ) : justIndexed && progress ? (
                <>
                  Indexed {plural(progress.total, "file")} in {fmtMs(num(progress.elapsedMs))}
                </>
              ) : null}
              {jobsRunning && jobs && <> {indexing || justIndexed ? "· " : ""}Jobs · {jobs.running} running</>}
            </span>
          )}
        </div>
        {repo && <AiPanel status={ai} setStatus={setAi} filePath={file?.path ?? null} progress={aiProgress} openSignal={aiOpen} />}
        <div className="theme-wrap">
          <button type="button" className="iso-btn quiet" aria-expanded={themeOpen} onClick={() => setThemeOpen(!themeOpen)}>
            Theme
          </button>
          {themeOpen && (
            <div className="popover theme-pop" role="dialog" aria-label="Theme" onKeyDown={(e) => e.key === "Escape" && setThemeOpen(false)}>
              <Seg
                label="Theme"
                value={theme}
                onChange={(t) => {
                  setTheme(t);
                  setThemeOpen(false);
                }}
                options={[
                  ["system", "System"],
                  ["light", "Light"],
                  ["dark", "Dark"],
                ]}
              />
            </div>
          )}
        </div>
      </header>
      <div className={`body${repo && sidebar ? " with-side" : ""}`}>
        {repo && sidebar && tree && <Sidebar key={repo.root} repo={repo.root} tree={tree} openPath={file?.path ?? null} onOpen={(p) => openFile(p)} emptyText={progress?.phase === "done" ? "No files to show." : "Listing files…"} />}
        <main className="content">
          {!repo && <StartScreen recent={recent} onOpen={(p) => void openRepo(p)} onPick={() => void pickRepo()} busy={busy} error={openError} />}
          {repo && compareReq && (
            <div className="compare-host" hidden={!compareOn}>
              <CompareView status={git} moduleMap={moduleMap} request={compareReq} onOpenFile={(p, line) => openFile(p, { line })} />
            </div>
          )}
          {repo && !compareOn && !file && <RepoOverview repo={repo} moduleMap={moduleMap} progress={indexing ? progress : null} onModuleRenamed={refreshRepoData} />}
          {repo && !compareOn && file && viewError && (
            <div className="start">
              <p className="err">
                Could not open {file.path}: {viewError}
              </p>
            </div>
          )}
          {repo && !compareOn && file && !viewError && !showView && <div className="loading muted">Opening {file.path}…</div>}
          {!compareOn && showView && showView.tier === "full" && (
            <Workbench
              key={showView.path}
              view={showView}
              focusName={file?.symbol}
              focusLine={file?.line}
              focusNonce={file?.nonce}
              restore={file?.restore ?? null}
              restoreNonce={file?.restore ? file.nonce : 0}
              onOpenFile={openFile}
              onPushHistory={pushHere}
              registerLocation={registerLocation}
              onOpenAi={() => setAiOpen((n) => n + 1)}
            />
          )}
          {!compareOn && showView && showView.tier !== "full" && <PlainFile key={showView.path} view={showView} focusLine={file?.line} onOpenFile={openFile} />}
        </main>
      </div>
    </div>
  );
}
