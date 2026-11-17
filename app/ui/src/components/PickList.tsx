// A searchable list in a popover: the branch menu and the compare view's revision pickers.
// Type to filter, arrows move, Enter picks, Esc closes. Long sections show their first rows and
// say how many more there are, so a repo with thousands of remote branches stays quick.
import { useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";

export interface PickItem {
  key: string;
  section: string;
  label: ReactNode;
  /** Matched against the filter (lowercased). */
  text: string;
  caption?: ReactNode;
  current?: boolean;
  onPick(): void;
  /** A secondary action shown on hover and focus ("Compare"). */
  action?: { label: string; title?: string; onSelect(): void };
}

/** Closes the popover on a click outside `box` and on Esc (returning focus to the button). */
export function useDismiss(open: boolean, close: () => void, box: RefObject<HTMLElement | null>) {
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => !box.current?.contains(e.target as Node) && close();
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      close();
      box.current?.querySelector<HTMLButtonElement>("button")?.focus();
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open, close, box]);
}

export function PickList({ items, placeholder, extra, perSection = 60, empty, ariaLabel, loading }: { items: PickItem[]; placeholder: string; extra?: (q: string) => PickItem[]; perSection?: number; empty?: ReactNode; ariaLabel: string; loading?: boolean }) {
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const query = q.trim().toLowerCase();
  const { shown, more } = useMemo(() => {
    const matched = query ? items.filter((it) => it.text.toLowerCase().includes(query)) : items;
    const per = new Map<string, number>();
    const more = new Map<string, number>();
    const shown: PickItem[] = [...(extra?.(q.trim()) ?? [])];
    for (const it of matched) {
      const n = per.get(it.section) ?? 0;
      if (n < perSection) shown.push(it);
      else more.set(it.section, (more.get(it.section) ?? 0) + 1);
      per.set(it.section, n + 1);
    }
    return { shown, more };
  }, [items, query, q, extra, perSection]);
  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  let lastSection = "";
  const rows: ReactNode[] = [];
  shown.forEach((it, i) => {
    if (it.section !== lastSection) {
      const prev = lastSection;
      if (prev && more.get(prev)) rows.push(<div key={`more-${prev}`} className="pick-more caption">{more.get(prev)} more · type to narrow</div>);
      lastSection = it.section;
      if (it.section) rows.push(<div key={`s-${it.section}`} className="menu-label">{it.section}</div>);
    }
    rows.push(
      <div key={it.key} className={`pick-row${i === active ? " on" : ""}${it.current ? " cur" : ""}`} data-i={i} role="option" aria-selected={i === active} onPointerMove={() => i !== active && setActive(i)}>
        <button type="button" className="pick-main" tabIndex={-1} onClick={() => it.onPick()}>
          <span className="pick-label">{it.label}</span>
          {it.caption && <span className="pick-cap">{it.caption}</span>}
        </button>
        {it.action && (
          <button
            type="button"
            className="iso-btn-drill pick-act"
            tabIndex={-1}
            title={it.action.title}
            onClick={(e) => {
              e.stopPropagation();
              it.action!.onSelect();
            }}
          >
            {it.action.label}
          </button>
        )}
      </div>,
    );
  });
  if (lastSection && more.get(lastSection)) rows.push(<div key={`more-${lastSection}`} className="pick-more caption">{more.get(lastSection)} more · type to narrow</div>);

  return (
    <div className="picklist">
      <input
        className="search pick-filter"
        type="search"
        placeholder={placeholder}
        aria-label={placeholder}
        value={q}
        autoFocus
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") (e.preventDefault(), setActive((a) => Math.min(shown.length - 1, a + 1)));
          else if (e.key === "ArrowUp") (e.preventDefault(), setActive((a) => Math.max(0, a - 1)));
          else if (e.key === "Enter") (e.preventDefault(), shown[active]?.onPick());
        }}
      />
      <div className="pick-list" ref={list} role="listbox" aria-label={ariaLabel}>
        {rows}
        {shown.length === 0 && <p className="caption pick-empty">{loading ? "Loading…" : query ? `Nothing matches “${q.trim()}”.` : empty}</p>}
      </div>
    </div>
  );
}
