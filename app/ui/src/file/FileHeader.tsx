// File header: the name, then one caption line with the path, language, size and counts.
// The stat pills return at Dig. The tier badge shows only for outline and text files.
import type { FileView } from "../ipc/bindings/FileView";
import { fileStats } from "../lib/cards";
import { fmtInt, plural } from "../lib/format";
import type { Detail } from "../lib/prefs";
import { ModuleDot, StatPill, TierBadge } from "../components/ui";

export function FileHeader({ view, detail = "read" }: { view: FileView; detail?: Detail }) {
  const s = fileStats(view);
  const counts = view.tier === "full" ? [plural(s.functions, "function"), plural(s.modules, "import")] : view.tier === "outline" ? [plural(view.outline.length, "symbol")] : [];
  return (
    <header className="top file-top">
      <div className="file-id">
        <h1>{view.name}</h1>
        <div className="file">
          <span>{[view.path, view.langLabel, `${fmtInt(view.lines)} lines`, ...counts].join(" · ")}</span>
          <span className="file-tags">
            {view.tier !== "full" && <TierBadge tier={view.tier} />}
            {view.module && (
              <span className="module-tag">
                <ModuleDot color={view.module.color} />
                {view.module.name}
              </span>
            )}
            {view.hasErrors && <span className="parse-warn">parsed with errors</span>}
          </span>
        </div>
      </div>
      {detail === "dig" && view.tier === "full" && (
        <div className="stats">
          <StatPill n={s.functions} noun="function" />
          <StatPill n={s.exported} noun="exported" pluralForm="exported" />
          <StatPill n={s.layers} noun="layer" />
          <StatPill n={s.types} noun="local type" />
          <StatPill n={s.modules} noun="imported module" />
          <StatPill n={s.symbols} noun="external symbol" />
        </div>
      )}
    </header>
  );
}
