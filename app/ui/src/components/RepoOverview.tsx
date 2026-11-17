// What the main area shows with a repo open and no file: one caption line of counts, the module
// map, and three short panels (languages, tiers, modules) that reveal more on demand.
import { useMemo, useRef, useState } from "react";
import type { IndexProgress } from "../ipc/bindings/IndexProgress";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import type { RepoSummary } from "../ipc/bindings/RepoSummary";
import type { Tier } from "../ipc/bindings/Tier";
import { api } from "../ipc/api";
import { fmtInt, num, plural } from "../lib/format";
import { ModuleDot, TIER_HELP, TierBadge } from "./ui";
import { ModuleMapCanvas, type ModuleMapHandle } from "./ModuleMapCanvas";

const SOURCE_TEXT = { cluster: "grouped by directory and imports", ai: "named by AI", user: "named by you" } as const;
const LIST_LIMIT = 200;
const TOP_LANGS = 5;
const PHASE_TEXT = { walk: "walking", parse: "parsing", resolve: "resolving imports", graph: "building the graph", done: "done" } as const;

export function RepoOverview({ repo, moduleMap, progress, onModuleRenamed }: { repo: RepoSummary; moduleMap: ModuleMap | null; progress: IndexProgress | null; onModuleRenamed(): void }) {
  const [sel, setSel] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [allLangs, setAllLangs] = useState(false);
  const [tiersOpen, setTiersOpen] = useState(false);
  const mapRef = useRef<ModuleMapHandle>(null);
  const maxLines = Math.max(1, ...repo.languages.map((l) => num(l.lines)));
  const mods = useMemo(() => (moduleMap ? [...moduleMap.modules].sort((a, b) => num(b.lines) - num(a.lines)) : []), [moduleMap]);
  const selected = moduleMap?.modules.find((m) => m.id === sel) ?? null;
  const deps = moduleMap && sel ? moduleMap.edges.filter((e) => e.from === sel).length : 0;
  const users = moduleMap && sel ? moduleMap.edges.filter((e) => e.to === sel).length : 0;
  const langs = allLangs ? repo.languages : repo.languages.slice(0, TOP_LANGS);
  const caption = [
    repo.root,
    plural(repo.files, "file"),
    `${fmtInt(num(repo.lines))} lines`,
    `${fmtInt(repo.indexed)} indexed`,
    plural(repo.languages.length, "language"),
    moduleMap ? plural(moduleMap.modules.length, "module") : null,
    moduleMap?.cycles.length ? plural(moduleMap.cycles.length, "module cycle") : null,
  ].filter(Boolean);

  return (
    <div className="repo-page">
      <header className="top">
        <div>
          <h1>{repo.name}</h1>
          <div className="file">{caption.join(" · ")}</div>
        </div>
      </header>
      <div className="repo-scroll">
        <section className="panel map-panel" aria-label="Module map">
          <div className="rbar">
            <div className="display-heading">Module map</div>
            <span className="caption">{moduleMap ? `${plural(moduleMap.modules.length, "module")} · ${plural(moduleMap.edges.length, "dependency", "dependencies")}` : ""}</span>
            <div className="gctl">
              <button type="button" className="iso-btn" onClick={() => mapRef.current?.fit()}>
                Fit
              </button>
            </div>
          </div>
          <div className="stage">
            {moduleMap ? (
              <ModuleMapCanvas ref={mapRef} map={moduleMap} selected={sel} onSelect={setSel} />
            ) : (
              <p className="muted map-wait">{progress ? `Building the module map · ${PHASE_TEXT[progress.phase]} ${fmtInt(progress.done)} / ${fmtInt(progress.total)}` : "Building the module map…"}</p>
            )}
            {selected && (
              <div className="legend legend-strip">
                <b className="mono">{selected.name}</b>
                <span>{selected.role ?? selected.dir}</span>
                <span>
                  <i style={{ background: "var(--accent)" }} />
                  depends on {plural(deps, "module")}
                </span>
                <span>
                  <i style={{ background: "var(--user)" }} />
                  used by {plural(users, "module")}
                </span>
              </div>
            )}
          </div>
        </section>
        <div className="repo-panels">
          <section className="panel short">
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
                {langs.map((l) => (
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
            {repo.languages.length > TOP_LANGS && (
              <button type="button" className="link-quiet" onClick={() => setAllLangs(!allLangs)}>
                {allLangs ? "Show fewer" : `+ ${repo.languages.length - TOP_LANGS} more`}
              </button>
            )}
          </section>
          <section className="panel short">
            <div className="eyebrow">Support tiers</div>
            <p>
              {fmtInt(repo.tiers.full)} full · {fmtInt(repo.tiers.outline)} outline · {fmtInt(repo.tiers.text)} text
            </p>
            <button type="button" className="link-quiet" aria-expanded={tiersOpen} onClick={() => setTiersOpen(!tiersOpen)}>
              What the tiers mean ›
            </button>
            {tiersOpen && (
              <ul className="tiers">
                {(["full", "outline", "text"] as Tier[]).map((t) => (
                  <li key={t}>
                    <TierBadge tier={t} />
                    <span>{TIER_HELP[t]}</span>
                  </li>
                ))}
              </ul>
            )}
          </section>
          {moduleMap && (
            <section className="panel short">
              <div className="eyebrow">Modules</div>
              <ul className={`modlist${mods.length > 4 ? " scrolls" : ""}`}>
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
                      <button type="button" className="iso-btn-drill rename-btn" onClick={() => setRenaming(m.id)}>
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
