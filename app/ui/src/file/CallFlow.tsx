// The call flow of one unit: numbered steps in execution order inside keyword-tagged frames,
// with the prototype's spine, staggered landing and fly-in from the map.
import { Fragment, memo, useEffect, useLayoutEffect, useMemo, useRef, type ReactNode } from "react";
import type { FlowUnit } from "../ipc/bindings/FlowUnit";
import { drillOf, exitKeyword, flowStats, keyOf, KIND_CLASS, metaParts, numberFlow, spinePath, STAT_LABELS, staggerMs, unitTitle, type CallItem, type FlowNode } from "../lib/flow";
import { lineRange } from "../lib/format";
import { prefersReducedMotion } from "../lib/theme";
import { StatPill } from "../components/ui";

export interface CallFlowProps {
  unit: FlowUnit;
  stack: string[];
  titles: Record<string, string>;
  units: ReadonlySet<string>;
  desc: string | null;
  builtins: boolean;
  cur: number;
  /** Changes whenever the flow should (re)play: open, drill, replay. */
  playKey: number;
  animate: boolean;
  /** Map rectangles captured before switching views, for the fly-in. Consumed once. */
  flyFrom: { current: Map<string, DOMRect> | null };
  hasLoops: boolean;
  onStep(i: number): void;
  onDrill(unit: string): void;
  onCrumb(depth: number): void;
  onLine(line: number): void;
}

export function CallFlow(p: CallFlowProps) {
  const { unit, builtins } = p;
  const bodyRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const spineRef = useRef<SVGSVGElement>(null);
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  const gen = useRef(0);

  const numbered = useMemo(() => numberFlow(unit.tree, builtins), [unit, builtins]);
  const stats = useMemo(() => flowStats(unit.tree, builtins), [unit, builtins]);

  const stopPlay = () => {
    gen.current++;
    timers.current.forEach(clearTimeout);
    timers.current = [];
  };
  const revealAll = () => {
    stopPlay();
    const body = bodyRef.current;
    if (!body) return;
    body.querySelectorAll<HTMLElement>(".hid, .off, .fly").forEach((e) => {
      e.classList.remove("hid", "off", "fly");
      e.style.transform = "";
      e.style.transition = "";
    });
    body.querySelectorAll(".sp-seg").forEach((x) => x.classList.add("on"));
  };

  const drawSpine = () => {
    const body = bodyRef.current;
    const sp = spineRef.current;
    if (!body || !sp) return;
    const br = body.getBoundingClientRect();
    sp.setAttribute("width", String(body.scrollWidth));
    sp.setAttribute("height", String(body.scrollHeight));
    const pts = [...body.querySelectorAll(".ftree .iso-step .iso-step-num")].map((n) => {
      const r = n.getBoundingClientRect();
      return { x: r.left - br.left + r.width / 2, y1: r.top - br.top, y2: r.bottom - br.top };
    });
    const on = new Set([...sp.querySelectorAll(".sp-seg.on")].map((s) => s.getAttribute("data-i")));
    // Frames with no steps in them: the spine goes around them, never through their header or "no calls".
    const empty = [...body.querySelectorAll<HTMLElement>(".ftree .iso-blk")]
      .filter((b) => !b.querySelector(".iso-step"))
      .map((b) => {
        const r = b.getBoundingClientRect();
        return { left: r.left - br.left, right: r.right - br.left, top: r.top - br.top, bottom: r.bottom - br.top };
      });
    let h = "";
    for (let i = 1; i < pts.length; i++) {
      h += `<path class="sp-seg${on.has(String(i)) ? " on" : ""}" data-i="${i}" pathLength="1" d="${spinePath(pts[i - 1], pts[i], empty)}"/>`;
    }
    sp.innerHTML = h;
  };

  const play = (src: Map<string, DOMRect> | null) => {
    stopPlay();
    const body = bodyRef.current;
    if (!body) return;
    if (prefersReducedMotion()) {
      revealAll();
      return;
    }
    const els = [...body.querySelectorAll<HTMLElement>(".ftree .iso-step, .ftree .iso-blk, .ftree .iso-exit, .ftree .iso-nocall")];
    els.forEach((e) => e.classList.add(e.classList.contains("iso-blk") ? "off" : "hid"));
    const g = gen.current;
    const steps = els.filter((e) => e.classList.contains("iso-step")).length || 1;
    const gap = staggerMs(steps);
    let t = 30;
    let step = 0;
    els.forEach((e) => {
      const isStep = e.classList.contains("iso-step");
      const my = isStep ? ++step : 0;
      const from = src && isStep && e.dataset.key ? src.get(e.dataset.key) : undefined;
      timers.current.push(
        setTimeout(() => {
          if (g !== gen.current) return;
          if (e.classList.contains("iso-blk")) {
            e.classList.remove("off");
            return;
          }
          if (from) {
            e.style.transition = "none";
            e.style.transform = "none";
            e.classList.remove("hid");
            const d = e.getBoundingClientRect();
            e.style.transform = `translate(${from.left - d.left}px, ${from.top - d.top}px)`;
            e.classList.add("fly");
            e.getBoundingClientRect();
            e.style.transition = "transform .38s cubic-bezier(.2,.8,.2,1), box-shadow .2s";
            e.style.transform = "";
            timers.current.push(setTimeout(() => e.classList.remove("fly"), 400));
          } else e.classList.remove("hid");
          if (my > 1) body.querySelector(`.sp-seg[data-i="${my - 1}"]`)?.classList.add("on");
        }, t),
      );
      t += isStep ? gap : gap / 3;
    });
    timers.current.push(
      setTimeout(() => {
        if (g === gen.current) timers.current = [];
      }, t + 420),
    );
  };

  // (Re)play on open, drill and replay; reveal at once when only the content changed.
  useLayoutEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
    // Steps are reused across units; do not leave focus on a step that now means something else.
    const active = document.activeElement as HTMLElement | null;
    if (active && bodyRef.current?.contains(active) && p.cur < 0) active.blur();
    drawSpine();
    if (p.animate) {
      const src = p.flyFrom.current;
      p.flyFrom.current = null;
      play(src);
    } else revealAll();
    return stopPlay;
  }, [p.playKey, unit, builtins]);

  // Keep the spine attached when the pane resizes.
  useEffect(() => {
    const body = bodyRef.current;
    if (!body) return;
    const ro = new ResizeObserver(() => {
      drawSpine();
      if (!timers.current.length) body.querySelectorAll(".sp-seg").forEach((x) => x.classList.add("on"));
    });
    ro.observe(body);
    return () => ro.disconnect();
  }, []);

  // Stepping: the current step is revealed, scrolled to and focused.
  useLayoutEffect(() => {
    if (p.cur < 0) return;
    if (timers.current.length) revealAll();
    const el = bodyRef.current?.querySelector<HTMLElement>(`.iso-step[data-i="${p.cur}"]`);
    if (!el) return;
    el.scrollIntoView({ block: "nearest", behavior: prefersReducedMotion() ? "auto" : "smooth" });
    el.focus({ preventScroll: true });
  }, [p.cur]);

  const onClick = (e: React.MouseEvent) => {
    const t = e.target as HTMLElement;
    const dr = t.closest<HTMLElement>("[data-unit]");
    if (dr) {
      p.onDrill(dr.dataset.unit!);
      return;
    }
    const st = t.closest<HTMLElement>(".iso-step");
    if (st) {
      if (timers.current.length) revealAll();
      p.onStep(Number(st.dataset.i));
      return;
    }
    const other = t.closest<HTMLElement>(".iso-blk-h, .iso-exit");
    if (other) {
      if (timers.current.length) revealAll();
      p.onLine(Number(other.closest<HTMLElement>("[data-line]")!.dataset.line));
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const st = (e.target as HTMLElement).closest<HTMLElement>(".iso-step");
    if (st && e.key === "Enter") {
      const d = st.querySelector<HTMLElement>("[data-unit]");
      if (d) p.onDrill(d.dataset.unit!);
      else p.onStep(Number(st.dataset.i));
    }
  };

  return (
    <section className="flowpane" aria-label="Call flow">
      <nav className="crumbs" aria-label="Drill-down path">
        {p.stack.map((id, i) => (
          <Fragment key={id + i}>
            {i > 0 && <span className="sep">›</span>}
            {i === p.stack.length - 1 ? (
              <span className="crumb cur">{p.titles[id] ?? id}</span>
            ) : (
              <button type="button" className="crumb" onClick={() => p.onCrumb(i)}>
                {p.titles[id] ?? id}
              </button>
            )}
          </Fragment>
        ))}
      </nav>
      <div className="fscroll" ref={scrollRef}>
        <div className="fbody" ref={bodyRef} onClick={onClick} onKeyDown={onKeyDown}>
          <header className="fentry">
            <div className="sig">
              <span className="sigtext">
                <span className="kw">
                  {unit.async ? "async " : ""}
                  {unit.method ? "method" : "function"}
                </span>{" "}
                <code>
                  {unit.method ?? unit.id}({unit.params.join(", ")})
                </code>
              </span>
              <span className="iso-lines">{lineRange(unit.start, unit.end)}</span>
            </div>
            {unit.method && (
              <div className="mt">
                on the controller returned by{" "}
                <button type="button" className="iso-chip" data-unit={unit.owner}>
                  {unit.owner}
                </button>
              </div>
            )}
            {p.desc && <p>{p.desc}</p>}
            <div className="fsum">
              {STAT_LABELS.map(([k, noun]) => (
                <StatPill key={k} n={stats[k]} noun={noun} />
              ))}
            </div>
          </header>
          <FlowTree nodes={numbered.nodes} units={p.units} cur={p.cur} />
          <svg className="spine" ref={spineRef} aria-hidden="true" />
        </div>
      </div>
      <FlowLegend hasLoops={p.hasLoops} />
    </section>
  );
}

const FlowTree = memo(function FlowTree({ nodes, units, cur }: { nodes: FlowNode[]; units: ReadonlySet<string>; cur: number }) {
  return <div className="ftree">{nodes.length ? renderNodes(nodes, units, cur) : <div className="iso-nocall">This function makes no calls.</div>}</div>;
});

function renderNodes(nodes: FlowNode[], units: ReadonlySet<string>, cur: number): ReactNode[] {
  return nodes.map((nd, i) => {
    if (nd.t === "hidden") return nd.inner.length ? <div key={i} className="iso-inner">{renderNodes(nd.inner, units, cur)}</div> : null;
    if (nd.t === "step") {
      const it = nd.item;
      const d = drillOf(it, units);
      return (
        <Fragment key={i}>
          <div className={`iso-step ${KIND_CLASS[it.target.kind]}${cur === nd.n - 1 ? " iso-step-cur" : ""}`} data-i={nd.n - 1} data-line={it.line} data-key={keyOf(it.target)} tabIndex={0} aria-current={cur === nd.n - 1 ? "step" : undefined}>
            <span className="iso-step-num">{nd.n}</span>
            <span className="iso-step-body">
              <code>{it.label}()</code>
              {it.await && <span className="iso-await">await</span>}
              <StepMeta it={it} />
            </span>
            {d ? (
              <button type="button" className="iso-btn-drill" data-unit={d} title={`Show the calls inside ${it.label}()`}>
                Open ›
              </button>
            ) : (
              <span />
            )}
          </div>
          {nd.inner.length > 0 && <div className="iso-inner">{renderNodes(nd.inner, units, cur)}</div>}
        </Fragment>
      );
    }
    if (nd.t === "block") {
      const it = nd.item;
      const body = renderNodes(nd.children, units, cur).filter(Boolean);
      return (
        <div key={i} className={`iso-blk iso-b-${it.kind}`} data-line={it.line}>
          <div className="iso-blk-h">
            <span className="iso-kw">{it.kw}</span>
            {it.label && <code>{it.label}</code>}
          </div>
          <div className="bc">{body.length ? body : <div className="iso-nocall">no calls</div>}</div>
        </div>
      );
    }
    const it = nd.item;
    return (
      <div key={i} className={`iso-exit x-${it.kind}${it.kind === "throw" ? " iso-exit-throw" : ""}${it.early ? " iso-exit-early" : ""}`} data-line={it.line}>
        <span className="iso-kw">{exitKeyword(it)}</span>
        {it.label && <code>{it.label}</code>}
        {it.methods && (
          <span className="mchips">
            {it.methods.map((m) =>
              m.unit && units.has(m.unit) ? (
                <button key={m.name} type="button" className="iso-chip" data-unit={m.unit}>
                  .{m.name}()
                </button>
              ) : (
                <span key={m.name} className="iso-chip static" title={m.from ? `function returned by ${m.from}()` : undefined}>
                  .{m.name}()
                </span>
              ),
            )}
          </span>
        )}
      </div>
    );
  });
}

function StepMeta({ it }: { it: CallItem }) {
  return (
    <span className="iso-step-meta">
      {metaParts(it).map((bit, i) => (
        <Fragment key={i}>
          {i > 0 && " · "}
          {bit.map((p, j) => (p.mono ? <code key={j}>{p.text}</code> : <Fragment key={j}>{p.text}</Fragment>))}
        </Fragment>
      ))}
    </span>
  );
}

function Sw({ c, s, dashed }: { c: string; s?: string; dashed?: boolean }) {
  return <span className="sw" style={{ ["--c" as string]: c, ["--s" as string]: s ?? "transparent", borderStyle: dashed ? "dashed" : undefined }} />;
}

function FlowLegend({ hasLoops }: { hasLoops: boolean }) {
  return (
    <div className="legend flegend">
      <div className="row">
        <span>
          <Sw c="var(--accent)" />
          this file
        </span>
        <span>
          <Sw c="var(--ext)" />
          external import
        </span>
        <span>
          <Sw c="var(--method)" />
          controller method or local helper
        </span>
        <span>
          <Sw c="var(--muted)" />
          parameter or built-in
        </span>
      </div>
      <div className="row">
        <span>
          <Sw c="var(--cond)" s="var(--cond-soft)" />
          conditional
        </span>
        <span>
          <Sw c="var(--try)" s="var(--try-soft)" />
          try
        </span>
        <span>
          <Sw c="var(--warn)" s="var(--warn-soft)" />
          catch
        </span>
        <span>
          <Sw c="var(--loop)" s="var(--loop-soft)" />
          loop{hasLoops ? "" : " (none in this file)"}
        </span>
        <span>
          <Sw c="var(--accent)" dashed />
          runs inside the call above
        </span>
      </div>
      <div>Numbers are execution order: arguments run before the call that receives them. Click a call to see its line in the code; Open › drills in.</div>
    </div>
  );
}

export { unitTitle };
