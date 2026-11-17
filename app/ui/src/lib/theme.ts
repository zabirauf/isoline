// Theme state: "system" follows prefers-color-scheme; "light" and "dark" set data-theme on <html>.
export type ThemeChoice = "system" | "light" | "dark";
export type Theme = "light" | "dark";

const KEY = "isoline.theme";

export function loadThemeChoice(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    /* storage unavailable */
  }
  return "system";
}

export function applyThemeChoice(choice: ThemeChoice) {
  const el = document.documentElement;
  if (choice === "system") el.removeAttribute("data-theme");
  else el.setAttribute("data-theme", choice);
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    /* storage unavailable */
  }
  notify();
}

const mq = typeof matchMedia !== "undefined" ? matchMedia("(prefers-color-scheme: dark)") : null;

export function effectiveTheme(): Theme {
  const t = document.documentElement.getAttribute("data-theme");
  if (t === "light" || t === "dark") return t;
  return mq?.matches ? "dark" : "light";
}

const subs = new Set<(t: Theme) => void>();
function notify() {
  const t = effectiveTheme();
  subs.forEach((f) => f(t));
}
mq?.addEventListener("change", notify);

/** Calls `f` whenever the effective theme changes. Returns an unsubscribe function. */
export function onThemeChange(f: (t: Theme) => void): () => void {
  subs.add(f);
  return () => subs.delete(f);
}

export function prefersReducedMotion(): boolean {
  return typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}
