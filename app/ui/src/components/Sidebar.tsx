// Source tree sidebar: virtualized (only visible rows are in the DOM), so 10k+ files scroll
// smoothly. Each file shows its module color, tier and index status.
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { TreeNode } from "../ipc/bindings/TreeNode";
import { dirname, fmtInt } from "../lib/format";
import { ModuleDot } from "./ui";

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

function countFiles(n: TreeNode): number {
  return n.isDir ? n.children.reduce((s, c) => s + countFiles(c), 0) : 1;
}

const STATUS_TEXT = { indexed: "Indexed", pending: "Waiting to be indexed", failed: "Indexing failed", skipped: "Skipped (ignored or too large)" } as const;

export interface SidebarProps {
  tree: TreeNode;
  openPath: string | null;
  onOpen(path: string): void;
}

export const Sidebar = memo(function Sidebar({ tree, openPath, onOpen }: SidebarProps) {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(tree.children.filter((c) => c.isDir).map((c) => c.path)));
  const [q, setQ] = useState("");
  const [scroll, setScroll] = useState(0);
  const [height, setHeight] = useState(600);
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const total = useMemo(() => countFiles(tree), [tree]);

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
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    setHeight(el.clientHeight);
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
    if (top < el.scrollTop || top + ROW > el.scrollTop + el.clientHeight) el.scrollTop = Math.max(0, top - el.clientHeight / 3);
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
                style={{ transform: `translateY(${i * ROW}px)`, paddingLeft: 10 + r.depth * 14 }}
                onClick={() => {
                  setActive(i);
                  activate(r);
                }}
                title={n.path}
              >
                {n.isDir ? <span className={`twisty${r.open ? " open" : ""}`}>›</span> : <ModuleDot color={n.color} title={n.module ? `Module: ${n.module}` : undefined} />}
                <span className="nm">{n.name}</span>
                {r.dir !== undefined && <span className="dirhint">{r.dir}</span>}
                {!n.isDir && n.tier && <span className={`tier-mini tier-${n.tier}`}>{n.tier}</span>}
                {!n.isDir && <span className={`fst fst-${n.status}`} title={STATUS_TEXT[n.status]} aria-label={STATUS_TEXT[n.status]} />}
                {n.isDir && n.status === "pending" && <span className="fst fst-pending" title="Some files are waiting to be indexed" />}
              </div>
            );
          })}
        </div>
        {rows.length === 0 && <p className="muted side-empty">No file matches “{q}”.</p>}
      </div>
    </nav>
  );
});
