// Draggable divider between the reading list and the graph. Sets --left on the split container;
// arrow keys move it in 3% steps. Hidden below 900px, where the panes stack.
import type { RefObject } from "react";

const KEY = "isoline.split";
const clamp = (v: number) => Math.min(75, Math.max(25, v));

export function initialSplit(): number {
  try {
    const v = Number(localStorage.getItem(KEY));
    if (v >= 25 && v <= 75) return v;
  } catch {
    /* storage unavailable */
  }
  return 42;
}

function set(target: HTMLElement, pct: number) {
  target.style.setProperty("--left", pct + "%");
  try {
    localStorage.setItem(KEY, String(pct));
  } catch {
    /* storage unavailable */
  }
}

export function Splitter({ target }: { target: RefObject<HTMLDivElement | null> }) {
  return (
    <div
      className="splitter"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize panes"
      tabIndex={0}
      onPointerDown={(e) => {
        const sp = e.currentTarget;
        const el = target.current;
        if (!el) return;
        sp.setPointerCapture(e.pointerId);
        sp.classList.add("drag");
        const mv = (ev: PointerEvent) => {
          const r = el.getBoundingClientRect();
          set(el, clamp(((ev.clientX - r.left) / r.width) * 100));
        };
        const up = () => {
          sp.classList.remove("drag");
          sp.removeEventListener("pointermove", mv);
          sp.removeEventListener("pointerup", up);
        };
        sp.addEventListener("pointermove", mv);
        sp.addEventListener("pointerup", up);
      }}
      onKeyDown={(e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        const el = target.current;
        if (!el) return;
        e.preventDefault();
        const cur = parseFloat(getComputedStyle(el).getPropertyValue("--left")) || 42;
        set(el, clamp(cur + (e.key === "ArrowLeft" ? -3 : 3)));
      }}
    />
  );
}
