// Repo-wide search over files and symbols. "/" focuses it from anywhere; results update as
// you type; arrows move, Enter opens, Esc closes.
import { useEffect, useRef, useState } from "react";
import type { SearchHit } from "../ipc/bindings/SearchHit";
import { api } from "../ipc/api";

export function SearchBox({ enabled, onPick }: { enabled: boolean; onPick(hit: SearchHit): void }) {
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const seq = useRef(0);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey) return;
      const t = e.target as HTMLElement;
      if (t.closest("input, textarea, select, .monaco-editor, [contenteditable]")) return;
      e.preventDefault();
      input.current?.focus();
      input.current?.select();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    const query = q.trim();
    if (!query || !enabled) {
      setHits([]);
      return;
    }
    const my = ++seq.current;
    const t = setTimeout(() => {
      api
        .search(query, 30)
        .then((h) => {
          if (my === seq.current) {
            setHits(h);
            setActive(0);
          }
        })
        .catch(() => my === seq.current && setHits([]));
    }, 90);
    return () => clearTimeout(t);
  }, [q, enabled]);

  const pick = (h: SearchHit) => {
    setOpen(false);
    input.current?.blur();
    onPick(h);
  };

  const showPop = open && q.trim().length > 0;
  return (
    <div className="searchbox">
      <input
        ref={input}
        className="iso-search top-search"
        type="search"
        placeholder={enabled ? "Search files and symbols   ( / )" : "Open a repo to search"}
        aria-label="Search files and symbols"
        aria-expanded={showPop}
        aria-controls="search-results"
        aria-activedescendant={showPop && hits[active] ? `hit-${active}` : undefined}
        role="combobox"
        disabled={!enabled}
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 120)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") (e.preventDefault(), setActive((a) => Math.min(hits.length - 1, a + 1)));
          else if (e.key === "ArrowUp") (e.preventDefault(), setActive((a) => Math.max(0, a - 1)));
          else if (e.key === "Enter" && hits[active]) (e.preventDefault(), pick(hits[active]));
          else if (e.key === "Escape") {
            e.stopPropagation();
            if (q) setQ("");
            else input.current?.blur();
            setOpen(false);
          }
        }}
      />
      {showPop && (
        <ul className="search-pop" id="search-results" role="listbox" aria-label="Search results">
          {hits.length === 0 && <li className="muted empty">No file or symbol matches “{q.trim()}”.</li>}
          {hits.map((h, i) => (
            <li
              key={h.kind + h.path + h.name + (h.line ?? "")}
              id={`hit-${i}`}
              role="option"
              aria-selected={i === active}
              className={i === active ? "on" : ""}
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => setActive(i)}
              onClick={() => pick(h)}
            >
              <span className="hk">{h.kind === "file" ? "file" : (h.symbolKind ?? "symbol")}</span>
              <code className="hn">{h.name}</code>
              <span className="hp">
                {h.path}
                {h.line ? `:${h.line}` : ""}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
