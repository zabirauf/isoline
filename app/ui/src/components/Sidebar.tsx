// Source tree sidebar: virtualized (only visible rows are in the DOM), so 10k+ files scroll
// smoothly. Each file shows its module color, tier and index status.
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { TreeNode } from "../ipc/bindings/TreeNode";
import { dirname, fmtInt, middleEllipsis } from "../lib/format";
import { ModuleDot } from "./ui";
import { usePeek } from "./Peek";

/** Pending and failed only: indexed is the norm and shows nothing. The meaning is in the peek. */
function StatusDot({ status, text }: { status: keyof typeof STATUS_WORD; text: string }) {
  const peek = usePeek();
  const bind = peek.bind(() => ({ kind: "text", title: STATUS_WORD[status], body: text.slice(text.indexOf(":") + 1).trim() }));
  return <span className={`fst fst-${status}`} role="img" aria-label={STATUS_WORD[status]} onPointerEnter={bind.onPointerEnter} onPointerLeave={bind.onPointerLeave} />;
}

const ROW = 26;
const OVERSCAN = 12;

interface Row {
  node: TreeNode;
  depth: number;
  open: boolean;
  /** Shown in filter mode: the folder the match lives in. */
  dir?: string;
}

function flatten(root: TreeNode, expanded: Set<string>): Row[] {
  const out: Row[] = [];
  const walk = (n: TreeNode, depth: number) => {
    for (const c of n.children) {
      const open = c.isDir && expanded.has(c.path);
      out.push({ node: c, depth, open });
      if (open) walk(c, depth + 1);
    }
  };
  walk(root, 0);
  return out;
}

function filtered(root: TreeNode, q: string): Row[] {
  const out: Row[] = [];
  const walk = (n: TreeNode) => {
    for (const c of n.children) {
      if (c.isDir) walk(c);
      else if (c.path.toLowerCase().includes(q)) out.push({ node: c, depth: 0, open: false, dir: dirname(c.path) });
    }
  };
  walk(root);
  return out;
}

/** Files under every folder, by path ("" is the root). */
function folderCounts(root: TreeNode): Map<string, number> {
  const m = new Map<string, number>();
  const walk = (n: TreeNode): number => {
    if (!n.isDir) return 1;
    let s = 0;
    for (const c of n.children) s += walk(c);
    m.set(n.path, s);
    return s;
  };
  walk(root);
  return m;
}

/**
 * Folders open at first sight: none, except a chain of lone folders at the top (a repo whose
 * only folder is `src/` opens it, and `src/main/` below it when that is alone too).
 */
export function defaultExpanded(root: TreeNode): Set<string> {
  const out = new Set<string>();
  let n: TreeNode | undefined = root;
  for (let depth = 0; n && depth < 6; depth++) {
    const dirs: TreeNode[] = n.children.filter((c) => c.isDir);
    if (dirs.length !== 1) break;
    // Below the root, keep going only while the folder holds nothing but that one folder.
    if (depth > 0 && n.children.length !== 1) break;
    out.add(dirs[0].path);
    n = dirs[0];
  }
  return out;
}

const STORE = "isoline.tree.v1:";
const MAX_STORED = 2000;

function loadExpanded(repo: string): Set<string> | null {
  try {
    const v = localStorage.getItem(STORE + repo);
    if (!v) return null;
    const a = JSON.parse(v);
    return Array.isArray(a) ? new Set(a.filter((x) => typeof x === "string")) : null;
  } catch {
    return null;
  }
}

function saveExpanded(repo: string, s: Set<string>) {
  try {
    localStorage.setItem(STORE + repo, JSON.stringify([...s].slice(0, MAX_STORED)));
  } catch {
    /* storage unavailable */
  }
}

export const STATUS_WORD = { indexed: "Indexed", pending: "Pending", failed: "Failed", skipped: "Skipped" } as const;
const STATUS_TEXT = {
  indexed: "Indexed: facts are current",
  pending: "Pending: waiting to be indexed",
  failed: "Failed: the file could not be indexed",
  skipped: "Skipped: ignored or too large",
} as const;

/** Width of one character of the tree's mono font, measured once. */
let charW = 0;
function monoCharWidth(): number {
  if (!charW) {
    const ctx = document.createElement("canvas").getContext("2d");
    if (ctx) {
      ctx.font = '12px "IBM Plex Mono", ui-monospace, monospace';
      charW = ctx.measureText("0000000000").width / 10 || 7.2;
    } else charW = 7.2;
  }
  return charW;
}

export interface SidebarProps {
  /** Repo root: expanded folders are remembered per repo. */
  repo: string;
  tree: TreeNode;
  openPath: string | null;
  onOpen(path: string): void;
  /** Shown while the tree is empty: "Listing files…" until the walk is known to be done. */
  emptyText?: string;
  /** Module names by id, for the color dots' titles. */
  moduleNames?: ReadonlyMap<string, string>;
}

export const Sidebar = memo(function Sidebar({ repo, tree, openPath, onOpen, emptyText = "Listing files…", moduleNames }: SidebarProps) {
  const [expanded, setExpanded] = useState<Set<string>>(() => loadExpanded(repo) ?? defaultExpanded(tree));
  // The tree usually arrives empty (the walk has not listed the files yet): the default
  // expansion is applied when the first files show up, unless the reader had their own.
  const seeded = useRef(tree.children.length > 0 || loadExpanded(repo) !== null);
  useEffect(() => {
    if (seeded.current || tree.children.length === 0) return;
    seeded.current = true;
    setExpanded((s) => new Set([...s, ...defaultExpanded(tree)]));
  }, [tree]);
  const firstSave = useRef(true);
  useEffect(() => {
    if (firstSave.current) {
      firstSave.current = false;
      return;
    }
    saveExpanded(repo, expanded);
  }, [repo, expanded]);
  const [q, setQ] = useState("");
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(600);
  const [width, setWidth] = useState(250);
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const counts = useMemo(() => folderCounts(tree), [tree]);
  const total = counts.get("") ?? 0;

  // Reveal the open file: expand its folders.
  useEffect(() => {
    if (!openPath) return;
    const parts = openPath.split("/");
    setExpanded((s) => {
      const n = new Set(s);
      for (let i = 1; i < parts.length; i++) n.add(parts.slice(0, i).join("/"));
      return n;
    });
  }, [openPath]);

  const query = q.trim().toLowerCase();
  const rows = useMemo(() => (query ? filtered(tree, query) : flatten(tree, expanded)), [tree, expanded, query]);

  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setHeight(el.clientHeight);
      setWidth(el.clientWidth);
    });
    ro.observe(el);
    setHeight(el.clientHeight);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  // Keep the open file in view once its row exists.
  useEffect(() => {
    if (!openPath) return;
    const i = rows.findIndex((r) => r.node.path === openPath);
    const el = listRef.current;
    if (i < 0 || !el) return;
    setActive(i);
    const top = i * ROW;
    if (top < el.scrollTop || top + ROW > el.scrollTop + el.clientHeight) el.scrollTop = Math.max(0, top - Math.floor(el.clientHeight / 3 / ROW) * ROW); // whole rows: none half-hidden at the top
  }, [openPath, rows]);

  const first = Math.max(0, Math.floor(scroll / ROW) - OVERSCAN);
  const last = Math.min(rows.length, Math.ceil((scroll + height) / ROW) + OVERSCAN);

  const toggle = (path: string) =>
    setExpanded((s) => {
      const n = new Set(s);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });

  const activate = (r: Row) => (r.node.isDir ? toggle(r.node.path) : onOpen(r.node.path));

  const focusRow = (i: number) => {
    const el = listRef.current;
    if (!el) return;
    const n = Math.max(0, Math.min(rows.length - 1, i));
    setActive(n);
    const top = n * ROW;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + ROW > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW - el.clientHeight;
    requestAnimationFrame(() => el.querySelector<HTMLElement>(`[data-row="${n}"]`)?.focus());
  };

  return (
    <nav className="sidebar" aria-label="Source tree">
      <div className="side-h">
        <span className="eyebrow">Files</span>
        <span className="caption">{fmtInt(total)}</span>
      </div>
      <input className="search side-filter" type="search" placeholder="Filter files" aria-label="Filter files" value={q} onChange={(e) => setQ(e.target.value)} />
      <div
        className="tree"
        ref={listRef}
        role="tree"
        aria-label="Files"
        onScroll={(e) => setScroll(e.currentTarget.scrollTop)}
        onKeyDown={(e) => {
          const r = rows[active];
          if (!r) return;
          if (e.key === "ArrowDown") (e.preventDefault(), focusRow(active + 1));
          else if (e.key === "ArrowUp") (e.preventDefault(), focusRow(active - 1));
          else if (e.key === "Home") (e.preventDefault(), focusRow(0));
          else if (e.key === "End") (e.preventDefault(), focusRow(rows.length - 1));
          else if (e.key === "Enter" || e.key === " ") (e.preventDefault(), activate(r));
          else if (e.key === "ArrowRight" && r.node.isDir && !r.open) (e.preventDefault(), toggle(r.node.path));
          else if (e.key === "ArrowLeft" && r.node.isDir && r.open) (e.preventDefault(), toggle(r.node.path));
        }}
      >
        <div style={{ height: rows.length * ROW, position: "relative" }}>
          {rows.slice(first, last).map((r, k) => {
            const i = first + k;
            const n = r.node;
            const indent = 10 + r.depth * 14;
            const badge = !n.isDir && !!n.tier && n.tier !== "full";
            const count = n.isDir ? (counts.get(n.path) ?? 0) : 0;
            // Room for the name: row minus indent, dot or twisty, badge, count, status dot and gaps.
            const room = width - indent - 14 - (badge ? 58 : 0) - (n.isDir ? 14 + 12 + String(count).length * 7 + (n.color ? 14 : 0) : 20) - (r.dir !== undefined ? 60 : 0);
            const shown = middleEllipsis(n.name, Math.max(4, Math.floor(room / (n.isDir ? monoCharWidth() * 1.05 : monoCharWidth()))));
            const modName = n.module ? (moduleNames?.get(n.module) ?? n.module) : null;
            const title = modName ? `${n.path} · module: ${modName}` : n.path;
            return (
              <div
                key={n.path}
                data-row={i}
                role="treeitem"
                aria-level={r.depth + 1}
                aria-expanded={n.isDir ? r.open : undefined}
                aria-selected={n.path === openPath}
                tabIndex={i === active ? 0 : -1}
                className={`row${n.isDir ? " dir" : " file"}${n.path === openPath ? " cur" : ""}`}
                style={{ transform: `translateY(${i * ROW}px)`, paddingLeft: indent }}
                onClick={() => {
                  setActive(i);
                  activate(r);
                }}
                title={title}
              >
                {n.isDir ? (
                  <>
                    <span className={`twisty${r.open ? " open" : ""}`}>›</span>
                    {n.color && <ModuleDot color={n.color} title={`Module: ${modName}`} />}
                  </>
                ) : (
                  <ModuleDot color={n.color} title={modName ? `Module: ${modName}` : undefined} />
                )}
                <span className="nm">{shown}</span>
                {r.dir !== undefined && <span className="dirhint">{r.dir}</span>}
                {badge && <span className={`tier-mini tier-${n.tier}`}>{n.tier}</span>}
                {!n.isDir && n.status !== "indexed" && <StatusDot status={n.status} text={STATUS_TEXT[n.status]} />}
                {n.isDir && <span className="dcount" aria-label={`${count} files`}>{fmtInt(count)}</span>}
                {n.isDir && n.status === "pending" && <StatusDot status="pending" text="Pending: some files in this folder are waiting to be indexed" />}
              </div>
            );
          })}
        </div>
        {rows.length === 0 && <p className="muted side-empty">{query ? `No file matches “${q}”.` : emptyText}</p>}
      </div>
    </nav>
  );
});
