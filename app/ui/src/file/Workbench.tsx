// The file view for full-tier files, faithful to the prototype: reading list on the left,
// dependency map and call flow on the right, one shared selection, draggable divider.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FileView } from "../ipc/bindings/FileView";
import type { FlowUnit } from "../ipc/bindings/FlowUnit";
import { cachedFlow } from "../ipc/api";
import { groupByLayer, isRuntimeModule, searchText, symbolIndex, type Order } from "../lib/cards";
import { cardState, focusSets, sameTarget, type Target } from "../lib/focus";
import { hasLoops as anyLoops, numberFlow, unitTitle } from "../lib/flow";
import { lineRange, plural } from "../lib/format";
import { prefersReducedMotion } from "../lib/theme";
import type { CodeContext } from "../editor/EditorHost";
import { CodeView } from "../editor/CodeView";
import { AiMark, Eyebrow, RichText, Seg } from "../components/ui";
import { FileHeader } from "./FileHeader";
import { FunctionCard } from "./FunctionCard";
import { DepMap, type DepMapHandle } from "./DepMap";
import { CallFlow } from "./CallFlow";
import { Splitter, initialSplit } from "./Splitter";

export interface WorkbenchProps {
  view: FileView;
  /** A symbol to focus when the file opens (from search or a link). */
  focusName?: string | null;
  focusLine?: number | null;
  onOpenFile(path: string, opts?: { line?: number | null; symbol?: string | null }): void;
}

type Mode = "map" | "flow";

export function Workbench({ view, focusName, focusLine, onOpenFile }: WorkbenchProps) {
  const path = view.path;
  const cards = view.functions;
  const [sel, setSel] = useState<Target | null>(null);
  const [hover, setHover] = useState<Target | null>(null);
  const [transitive, setTransitive] = useState(false);
  const [showTypes, setShowTypes] = useState(false);
  const [showExt, setShowExt] = useState(true);
  const [order, setOrder] = useState<Order>("entry");
  const [detail, setDetail] = useState<"brief" | "full">("full");
  const [q, setQ] = useState("");
  const [openCode, setOpenCode] = useState<Set<string>>(new Set());
  const [allOpen, setAllOpen] = useState(false);
  const [mode, setMode] = useState<Mode>("map");
  const [stack, setStack] = useState<string[]>([]);
  const [builtins, setBuiltins] = useState(false);
  const [cur, setCur] = useState(-1);
  const [play, setPlay] = useState({ key: 0, animate: true });
  const [codeStep, setCodeStep] = useState<{ card: string; line: number; nonce: number } | null>(null);
  const [flows, setFlows] = useState<Record<string, FlowUnit>>({});
  const [flowError, setFlowError] = useState<string | null>(null);
  const flyFrom = useRef<Map<string, DOMRect> | null>(null);
  const mapRef = useRef<DepMapHandle>(null);
  const leftRef = useRef<HTMLDivElement>(null);
  const toolbarRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const mainRef = useRef<HTMLDivElement>(null);
  const [split] = useState(initialSplit);

  const syms = useMemo(() => symbolIndex(view), [view]);
  const unitIds = useMemo(() => new Set(view.flowUnits.map((u) => u.id)), [view]);
  const unitOwner = useMemo(() => Object.fromEntries(view.flowUnits.map((u) => [u.id, u.owner])), [view]);
  const unitTitles = useMemo(() => Object.fromEntries(view.flowUnits.map((u) => [u.id, unitTitle(u)])), [view]);
  const byName = useMemo(() => new Map(cards.map((c) => [c.name, c])), [cards]);
  const groups = useMemo(() => groupByLayer(cards, order), [cards, order]);
  const texts = useMemo(() => new Map(cards.map((c) => [c.name, searchText(c)])), [cards]);
  const query = q.trim().toLowerCase();
  const matches = useMemo(() => new Set(cards.filter((c) => !query || texts.get(c.name)!.includes(query)).map((c) => c.name)), [cards, texts, query]);
  const S = useMemo(() => (sel ? focusSets(cards, sel, { transitive, showTypes }) : null), [sel, cards, transitive, showTypes]);
  const loops = useMemo(() => anyLoops(Object.values(flows)), [flows]);

  // Reset everything when another file opens.
  useEffect(() => {
    setSel(null);
    setHover(null);
    setMode("map");
    setStack([]);
    setCur(-1);
    setOpenCode(new Set());
    setAllOpen(false);
    setCodeStep(null);
    setFlows({});
    setQ("");
  }, [path]);

  const loadFlow = useCallback(
    (unit: string) => {
      setFlowError(null);
      cachedFlow(path, unit)
        .then((u) => setFlows((f) => (f[unit] ? f : { ...f, [unit]: u })))
        .catch((e: unknown) => setFlowError(String(e)));
    },
    [path],
  );

  const scrollCard = useCallback((name: string) => {
    const el = document.getElementById(`card-${name}`);
    const left = leftRef.current;
    if (!el || !left) return;
    el.style.scrollMarginTop = (toolbarRef.current?.offsetHeight ?? 0) + 12 + "px";
    el.scrollIntoView({ behavior: prefersReducedMotion() ? "auto" : "smooth", block: "start" });
  }, []);

  const openFlow = useCallback(
    (unit: string) => {
      if (mode === "map") flyFrom.current = mapRef.current?.captureRects() ?? null;
      setStack([unit]);
      setMode("flow");
      setCur(-1);
      setPlay((p) => ({ key: p.key + 1, animate: true }));
      loadFlow(unit);
    },
    [mode, loadFlow],
  );

  const select = useCallback(
    (t: Target | null, from: "graph" | "chip" | "list" | "code") => {
      if (t && from === "graph" && sameTarget(t, sel)) t = null;
      setSel(t);
      setHover(null);
      if (!t) {
        setMode("map");
        return;
      }
      if (t.kind === "fn") {
        const card = byName.get(t.id);
        if (card?.unit && unitIds.has(card.unit)) openFlow(card.unit);
        else setMode("map");
        if (from !== "list") requestAnimationFrame(() => scrollCard(t!.id));
        if (from !== "graph") requestAnimationFrame(() => mapRef.current?.centerOn(t!.id));
      } else {
        setMode("map");
        const first = groups.flatMap((g) => g.cards).find((f) => f.uses.includes(t!.id) || f.typeUses.includes(t!.id));
        if (first && from !== "list") requestAnimationFrame(() => scrollCard(first.name));
      }
    },
    [sel, byName, unitIds, openFlow, scrollCard, groups],
  );

  // Open with a symbol focused (search result, impact link).
  useEffect(() => {
    if (!focusName && !focusLine) return;
    const card = (focusName && byName.get(focusName)) || cards.find((c) => focusLine && c.start <= focusLine && focusLine <= c.end);
    if (card) {
      const t = setTimeout(() => select({ kind: "fn", id: card.name }, "chip"), 60);
      return () => clearTimeout(t);
    }
    // select is stable enough here; only react to a new focus request.
  }, [view, focusName, focusLine]);

  const curUnitId = stack[stack.length - 1] ?? null;
  const curUnit = curUnitId ? flows[curUnitId] : undefined;
  const numbered = useMemo(() => (curUnit ? numberFlow(curUnit.tree, builtins) : null), [curUnit, builtins]);
  const stepCount = numbered?.steps.length ?? 0;

  const syncCode = useCallback(
    (line: number) => {
      if (!curUnitId) return;
      const owner = unitOwner[curUnitId];
      setOpenCode((s) => (s.has(owner) ? s : new Set(s).add(owner)));
      setCodeStep((c) => ({ card: owner, line, nonce: (c?.nonce ?? 0) + 1 }));
    },
    [curUnitId, unitOwner],
  );

  const setStep = useCallback(
    (i: number) => {
      if (!numbered || numbered.steps.length === 0) return;
      const n = Math.max(0, Math.min(numbered.steps.length - 1, i));
      setCur(n);
      syncCode(numbered.steps[n].line);
    },
    [numbered, syncCode],
  );

  const drill = useCallback(
    (unit: string) => {
      setStack((s) => {
        const at = s.indexOf(unit);
        return at >= 0 ? s.slice(0, at + 1) : [...s, unit];
      });
      const owner = unitOwner[unit];
      setSel({ kind: "fn", id: owner });
      setCur(-1);
      setPlay((p) => ({ key: p.key + 1, animate: true }));
      loadFlow(unit);
      requestAnimationFrame(() => scrollCard(owner));
    },
    [unitOwner, loadFlow, scrollCard],
  );

  const crumb = useCallback(
    (depth: number) => {
      const next = stack.slice(0, depth + 1);
      setStack(next);
      const owner = unitOwner[next[next.length - 1]];
      setSel({ kind: "fn", id: owner });
      setCur(-1);
      setPlay((p) => ({ key: p.key + 1, animate: true }));
      requestAnimationFrame(() => scrollCard(owner));
    },
    [stack, unitOwner, scrollCard],
  );

  // Keyboard: Esc clears, arrows step the flow, F focuses the filter.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      const typing = t.closest("input, textarea, select, .monaco-editor, [contenteditable]");
      if (e.key === "Escape") {
        if (t === filterRef.current && q) return;
        if (document.querySelector(".search-pop")) return;
        select(null, "list");
        return;
      }
      if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (mode === "flow" && (e.key === "ArrowRight" || e.key === "ArrowLeft") && !t.closest(".splitter")) {
        e.preventDefault();
        setStep(e.key === "ArrowRight" ? cur + 1 : cur < 0 ? 0 : cur - 1);
        return;
      }
      if (e.key === "f") {
        e.preventDefault();
        filterRef.current?.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [mode, cur, setStep, select, q]);

  const toggleCode = useCallback((name: string) => {
    setOpenCode((s) => {
      const n = new Set(s);
      if (n.has(name)) n.delete(name);
      else n.add(name);
      return n;
    });
  }, []);

  const onHeader = useCallback((name: string) => select(sel && sel.kind === "fn" && sel.id === name ? null : { kind: "fn", id: name }, "list"), [select, sel]);
  const onTarget = useCallback((t: Target) => select(t, "chip"), [select]);
  const openFileCb = useCallback((p: string, line?: number | null) => onOpenFile(p, { line }), [onOpenFile]);

  const codeContext = useMemo<CodeContext>(
    () => ({
      path,
      cards: new Map(cards.map((c) => [c.name, { name: c.name, start: c.start, end: c.end, desc: c.summary?.desc ?? c.doc }])),
      syms: new Map(
        [...syms.entries()].map(([n, s]) => [n, { name: n, label: s.label, from: s.module.id, member: s.sym.member, receiver: s.sym.member ? s.label.split(".")[0] : null, typeOnly: s.sym.typeOnly }]),
      ),
      types: new Map(view.types.map((t) => [t.name, { name: t.name, start: t.start, end: t.end, desc: t.summary ?? t.doc }])),
      onJumpToCard: (name) => select({ kind: "fn", id: name }, "code"),
      onSymbol: (name) => select({ kind: "sym", id: name }, "code"),
    }),
    [path, cards, syms, view.types, select],
  );
  const cardContexts = useMemo(() => new Map(cards.map((c) => [c.name, { ...codeContext, self: c.name }])), [cards, codeContext]);

  const entries = cards.filter((c) => c.exported);
  const hiddenMap = mode === "flow";
  const flowDesc = curUnit ? (curUnit.method ? null : (byName.get(curUnit.owner)?.summary?.desc ?? byName.get(curUnit.owner)?.doc ?? null)) : null;

  return (
    <div className="workbench">
      <FileHeader view={view} />
      <div className="main" ref={mainRef} style={{ ["--left" as string]: split + "%" }}>
        <div className="pane-left" ref={leftRef}>
          <div className="inner">
            <section className="overview" aria-labelledby="ov-h">
              <Eyebrow id="ov-h">What this file does</Eyebrow>
              {view.overview ? (
                <p>
                  <RichText text={view.overview.text} />
                  <AiMark model={view.overview.model} />
                </p>
              ) : (
                <p className="muted">No overview yet. Turn on AI summaries to get one; the cards below come from the code itself.</p>
              )}
              {entries.length > 0 && (
                <div className="flow">
                  <span className="lab">Start at</span>
                  <span className="chips">
                    {entries.map((c) => (
                      <button key={c.name} type="button" className="iso-chip" onClick={() => select({ kind: "fn", id: c.name }, "chip")}>
                        {c.name}
                      </button>
                    ))}
                  </span>
                </div>
              )}
              <div className="flow">
                <span className="lab">Imported by</span>
                <span className="chips">
                  {view.importers.length ? (
                    view.importers.map((f) => (
                      <button key={f.path} type="button" className="filelink" onClick={() => onOpenFile(f.path, { line: f.line })} title={`Open ${f.path}${f.line ? ` at line ${f.line}` : ""}`}>
                        {f.path}
                      </button>
                    ))
                  ) : (
                    <span className="none">no files in this repo</span>
                  )}
                </span>
              </div>
            </section>

            <div className="toolbar" role="toolbar" aria-label="List controls" ref={toolbarRef}>
              <input ref={filterRef} className="search" type="search" placeholder="Filter functions, steps, symbols (F)" aria-label="Filter functions" value={q} onChange={(e) => setQ(e.target.value)} />
              <Seg
                label="Order"
                value={order}
                onChange={setOrder}
                options={[
                  ["deps", "Leaves first", "Topological order: every function appears after what it calls"],
                  ["entry", "Entry points first", "Reverse topological order: entry points first"],
                ]}
              />
              <Seg
                label="Detail"
                value={detail}
                onChange={setDetail}
                options={[
                  ["brief", "Summary"],
                  ["full", "+ Algorithm"],
                ]}
              />
              <button
                type="button"
                className="iso-btn"
                onClick={() => {
                  const next = !allOpen;
                  setAllOpen(next);
                  setOpenCode(next ? new Set(cards.map((c) => c.name)) : new Set());
                }}
              >
                {allOpen ? "Collapse all code" : "Expand all code"}
              </button>
              {sel && (
                <div className="focusbar">
                  <span>
                    Focused on <code>{sel.kind === "sym" ? (syms.get(sel.id)?.label ?? sel.id) : sel.id}</code>. Blue: what it depends on. Amber: what depends on it.
                  </span>
                  <button type="button" className="iso-btn" onClick={() => select(null, "list")}>
                    Clear (Esc)
                  </button>
                </div>
              )}
            </div>

            {cards.length === 0 && <p className="muted empty-note">No functions in this file. Its imports and types are listed below.</p>}
            {query && matches.size === 0 && <p className="muted empty-note">No function matches “{q}”.</p>}
            {groups.map((g) => {
              const visible = g.cards.filter((c) => matches.has(c.name));
              return (
                <section className="group" key={g.level} hidden={visible.length === 0}>
                  <div className="group-h">
                    <h2>{g.title}</h2>
                    <span>
                      {g.subtitle} · {g.cards.length}
                    </span>
                  </div>
                  {g.cards.map((c) => (
                    <FunctionCard
                      key={c.name}
                      card={c}
                      path={path}
                      state={cardState(c.name, sel, S)}
                      hidden={!matches.has(c.name)}
                      brief={detail === "brief"}
                      codeOpen={openCode.has(c.name)}
                      syms={syms}
                      codeContext={cardContexts.get(c.name)!}
                      stepLine={codeStep && codeStep.card === c.name ? codeStep.line : null}
                      stepNonce={codeStep && codeStep.card === c.name ? codeStep.nonce : 0}
                      onToggleCode={toggleCode}
                      onHeader={onHeader}
                      onTarget={onTarget}
                      onOpenFile={openFileCb}
                    />
                  ))}
                </section>
              );
            })}

            <LocalTypes view={view} context={codeContext} />
            <Imports view={view} context={codeContext} onOpenFile={(p) => onOpenFile(p)} onTarget={onTarget} />
          </div>
        </div>

        <Splitter target={mainRef} />

        <div className="pane-right">
          <div className="rbar">
            <div className="tabs" role="tablist" aria-label="Graph views">
              <button type="button" role="tab" aria-selected={mode === "map"} onClick={() => setMode("map")}>
                Dependency map
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={mode === "flow"}
                disabled={!stack.length}
                title={stack.length ? undefined : "Select a function first"}
                onClick={() => {
                  if (!stack.length || mode === "flow") return;
                  flyFrom.current = mapRef.current?.captureRects() ?? null;
                  setMode("flow");
                  setPlay((p) => ({ key: p.key + 1, animate: true }));
                }}
              >
                Call flow
              </button>
            </div>
            {mode === "map" ? (
              <div className="gctl">
                <button type="button" className="iso-btn" aria-pressed={showExt} onClick={() => setShowExt(!showExt)}>
                  External imports
                </button>
                <button type="button" className="iso-btn" aria-pressed={showTypes} onClick={() => setShowTypes(!showTypes)}>
                  Type-only imports
                </button>
                <button type="button" className="iso-btn" aria-pressed={transitive} title="Follow dependencies and dependents all the way, for impact analysis" onClick={() => setTransitive(!transitive)}>
                  Transitive
                </button>
                <button type="button" className="iso-btn" aria-label="Zoom out" onClick={() => mapRef.current?.zoomBy(0.8)}>
                  −
                </button>
                <button type="button" className="iso-btn" aria-label="Zoom in" onClick={() => mapRef.current?.zoomBy(1.25)}>
                  +
                </button>
                <button type="button" className="iso-btn" onClick={() => mapRef.current?.fit(true)}>
                  Fit
                </button>
              </div>
            ) : (
              <div className="fctl">
                <button type="button" className="iso-btn" title="Play the call sequence again" onClick={() => setPlay((p) => ({ key: p.key + 1, animate: true }))}>
                  Replay
                </button>
                <button type="button" className="iso-btn" aria-label="Previous call" onClick={() => setStep(cur < 0 ? 0 : cur - 1)}>
                  ◀
                </button>
                <span className="fpos" aria-live="polite">
                  {cur < 0 ? "–" : cur + 1} / {stepCount}
                </span>
                <button type="button" className="iso-btn" aria-label="Next call" onClick={() => setStep(cur + 1)}>
                  ▶
                </button>
                <button
                  type="button"
                  className="iso-btn"
                  aria-pressed={builtins}
                  title="Show calls like Date.now() and Object.hasOwn()"
                  onClick={() => {
                    setBuiltins(!builtins);
                    setCur(-1);
                    setPlay((p) => ({ key: p.key + 1, animate: false }));
                  }}
                >
                  Built-ins
                </button>
              </div>
            )}
          </div>
          <div className={`stage${mode === "flow" ? " flowmode" : ""}`}>
            <DepMap
              ref={mapRef}
              view={view}
              sel={sel}
              hover={hover}
              showExt={showExt}
              showTypes={showTypes}
              transitive={transitive}
              query={q}
              matches={matches}
              syms={syms}
              hidden={hiddenMap}
              onSelect={(t) => select(t, "graph")}
              onHover={setHover}
              onOpenFile={(p) => onOpenFile(p)}
            />
            {mode === "flow" &&
              (curUnit ? (
                <CallFlow
                  unit={curUnit}
                  stack={stack}
                  titles={unitTitles}
                  units={unitIds}
                  desc={flowDesc}
                  builtins={builtins}
                  cur={cur}
                  playKey={play.key}
                  animate={play.animate}
                  flyFrom={flyFrom}
                  hasLoops={loops}
                  onStep={setStep}
                  onDrill={drill}
                  onCrumb={crumb}
                  onLine={syncCode}
                />
              ) : (
                <section className="flowpane">
                  <p className="muted flow-wait">{flowError ? `Could not load the call flow: ${flowError}` : "Loading the call flow…"}</p>
                </section>
              ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function LocalTypes({ view, context }: { view: FileView; context: CodeContext }) {
  const [open, setOpen] = useState(false);
  if (!view.types.length) return null;
  return (
    <details className="misc" onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary>
        Local types<span>{plural(view.types.length, "declaration")}</span>
      </summary>
      <div className="tlist">
        {view.types.map((t) => (
          <div key={t.id}>
            <p>
              <code className="iso-fname">{t.name}</code> <span className="iso-lines">{lineRange(t.start, t.end)}</span>
              {t.exported && <span className="iso-pill">export</span>}
              <br />
              {t.summary ?? t.doc ?? <span className="none">No summary yet.</span>}
            </p>
            {open && <CodeView path={view.path} start={t.start} end={t.end} fit context={context} ariaLabel={`Code of ${t.name}`} />}
          </div>
        ))}
      </div>
    </details>
  );
}

function Imports({ view, context, onOpenFile, onTarget }: { view: FileView; context: CodeContext; onOpenFile(p: string): void; onTarget(t: Target): void }) {
  const [open, setOpen] = useState(false);
  if (!view.modules.length) return null;
  const r = view.importsRange;
  return (
    <details className="misc" onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary>
        Imports<span>{r ? `lines ${r.start}–${r.end}` : plural(view.modules.length, "module")}</span>
      </summary>
      <div className="tlist">
        <ul className="imports">
          {view.modules.map((m) => (
            <li key={m.id}>
              {m.resolved ? (
                <button type="button" className="filelink" onClick={() => onOpenFile(m.resolved!)} title={`Open ${m.resolved}`}>
                  {m.id}
                </button>
              ) : (
                <code className={isRuntimeModule(m) ? "muted" : "ext"}>{m.id}</code>
              )}
              <span className="caption">{isRuntimeModule(m) ? "parameter, not an import" : m.external ? "outside the repo" : m.resolved ? `→ ${m.resolved}` : "unresolved"}</span>
              <span className="chips">
                {m.symbols.map((s) => (
                  <button key={s.name} type="button" className={`iso-chip iso-chip-ext${s.typeOnly ? " iso-chip-type" : ""}`} onClick={() => onTarget({ kind: "sym", id: s.name })}>
                    {s.member ? "." : ""}
                    {s.name}
                  </button>
                ))}
              </span>
            </li>
          ))}
        </ul>
        {open && r && <CodeView path={view.path} start={r.start} end={r.end} fit context={context} ariaLabel="Import block" />}
      </div>
    </details>
  );
}
