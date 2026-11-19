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
import { ArchitecturePanel } from "./ArchitecturePanel";
import type { AiScope } from "../ipc/bindings/AiScope";
import type { AiStatus } from "../ipc/bindings/AiStatus";
import type { Architecture } from "../ipc/bindings/Architecture";
import type { AiJob } from "../lib/aiRuns";

const SOURCE_TEXT = { cluster: "grouped by directory and imports", ai: "named by AI", user: "named by you" } as const;
const LIST_LIMIT = 200;
/** Modules listed before "Show all". */
const MODS_SHORT = 6;
const TOP_LANGS = 5;
const PHASE_TEXT = { walk: "walking", parse: "parsing", resolve: "resolving imports", graph: "building the graph", done: "done" } as const;

export interface RepoAi {
  arch: Architecture | null;
  status: AiStatus | null;
  job: AiJob | null;
  onRun(scope: AiScope, path: string | null, budgetUsd: number): Promise<void>;
  onOpenAi(): void;
}

export function RepoOverview({ repo, moduleMap, progress, onModuleRenamed, ai, sel, setSel }: { repo: RepoSummary; moduleMap: ModuleMap | null; progress: IndexProgress | null; onModuleRenamed(): void; ai: RepoAi; /** The selected module: focused in the map and, in the sidebar, its files only. */ sel: string | null; setSel(id: string | null): void }) {
  const [modsOpen, setModsOpen] = useState(false);
  const [modQ, setModQ] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [allLangs, setAllLangs] = useState(false);
  const [tiersOpen, setTiersOpen] = useState(false);
  const mapRef = useRef<ModuleMapHandle>(null);
  const maxLines = Math.max(1, ...repo.languages.map((l) => num(l.lines)));
  const mods = useMemo(() => (moduleMap ? [...moduleMap.modules].sort((a, b) => num(b.lines) - num(a.lines)) : []), [moduleMap]);
  const selected = moduleMap?.modules.find((m) => m.id === sel) ?? null;
  const matchedMods = useMemo(() => {
    const q = modQ.trim().toLowerCase();
    return q ? mods.filter((m) => m.name.toLowerCase().includes(q) || m.dir.toLowerCase().includes(q)) : mods;
  }, [mods, modQ]);
  // Short: the largest few, and the selected one if it is not among them.
  const shownMods = useMemo(() => {
    const list = matchedMods.slice(0, modsOpen ? LIST_LIMIT : MODS_SHORT);
    if (selected && !list.some((m) => m.id === selected.id)) list.unshift(selected);
    return list;
  }, [matchedMods, modsOpen, selected]);
  const byId = useMemo(() => new Map((moduleMap?.modules ?? []).map((m) => [m.id, m])), [moduleMap]);
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
        <ArchitecturePanel arch={ai.arch} modules={byId} ai={ai.status} job={ai.job} onRun={ai.onRun} onOpenAi={ai.onOpenAi} onSelect={(id) => setSel(id === sel ? null : id)} selected={sel} />
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
                <span className="caption">The file tree shows this module's files.</span>
                <button type="button" className="link-quiet" onClick={() => setSel(null)}>
                  Clear
                </button>
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
            <section className={`panel short mods${modsOpen ? " open" : ""}`} aria-labelledby="mods-h">
              <div className="mods-h">
                <div className="eyebrow" id="mods-h">
                  Modules
                </div>
                <span className="caption">{fmtInt(mods.length)}</span>
                {modsOpen && <input className="search" type="search" placeholder="Filter by name or folder" aria-label="Filter modules" value={modQ} onChange={(e) => setModQ(e.target.value)} autoFocus />}
              </div>
              <ul className="modlist">
                {shownMods.map((m) => (
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
                      <button type="button" className="modname" onClick={() => setSel(m.id === sel ? null : m.id)} aria-pressed={m.id === sel} title={m.id === sel ? "Show every file again" : "Focus the map and the file tree on this module"}>
                        {m.name}
                      </button>
                    )}
                    <span className="caption">
                      <code>{m.dir || "/"}</code> · {plural(m.files, "file")} · {SOURCE_TEXT[m.source]}
                    </span>
                    {renaming !== m.id && (
                      <button type="button" className="iso-btn-drill rename-btn" onClick={() => setRenaming(m.id)}>
                        Rename
                      </button>
                    )}
                  </li>
                ))}
              </ul>
              {modsOpen && matchedMods.length === 0 && <p className="caption">No module matches “{modQ}”.</p>}
              {modsOpen && matchedMods.length > LIST_LIMIT && <p className="caption">The {fmtInt(LIST_LIMIT)} largest of {fmtInt(matchedMods.length)}. Filter to find the others; the map shows them all.</p>}
              {mods.length > MODS_SHORT && (
                <button type="button" className="link-quiet" aria-expanded={modsOpen} onClick={() => (setModsOpen(!modsOpen), setModQ(""))}>
                  {modsOpen ? "Show fewer" : `Show all ${fmtInt(mods.length)} modules ›`}
                </button>
              )}
            </section>
          )}
        </div>
      </div>
    </div>
  );
}
