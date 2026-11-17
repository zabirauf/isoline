// What the main area shows with a repo open and no file: summary pills, languages, support tiers
// with what each means, and the module map.
import { useMemo, useRef, useState } from "react";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import type { RepoSummary } from "../ipc/bindings/RepoSummary";
import type { Tier } from "../ipc/bindings/Tier";
import { api } from "../ipc/api";
import { fmtInt, num, plural } from "../lib/format";
import { ModuleDot, StatPill, TIER_HELP, TierBadge } from "./ui";
import { ModuleMapCanvas, type ModuleMapHandle } from "./ModuleMapCanvas";

const SOURCE_TEXT = { cluster: "grouped by directory and imports", ai: "named by AI", user: "named by you" } as const;
const LIST_LIMIT = 200;

export function RepoOverview({ repo, moduleMap, onModuleRenamed }: { repo: RepoSummary; moduleMap: ModuleMap | null; onModuleRenamed(): void }) {
  const [sel, setSel] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const mapRef = useRef<ModuleMapHandle>(null);
  const maxLines = Math.max(1, ...repo.languages.map((l) => num(l.lines)));
  const mods = useMemo(() => (moduleMap ? [...moduleMap.modules].sort((a, b) => num(b.lines) - num(a.lines)) : []), [moduleMap]);
  const selected = moduleMap?.modules.find((m) => m.id === sel) ?? null;
  const deps = moduleMap && sel ? moduleMap.edges.filter((e) => e.from === sel).length : 0;
  const users = moduleMap && sel ? moduleMap.edges.filter((e) => e.to === sel).length : 0;

  return (
    <div className="repo-page">
      <header className="top">
        <div>
          <h1>{repo.name}</h1>
          <div className="file">
            {repo.root} · {plural(repo.files, "file")} · {fmtInt(num(repo.lines))} lines
          </div>
        </div>
        <div className="stats">
          <StatPill n={repo.files} noun="file" />
          <StatPill n={num(repo.lines)} noun="line" />
          <StatPill n={repo.indexed} noun="indexed" pluralForm="indexed" />
          <StatPill n={repo.languages.length} noun="language" />
          {moduleMap && <StatPill n={moduleMap.modules.length} noun="module" />}
          {moduleMap && moduleMap.cycles.length > 0 && <StatPill n={moduleMap.cycles.length} noun="module cycle" />}
        </div>
      </header>
      <div className="repo-scroll">
        <section className="panel map-panel" aria-label="Module map">
          <div className="rbar">
            <div className="display-heading">Module map</div>
            <span className="caption">{moduleMap ? `${plural(moduleMap.modules.length, "module")} · ${plural(moduleMap.edges.length, "dependency", "dependencies")}` : "Loading…"}</span>
            <div className="gctl">
              <button type="button" className="iso-btn" aria-label="Zoom out" onClick={() => mapRef.current?.zoomBy(0.8)}>
                −
              </button>
              <button type="button" className="iso-btn" aria-label="Zoom in" onClick={() => mapRef.current?.zoomBy(1.25)}>
                +
              </button>
              <button type="button" className="iso-btn" onClick={() => mapRef.current?.fit()}>
                Fit
              </button>
            </div>
          </div>
          <div className="stage">
            {moduleMap && <ModuleMapCanvas ref={mapRef} map={moduleMap} selected={sel} onSelect={setSel} />}
            <div className="legend">
              {selected ? (
                <div>
                  <b className="mono">{selected.name}</b> · {selected.role ?? selected.dir} · depends on {plural(deps, "module")}, used by {plural(users, "module")}
                </div>
              ) : (
                <div className="row">
                  <span>
                    <i style={{ background: "var(--accent)" }} />
                    depends on
                  </span>
                  <span>
                    <i style={{ background: "var(--user)" }} />
                    used by
                  </span>
                </div>
              )}
              <div>Entry points first; arrows run from a module to what it imports. Scroll to zoom, drag to pan.</div>
            </div>
          </div>
        </section>
        <div className="repo-panels">
          <section className="panel">
            <div className="eyebrow">Languages</div>
            <table className="langs">
              <thead>
                <tr>
                  <th scope="col">
                    <span className="sr-only">Language</span>
                  </th>
                  <th scope="col">Files</th>
                  <th scope="col">Lines</th>
                  <th scope="col">
                    <span className="sr-only">Share of lines</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {repo.languages.map((l) => (
                  <tr key={l.lang}>
                    <td>{l.label}</td>
                    <td className="n">{fmtInt(l.files)}</td>
                    <td className="n">{fmtInt(num(l.lines))}</td>
                    <td className="bar">
                      <span style={{ width: `${(num(l.lines) / maxLines) * 100}%` }} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
          <section className="panel">
            <div className="eyebrow">Support tiers</div>
            <p className="muted">Every file opens. How much Isoline can tell you about it depends on its language.</p>
            <ul className="tiers">
              {(["full", "outline", "text"] as Tier[]).map((t) => (
                <li key={t}>
                  <TierBadge tier={t} />
                  <b>{fmtInt(repo.tiers[t])}</b>
                  <span>{TIER_HELP[t]}</span>
                </li>
              ))}
            </ul>
          </section>
          {moduleMap && (
            <section className="panel">
              <div className="eyebrow">Modules</div>
              <ul className={`modlist${mods.length > 8 ? " scrolls" : ""}`}>
                {mods.slice(0, LIST_LIMIT).map((m) => (
                  <li key={m.id} className={m.id === sel ? "on" : ""}>
                    <ModuleDot color={m.color} />
                    {renaming === m.id ? (
                      <form
                        className="rename"
                        onSubmit={(e) => {
                          e.preventDefault();
                          const name = new FormData(e.currentTarget).get("name") as string;
                          void api.renameModule(m.id, name.trim() || undefined).then(() => {
                            setRenaming(null);
                            onModuleRenamed();
                          });
                        }}
                      >
                        <input name="name" className="search" defaultValue={m.name} aria-label={`New name for ${m.name}`} autoFocus onKeyDown={(e) => e.key === "Escape" && setRenaming(null)} />
                        <button type="submit" className="iso-btn">
                          Save
                        </button>
                      </form>
                    ) : (
                      <button type="button" className="modname" onClick={() => setSel(m.id === sel ? null : m.id)} aria-pressed={m.id === sel}>
                        {m.name}
                      </button>
                    )}
                    <span className="caption">
                      <code>{m.dir}</code> · {plural(m.files, "file")} · {SOURCE_TEXT[m.source]}
                    </span>
                    {renaming !== m.id && (
                      <button type="button" className="iso-btn-drill" onClick={() => setRenaming(m.id)}>
                        Rename
                      </button>
                    )}
                  </li>
                ))}
              </ul>
              {mods.length > LIST_LIMIT && <p className="caption">The {fmtInt(LIST_LIMIT)} largest of {plural(mods.length, "module")}. The map shows them all.</p>}
            </section>
          )}
        </div>
      </div>
    </div>
  );
}
