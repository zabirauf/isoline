// Module-level map on canvas, for repo scale. Level of detail by zoom: one block per depth column
// with counts and bundled edges when far out, boxes in the middle, names when close (text is
// always drawn in screen space at a readable size). Only what is on screen is drawn, and a frame is
// drawn only when something changed, so panning stays smooth with 5,000 modules.
import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import type { ModuleInfo } from "../ipc/bindings/ModuleInfo";
import { layoutModules, mapDetail, MOD_H, MOD_W, SpatialGrid } from "../lib/moduleLayout";
import { fitMiddle, fmtInt, num, plural } from "../lib/format";
import { onThemeChange, effectiveTheme } from "../lib/theme";

export interface ModuleMapHandle {
  fit(): void;
  zoomBy(f: number): void;
  /**
   * For the scale test: how many frames were drawn and how long the slowest took, overall and
   * among frames that reused the base bitmap (every pan step but one in a few hundred pixels).
   */
  stats(): { frames: number; worstMs: number; lastMs: number; totalMs: number; worstBlitMs: number };
}

interface Props {
  map: ModuleMap;
  selected: string | null;
  onSelect(id: string | null): void;
}

type Pal = Record<"bg" | "surface" | "ink" | "muted" | "line" | "grid" | "accent" | "user" | "accentSoft" | "userSoft", string>;

function readPalette(el: Element): Pal {
  const cs = getComputedStyle(el);
  const v = (n: string) => cs.getPropertyValue(n).trim();
  return { bg: v("--bg"), surface: v("--surface"), ink: v("--ink"), muted: v("--muted"), line: v("--line"), grid: v("--grid"), accent: v("--accent"), user: v("--user"), accentSoft: v("--accent-soft"), userSoft: v("--user-soft") };
}

/** Module colors are given for the light theme; lighten them for dark so they keep contrast. */
function themed(color: string, dark: boolean): string {
  if (!dark || !/^#[0-9a-f]{6}$/i.test(color)) return color;
  const c = parseInt(color.slice(1), 16);
  const mix = (x: number) => Math.round(x + (255 - x) * 0.35);
  return `rgb(${mix(c >> 16)}, ${mix((c >> 8) & 255)}, ${mix(c & 255)})`;
}

/** Extra room around the view in the base bitmap, in CSS pixels: panning this far is a blit. */
const MARGIN = 160;
/** How long after the last wheel or pan step the base is drawn again, sharp and centred. */
const SETTLE_MS = 140;
/** More edges than this touching the view are drawn as bundles between depth columns. */
const EDGE_LIMIT = 2500;

/** The base layer drawn for one view: the view transform `k, x, y` it was drawn at. */
interface Base {
  c: HTMLCanvasElement;
  k: number;
  x: number;
  y: number;
  w: number;
  h: number;
  dpr: number;
  L: unknown;
  pal: Pal;
}

export const ModuleMapCanvas = forwardRef<ModuleMapHandle, Props>(function ModuleMapCanvas({ map, selected, onSelect }, ref) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const tip = useRef<HTMLDivElement>(null);
  const T = useRef({ k: 1, x: 0, y: 0 });
  const dirty = useRef(true);
  const raf = useRef(0);
  const hover = useRef(-1);
  const perf = useRef({ frames: 0, worstMs: 0, lastMs: 0, totalMs: 0, worstBlitMs: 0 });
  const base = useRef<Base | null>(null);
  const settle = useRef(0);
  const lastWheel = useRef(0);
  const rebuilds = useRef<string[]>([]);
  const [, setTick] = useState(0);
  const L = useMemo(() => layoutModules(map), [map]);
  const grid = useMemo(() => new SpatialGrid(L), [L]);
  const out = useMemo(() => {
    const o: number[][] = Array.from({ length: map.modules.length }, () => []);
    const inc: number[][] = Array.from({ length: map.modules.length }, () => []);
    for (let i = 0; i < L.from.length; i++) {
      o[L.from[i]].push(i);
      inc[L.to[i]].push(i);
    }
    return { o, inc };
  }, [L, map]);
  const pal = useRef<Pal | null>(null);
  const dark = useRef(effectiveTheme() === "dark");
  const selIdx = selected ? (L.index.get(selected) ?? -1) : -1;
  const selRef = useRef(selIdx);
  selRef.current = selIdx;

  // Always draw with the latest render's data, even from listeners registered once.
  const drawRef = useRef<() => void>(() => {});
  const request = () => {
    dirty.current = true;
    if (!raf.current) raf.current = requestAnimationFrame(() => drawRef.current());
  };

  const fit = () => {
    const c = canvas.current;
    if (!c) return;
    const w = c.clientWidth;
    const h = c.clientHeight;
    if (!w || !h) return;
    // Leave room for the column heads at the top and the legend over the bottom.
    const top = 34;
    const legend = 70;
    const k = Math.min(1.2, Math.min((w - 40) / Math.max(1, L.W), (h - top - legend - 20) / Math.max(1, L.H)));
    T.current = { k, x: (w - L.W * k) / 2, y: top + (h - top - legend - L.H * k) / 2 };
    request();
  };

  const zoomAt = (f: number, mx: number, my: number) => {
    const t = T.current;
    const k = Math.min(4, Math.max(0.02, t.k * f));
    T.current = { k, x: mx - (mx - t.x) * (k / t.k), y: my - (my - t.y) * (k / t.k) };
    request();
  };

  useImperativeHandle(ref, () => ({
    fit,
    zoomBy: (f) => {
      const c = canvas.current!;
      zoomAt(f, c.clientWidth / 2, c.clientHeight / 2);
    },
    stats: () => ({ ...perf.current }),
  }));

  // The base layer (every edge, box and name, nothing highlighted) is drawn into an offscreen
  // bitmap a margin larger than the view. Panning moves the bitmap; zooming scales it until the
  // wheel rests, then it is drawn again at the new scale. Hover and selection are drawn over it
  // each frame: a wash of the background dims the rest, then the focused module and its
  // neighbours are drawn on top. Stroking tens of thousands of edges is what costs (the
  // browser rasterizes them on commit), so it happens once per view, not once per frame.
  function drawBase(target: HTMLCanvasElement, w: number, h: number, dpr: number, k: number, x: number, y: number) {
    const ctx = target.getContext("2d")!;
    const P = pal.current!;
    const screen = () => ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const world = () => ctx.setTransform(dpr * k, 0, 0, dpr * k, dpr * x, dpr * y);
    screen();
    ctx.fillStyle = P.bg;
    ctx.fillRect(0, 0, w, h);
    const x0 = -x / k;
    const y0 = -y / k;
    const x1 = (w - x) / k;
    const y1 = (h - y) / k;
    const detail = mapDetail(k);
    const px = (n: number) => n / k;
    world();
    const vis = grid.query(x0, y0, x1, y1);
    if (detail === "aggregate") {
      bundles(ctx, k, 1);
      ctx.globalAlpha = 1;
      ctx.fillStyle = P.surface;
      ctx.strokeStyle = P.line;
      ctx.lineWidth = px(1);
      for (const col of L.columns) {
        ctx.fillRect(col.x - px(6), col.y0 - px(6), col.w + px(12), col.y1 - col.y0 + px(12));
        ctx.strokeRect(col.x - px(6), col.y0 - px(6), col.w + px(12), col.y1 - col.y0 + px(12));
      }
      ctx.fillStyle = P.muted;
      ctx.globalAlpha = 0.3;
      ctx.beginPath();
      for (const i of vis) ctx.rect(L.x[i], L.y[i], MOD_W, MOD_H);
      ctx.fill();
      ctx.globalAlpha = 1;
      return;
    }
    // Edges: one path. Only edges that start or end on screen; when even those are too many to
    // tell apart (and to rasterize quickly), bundled per pair of depth columns instead.
    const onScreen = new Uint8Array(L.x.length);
    for (const i of vis) onScreen[i] = 1;
    const E = L.from.length;
    const pick: number[] = [];
    for (let i = 0; i < E && pick.length <= EDGE_LIMIT; i++) if ((onScreen[L.from[i]] || onScreen[L.to[i]]) && edgeVisible(i, x0, y0, x1, y1)) pick.push(i);
    if (pick.length > EDGE_LIMIT) bundles(ctx, k, 0.8);
    else {
      const straight = detail === "boxes" || pick.length > EDGE_LIMIT / 4;
      ctx.beginPath();
      for (const i of pick) edgePath(ctx, i, straight);
      ctx.strokeStyle = P.muted;
      ctx.globalAlpha = 0.3;
      ctx.lineWidth = px(1.1);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    ctx.beginPath();
    for (const i of vis) ctx.rect(L.x[i], L.y[i], MOD_W, MOD_H);
    ctx.fillStyle = P.surface;
    ctx.fill();
    ctx.strokeStyle = P.line;
    ctx.lineWidth = px(1);
    ctx.stroke();
    if (detail === "labels") {
      screen();
      for (const i of vis) label(ctx, i, k, x, y);
    }
  }

  const edgeVisible = (i: number, x0: number, y0: number, x1: number, y1: number) => {
    const a = L.from[i];
    const b = L.to[i];
    const sx = L.x[a] + MOD_W;
    const tx = L.x[b];
    const sy = L.y[a];
    const ty = L.y[b];
    return !(Math.max(sx, tx) < x0 || Math.min(sx, tx) > x1 || Math.max(sy, ty) + MOD_H < y0 || Math.min(sy, ty) > y1);
  };
  const curve = (ctx: CanvasRenderingContext2D, sx: number, sy: number, tx: number, ty: number) => {
    ctx.moveTo(sx, sy);
    const dx = Math.max(40, Math.abs(tx - sx) * 0.45);
    ctx.bezierCurveTo(sx + dx, sy, tx - dx, ty, tx, ty);
  };
  const edgePath = (ctx: CanvasRenderingContext2D, i: number, straight: boolean) => {
    const a = L.from[i];
    const b = L.to[i];
    const sx = L.x[a] + MOD_W;
    const sy = L.y[a] + MOD_H / 2;
    const tx = L.x[b];
    const ty = L.y[b] + MOD_H / 2;
    if (straight) {
      ctx.moveTo(sx, sy);
      ctx.lineTo(tx, ty);
    } else curve(ctx, sx, sy, tx, ty);
  };
  /** Edges summed per pair of depth columns; back edges (cycles) arc above the columns. */
  const bundles = (ctx: CanvasRenderingContext2D, k: number, alpha: number) => {
    const P = pal.current!;
    ctx.strokeStyle = P.muted;
    for (const b of L.bundles) {
      const A = L.columns[b.from];
      const B = L.columns[b.to];
      ctx.beginPath();
      if (B.x > A.x) curve(ctx, A.x + A.w, (A.y0 + A.y1) / 2, B.x, (B.y0 + B.y1) / 2);
      else {
        const top = Math.min(A.y0, B.y0) - 26 / k;
        ctx.moveTo(A.x + A.w / 2, A.y0);
        ctx.bezierCurveTo(A.x + A.w / 2, top, B.x + B.w / 2, top, B.x + B.w / 2, B.y0);
      }
      ctx.lineWidth = Math.min(8, 1 + Math.log2(b.count)) / k;
      ctx.globalAlpha = alpha * (B.x > A.x ? 0.28 : 0.14);
      ctx.stroke();
    }
  };
  /** A module's name in screen space at a readable size: a color dot, the name, a caption if room. */
  const label = (ctx: CanvasRenderingContext2D, i: number, k: number, x: number, y: number) => {
    const P = pal.current!;
    const m = map.modules[i];
    const sx = L.x[i] * k + x;
    const sy = L.y[i] * k + y;
    const bw = MOD_W * k;
    const bh = MOD_H * k;
    const twoLines = bh >= 36;
    const ny = twoLines ? sy + bh / 2 - 3 : sy + bh / 2 + 4;
    ctx.fillStyle = themed(m.color, dark.current);
    ctx.beginPath();
    ctx.arc(sx + 10, ny - 4, 3.5, 0, Math.PI * 2);
    ctx.fill();
    ctx.font = '600 12px "IBM Plex Sans", sans-serif';
    ctx.fillStyle = P.ink;
    ctx.fillText(fitMiddle(m.name, bw - 26, (t) => ctx.measureText(t).width), sx + 19, ny);
    if (twoLines) {
      ctx.font = '11px "IBM Plex Sans", sans-serif';
      ctx.fillStyle = P.muted;
      const cap = bw >= 150 ? `${plural(m.files, "file")} · ${fmtInt(num(m.lines))} lines` : plural(m.files, "file");
      ctx.fillText(fitMiddle(cap, bw - 26, (t) => ctx.measureText(t).width), sx + 19, ny + 15);
    }
  };

  function draw() {
    raf.current = 0;
    if (!dirty.current) return;
    dirty.current = false;
    const c = canvas.current;
    if (!c) return;
    const t0 = performance.now();
    const ctx = c.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth;
    const h = c.clientHeight;
    if (!w || !h) return;
    if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
    }
    const P = (pal.current ??= readPalette(c));
    const { k, x, y } = T.current;
    const detail = mapDetail(k);

    // The base bitmap: reuse it when it covers the view at this scale, scale it while the wheel
    // turns (within reason), else draw it again centred on the view.
    let B = base.current;
    const same = !!B && B.w === w && B.h === h && B.dpr === dpr && B.L === L && B.pal === P;
    const ox = B ? x - B.x : 0;
    const oy = B ? y - B.y : 0;
    const covers = same && B!.k === k && ox <= MARGIN && ox >= -MARGIN && oy <= MARGIN && oy >= -MARGIN;
    const s = B ? k / B.k : 1;
    const zooming = performance.now() - lastWheel.current < SETTLE_MS;
    const preview = same && !covers && zooming && s > 0.5 && s < 3;
    const rebuilt = !covers && !preview;
    if (rebuilt) {
      const bw = w + 2 * MARGIN;
      const bh = h + 2 * MARGIN;
      const el = B?.c ?? document.createElement("canvas");
      if (el.width !== Math.round(bw * dpr) || el.height !== Math.round(bh * dpr)) {
        el.width = Math.round(bw * dpr);
        el.height = Math.round(bh * dpr);
      }
      const tb = performance.now();
      drawBase(el, bw, bh, dpr, k, x + MARGIN, y + MARGIN);
      rebuilds.current = [...rebuilds.current.slice(-19), `${detail} ${Math.round(performance.now() - tb)}`];
      B = base.current = { c: el, k, x, y, w, h, dpr, L, pal: P };
    }
    // Blit: bitmap pixel u lands at dpr * ((u / dpr - MARGIN) * s + (x - B.x * s)) on screen.
    const sc = k / B!.k;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    if (sc !== 1) {
      ctx.fillStyle = P.bg;
      ctx.fillRect(0, 0, c.width, c.height);
    }
    ctx.setTransform(sc, 0, 0, sc, dpr * (x - (B!.x + MARGIN) * sc), dpr * (y - (B!.y + MARGIN) * sc));
    ctx.drawImage(B!.c, 0, 0);
    if (sc !== 1 || ox !== 0 || oy !== 0) {
      // Draw it again, centred and sharp, once the view rests.
      clearTimeout(settle.current);
      settle.current = window.setTimeout(() => {
        base.current = base.current && { ...base.current, k: NaN };
        request();
      }, SETTLE_MS);
    }

    const screen = () => ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const world = () => ctx.setTransform(dpr * k, 0, 0, dpr * k, dpr * x, dpr * y);
    const px = (n: number) => n / k;
    const sel = selRef.current >= 0 ? selRef.current : hover.current;
    if (sel >= 0) {
      const depSet = new Set<number>();
      const userSet = new Set<number>();
      for (const e of out.o[sel]) depSet.add(L.to[e]);
      for (const e of out.inc[sel]) userSet.add(L.from[e]);
      screen();
      ctx.globalAlpha = 0.75;
      ctx.fillStyle = P.bg;
      ctx.fillRect(0, 0, w, h);
      ctx.globalAlpha = 1;
      world();
      ctx.lineWidth = px(1.8);
      ctx.beginPath();
      for (const e of out.o[sel]) edgePath(ctx, e, false);
      ctx.strokeStyle = P.accent;
      ctx.stroke();
      ctx.beginPath();
      for (const e of out.inc[sel]) edgePath(ctx, e, false);
      ctx.strokeStyle = P.user;
      ctx.stroke();
      const lit = [sel, ...depSet, ...userSet];
      if (detail === "aggregate") {
        ctx.fillStyle = P.ink;
        ctx.fillRect(L.x[sel] - px(2), L.y[sel] - px(2), MOD_W + px(4), MOD_H + px(4));
        ctx.fillStyle = P.accent;
        for (const i of depSet) ctx.fillRect(L.x[i], L.y[i], MOD_W, MOD_H);
        ctx.fillStyle = P.user;
        for (const i of userSet) ctx.fillRect(L.x[i], L.y[i], MOD_W, MOD_H);
      } else {
        for (const i of lit) {
          const dep = i !== sel && depSet.has(i);
          const use = i !== sel && !dep && userSet.has(i);
          ctx.fillStyle = dep ? P.accentSoft : use ? P.userSoft : P.surface;
          ctx.fillRect(L.x[i], L.y[i], MOD_W, MOD_H);
          ctx.strokeStyle = i === sel ? P.ink : dep ? P.accent : use ? P.user : P.line;
          ctx.lineWidth = px(i === sel ? 2 : 1.6);
          ctx.strokeRect(L.x[i], L.y[i], MOD_W, MOD_H);
        }
        if (detail === "labels") {
          screen();
          const x0 = -x / k - MOD_W;
          const x1 = (w - x) / k;
          const y0 = -y / k - MOD_H;
          const y1 = (h - y) / k;
          for (const i of lit) if (L.x[i] >= x0 && L.x[i] <= x1 && L.y[i] >= y0 && L.y[i] <= y1) label(ctx, i, k, x, y);
        }
      }
    }

    // Column heads in screen space on one line above the tallest column, like the file map.
    screen();
    ctx.textAlign = "left";
    const headY = Math.max(16, Math.min(...L.columns.map((col) => col.y0)) * k + y - (detail === "aggregate" ? 30 : 12));
    let lastRight = -Infinity;
    for (const col of L.columns) {
      const sx = col.x * k + x;
      const sw = Math.max(col.w * k, 1);
      if (sx + sw < 0 || sx > w) continue;
      ctx.font = '600 11px "IBM Plex Sans Condensed", sans-serif';
      const text = (sw >= 120 ? col.label : `Depth ${col.level}`).toUpperCase();
      const tw = ctx.measureText(text).width;
      if (sx < lastRight + 8) continue;
      ctx.fillStyle = P.muted;
      ctx.fillText(text, sx, headY);
      let right = sx + tw;
      if (detail === "aggregate") {
        ctx.font = '600 12px "IBM Plex Sans", sans-serif';
        ctx.fillStyle = P.ink;
        const count = fmtInt(col.count);
        ctx.fillText(count, sx, headY + 15);
        right = Math.max(right, sx + ctx.measureText(count).width);
      }
      lastRight = right;
    }
    const ms = performance.now() - t0;
    perf.current = { frames: perf.current.frames + 1, worstMs: Math.max(perf.current.worstMs, ms), lastMs: ms, totalMs: perf.current.totalMs + ms, worstBlitMs: rebuilt ? perf.current.worstBlitMs : Math.max(perf.current.worstBlitMs, ms) };
  }

  drawRef.current = draw;

  // Frame timings for the scale check in scripts/screenshots.mjs.
  useEffect(() => {
    const w = window as unknown as { __isolineMapStats?: () => object; __isolineMapReset?: () => void };
    w.__isolineMapStats = () => ({ ...perf.current, modules: map.modules.length, edges: map.edges.length, rebuilds: rebuilds.current });
    w.__isolineMapReset = () => {
      perf.current = { frames: 0, worstMs: 0, lastMs: 0, totalMs: 0, worstBlitMs: 0 };
      rebuilds.current = [];
    };
  }, [map]);

  // Fit on new data and keep the canvas sized to its box.
  useEffect(() => {
    fit();
    const el = wrap.current!;
    const ro = new ResizeObserver(() => request());
    ro.observe(el);
    return () => ro.disconnect();
  }, [L]);

  useEffect(() => request());

  useEffect(
    () =>
      onThemeChange((t) => {
        dark.current = t === "dark";
        pal.current = null;
        request();
      }),
    [],
  );

  useEffect(() => {
    const c = canvas.current!;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      lastWheel.current = performance.now();
      const r = c.getBoundingClientRect();
      zoomAt(Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top);
    };
    c.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      c.removeEventListener("wheel", onWheel);
      cancelAnimationFrame(raf.current);
      clearTimeout(settle.current);
      raf.current = 0;
    };
  }, []);

  const drag = useRef<{ x: number; y: number; tx: number; ty: number; moved: boolean } | null>(null);
  const world = (e: { clientX: number; clientY: number }) => {
    const r = canvas.current!.getBoundingClientRect();
    const { k, x, y } = T.current;
    return { wx: (e.clientX - r.left - x) / k, wy: (e.clientY - r.top - y) / k, sx: e.clientX - r.left, sy: e.clientY - r.top };
  };
  const tipFor = useRef(-1);
  const showTip = (i: number, sx: number, sy: number) => {
    const el = tip.current!;
    if (i < 0) {
      el.classList.remove("on");
      tipFor.current = -1;
      return;
    }
    const w = wrap.current!.clientWidth;
    el.style.left = (sx + 314 > w ? Math.max(8, sx - 314) : sx + 14) + "px";
    el.style.top = sy + 14 + "px";
    if (tipFor.current === i) return;
    tipFor.current = i;
    const m: ModuleInfo = map.modules[i];
    el.innerHTML = "";
    const b = document.createElement("b");
    b.textContent = m.name;
    el.append(b, `${m.dir} · ${plural(m.files, "file")} · ${fmtInt(num(m.lines))} lines`);
    if (m.role) el.append(document.createElement("br"), m.role);
    el.append(document.createElement("br"), `depends on ${out.o[i].length} · used by ${out.inc[i].length}`);
    el.classList.add("on");
  };

  return (
    <div className="mmap" ref={wrap}>
      <canvas
        ref={canvas}
        className="mmap-canvas"
        tabIndex={0}
        role="img"
        aria-label={`Module map: ${plural(map.modules.length, "module")}, ${plural(map.edges.length, "dependency", "dependencies")}`}
        onPointerDown={(e) => {
          drag.current = { x: e.clientX, y: e.clientY, tx: T.current.x, ty: T.current.y, moved: false };
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.classList.add("panning");
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (d) {
            const dx = e.clientX - d.x;
            const dy = e.clientY - d.y;
            if (Math.abs(dx) + Math.abs(dy) > 3) d.moved = true;
            T.current = { ...T.current, x: d.tx + dx, y: d.ty + dy };
            showTip(-1, 0, 0);
            request();
            return;
          }
          const p = world(e);
          const i = grid.hit(p.wx, p.wy);
          if (i !== hover.current) {
            hover.current = i;
            if (selRef.current < 0) request();
          }
          showTip(i, p.sx, p.sy);
        }}
        onPointerUp={(e) => {
          const d = drag.current;
          drag.current = null;
          e.currentTarget.classList.remove("panning");
          if (d && !d.moved) {
            const p = world(e);
            const i = grid.hit(p.wx, p.wy);
            onSelect(i >= 0 && i !== selRef.current ? map.modules[i].id : null);
            setTick((x) => x + 1);
          }
        }}
        onPointerLeave={() => {
          hover.current = -1;
          showTip(-1, 0, 0);
          request();
        }}
        onKeyDown={(e) => {
          const c = canvas.current!;
          const step = 60;
          if (e.key === "+" || e.key === "=") zoomAt(1.25, c.clientWidth / 2, c.clientHeight / 2);
          else if (e.key === "-") zoomAt(0.8, c.clientWidth / 2, c.clientHeight / 2);
          else if (e.key === "ArrowLeft") (T.current.x += step), request();
          else if (e.key === "ArrowRight") (T.current.x -= step), request();
          else if (e.key === "ArrowUp") (T.current.y += step), request();
          else if (e.key === "ArrowDown") (T.current.y -= step), request();
          else if (e.key === "Escape") onSelect(null);
          else return;
          e.preventDefault();
        }}
      />
      <div className="tip" ref={tip} />
    </div>
  );
});
