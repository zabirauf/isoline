// The compare view's pickers: one revision (branch, tag, commit, or the working tree), and a pull
// request from GitHub through `gh`.
import { useCallback, useRef, useState } from "react";
import type { GitRefs } from "../ipc/bindings/GitRefs";
import type { PullRequest } from "../ipc/bindings/PullRequest";
import type { PullRequests } from "../ipc/bindings/PullRequests";
import { api } from "../ipc/api";
import { fmtAgo, fmtInt } from "../lib/format";
import { refLabel, WORKTREE } from "../lib/refs";
import { PickList, useDismiss, type PickItem } from "../components/PickList";

export function RefPicker({ role, value, refs, onPick, allowWorktree }: { role: "Base" | "Head"; value: string | null; refs: GitRefs | null; onPick(spec: string): void; allowWorktree: boolean }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setOpen(false), []);
  useDismiss(open, close, box);
  const pick = (s: string) => {
    setOpen(false);
    onPick(s);
  };
  const items: PickItem[] = [];
  if (allowWorktree) items.push({ key: "wt", section: "", label: "Working tree", text: "working tree uncommitted", caption: "uncommitted changes on top of HEAD", current: value === WORKTREE, onPick: () => pick(WORKTREE) });
  for (const b of refs?.branches ?? [])
    items.push({ key: "b:" + b.name, section: b.remote ? "Remote branches" : "Local branches", label: <code>{b.name}</code>, text: b.name, current: value === b.name, caption: <>{b.current ? <b className="pick-curtag">current · </b> : null}{b.subject} · {fmtAgo(b.date)}</>, onPick: () => pick(b.name) });
  for (const t of refs?.tags ?? []) items.push({ key: "t:" + t.name, section: "Tags", label: <code>{t.name}</code>, text: t.name, current: value === t.name, caption: <>{t.sha} · {fmtAgo(t.date)}</>, onPick: () => pick(t.name) });
  for (const c of refs?.commits ?? [])
    items.push({
      key: "c:" + c.sha,
      section: "Recent commits",
      label: <code>{c.short}</code>,
      text: `${c.short} ${c.sha} ${c.subject} ${c.author}`,
      current: value === c.sha,
      caption: (
        <>
          {c.subject} · {c.author} · {fmtAgo(c.date)}
        </>
      ),
      onPick: () => pick(c.sha),
    });
  // Anything git can resolve: HEAD~3, a sha from elsewhere, a remote branch not fetched in the list.
  const free = (q: string): PickItem[] =>
    q && !items.some((i) => i.text === q) && !/\s/.test(q)
      ? [{ key: "free", section: "", label: <>Use <code>{q}</code></>, text: q, caption: "any revision git understands", onPick: () => pick(q) }]
      : [];
  const label = value ? refLabel(value, refs) : "Choose…";
  return (
    <div className="menu ref-picker" ref={box}>
      <button type="button" className="iso-btn menu-btn ref-btn" aria-haspopup="dialog" aria-expanded={open} aria-label={`${role}: ${label}`} onClick={() => setOpen(!open)}>
        <span className="ref-role">{role}</span>
        <span className={value === WORKTREE ? "ref-val" : "ref-val mono"}>{label}</span>
        <span className="caret" aria-hidden="true">
          ▾
        </span>
      </button>
      {open && (
        <div className="popover branch-pop menu-left" role="dialog" aria-label={`${role} revision`}>
          <PickList items={items} placeholder="Branch, tag or commit" extra={free} ariaLabel={`${role} revisions`} loading={!refs} empty="No branches or commits yet." />
        </div>
      )}
    </div>
  );
}

export function PrPicker({ value, onPick, github }: { value: PullRequest | null; onPick(pr: PullRequest): void; github: boolean }) {
  const [open, setOpen] = useState(false);
  const [prs, setPrs] = useState<PullRequests | null>(null);
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setOpen(false), []);
  useDismiss(open, close, box);
  const load = () => {
    setError(null);
    void api
      .gitPrs()
      .then(setPrs)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  };
  const items: PickItem[] = (prs?.items ?? []).map((p) => ({
    key: String(p.number),
    section: "Open pull requests",
    label: (
      <>
        <span className="pr-num">#{p.number}</span> {p.title}
        {p.draft && <span className="tier-mini pr-draft">draft</span>}
      </>
    ),
    text: `#${p.number} ${p.number} ${p.title} ${p.author} ${p.headRef}`,
    current: value?.number === p.number,
    caption: (
      <>
        {p.author} · <code>{p.headRef}</code> → <code>{p.baseRef}</code> · <span className="c-add">+{fmtInt(p.additions)}</span> <span className="c-del">−{fmtInt(p.deletions)}</span> · {fmtAgo(p.updated)}
      </>
    ),
    onPick: () => {
      setOpen(false);
      onPick(p);
    },
  }));
  return (
    <div className="menu pr-picker" ref={box}>
      <button
        type="button"
        className="iso-btn menu-btn"
        aria-haspopup="dialog"
        aria-expanded={open}
        title={github ? undefined : "No GitHub remote: pull requests need a github.com remote and the GitHub CLI (gh)."}
        onClick={() => {
          if (!open) load();
          setOpen(!open);
        }}
      >
        Pull requests <span className="caret" aria-hidden="true">▾</span>
      </button>
      {open && (
        <div className="popover branch-pop pr-pop menu-right" role="dialog" aria-label="Pull requests">
          {error && <pre className="git-err">{error}</pre>}
          {prs && !prs.available ? (
            <p className="caption">{prs.message ?? "Pull requests need the GitHub CLI (gh), signed in."}</p>
          ) : (
            <PickList items={items} placeholder="Number, title, author or branch" ariaLabel="Pull requests" loading={!prs && !error} empty="No open pull requests." />
          )}
        </div>
      )}
    </div>
  );
}
