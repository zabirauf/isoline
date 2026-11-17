// The change graph: changed modules as boxes (with their +/− lines), dependencies that appeared
// (loop green), vanished (warn, dashed) or changed weight (muted, +n/−n), and the unchanged modules
// at their other end (dashed boxes). Click a module to filter the file list to it.
import { useMemo } from "react";
import type { ModuleChange } from "../ipc/bindings/ModuleChange";
import type { ModuleEdgeChange } from "../ipc/bindings/ModuleEdgeChange";
import { buildChangeGraph, CG_H, CG_W, edgeLabel, type CGEdge, type CGNode } from "../lib/changeGraph";
import { fitMiddle, fmtInt, plural } from "../lib/format";
import { usePeek } from "../components/Peek";

let measureCtx: CanvasRenderingContext2D | null = null;
function measure(font: string) {
  measureCtx ??= document.createElement("canvas").getContext("2d");
  return (t: string) => {
    if (!measureCtx) return t.length * 7;
    measureCtx.font = font;
    return measureCtx.measureText(t).width;
  };
}

const EDGE_TEXT = { added: "New dependency", removed: "Dependency removed", more: "More imports", fewer: "Fewer imports" } as const;

export function ChangeGraphView({ modules, edges, info, selected, onSelect }: { modules: ModuleChange[]; edges: ModuleEdgeChange[]; info(id: string): { name: string; color: string } | null; selected: string | null; onSelect(id: string | null): void }) {
  const g = useMemo(() => buildChangeGraph(modules, edges, info), [modules, edges, info]);
  const peek = usePeek();
  const maxLines = Math.max(1, ...modules.map((m) => m.additions + m.deletions));
  const nameFit = useMemo(() => measure('600 12.5px "IBM Plex Sans", system-ui, sans-serif'), []);
  const byId = useMemo(() => new Map(g.nodes.map((n) => [n.id, n])), [g]);
  if (g.nodes.length === 0) return <p className="caption cg-empty">No module has changed files.</p>;

  const nodePeek = (n: CGNode) => () => ({
    kind: "text" as const,
    title: n.name,
    body: n.change ? (
      <>
        <code>{n.change.dir || "(repo root)"}</code>
        <div>
          {plural(n.change.files, "file")} changed · <span className="c-add">+{fmtInt(n.change.additions)}</span> <span className="c-del">−{fmtInt(n.change.deletions)}</span> · {plural(n.change.symbols, "symbol")} changed
        </div>
        <div className="muted">Click to show only its files.</div>
      </>
    ) : (
      <>No changed files. It is here because a dependency on it changed.</>
    ),
  });
  const edgePeek = (e: CGEdge) => () => ({
    kind: "text" as const,
    title: EDGE_TEXT[e.kind],
    body: (
      <>
        {byId.get(e.from)?.name} imports {byId.get(e.to)?.name}: {plural(e.before, "file-level import")} before, {e.after} after.
      </>
    ),
  });

  return (
    <div className="cg-wrap">
      <svg className="cg" viewBox={`0 0 ${g.width} ${g.height}`} width={g.width} height={g.height} role="img" aria-label={`Change graph: ${plural(modules.length, "changed module")}, ${plural(g.edges.length, "changed dependency", "changed dependencies")}`}>
        <defs>
          {(["added", "removed", "more"] as const).map((k) => (
            <marker key={k} id={`cg-arrow-${k}`} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
              <path d="M0,0 L8,4 L0,8 z" className={`cg-head cg-${k}`} />
            </marker>
          ))}
        </defs>
        {g.edges.map((e) => (
          <g key={`${e.from}>${e.to}`} className={`cg-edge cg-${e.kind}`} {...peek.bind(edgePeek(e))}>
            <path d={e.d} className="cg-hit" />
            <path d={e.d} className="cg-line" markerEnd={`url(#cg-arrow-${e.kind === "fewer" ? "more" : e.kind})`} />
            <text x={e.lx} y={e.ly - 5} textAnchor="middle" className="cg-elabel">
              {edgeLabel(e)}
            </text>
          </g>
        ))}
        {g.nodes.map((n) => {
          const c = n.change;
          const total = c ? c.additions + c.deletions : 0;
          const barW = c ? Math.max(4, ((CG_W - 24) * total) / maxLines) : 0;
          const addW = c && total ? (barW * c.additions) / total : 0;
          const pk = peek.bind(nodePeek(n));
          const toggle = () => n.changed && onSelect(selected === n.id ? null : n.id);
          return (
            <g
              key={n.id}
              className={`cg-node${n.changed ? " changed" : " context"}${selected === n.id ? " sel" : ""}`}
              transform={`translate(${n.x},${n.y})`}
              style={{ ["--mc" as string]: n.color }}
              role={n.changed ? "button" : undefined}
              tabIndex={n.changed ? 0 : -1}
              aria-label={n.changed ? `${n.name}: ${c?.files} files changed` : n.name}
              aria-pressed={n.changed ? selected === n.id : undefined}
              {...pk}
              onClick={toggle}
              onKeyDown={(ev) => {
                pk.onKeyDown(ev);
                if (ev.key === "Enter" || ev.key === " ") (ev.preventDefault(), toggle());
              }}
            >
              <rect width={CG_W} height={CG_H} rx={8} className="cg-box" />
              <rect x={0} y={8} width={3} height={CG_H - 16} rx={1.5} className="cg-mc" />
              <text x={12} y={19} className="cg-name">
                {fitMiddle(n.name, CG_W - 22, nameFit)}
              </text>
              {c ? (
                <>
                  <text x={12} y={35} className="cg-sub">
                    {plural(c.files, "file")} · <tspan className="c-add">+{fmtInt(c.additions)}</tspan> <tspan className="c-del">−{fmtInt(c.deletions)}</tspan>
                  </text>
                  <rect x={12} y={CG_H - 9} width={addW} height={3} rx={1.5} className="cg-bar-add" />
                  <rect x={12 + addW} y={CG_H - 9} width={barW - addW} height={3} rx={1.5} className="cg-bar-del" />
                </>
              ) : (
                <text x={12} y={35} className="cg-sub">
                  unchanged
                </text>
              )}
            </g>
          );
        })}
      </svg>
      {g.hidden > 0 && <p className="caption">The {fmtInt(modules.length - g.hidden)} most changed of {plural(modules.length, "module")} are shown.</p>}
    </div>
  );
}

export function ChangeGraphLegend() {
  return (
    <div className="cg-legend caption">
      <span>
        <i className="lg-line cg-added" /> new dependency
      </span>
      <span>
        <i className="lg-line cg-removed" /> removed
      </span>
      <span>
        <i className="lg-line cg-more" /> import count changed
      </span>
      <span>
        <i className="lg-box" /> unchanged module
      </span>
    </div>
  );
}
