// Module-level map on canvas, for repo scale. Level of detail by zoom: dots when far out, boxes
// in the middle, names and captions when close. Only what is on screen is drawn, and a frame is
// drawn only when something changed, so panning stays smooth with 5,000 modules.
import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import type { ModuleInfo } from "../ipc/bindings/ModuleInfo";
import { layoutModules, MOD_H, MOD_W, SpatialGrid } from "../lib/moduleLayout";
import { fmtInt, num, plural } from "../lib/format";
import { onThemeChange, effectiveTheme } from "../lib/theme";

export interface ModuleMapHandle {
  fit(): void;
  zoomBy(f: number): void;
  /** For the scale test: how many frames were drawn and how long the slowest took. */
  stats(): { frames: number; worstMs: number; lastMs: number };
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

export const ModuleMapCanvas = forwardRef<ModuleMapHandle, Props>(function ModuleMapCanvas({ map, selected, onSelect }, ref) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const tip = useRef<HTMLDivElement>(null);
  const T = useRef({ k: 1, x: 0, y: 0 });
  const dirty = useRef(true);
  const raf = useRef(0);
  const hover = useRef(-1);
  const perf = useRef({ frames: 0, worstMs: 0, lastMs: 0 });
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
    const pad = L.vertical ? 190 : 0;
    // Leave room at the bottom for the legend that sits over the canvas.
    const legend = 84;
    const k = Math.min(1.2, Math.min(w / (L.W + 80 + pad), (h - 40 - legend) / (L.H + 60)));
    T.current = { k, x: (w - (L.W - pad) * k) / 2, y: (h - legend - L.H * k) / 2 + 14 };
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
    if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
    }
    const P = (pal.current ??= readPalette(c));
    const { k, x, y } = T.current;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = P.bg;
    ctx.fillRect(0, 0, w, h);
    ctx.setTransform(dpr * k, 0, 0, dpr * k, dpr * x, dpr * y);
    const x0 = -x / k;
    const y0 = -y / k;
    const x1 = (w - x) / k;
    const y1 = (h - y) / k;
    const sel = selRef.current >= 0 ? selRef.current : hover.current;
    const focus = sel >= 0;
    const depSet = new Set<number>();
    const userSet = new Set<number>();
    if (focus) {
      for (const e of out.o[sel]) depSet.add(L.to[e]);
      for (const e of out.inc[sel]) userSet.add(L.from[e]);
    }

    // Column heads.
    if (k > 0.25) {
      ctx.font = `600 ${11 / Math.max(k, 0.6)}px "IBM Plex Sans Condensed", sans-serif`;
      ctx.fillStyle = P.muted;
      ctx.strokeStyle = P.grid;
      ctx.lineWidth = 1 / k;
      for (const col of L.columns) {
        if (L.vertical) {
          if (col.y > y1 || col.y + MOD_H < y0) continue;
          ctx.fillText(col.label.toUpperCase(), -24 - ctx.measureText(col.label.toUpperCase()).width, col.y + MOD_H / 2 + 4);
          continue;
        }
        if (col.x > x1 || col.x + MOD_W < x0) continue;
        ctx.fillText(col.label.toUpperCase(), col.x, -24);
        ctx.beginPath();
        ctx.moveTo(col.x, -16);
        ctx.lineTo(col.x + MOD_W, -16);
        ctx.stroke();
      }
    }

    // Edges: straight when far out, curves when close; only those crossing the viewport.
    const curves = k > 0.3;
    const edgePath = (i: number) => {
      const a = L.from[i];
      const b = L.to[i];
      const v = L.vertical;
      const sx = v ? L.x[a] + MOD_W / 2 : L.x[a] + MOD_W;
      const sy = v ? L.y[a] + MOD_H : L.y[a] + MOD_H / 2;
      const tx = v ? L.x[b] + MOD_W / 2 : L.x[b];
      const ty = v ? L.y[b] : L.y[b] + MOD_H / 2;
      if (Math.max(sx, tx) < x0 || Math.min(sx, tx) > x1 || Math.max(sy, ty) < y0 || Math.min(sy, ty) > y1) return;
      ctx.moveTo(sx, sy);
      if (curves && v) {
        const dy = Math.max(30, Math.abs(ty - sy) * 0.45);
        ctx.bezierCurveTo(sx, sy + dy, tx, ty - (ty > sy ? dy : -dy), tx, ty);
      } else if (curves) {
        const dx = Math.max(40, (tx - sx) * 0.45);
        ctx.bezierCurveTo(sx + dx, sy, tx - dx, ty, tx, ty);
      } else ctx.lineTo(tx, ty);
    };
    const E = L.from.length;
    if (!focus || E < 20000) {
      ctx.beginPath();
      for (let i = 0; i < E; i++) edgePath(i);
      ctx.strokeStyle = P.muted;
      ctx.globalAlpha = focus ? 0.06 : k < 0.15 ? 0.12 : 0.3;
      ctx.lineWidth = Math.max(0.6, 1.1 / Math.max(k, 0.5)) / Math.max(1, 1 / k / 4);
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
    if (focus) {
      ctx.lineWidth = 1.8 / Math.min(1, k * 1.5);
      ctx.beginPath();
      for (const e of out.o[sel]) edgePath(e);
      ctx.strokeStyle = P.accent;
      ctx.stroke();
      ctx.beginPath();
      for (const e of out.inc[sel]) edgePath(e);
      ctx.strokeStyle = P.user;
      ctx.stroke();
    }

    // Nodes.
    const vis = grid.query(x0, y0, x1, y1);
    const boxes = k > 0.12;
    const byColor = new Map<string, number[]>();
    for (const i of vis) {
      const col = map.modules[i].color;
      let arr = byColor.get(col);
      if (!arr) byColor.set(col, (arr = []));
      arr.push(i);
    }
    const alphaOf = (i: number) => (!focus || i === sel || depSet.has(i) || userSet.has(i) ? 1 : 0.25);
    if (boxes) {
      for (const i of vis) {
        ctx.globalAlpha = alphaOf(i);
        ctx.fillStyle = i === sel ? P.surface : depSet.has(i) ? P.accentSoft : userSet.has(i) ? P.userSoft : P.surface;
        ctx.fillRect(L.x[i], L.y[i], MOD_W, MOD_H);
      }
      ctx.lineWidth = 1 / Math.min(1, k * 1.2);
      for (const i of vis) {
        ctx.globalAlpha = alphaOf(i);
        ctx.strokeStyle = i === sel ? P.ink : depSet.has(i) ? P.accent : userSet.has(i) ? P.user : P.line;
        ctx.lineWidth = (i === sel ? 2 : depSet.has(i) || userSet.has(i) ? 1.6 : 1) / Math.min(1, k * 1.2);
        ctx.strokeRect(L.x[i], L.y[i], MOD_W, MOD_H);
      }
    }
    for (const [col, idx] of byColor) {
      ctx.fillStyle = themed(col, dark.current);
      for (const i of idx) {
        ctx.globalAlpha = alphaOf(i);
        if (boxes) ctx.fillRect(L.x[i], L.y[i], 5, MOD_H);
        else ctx.fillRect(L.x[i], L.y[i], MOD_W, MOD_H);
      }
    }
    ctx.globalAlpha = 1;
    if (k * 12 >= 7) {
      ctx.textBaseline = "alphabetic";
      for (const i of vis) {
        const m = map.modules[i];
        ctx.globalAlpha = alphaOf(i);
        ctx.fillStyle = P.ink;
        ctx.font = '600 12px "IBM Plex Mono", monospace';
        ctx.fillText(m.name, L.x[i] + 14, L.y[i] + 19, MOD_W - 22);
        if (k >= 0.75) {
          ctx.fillStyle = P.muted;
          ctx.font = '10.5px "IBM Plex Sans", sans-serif';
          ctx.fillText(`${plural(m.files, "file")} · ${fmtInt(num(m.lines))} lines`, L.x[i] + 14, L.y[i] + 35, MOD_W - 22);
        }
      }
      ctx.globalAlpha = 1;
    }
    const ms = performance.now() - t0;
    perf.current = { frames: perf.current.frames + 1, worstMs: Math.max(perf.current.worstMs, ms), lastMs: ms };
  }

  drawRef.current = draw;

  // Frame timings for the scale check in scripts/screenshots.mjs.
  useEffect(() => {
    (window as unknown as { __isolineMapStats?: () => object }).__isolineMapStats = () => ({ ...perf.current, modules: map.modules.length });
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

  // Frame timings for the scale check in scripts/screenshots.mjs.
  useEffect(() => {
    (window as unknown as { __isolineMapStats?: () => unknown }).__isolineMapStats = () => ({ ...perf.current });
  }, []);

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
      const r = c.getBoundingClientRect();
      zoomAt(Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top);
    };
    c.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      c.removeEventListener("wheel", onWheel);
      cancelAnimationFrame(raf.current);
      raf.current = 0;
    };
  }, []);

  const drag = useRef<{ x: number; y: number; tx: number; ty: number; moved: boolean } | null>(null);
  const world = (e: { clientX: number; clientY: number }) => {
    const r = canvas.current!.getBoundingClientRect();
    const { k, x, y } = T.current;
    return { wx: (e.clientX - r.left - x) / k, wy: (e.clientY - r.top - y) / k, sx: e.clientX - r.left, sy: e.clientY - r.top };
  };
  const showTip = (i: number, sx: number, sy: number) => {
    const el = tip.current!;
    if (i < 0) {
      el.classList.remove("on");
      return;
    }
    const m: ModuleInfo = map.modules[i];
    el.innerHTML = "";
    const b = document.createElement("b");
    b.textContent = m.name;
    el.append(b, `${m.dir} · ${plural(m.files, "file")} · ${fmtInt(num(m.lines))} lines`);
    if (m.role) el.append(document.createElement("br"), m.role);
    el.append(document.createElement("br"), `depends on ${out.o[i].length} · used by ${out.inc[i].length}`);
    el.classList.add("on");
    const w = wrap.current!.clientWidth;
    el.style.left = (sx + 314 > w ? Math.max(8, sx - 314) : sx + 14) + "px";
    el.style.top = sy + 14 + "px";
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
