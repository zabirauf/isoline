// The dependency map for one file, as SVG: the prototype's layout, focus colors, pan and zoom.
import { forwardRef, useCallback, useEffect, useId, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type ReactElement } from "react";
import type { FileView } from "../ipc/bindings/FileView";
import { type SymInfo } from "../lib/cards";
import { edgeFocus, focusSets, type Target } from "../lib/focus";
import { edgePath, layoutMap, MOD_HEAD, SYM_ROW, symLabel, type MapLayout } from "../lib/mapLayout";
import { prefersReducedMotion } from "../lib/theme";

export interface DepMapHandle {
  fit(anim: boolean): void;
  centerOn(fn: string): void;
  zoomBy(f: number): void;
  /** Screen rectangles of nodes ("f:<card>") and symbols ("s:<name>"), for the call-flow fly-in. */
  captureRects(): Map<string, DOMRect>;
}

export interface DepMapProps {
  view: FileView;
  sel: Target | null;
  hover: Target | null;
  showExt: boolean;
  showTypes: boolean;
  transitive: boolean;
  query: string;
  matches: Set<string>;
  syms: Map<string, SymInfo>;
  hidden: boolean;
  onSelect(t: Target | null): void;
  onHover(t: Target | null): void;
  onOpenFile(path: string): void;
  /** ⌘-click on a function node: go to its definition. */
  onGoTo(name: string): void;
  /** The "+N modules" stub turns the external-imports column on. */
  onShowExt(): void;
}

/**
 * Below this scale node captions (10.5px) get too small to read; fit stops here and the reader
 * pans. The spec says 0.6, but at 0.6 captions render at about 6px, which fails its own
 * acceptance ("captions legible at default zoom"); 0.9 keeps them near 9.5px.
 */
export const MIN_FIT_SCALE = 0.9;

const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

export const DepMap = forwardRef<DepMapHandle, DepMapProps>(function DepMap(p, ref) {
  const svgRef = useRef<SVGSVGElement>(null);
  const vpRef = useRef<SVGGElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const T = useRef({ k: 1, x: 0, y: 0 });
  const uid = useId().replace(/:/g, "");
  const { view, showExt, showTypes } = p;

  const G: MapLayout = useMemo(() => layoutMap(view.functions, view.modules, { showExt, showTypes, collapseModules: true }), [view, showExt, showTypes]);
  const [hoverMod, setHoverMod] = useState<string | null>(null);
  const extModules = view.modules.filter((m) => m.symbols.some((s) => showTypes || !s.typeOnly)).length;

  const headsRef = useRef<SVGGElement>(null);
  const applyT = useCallback((anim: boolean) => {
    const vp = vpRef.current;
    if (!vp) return;
    const reduce = prefersReducedMotion();
    vp.classList.toggle("anim", anim && !reduce);
    vp.style.transform = `translate(${T.current.x}px, ${T.current.y}px) scale(${T.current.k})`;
    // Column heads are drawn at screen size, following the columns sideways only.
    const heads = headsRef.current;
    if (heads) {
      heads.classList.toggle("anim", anim && !reduce);
      const gs = [...heads.querySelectorAll<SVGGElement>("g[data-x]")];
      gs.forEach((g, i) => {
        const x = Number(g.dataset.x) * T.current.k + T.current.x;
        const w = Number(g.dataset.w) * T.current.k;
        const next = gs[i + 1] ? Number(gs[i + 1].dataset.x) * T.current.k + T.current.x : Infinity;
        g.style.transform = `translate(${x}px, 0)`;
        const line = g.querySelector("line");
        if (line) line.setAttribute("x2", String(w));
        const text = g.querySelector("text");
        if (!text) return;
        // The full label, else the short one, else nothing: heads never run into each other.
        const room = next - x - 10;
        text.textContent = g.dataset.label!;
        if (text.getComputedTextLength() > room) text.textContent = g.dataset.short!;
        text.style.visibility = text.getComputedTextLength() > room ? "hidden" : "";
      });
    }
  }, []);

  const fit = useCallback(
    (anim: boolean) => {
      const svg = svgRef.current;
      if (!svg) return;
      const r = svg.getBoundingClientRect();
      if (!r.width || !r.height) return;
      const bw = G.W + 40 + (showExt ? 0 : 120);
      const bh = G.H + 70;
      const fitK = Math.min(1.1, Math.min(r.width / bw, (r.height - 70) / bh) * 0.95);
      if (fitK >= MIN_FIT_SCALE) {
        T.current = { k: fitK, x: (r.width - (G.W + (showExt ? 0 : 120)) * fitK) / 2, y: Math.max(52, (r.height - 60 - G.H * fitK) / 2 + 30) };
      } else {
        // Too big to fit legibly: show the entry-point column at a readable scale; the reader pans.
        const k = MIN_FIT_SCALE;
        const first = G.cols[0] ?? [];
        const top = first.length ? Math.min(...first.map((n) => n.y)) : 0;
        const bottom = first.length ? Math.max(...first.map((n) => n.y + n.h)) : G.H;
        T.current = { k, x: 24, y: Math.max(52, (r.height - (bottom - top) * k) / 2 - top * k) };
      }
      applyT(anim);
    },
    [G, applyT, showExt],
  );

  const zoomBy = useCallback(
    (f: number) => {
      const r = svgRef.current!.getBoundingClientRect();
      const mx = r.width / 2;
      const my = r.height / 2;
      const t = T.current;
      const k = Math.min(3, Math.max(0.15, t.k * f));
      T.current = { k, x: mx - (mx - t.x) * (k / t.k), y: my - (my - t.y) * (k / t.k) };
      applyT(true);
    },
    [applyT],
  );

  useImperativeHandle(
    ref,
    () => ({
      fit,
      zoomBy,
      centerOn(fn: string) {
        const n = G.nodes["f:" + fn];
        const svg = svgRef.current;
        if (!n || !svg) return;
        const r = svg.getBoundingClientRect();
        const t = T.current;
        const cx = (n.x + n.w / 2) * t.k + t.x;
        const cy = (n.y + n.h / 2) * t.k + t.y;
        if (t.k >= 0.85 && cx > 40 && cx < r.width - 40 && cy > 60 && cy < r.height - 90) return;
        const k = Math.max(t.k, 0.95);
        T.current = { k, x: r.width / 2 - (n.x + n.w / 2) * k, y: r.height / 2 - (n.y + n.h / 2) * k };
        applyT(true);
      },
      captureRects() {
        const m = new Map<string, DOMRect>();
        const vp = vpRef.current;
        if (!vp) return m;
        vp.querySelectorAll<SVGGElement>(".node").forEach((g) => m.set("f:" + g.dataset.fn, g.querySelector("rect.box")!.getBoundingClientRect()));
        vp.querySelectorAll<SVGGElement>(".sym").forEach((g) => m.set("s:" + g.dataset.sym, g.getBoundingClientRect()));
        return m;
      },
    }),
    [G, fit, zoomBy, applyT],
  );

  // Refit when the layout changes and whenever the stage is resized (splitter, window).
  useLayoutEffect(() => fit(false), [fit]);
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    let t: ReturnType<typeof setTimeout> | undefined;
    const ro = new ResizeObserver(() => {
      clearTimeout(t);
      t = setTimeout(() => fit(false), 60);
    });
    ro.observe(svg);
    return () => {
      ro.disconnect();
      clearTimeout(t);
    };
  }, [fit]);

  // Wheel zoom must be non-passive to stop the page from scrolling.
  useEffect(() => {
    const svg = svgRef.current!;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = svg.getBoundingClientRect();
      const mx = e.clientX - r.left;
      const my = e.clientY - r.top;
      const t = T.current;
      const k = Math.min(3, Math.max(0.15, t.k * Math.exp(-e.deltaY * 0.0015)));
      T.current = { k, x: mx - (mx - t.x) * (k / t.k), y: my - (my - t.y) * (k / t.k) };
      applyT(false);
    };
    svg.addEventListener("wheel", onWheel, { passive: false });
    return () => svg.removeEventListener("wheel", onWheel);
  }, [applyT]);

  const drag = useRef<{ x: number; y: number; tx: number; ty: number; moved: boolean } | null>(null);

  const focusT = p.hover ?? p.sel;
  const S = useMemo(() => (focusT ? focusSets(view.functions, focusT, { transitive: p.transitive, showTypes }) : null), [focusT, view, p.transitive, showTypes]);

  const tip = (e: React.PointerEvent, html: string | null) => {
    const el = tipRef.current;
    const stage = el?.parentElement;
    if (!el || !stage) return;
    if (!html) {
      el.classList.remove("on");
      return;
    }
    el.innerHTML = html;
    el.classList.add("on");
    const r = stage.getBoundingClientRect();
    let x = e.clientX - r.left + 14;
    const y = e.clientY - r.top + 14;
    if (x + 310 > r.width) x = Math.max(8, e.clientX - r.left - 314);
    el.style.left = x + "px";
    el.style.top = y + "px";
  };

  const targetOf = (el: Element | null): Target | null => {
    const n = el?.closest<SVGGElement>(".node");
    if (n) return { kind: "fn", id: n.dataset.fn! };
    const s = el?.closest<SVGGElement>(".sym");
    if (s) return { kind: "sym", id: s.dataset.sym! };
    return null;
  };

  const q = p.query.trim().toLowerCase();
  const focusedEdges: ReactElement[] = [];
  const plainEdges: ReactElement[] = [];
  G.edges.forEach((e, i) => {
    const f = S && focusT ? edgeFocus(e.from, e.to, focusT, S, p.transitive) : { dep: false, user: false };
    const cls = `edge${e.kind === "call" ? "" : " ext"}${e.kind === "type" ? " type" : ""}${f.dep ? " dep" : ""}${f.user ? " user" : ""}`;
    const marker = f.dep ? "ad" : f.user ? "au" : e.kind === "call" ? "ar" : "ax";
    const el = <path key={i} className={cls} d={edgePath(e)} markerEnd={`url(#${uid}${marker})`} />;
    (f.dep || f.user ? focusedEdges : plainEdges).push(el);
  });

  return (
    <>
      <svg
        ref={svgRef}
        className="dmap"
        aria-label="Dependency map"
        role="group"
        onPointerDown={(e) => {
          if ((e.target as Element).closest(".node, .sym, .mp-link")) return;
          drag.current = { x: e.clientX, y: e.clientY, tx: T.current.x, ty: T.current.y, moved: false };
          svgRef.current!.setPointerCapture(e.pointerId);
          svgRef.current!.classList.add("panning");
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (d) {
            const dx = e.clientX - d.x;
            const dy = e.clientY - d.y;
            if (Math.abs(dx) + Math.abs(dy) > 3) d.moved = true;
            T.current = { ...T.current, x: d.tx + dx, y: d.ty + dy };
            applyT(false);
            return;
          }
          const t = targetOf(e.target as Element);
          if (t?.kind === "fn") {
            const f = view.functions.find((c) => c.name === t.id)!;
            tip(e, `<b>${esc(f.name)}</b>${esc(f.summary?.desc ?? f.doc ?? f.signature)}`);
          } else if (t?.kind === "sym") {
            const info = p.syms.get(t.id);
            const users = view.functions.filter((f) => f.uses.includes(t.id) || f.typeUses.includes(t.id)).length;
            tip(e, `<b>${esc(info?.label ?? t.id)}</b>${info?.sym.member ? `reached through a parameter` : `from ${esc(info?.module.id ?? "")}`}<br>used by ${users} function${users === 1 ? "" : "s"}`);
          } else tip(e, null);
        }}
        onPointerUp={() => {
          if (drag.current && !drag.current.moved) p.onSelect(null);
          drag.current = null;
          svgRef.current!.classList.remove("panning");
        }}
        onPointerOver={(e) => {
          const t = targetOf(e.target as Element);
          if (t && !(p.hover && p.hover.kind === t.kind && p.hover.id === t.id)) p.onHover(t);
        }}
        onPointerOut={(e) => {
          const rel = e.relatedTarget as Element | null;
          if (rel && rel.closest && rel.closest(".node, .sym")) return;
          tip(e, null);
          if (p.hover) p.onHover(null);
        }}
        onClick={(e) => {
          const t = targetOf(e.target as Element);
          if (t?.kind === "fn" && (e.metaKey || e.ctrlKey)) p.onGoTo(t.id);
          else if (t) p.onSelect(t);
        }}
        onKeyDown={(e) => {
          const t = targetOf(e.target as Element);
          if (t && (e.key === "Enter" || e.key === " ")) {
            e.preventDefault();
            if (t.kind === "fn" && (e.metaKey || e.ctrlKey)) p.onGoTo(t.id);
            else p.onSelect(t);
          }
        }}
      >
        <defs>
          {(
            [
              ["ar", "var(--muted)", 0.5],
              ["ax", "var(--ext)", 0.4],
              ["ad", "var(--accent)", 1],
              ["au", "var(--user)", 1],
            ] as const
          ).map(([id, fill, op]) => (
            <marker key={id} id={uid + id} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
              <path d="M0,0 L8,4 L0,8 z" style={{ fill, opacity: op }} />
            </marker>
          ))}
        </defs>
        <g ref={vpRef} className={`vp${focusT ? " focus" : ""}`}>
          {plainEdges}
          {focusedEdges}
          {G.cols.flat().map((n) => {
            if (n.kind === "fn") {
              const f = n.f;
              const st = focusT?.kind === "fn" && focusT.id === f.name ? " sel" : S?.deps.has(f.name) ? " dep" : S?.users.has(f.name) ? " user" : "";
              const nomatch = q && !p.matches.has(f.name) ? " nomatch" : "";
              return (
                <g key={n.key} className={`node${st}${nomatch}`} data-fn={f.name} transform={`translate(${n.x},${n.y})`} tabIndex={p.hidden ? -1 : 0} role="button" aria-label={f.name}>
                  <rect className="box" width={n.w} height={n.h} rx={7} />
                  {f.exported && <rect className="stripe" width={4} height={n.h} rx={2} />}
                  <text className="nm" x={14} y={21}>
                    {f.name}
                  </text>
                  <text className="cap" x={14} y={38}>
                    {n.cap}
                  </text>
                </g>
              );
            }
            const lit = n.vis.some((s) => (focusT?.kind === "sym" && focusT.id === s.name) || S?.syms.has(s.name));
            const nomatch = q && !(n.vis.some((s) => s.name.toLowerCase().includes(q)) || n.id.toLowerCase().includes(q)) ? " nomatch" : "";
            const resolved = n.m.resolved;
            // Collapsed to "N symbols" until the module is focused or hovered.
            const expanded = lit || hoverMod === n.id;
            const fullH = MOD_HEAD + n.vis.length * SYM_ROW + 6;
            const h = expanded ? fullH : n.h;
            return (
              <g
                key={n.key}
                className={`mod${lit ? " lit" : ""}${expanded ? " expanded" : ""}${nomatch}`}
                data-mod={n.id}
                transform={`translate(${n.x},${n.y})`}
                onPointerEnter={() => setHoverMod(n.id)}
                onPointerLeave={() => setHoverMod((m) => (m === n.id ? null : m))}
              >
                <rect className="box" width={n.w} height={h} rx={7} />
                <rect className="hd" width={n.w} height={22} rx={7} />
                <rect className="hd" y={12} width={n.w} height={10} />
                {resolved ? (
                  <text
                    className="mp mp-link"
                    x={10}
                    y={15}
                    role="link"
                    tabIndex={p.hidden ? -1 : 0}
                    onClick={(e) => {
                      e.stopPropagation();
                      p.onOpenFile(resolved);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") p.onOpenFile(resolved);
                    }}
                  >
                    <title>{`Open ${resolved}`}</title>
                    {n.label}
                  </text>
                ) : (
                  <text className="mp" x={10} y={15}>
                    {n.label}
                  </text>
                )}
                {!expanded && (
                  <text className="symcount" x={14} y={MOD_HEAD + 13}>
                    {n.vis.length} symbol{n.vis.length === 1 ? "" : "s"}
                  </text>
                )}
                {expanded && n.vis.map((s, i) => {
                  const ss = focusT?.kind === "sym" && focusT.id === s.name ? " sel" : S?.syms.has(s.name) ? " dep" : "";
                  return (
                    <g key={s.name} className={`sym${s.typeOnly ? " typeonly" : ""}${ss}`} data-sym={s.name} transform={`translate(0,${MOD_HEAD + i * SYM_ROW})`}>
                      <rect width={n.w} height={SYM_ROW} />
                      <text x={14} y={13}>
                        {symLabel(s)}
                      </text>
                      <circle cx={0} cy={SYM_ROW / 2} r={2.5} />
                    </g>
                  );
                })}
              </g>
            );
          })}
        </g>
        <g ref={headsRef} className="heads" aria-hidden="true">
          {G.colMeta.map((c, i) => (
            <g key={"c" + i + c.label} data-x={c.x} data-w={c.w} data-label={c.label} data-short={c.label.split(" · ")[0]}>
              <text className="colhead" x={0} y={22}>
                {c.label}
              </text>
              <line className="colrule" x1={0} x2={c.w} y1={30} y2={30} />
            </g>
          ))}
        </g>
      </svg>
      {G.cols.length === 0 && <p className="map-empty muted">No functions to map in this file.</p>}
      {!showExt && extModules > 0 && (
        <button type="button" className="ext-stub" onClick={p.onShowExt} title="">
          + {extModules} module{extModules === 1 ? "" : "s"}
        </button>
      )}
      {focusT && (
        <div className="legend mlegend legend-strip" aria-hidden="true">
          <span>
            <i style={{ background: "var(--accent)" }} />
            depends on
          </span>
          <span>
            <i style={{ background: "var(--user)" }} />
            used by
          </span>
          {showExt && (
            <span>
              <i style={{ background: "var(--ext)" }} />
              external import
            </span>
          )}
        </div>
      )}
      <div className="tip" ref={tipRef} />
    </>
  );
});
