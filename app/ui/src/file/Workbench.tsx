// The file view for full-tier files: reading list on the left, dependency map and call flow on
// the right, one shared selection, draggable divider. Summary first, detail on demand
// (docs/ux/progressive-disclosure.md): a Glance / Read / Dig level, relations behind counts,
// typed signatures with definition peeks, and go to definition with a location history.
import { AiLine } from "../components/AiLine";
import type { FileAi } from "../lib/aiRuns";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { FileView } from "../ipc/bindings/FileView";
import type { FlowUnit } from "../ipc/bindings/FlowUnit";
import type { SymbolTarget } from "../ipc/bindings/SymbolTarget";
import { cachedFlow } from "../ipc/api";
import { groupByLayer, isRuntimeModule, searchText, symbolIndex, type Order } from "../lib/cards";
import { cardState, focusSets, sameTarget, type Target } from "../lib/focus";
import { hasLoops as anyLoops, numberFlow, unitTitle, type CallItem } from "../lib/flow";
import { basename, lineRange, middleEllipsis, plural } from "../lib/format";
import { detailForKey, getPrefs, setPrefs, subscribePrefs, type Detail } from "../lib/prefs";
import { indexTargets } from "../lib/sig";
import { prefersReducedMotion } from "../lib/theme";
import type { CodeContext } from "../editor/EditorHost";
import { CodeView } from "../editor/CodeView";
import { AiMark, Eyebrow, RichText, Seg } from "../components/ui";
import { Menu } from "../components/Menu";
import { usePeek, type PeekSubject, type SymbolPeek } from "../components/Peek";
import { FileHeader } from "./FileHeader";
import { FunctionCard, type Rel } from "./FunctionCard";
import { DepMap, type DepMapHandle } from "./DepMap";
import { CallFlow } from "./CallFlow";
import { Splitter, initialSplit } from "./Splitter";
import type { TypeLinkHandlers } from "./Signature";

/** Where the reader is, for the location history. */
export interface WorkbenchLocation {
  selection: string | null;
  scroll: number;
}

export interface WorkbenchProps {
  view: FileView;
  /** A symbol to focus when the file opens (from search, a link or go to definition). */
  focusName?: string | null;
  focusLine?: number | null;
  /** Changes on every open, so the same symbol can be focused again. */
  focusNonce?: number;
  /** Restore a previous selection and scroll (Back, Forward). Changes with `restoreNonce`. */
  restore?: WorkbenchLocation | null;
  restoreNonce?: number;
  onOpenFile(path: string, opts?: { line?: number | null; symbol?: string | null }): void;
  /** Record the current location before an in-file go to definition. */
  onPushHistory(): void;
  /** Lets the app read the current location when it records history. */
  registerLocation(get: (() => WorkbenchLocation) | null): void;
  /** Open the AI panel (the "Turn on AI…" line). */
  onOpenAi(): void;
  /** What the summary line shows for this file. */
  ai: FileAi;
  /** Summarize this file now. */
  onSummarize(): void;
}

type Mode = "map" | "flow";

const usePrefs = () => useSyncExternalStore(subscribePrefs, getPrefs);

export function Workbench({ view, focusName, focusLine, focusNonce, restore, restoreNonce, onOpenFile, onPushHistory, registerLocation, onOpenAi, ai, onSummarize }: WorkbenchProps) {
  const path = view.path;
  const cards = view.functions;
  const prefs = usePrefs();
  const detail = prefs.detail;
  const peek = usePeek();
  const [sel, setSel] = useState<Target | null>(null);
  const [hover, setHover] = useState<Target | null>(null);
  const [transitive, setTransitive] = useState(false);
  const [showTypes, setShowTypes] = useState(false);
  const [order, setOrder] = useState<Order>("entry");
  const [q, setQ] = useState("");
  const [openCode, setOpenCode] = useState<Set<string>>(new Set());
  const [allOpen, setAllOpen] = useState(false);
  const [relOpen, setRelOpen] = useState<Map<string, Set<Rel>>>(new Map());
  const [moreParams, setMoreParams] = useState<Set<string>>(new Set());
  const [typesOpen, setTypesOpen] = useState(false);
  const [importsOpen, setImportsOpen] = useState(false);
  const [typeFocus, setTypeFocus] = useState<{ name: string; nonce: number } | null>(null);
  const [importersOpen, setImportersOpen] = useState(false);
  const [mode, setMode] = useState<Mode>("map");
  const [stack, setStack] = useState<string[]>([]);
  const [builtins, setBuiltins] = useState(false);
  const [showKey, setShowKey] = useState(false);
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
  const showExt = prefs.mapExternal;

  const syms = useMemo(() => symbolIndex(view), [view]);
  const idx = useMemo(() => indexTargets(view.targets ?? []), [view]);
  const unitIds = useMemo(() => new Set(view.flowUnits.map((u) => u.id)), [view]);
  const unitOwner = useMemo(() => Object.fromEntries(view.flowUnits.map((u) => [u.id, u.owner])), [view]);
  const unitTitles = useMemo(() => Object.fromEntries(view.flowUnits.map((u) => [u.id, unitTitle(u)])), [view]);
  const byName = useMemo(() => new Map(cards.map((c) => [c.name, c])), [cards]);
  const typeNames = useMemo(() => new Set(view.types.map((t) => t.name)), [view]);
  const groups = useMemo(() => groupByLayer(cards, order), [cards, order]);
  const texts = useMemo(() => new Map(cards.map((c) => [c.name, searchText(c)])), [cards]);
  const query = q.trim().toLowerCase();
  const matches = useMemo(() => new Set(cards.filter((c) => !query || texts.get(c.name)!.includes(query)).map((c) => c.name)), [cards, texts, query]);
  const S = useMemo(() => (sel ? focusSets(cards, sel, { transitive, showTypes }) : null), [sel, cards, transitive, showTypes]);
  const loops = useMemo(() => anyLoops(Object.values(flows)), [flows]);

  // Reset per-file state when another file opens (per-card overrides are session-only).
  useEffect(() => {
    setSel(null);
    setHover(null);
    setMode("map");
    setStack([]);
    setCur(-1);
    setOpenCode(new Set());
    setAllOpen(false);
    setRelOpen(new Map());
    setMoreParams(new Set());
    setTypesOpen(cards.length === 0);
    setImportsOpen(cards.length === 0);
    setTypeFocus(null);
    setImportersOpen(false);
    setCodeStep(null);
    setFlows({});
    setQ("");
  }, [path, cards.length]);

  // The app records history from here: the selected card and how far the list is scrolled.
  const selRef = useRef(sel);
  selRef.current = sel;
  useEffect(() => {
    registerLocation(() => ({ selection: selRef.current?.kind === "fn" ? selRef.current.id : null, scroll: leftRef.current?.scrollTop ?? 0 }));
    return () => registerLocation(null);
  }, [registerLocation]);

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
    if (!el || !leftRef.current) return;
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
      if (!getPrefs().flowKeySeen) {
        setShowKey(true);
        setPrefs({ flowKeySeen: true });
      }
    },
    [mode, loadFlow],
  );

  const select = useCallback(
    (t: Target | null, from: "graph" | "chip" | "list" | "code" | "history") => {
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
        if (from !== "list" && from !== "history") requestAnimationFrame(() => scrollCard(t!.id));
        if (from !== "graph") requestAnimationFrame(() => mapRef.current?.centerOn(t!.id));
      } else {
        setMode("map");
        const first = groups.flatMap((g) => g.cards).find((f) => f.uses.includes(t!.id) || f.typeUses.includes(t!.id));
        if (first && from !== "list") requestAnimationFrame(() => scrollCard(first.name));
      }
    },
    [sel, byName, unitIds, openFlow, scrollCard, groups],
  );

  // Open with a symbol focused (search result, impact link, go to definition from another file).
  useEffect(() => {
    if (!focusName && !focusLine) return;
    const card = (focusName && byName.get(focusName)) || cards.find((c) => focusLine && c.start <= focusLine && focusLine <= c.end);
    if (card) {
      const t = setTimeout(() => select({ kind: "fn", id: card.name }, "chip"), 60);
      return () => clearTimeout(t);
    }
    if (focusName && typeNames.has(focusName)) {
      setTypesOpen(true);
      setTypeFocus({ name: focusName, nonce: Date.now() });
    }
  }, [view, focusName, focusLine, focusNonce]);

  // Back and Forward: the same card selected and the same scroll position.
  useEffect(() => {
    if (!restore) return;
    if (restore.selection && byName.has(restore.selection)) select({ kind: "fn", id: restore.selection }, "history");
    else select(null, "history");
    let n = 0;
    const apply = () => {
      if (leftRef.current) leftRef.current.scrollTop = restore.scroll;
      if (++n < 4) requestAnimationFrame(apply);
    };
    requestAnimationFrame(apply);
  }, [restoreNonce]);

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

  // ---------- go to definition (spec section 2) ----------
  const cardOf = useCallback(
    (name: string) => {
      const c = byName.get(name);
      return c ? { desc: c.summary?.desc ?? c.doc, start: c.start, end: c.end } : null;
    },
    [byName],
  );
  const peekFor = useCallback(
    (name: string, target: SymbolTarget | null): SymbolPeek => ({
      kind: "symbol",
      name,
      target: target ?? idx.get(name) ?? null,
      idx,
      langLabel: view.langLabel,
      viewPath: path,
      card: cardOf(name),
      cardOf,
      onGoTo: (n, t) => goToRef.current(n, t ?? null),
    }),
    [idx, view.langLabel, path, cardOf],
  );

  /** Focus a function card, open its code and scroll to it. */
  const revealCard = useCallback(
    (name: string) => {
      setOpenCode((s) => (s.has(name) ? s : new Set(s).add(name)));
      select({ kind: "fn", id: name }, "code");
      requestAnimationFrame(() => scrollCard(name));
    },
    [select, scrollCard],
  );

  const goTo = useCallback(
    (name: string, target: SymbolTarget | null, anchor?: Element | null) => {
      peek.close();
      const t = target ?? idx.get(name) ?? null;
      if (byName.has(name) && (!t || t.origin === "local")) {
        onPushHistory();
        revealCard(name);
        return;
      }
      if (typeNames.has(name) || (t?.origin === "local" && t.path === path)) {
        onPushHistory();
        setTypesOpen(true);
        setTypeFocus({ name, nonce: Date.now() });
        return;
      }
      if (t && (t.origin === "imported" || t.origin === "local") && t.path) {
        onOpenFile(t.path, { symbol: name, line: t.start || null });
        return;
      }
      if (t && t.origin === "external" && anchor) peek.show(peekFor(name, t), anchor.getBoundingClientRect(), true);
    },
    [peek, idx, byName, typeNames, path, onPushHistory, revealCard, onOpenFile, peekFor],
  );
  const goToRef = useRef(goTo);
  goToRef.current = goTo;

  const links = useMemo<TypeLinkHandlers>(
    () => ({
      idx,
      peekFor: (name, target) => peekFor(name, target),
      onType: (name, target, el) => goToRef.current(name, target, el),
    }),
    [idx, peekFor],
  );

  // Keyboard: Esc clears, arrows step the flow, F focuses the filter, 1 2 3 pick the level.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      const typing = t.closest("input, textarea, select, .monaco-editor, [contenteditable]");
      if (e.key === "Escape") {
        if (t === filterRef.current && q) return;
        if (document.querySelector(".search-pop, .iso-peek, .menu-pop")) return;
        select(null, "list");
        return;
      }
      if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (mode === "flow" && (e.key === "ArrowRight" || e.key === "ArrowLeft") && !t.closest(".splitter, .mmap-canvas")) {
        e.preventDefault();
        setStep(e.key === "ArrowRight" ? cur + 1 : cur < 0 ? 0 : cur - 1);
        return;
      }
      const d = detailForKey(e.key);
      if (d) {
        e.preventDefault();
        setPrefs({ detail: d });
        return;
      }
      if (e.key === "f") {
        e.preventDefault();
        filterRef.current?.focus();
      } else if (mode === "map" && (e.key === "+" || e.key === "=" || e.key === "-")) {
        e.preventDefault();
        mapRef.current?.zoomBy(e.key === "-" ? 0.8 : 1.25);
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
  const toggleRel = useCallback((name: string, r: Rel) => {
    setRelOpen((m) => {
      const n = new Map(m);
      const s = new Set(n.get(name) ?? []);
      if (s.has(r)) s.delete(r);
      else s.add(r);
      n.set(name, s);
      return n;
    });
  }, []);
  const onMoreParams = useCallback((name: string) => setMoreParams((s) => new Set(s).add(name)), []);

  const onHeader = useCallback(
    (name: string, cmd: boolean) => {
      if (cmd) {
        onPushHistory();
        revealCard(name);
      } else select(sel && sel.kind === "fn" && sel.id === name ? null : { kind: "fn", id: name }, "list");
    },
    [select, sel, revealCard, onPushHistory],
  );
  const onTarget = useCallback(
    (t: Target, cmd: boolean) => {
      if (!cmd) select(t, "chip");
      else goToRef.current(t.id, idx.get(t.id) ?? null);
    },
    [select, idx],
  );
  const onPreview = useCallback((name: string | null) => setHover(name ? { kind: "fn", id: name } : null), []);
  const openFileCb = useCallback((p: string, line?: number | null) => onOpenFile(p, { line }), [onOpenFile]);

  const codeContext = useMemo<CodeContext>(
    () => ({
      path,
      cards: new Map(cards.map((c) => [c.name, { name: c.name, start: c.start, end: c.end, desc: c.summary?.desc ?? c.doc }])),
      syms: new Map(
        [...syms.entries()].map(([n, s]) => [n, { name: n, label: s.label, from: s.module.id, member: s.sym.member, receiver: s.sym.member ? s.label.split(".")[0] : null, typeOnly: s.sym.typeOnly }]),
      ),
      types: new Map(view.types.map((t) => [t.name, { name: t.name, start: t.start, end: t.end, desc: t.summary ?? t.doc }])),
      targets: idx,
      onGoTo: (name) => goToRef.current(name, null),
      onJumpToCard: (name) => select({ kind: "fn", id: name }, "code"),
      onSymbol: (name) => select({ kind: "sym", id: name }, "code"),
      onHoverName: (name, rect) => peek.hoverAt(name ? (peekFor(name, null) as PeekSubject) : null, rect),
    }),
    [path, cards, syms, view.types, idx, select, peek, peekFor],
  );
  const cardContexts = useMemo(() => new Map(cards.map((c) => [c.name, { ...codeContext, self: c.name }])), [cards, codeContext]);

  const goToStep = useCallback(
    (it: CallItem) => {
      const t = it.target;
      const name = t.kind === "method" ? (t.of ?? null) : (t.id ?? null);
      if (name) goToRef.current(name, idx.get(name) ?? null);
    },
    [idx],
  );
  const openStepPath = useCallback(
    (it: CallItem) => {
      const name = it.target.id ?? "";
      const t = idx.get(name);
      if (t?.path) onOpenFile(t.path, { symbol: name, line: t.start || null });
      else {
        const m = view.modules.find((x) => x.id === it.target.module);
        if (m?.resolved) onOpenFile(m.resolved, { line: null });
      }
    },
    [idx, view.modules, onOpenFile],
  );

  const entries = cards.filter((c) => c.exported);
  const hiddenMap = mode === "flow";
  const unitRef = curUnit ? view.flowUnits.find((u) => u.id === curUnit.id) : undefined;
  const flowDesc = curUnit ? (unitRef?.summary ?? (curUnit.method ? null : (byName.get(curUnit.owner)?.summary?.desc ?? byName.get(curUnit.owner)?.doc ?? null))) : null;
  const flowSig = curUnit && !curUnit.method ? (byName.get(curUnit.owner)?.sig ?? null) : null;
  const startAt = (
    <span className="chips">
      {entries.map((c) => (
        <button key={c.name} type="button" className="iso-chip" onClick={(e) => (e.metaKey || e.ctrlKey ? goToRef.current(c.name, null) : select({ kind: "fn", id: c.name }, "chip"))}>
          {c.name}
        </button>
      ))}
    </span>
  );
  const nDeps = S ? S.deps.size : 0;
  const nUsers = S ? S.users.size : 0;
  const allDetail = detail === "dig";

  return (
    <div className="workbench">
      <FileHeader view={view} detail={detail} />
      <div className="main" ref={mainRef} style={{ ["--left" as string]: split + "%" }}>
        <div className="pane-left" ref={leftRef}>
          <div className="inner">
            <Overview view={view} detail={detail} prefs={prefs} startAt={entries.length ? startAt : null} importersOpen={importersOpen || allDetail} onImporters={() => setImportersOpen(true)} onOpenFile={onOpenFile} onOpenAi={onOpenAi} ai={ai} onSummarize={onSummarize} />

            <div className="toolbar" role="toolbar" aria-label="List controls" ref={toolbarRef}>
              <input ref={filterRef} className="search" type="search" placeholder="Filter functions (f)" aria-label="Filter functions" value={q} onChange={(e) => setQ(e.target.value)} />
              <Seg
                label="Detail"
                value={detail}
                onChange={(d: Detail) => setPrefs({ detail: d })}
                options={[
                  ["glance", "Glance", "One row per function (1)"],
                  ["read", "Read", "Descriptions, signatures and steps (2)"],
                  ["dig", "Dig", "Everything open (3)"],
                ]}
              />
              <Menu
                label={order === "entry" ? "Entry points first" : "Leaves first"}
                ariaLabel="Order"
                align="left"
                items={[
                  { kind: "radio", label: "Entry points first", checked: order === "entry", onSelect: () => setOrder("entry") },
                  { kind: "radio", label: "Leaves first", checked: order === "deps", onSelect: () => setOrder("deps") },
                ]}
              />
              {allDetail && (
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
              )}
              {sel && (
                <div className="focusbar">
                  <span>
                    Focused on <code>{sel.kind === "sym" ? (syms.get(sel.id)?.label ?? sel.id) : sel.id}</code> · <span className="fb-dep">{nDeps} depends on</span> · <span className="fb-user">{nUsers} used by</span>
                  </span>
                  <button type="button" className="iso-btn quiet" onClick={() => select(null, "list")}>
                    Clear (Esc)
                  </button>
                </div>
              )}
            </div>

            {!prefs.hintSeen && (
              <div className="hint-strip" role="note">
                <span>Hover a type to see its definition. Click to go there, ⌘-click anything to open its definition, ⌘[ to come back. Hold ⌘ to see every link.</span>
                <button type="button" className="iso-btn quiet" onClick={() => setPrefs({ hintSeen: true })}>
                  Got it
                </button>
              </div>
            )}

            {cards.length === 0 && <p className="muted empty-note">No functions in this file. Its imports and types are listed below.</p>}
            {query && matches.size === 0 && (
              <p className="muted empty-note">
                No function matches “{q}”.{" "}
                <button type="button" className="iso-btn quiet" onClick={() => setQ("")}>
                  Clear filter (Esc)
                </button>
              </p>
            )}
            {groups.map((g) => {
              const visible = g.cards.filter((c) => matches.has(c.name));
              return (
                <section className={`group group-${detail}`} key={g.level} hidden={visible.length === 0}>
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
                      detail={detail}
                      codeOpen={openCode.has(c.name)}
                      syms={syms}
                      codeContext={cardContexts.get(c.name)!}
                      links={links}
                      relOpen={relOpen.get(c.name) ?? EMPTY_SET}
                      allParams={moreParams.has(c.name)}
                      stepLine={codeStep && codeStep.card === c.name ? codeStep.line : null}
                      stepNonce={codeStep && codeStep.card === c.name ? codeStep.nonce : 0}
                      onToggleCode={toggleCode}
                      onToggleRel={toggleRel}
                      onMoreParams={onMoreParams}
                      onHeader={onHeader}
                      onTarget={onTarget}
                      onPreview={onPreview}
                      onOpenFile={openFileCb}
                    />
                  ))}
                </section>
              );
            })}

            {detail !== "glance" && (
              <>
                <LocalTypes view={view} context={codeContext} open={typesOpen || allDetail} onToggle={setTypesOpen} focus={typeFocus} />
                <Imports view={view} context={codeContext} open={importsOpen || allDetail} onToggle={setImportsOpen} onOpenFile={(p) => onOpenFile(p)} onTarget={(t) => select(t, "chip")} />
              </>
            )}
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
                onClick={() => {
                  if (mode === "flow") return;
                  if (stack.length) {
                    flyFrom.current = mapRef.current?.captureRects() ?? null;
                    setPlay((p) => ({ key: p.key + 1, animate: true }));
                  }
                  setMode("flow");
                }}
              >
                Call flow
              </button>
            </div>
            {mode === "map" ? (
              <div className="gctl">
                <button type="button" className="iso-btn" aria-pressed={showExt} onClick={() => setPrefs({ mapExternal: !showExt })}>
                  External imports
                </button>
                <button type="button" className="iso-btn" onClick={() => mapRef.current?.fit(true)}>
                  Fit
                </button>
                <Menu
                  label="More"
                  items={[
                    { kind: "toggle", label: "Type-only imports", checked: showTypes, onSelect: () => setShowTypes(!showTypes) },
                    { kind: "toggle", label: "Transitive", checked: transitive, onSelect: () => setTransitive(!transitive), hint: "impact" },
                    { kind: "sep" },
                    { kind: "action", label: "Zoom in", hint: "+", onSelect: () => mapRef.current?.zoomBy(1.25) },
                    { kind: "action", label: "Zoom out", hint: "−", onSelect: () => mapRef.current?.zoomBy(0.8) },
                  ]}
                />
              </div>
            ) : (
              <div className="fctl">
                <button type="button" className="iso-btn" title="" disabled={!stack.length} onClick={() => setPlay((p) => ({ key: p.key + 1, animate: true }))}>
                  Replay
                </button>
                <button type="button" className="iso-btn" aria-label="Previous call" disabled={!stack.length} onClick={() => setStep(cur < 0 ? 0 : cur - 1)}>
                  ◀
                </button>
                <span className="fpos" aria-live="polite">
                  {cur < 0 ? "–" : cur + 1} / {stepCount}
                </span>
                <button type="button" className="iso-btn" aria-label="Next call" disabled={!stack.length} onClick={() => setStep(cur + 1)}>
                  ▶
                </button>
                <button type="button" className="iso-btn" aria-pressed={showKey} onClick={() => setShowKey(!showKey)}>
                  Key
                </button>
                {allDetail ? (
                  <button
                    type="button"
                    className="iso-btn"
                    aria-pressed={builtins}
                    onClick={() => {
                      setBuiltins(!builtins);
                      setCur(-1);
                      setPlay((p) => ({ key: p.key + 1, animate: false }));
                    }}
                  >
                    Built-ins
                  </button>
                ) : (
                  <Menu
                    label="More"
                    items={[
                      {
                        kind: "toggle",
                        label: "Built-ins",
                        hint: "Date.now() and the like",
                        checked: builtins,
                        onSelect: () => {
                          setBuiltins(!builtins);
                          setCur(-1);
                          setPlay((p) => ({ key: p.key + 1, animate: false }));
                        },
                      },
                    ]}
                  />
                )}
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
              onGoTo={(name) => goToRef.current(name, null)}
              onShowExt={() => setPrefs({ mapExternal: true })}
            />
            {mode === "flow" &&
              (!stack.length ? (
                <section className="flowpane flow-empty">
                  <p className="muted">Select a function to see its calls in order.</p>
                  {entries.length > 0 && (
                    <div className="flow">
                      <span className="lab">Start at</span>
                      {startAt}
                    </div>
                  )}
                </section>
              ) : curUnit ? (
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
                  detail={detail}
                  sig={flowSig}
                  links={links}
                  showKey={showKey}
                  onGoToStep={goToStep}
                  onOpenStepPath={openStepPath}
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

const EMPTY_SET: ReadonlySet<Rel> = new Set();

/** The overview box: model text if any, else one line about summaries; "Start at"; "Imported by". */
function Overview({
  view,
  detail,
  prefs,
  startAt,
  importersOpen,
  onImporters,
  onOpenFile,
  onOpenAi,
  ai,
  onSummarize,
}: {
  view: FileView;
  detail: Detail;
  prefs: ReturnType<typeof getPrefs>;
  startAt: React.ReactNode;
  importersOpen: boolean;
  onImporters(): void;
  onOpenFile: WorkbenchProps["onOpenFile"];
  onOpenAi(): void;
  ai: FileAi;
  onSummarize(): void;
}) {
  const peek = usePeek();
  const [more, setMore] = useState(false);
  const imp = view.importers;
  const shown = importersOpen ? imp : imp.slice(0, 2);
  const aiLine = ai.kind !== "none" && !(ai.kind === "off" && prefs.aiNoticeHidden);
  if (!view.overview && !aiLine && !startAt && (detail === "glance" || !imp.length)) return null;
  return (
    <section className={`overview${view.overview ? "" : " overview-quiet"}`} aria-labelledby="ov-h">
      {view.overview && (
        <>
          <Eyebrow id="ov-h">What this file does</Eyebrow>
          <p className={detail === "glance" && !more ? "clamp3" : undefined}>
            <RichText text={view.overview.text} />
            <AiMark model={view.overview.model} />
          </p>
          {detail === "glance" && !more && (
            <button type="button" className="link-quiet" onClick={() => setMore(true)}>
              More
            </button>
          )}
        </>
      )}
      {aiLine && <AiLine ai={ai} hidden={prefs.aiNoticeHidden} onOpenAi={onOpenAi} onSummarize={onSummarize} id={view.overview ? undefined : "ov-h"} />}
      {startAt && (
        <div className="flow">
          <span className="lab">Start at</span>
          {startAt}
        </div>
      )}
      {detail !== "glance" && imp.length > 0 && (
        <div className="flow">
          <span className="lab">Imported by</span>
          <span className="chips importers">
            <span className="muted">{plural(imp.length, "file")}</span>
            {shown.map((f) => (
              <button
                key={f.path}
                type="button"
                className="pathlink"
                {...peek.bind(() => ({ kind: "text", title: f.path, body: <span className="muted">{f.line ? `Imports this file at line ${f.line}.` : "Imports this file."} Click to open it.</span> }))}
                onClick={() => onOpenFile(f.path, { line: f.line })}
              >
                {importersOpen ? f.path : middleEllipsis(basename(f.path), 18)}
              </button>
            ))}
            {!importersOpen && imp.length > 2 && (
              <button type="button" className="link-quiet" onClick={onImporters}>
                + {imp.length - 2} more
              </button>
            )}
          </span>
        </div>
      )}
    </section>
  );
}

function LocalTypes({ view, context, open, onToggle, focus }: { view: FileView; context: CodeContext; open: boolean; onToggle(o: boolean): void; focus: { name: string; nonce: number } | null }) {
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    if (!focus) return;
    requestAnimationFrame(() => {
      const el = ref.current?.querySelector<HTMLElement>(`[data-type="${CSS.escape(focus.name)}"]`);
      el?.scrollIntoView({ behavior: prefersReducedMotion() ? "auto" : "smooth", block: "start" });
    });
  }, [focus]);
  if (!view.types.length) return null;
  return (
    <details className="misc" open={open} ref={ref} onToggle={(e) => onToggle((e.target as HTMLDetailsElement).open)}>
      <summary>
        Local types<span>{plural(view.types.length, "declaration")}</span>
      </summary>
      <div className="tlist">
        {view.types.map((t) => (
          <div key={t.id} data-type={t.name} className={focus?.name === t.name ? "type-focus" : undefined}>
            <p>
              <code className="iso-fname">{t.name}</code> <span className="iso-lines">{lineRange(t.start, t.end)}</span>
              {t.exported && <span className="iso-pill">export</span>}
              <br />
              {t.summary ?? t.doc ?? null}
            </p>
            {open && <CodeView path={view.path} start={t.start} end={t.end} fit context={context} stepLine={focus?.name === t.name ? t.start : null} stepNonce={focus?.name === t.name ? focus.nonce : 0} ariaLabel={`Code of ${t.name}`} />}
          </div>
        ))}
      </div>
    </details>
  );
}

function Imports({ view, context, open, onToggle, onOpenFile, onTarget }: { view: FileView; context: CodeContext; open: boolean; onToggle(o: boolean): void; onOpenFile(p: string): void; onTarget(t: Target): void }) {
  if (!view.modules.length) return null;
  const r = view.importsRange;
  return (
    <details className="misc" open={open} onToggle={(e) => onToggle((e.target as HTMLDetailsElement).open)}>
      <summary>
        Imports<span>{r ? `lines ${r.start}–${r.end}` : plural(view.modules.length, "module")}</span>
      </summary>
      <div className="tlist">
        <ul className="imports">
          {view.modules.map((m) => (
            <li key={m.id}>
              {m.resolved ? (
                <button type="button" className="pathlink" onClick={() => onOpenFile(m.resolved!)}>
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
