// The branch button in the app bar: the current branch, how far it is from its upstream, and how
// many files have uncommitted changes. Opens a list of local and remote branches to switch to,
// create a branch from the current one, fetch, or compare.
import { useCallback, useRef, useState } from "react";
import type { GitBranch } from "../ipc/bindings/GitBranch";
import type { GitRefs } from "../ipc/bindings/GitRefs";
import type { GitStatus } from "../ipc/bindings/GitStatus";
import { api } from "../ipc/api";
import { fmtAgo, plural } from "../lib/format";
import { validBranchName } from "../lib/refs";
import { PickList, useDismiss, type PickItem } from "./PickList";
import { usePeek } from "./Peek";

export function branchText(s: GitStatus): string {
  return s.branch ?? (s.head ? `detached · ${s.head}` : "no commits yet");
}

export function BranchMenu({ status, onSwitched, onStatus, onCompare }: { status: GitStatus; onSwitched(s: GitStatus): void; onStatus(s: GitStatus): void; onCompare(base: string | null, head: string | null): void }) {
  const [open, setOpen] = useState(false);
  const [refs, setRefs] = useState<GitRefs | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const peek = usePeek();
  const close = useCallback(() => {
    setOpen(false);
    setError(null);
  }, []);
  useDismiss(open, close, box);

  const load = () => {
    setRefs(null);
    void api
      .gitRefs()
      .then(setRefs)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  };

  const run = async (label: string, f: () => Promise<GitStatus>, closeAfter: boolean) => {
    setBusy(label);
    setError(null);
    try {
      const s = await f();
      if (closeAfter) {
        setOpen(false);
        onSwitched(s);
      } else {
        onStatus(s);
        load();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const cur = branchText(status);
  const item = (b: GitBranch): PickItem => ({
    key: (b.remote ? "r:" : "l:") + b.name,
    section: b.remote ? "Remote branches" : "Local branches",
    label: <code>{b.name}</code>,
    text: b.name,
    current: b.current,
    caption: (
      <>
        {b.current && <b className="pick-curtag">current · </b>}
        {b.subject} · {fmtAgo(b.date)}
      </>
    ),
    onPick: () => {
      if (b.current) return;
      void run(`Switching to ${b.name}…`, () => api.gitSwitch(b.name, false), true);
    },
    action: b.current
      ? undefined
      : {
          label: "Compare",
          title: `Compare ${b.name} with ${cur}`,
          onSelect: () => {
            setOpen(false);
            onCompare(b.name, status.branch ?? status.head);
          },
        },
  });
  const items = refs ? refs.branches.map(item) : [];
  const create = (q: string): PickItem[] => {
    if (!q || !refs || refs.branches.some((b) => b.name === q || b.name.endsWith("/" + q)) || !validBranchName(q)) return [];
    return [
      {
        key: "create",
        section: "",
        label: (
          <>
            Create branch <code>{q}</code>
          </>
        ),
        text: q,
        caption: <>from {cur}</>,
        onPick: () => void run(`Creating ${q}…`, () => api.gitSwitch(q, true), true),
      },
    ];
  };

  const dirty = status.changed + status.untracked;
  return (
    <div className="menu branch-menu" ref={box}>
      <button
        type="button"
        className="iso-btn menu-btn branch-btn"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Branch ${cur}`}
        onClick={() => {
          if (!open) load();
          setOpen(!open);
          setError(null);
        }}
      >
        <span className="branch-name">{cur}</span>
        {(status.ahead > 0 || status.behind > 0) && (
          <span className="branch-ab" aria-label={`${status.ahead} ahead, ${status.behind} behind ${status.upstream ?? "upstream"}`}>
            {status.ahead > 0 && <>↑{status.ahead}</>}
            {status.behind > 0 && <>↓{status.behind}</>}
          </span>
        )}
        {dirty > 0 && (
          <span
            className="branch-dirty"
            {...peek.bind(() => ({
              kind: "text",
              title: "Uncommitted changes",
              body: [status.changed ? plural(status.changed, "changed file") : null, status.untracked ? plural(status.untracked, "new file") : null].filter(Boolean).join(", ") + ".",
            }))}
          >
            {dirty}
          </span>
        )}
        <span className="caret" aria-hidden="true">
          ▾
        </span>
      </button>
      {open && (
        <div className="popover branch-pop menu-left" role="dialog" aria-label="Branches">
          {status.operation && <p className="caption err">A {status.operation} is in progress. Finish or abort it before switching branches.</p>}
          {dirty > 0 && !status.operation && (
            <p className="caption">
              {plural(dirty, "file")} with uncommitted changes. Git carries them to the other branch, or refuses to switch if they conflict.
            </p>
          )}
          <PickList items={items} placeholder="Switch to a branch, or name a new one" extra={create} ariaLabel="Branches" loading={!refs && !error} empty="No branches yet." />
          {busy && <p className="caption">{busy}</p>}
          {error && <pre className="git-err">{error}</pre>}
          <div className="pick-foot">
            <button type="button" className="iso-btn" disabled={!!busy} onClick={() => void run("Fetching…", () => api.gitFetch(), false)}>
              Fetch
            </button>
            {status.upstream && <span className="caption">{status.upstream}</span>}
            <button
              type="button"
              className="iso-btn"
              onClick={() => {
                setOpen(false);
                onCompare(null, null);
              }}
            >
              Compare branches…
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
