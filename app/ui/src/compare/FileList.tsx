// Changed files, grouped by module, each with its added, deleted and modified symbols and import
// changes. Renders the first rows only and grows on request, so a thousand-file comparison stays
// quick.
import { memo, useMemo, useState } from "react";
import type { ChangeKind } from "../ipc/bindings/ChangeKind";
import type { FileChange } from "../ipc/bindings/FileChange";
import type { SymbolChange } from "../ipc/bindings/SymbolChange";
import { basename, dirname, fmtInt, plural } from "../lib/format";
import { ModuleDot } from "../components/ui";
import { usePeek } from "../components/Peek";

const BADGE: Record<ChangeKind, [string, string]> = { added: ["A", "Added"], modified: ["M", "Modified"], deleted: ["D", "Deleted"], renamed: ["R", "Renamed"] };
const MARK: Record<ChangeKind, string> = { added: "+", modified: "~", deleted: "−", renamed: "→" };
const PAGE = 150;
const SYMS_SHOWN = 8;

export function ChangeBadge({ change }: { change: ChangeKind }) {
  return (
    <span className={`cbadge c-${change}`} title={BADGE[change][1]} aria-label={BADGE[change][1]}>
      {BADGE[change][0]}
    </span>
  );
}

export function SymbolMark({ change }: { change: ChangeKind }) {
  return (
    <span className={`smark c-${change}`} aria-hidden="true">
      {MARK[change]}
    </span>
  );
}

/** Five blocks, GitHub style: green for added lines, warn for deleted, line for the rest. */
function DiffBar({ add, del }: { add: number; del: number }) {
  const total = add + del;
  const n = total === 0 ? 0 : Math.min(5, Math.max(1, Math.ceil(Math.log10(total + 1) * 2)));
  const a = total ? Math.round((n * add) / total) : 0;
  return (
    <span className="dbar" aria-hidden="true">
      {Array.from({ length: 5 }, (_, i) => (
        <i key={i} className={i < a ? "a" : i < n ? "d" : ""} />
      ))}
    </span>
  );
}

const KIND_WORD: Record<string, string> = { function: "", method: "", class: "class", interface: "interface", type: "type", enum: "enum", struct: "struct", trait: "trait", impl: "impl", module: "module", variable: "var", constant: "const", field: "field", selector: "selector" };

function SymbolRow({ s, onReveal }: { s: SymbolChange; onReveal(): void }) {
  const peek = usePeek();
  const sigChanged = s.change === "modified" && s.signatureBefore !== s.signatureAfter;
  return (
    <li className={`cs c-${s.change}`}>
      <button type="button" className="cs-name" onClick={onReveal} title={`Show ${s.name} in the diff`}>
        <SymbolMark change={s.change} />
        {KIND_WORD[s.kind] ? <span className="cs-kind">{KIND_WORD[s.kind]}</span> : null}
        <code>{s.name}</code>
      </button>
      {(sigChanged || s.callsAdded.length > 0 || s.callsDeleted.length > 0 || s.callers > 0) && (
        <span className="cs-meta">
      {sigChanged && (
        <span
          className="cs-chip sig"
          tabIndex={0}
          {...peek.bind(() => ({
            kind: "text",
            title: "Signature changed",
            body: (
              <div className="sig-diff">
                <code className="c-del">{s.signatureBefore}</code>
                <code className="c-add">{s.signatureAfter}</code>
              </div>
            ),
          }))}
        >
          signature changed
        </span>
      )}
      {s.callsAdded.length > 0 && (
        <span className="cs-chip calls" title={`Now calls ${s.callsAdded.join(", ")}`}>
          <span className="c-add">+</span>calls {s.callsAdded.slice(0, 3).join(", ")}
          {s.callsAdded.length > 3 ? ` +${s.callsAdded.length - 3}` : ""}
        </span>
      )}
      {s.callsDeleted.length > 0 && (
        <span className="cs-chip calls" title={`No longer calls ${s.callsDeleted.join(", ")}`}>
          <span className="c-del">−</span>calls {s.callsDeleted.slice(0, 3).join(", ")}
          {s.callsDeleted.length > 3 ? ` +${s.callsDeleted.length - 3}` : ""}
        </span>
      )}
      {s.callers > 0 && <span className="caption cs-callers">{plural(s.callers, "call site")} elsewhere</span>}
        </span>
      )}
    </li>
  );
}

const FileRow = memo(function FileRow({ f, selected, onSelect, onReveal, onOpenFile }: { f: FileChange; selected: boolean; onSelect(): void; onReveal(sym: string): void; onOpenFile(path: string): void }) {
  const [allSyms, setAllSyms] = useState(false);
  const dir = dirname(f.path);
  const syms = allSyms ? f.symbols : f.symbols.slice(0, SYMS_SHOWN);
  return (
    <div className={`cf${selected ? " sel" : ""}`}>
      <button type="button" className="cf-head" onClick={onSelect} aria-pressed={selected} title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}>
        <ChangeBadge change={f.change} />
        <span className="cf-path">
          {dir && <span className="cf-dir">{dir}/</span>}
          <span className="cf-base">{basename(f.path)}</span>
        </span>
        <span className="cf-nums">
          {f.binary ? (
            <span className="caption">binary</span>
          ) : (
            <>
              <span className="c-add">+{fmtInt(f.additions)}</span> <span className="c-del">−{fmtInt(f.deletions)}</span>
            </>
          )}
        </span>
        <DiffBar add={f.additions} del={f.deletions} />
      </button>
      {f.oldPath && (
        <div className="caption cf-from">
          renamed from <code>{f.oldPath}</code>
        </div>
      )}
      {syms.length > 0 && (
        <ul className="cf-syms">
          {syms.map((s) => (
            <SymbolRow key={s.change + s.kind + s.name} s={s} onReveal={() => onReveal(s.name)} />
          ))}
          {f.symbols.length > SYMS_SHOWN && (
            <li>
              <button type="button" className="link-quiet" onClick={() => setAllSyms(!allSyms)}>
                {allSyms ? "Show fewer" : `+ ${f.symbols.length - SYMS_SHOWN} more`}
              </button>
            </li>
          )}
        </ul>
      )}
      {f.imports.length > 0 && (
        <ul className="cf-imps">
          {f.imports.map((im) => (
            <li key={im.change + im.specifier} className={`c-${im.change}`}>
              <SymbolMark change={im.change} />
              <span className="cs-kind">import</span>
              <code>{im.specifier}</code>
              {im.file && im.file !== im.specifier && (
                <>
                  <span className="muted"> → </span>
                  <button type="button" className="filelink" onClick={() => onOpenFile(im.file!)}>
                    {im.file}
                  </button>
                </>
              )}
              {im.external && <span className="caption"> · package {im.external}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
});

export function FileList({ files, selected, module, onSelect, onReveal, onOpenFile, onClearModule }: { files: FileChange[]; selected: string | null; module: { id: string; name: string } | null; onSelect(path: string): void; onReveal(path: string, sym: string): void; onOpenFile(path: string): void; onClearModule(): void }) {
  const [limit, setLimit] = useState(PAGE);
  const [q, setQ] = useState("");
  const query = q.trim().toLowerCase();
  const shown = useMemo(() => files.filter((f) => (!module || (f.module?.id ?? "") === module.id) && (!query || f.path.toLowerCase().includes(query) || f.symbols.some((s) => s.name.toLowerCase().includes(query)))), [files, module, query]);
  const groups = useMemo(() => {
    const out: { id: string; name: string; color: string | null; files: FileChange[] }[] = [];
    const at = new Map<string, number>();
    for (const f of shown.slice(0, limit)) {
      const id = f.module?.id ?? "";
      if (!at.has(id)) {
        at.set(id, out.length);
        out.push({ id, name: f.module?.name ?? "No module", color: f.module?.color ?? null, files: [] });
      }
      out[at.get(id)!].files.push(f);
    }
    return out;
  }, [shown, limit]);
  return (
    <section className="cmp-files" aria-label="Changed files">
      <div className="cmp-files-h">
        <div className="display-heading">Files</div>
        <span className="caption">{shown.length === files.length ? plural(files.length, "file") : `${fmtInt(shown.length)} of ${plural(files.length, "file")}`}</span>
        {module && (
          <button type="button" className="iso-chip cmp-modfilter" onClick={onClearModule} title="Show every module">
            {module.name} ×
          </button>
        )}
        <input className="search cmp-filter" type="search" placeholder="Filter files and symbols" aria-label="Filter files and symbols" value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      {groups.map((g) => (
        <div key={g.id} className="cf-group">
          <div className="cf-group-h">
            <ModuleDot color={g.color} />
            <span>{g.name}</span>
            <span className="caption">{plural(g.files.length, "file")}</span>
          </div>
          {g.files.map((f) => (
            <FileRow key={f.path} f={f} selected={f.path === selected} onSelect={() => onSelect(f.path)} onReveal={(s) => onReveal(f.path, s)} onOpenFile={onOpenFile} />
          ))}
        </div>
      ))}
      {shown.length > limit && (
        <button type="button" className="iso-btn-reveal" onClick={() => setLimit(limit + PAGE * 2)}>
          Show {fmtInt(Math.min(PAGE * 2, shown.length - limit))} more of {fmtInt(shown.length - limit)}
        </button>
      )}
      {shown.length === 0 && <p className="caption">{files.length ? "No file matches." : "No files changed."}</p>}
    </section>
  );
}
