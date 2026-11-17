// File header: title, "path · language · lines", tier and module, and stat pills.
import type { FileView } from "../ipc/bindings/FileView";
import { fileStats } from "../lib/cards";
import { fmtInt } from "../lib/format";
import { ModuleDot, StatPill, TierBadge } from "../components/ui";

export function FileHeader({ view }: { view: FileView }) {
  const s = fileStats(view);
  return (
    <header className="top file-top">
      <div>
        <h1>{view.name}</h1>
        <div className="file">
          {view.path} · {view.langLabel} · {fmtInt(view.lines)} lines
          <span className="file-tags">
            <TierBadge tier={view.tier} />
            {view.module && (
              <span className="module-tag" title={`Module: ${view.module.name}`}>
                <ModuleDot color={view.module.color} />
                {view.module.name}
              </span>
            )}
            {view.hasErrors && (
              <span className="parse-warn" title="The parser hit syntax errors; some facts may be missing.">
                parsed with errors
              </span>
            )}
          </span>
        </div>
      </div>
      {view.tier === "full" ? (
        <div className="stats">
          <StatPill n={s.functions} noun="function" />
          <StatPill n={s.exported} noun="exported" pluralForm="exported" />
          <StatPill n={s.layers} noun="layer" />
          <StatPill n={s.types} noun="local type" />
          <StatPill n={s.modules} noun="imported module" />
          <StatPill n={s.symbols} noun="external symbol" />
        </div>
      ) : (
        <div className="stats">
          {view.tier === "outline" && <StatPill n={view.outline.length} noun="symbol" />}
          {view.modules.length > 0 && <StatPill n={view.modules.length} noun="import" />}
          <StatPill n={view.lines} noun="line" />
        </div>
      )}
    </header>
  );
}
