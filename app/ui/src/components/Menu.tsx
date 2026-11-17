// A quiet menu button ("More ▾", "Entry points first ▾", "openclaw ▾"): a list of actions and
// toggles in a popover. A toggle that is on gets the accent-soft fill, as toggle buttons do (no
// check glyph: the design system has no icon set). Arrow keys move, Esc closes.
import { useEffect, useRef, useState, type ReactNode } from "react";

export type MenuItem =
  | { kind: "action"; label: ReactNode; onSelect(): void; hint?: string }
  | { kind: "toggle"; label: ReactNode; checked: boolean; onSelect(): void; hint?: string }
  | { kind: "radio"; label: ReactNode; checked: boolean; onSelect(): void }
  | { kind: "sep" }
  | { kind: "label"; label: ReactNode };

export function Menu({ label, items, className, align = "right", ariaLabel }: { label: ReactNode; items: MenuItem[]; className?: string; align?: "left" | "right"; ariaLabel?: string }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
        box.current?.querySelector<HTMLButtonElement>(".menu-btn")?.focus();
      }
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey, true);
    requestAnimationFrame(() => box.current?.querySelector<HTMLElement>("[role^=menuitem]")?.focus());
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open]);
  return (
    <div className={`menu ${className ?? ""}`} ref={box}>
      <button type="button" className="iso-btn menu-btn" aria-haspopup="menu" aria-expanded={open} aria-label={ariaLabel} onClick={() => setOpen(!open)}>
        {label} <span className="caret" aria-hidden="true">▾</span>
      </button>
      {open && (
        <div
          className={`popover menu-pop menu-${align}`}
          role="menu"
          onKeyDown={(e) => {
            if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
            e.preventDefault();
            const els = [...(e.currentTarget.querySelectorAll<HTMLElement>("[role^=menuitem]") ?? [])];
            const i = els.indexOf(document.activeElement as HTMLElement);
            els[(i + (e.key === "ArrowDown" ? 1 : els.length - 1)) % els.length]?.focus();
          }}
        >
          {items.map((it, i) => {
            if (it.kind === "sep") return <div key={i} className="menu-sep" role="separator" />;
            if (it.kind === "label")
              return (
                <div key={i} className="menu-label">
                  {it.label}
                </div>
              );
            const role = it.kind === "toggle" ? "menuitemcheckbox" : it.kind === "radio" ? "menuitemradio" : "menuitem";
            return (
              <button
                key={i}
                type="button"
                role={role}
                aria-checked={it.kind === "action" ? undefined : it.checked}
                className={`menu-item${it.kind !== "action" && it.checked ? " on" : ""}`}
                onClick={() => {
                  it.onSelect();
                  if (it.kind !== "toggle") setOpen(false);
                }}
              >
                <span className="lab">{it.label}</span>
                {"hint" in it && it.hint && <span className="hint">{it.hint}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
