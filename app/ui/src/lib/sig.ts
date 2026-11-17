// Typed signatures (spec section 1): the parameter layout rules, how type text is shortened,
// and how each name inside a type is looked up in FileView.targets to get its origin.
// The UI does not parse code; it only splits the backend's type text into names and punctuation.
import type { ParamInfo } from "../ipc/bindings/ParamInfo";
import type { SigInfo } from "../ipc/bindings/SigInfo";
import type { SymbolOrigin } from "../ipc/bindings/SymbolOrigin";
import type { SymbolTarget } from "../ipc/bindings/SymbolTarget";
import type { Detail } from "./prefs";

export type Origin = SymbolOrigin | "unknown";

export interface TypeTok {
  text: string;
  /** name: an identifier; punct: brackets, operators and separators; text: literals and the rest. */
  kind: "name" | "punct" | "space" | "text";
  origin?: Origin;
  target?: SymbolTarget;
}

const TOKEN = /([A-Za-z_$][\w$]*)|(\s+)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`[^`]*`|\d+(?:\.\d+)?)|(…)|([<>[\]{}()|&,:;=?.*+\-!]+)|(.)/g;

export type TargetIndex = Map<string, SymbolTarget>;

/** Index targets by name; when a name has several, a local one wins, then imported, external, builtin. */
export function indexTargets(targets: SymbolTarget[]): TargetIndex {
  const rank: Record<SymbolOrigin, number> = { local: 0, imported: 1, external: 2, builtin: 3 };
  const m: TargetIndex = new Map();
  for (const t of targets) {
    const cur = m.get(t.name);
    if (!cur || rank[t.origin] < rank[cur.origin]) m.set(t.name, t);
  }
  return m;
}

export function originOf(name: string, idx: TargetIndex): Origin {
  return idx.get(name)?.origin ?? "unknown";
}

/** Splits type text into names (looked up in targets), punctuation, spaces and literals. */
export function tokenizeType(text: string, idx: TargetIndex): TypeTok[] {
  const out: TypeTok[] = [];
  TOKEN.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = TOKEN.exec(text)) && m[0] !== "") {
    if (m[1]) {
      // A member after a dot (`ns.Type`, `x.y`) is not looked up on its own.
      const prev = out.length ? out[out.length - 1] : null;
      const member = prev?.kind === "punct" && prev.text.endsWith(".");
      const target = member ? undefined : idx.get(m[1]);
      out.push({ text: m[1], kind: "name", origin: member ? "unknown" : (target?.origin ?? "unknown"), target });
    } else if (m[2]) out.push({ text: m[2], kind: "space" });
    else if (m[3]) out.push({ text: m[3], kind: "text" });
    else if (m[4] || m[5]) out.push({ text: m[0], kind: "punct" });
    else out.push({ text: m[0], kind: "text" });
  }
  return out;
}

/** Index just past the bracket that closes the one at `open`, or -1. */
function closing(s: string, open: number): number {
  const pairs: Record<string, string> = { "(": ")", "<": ">", "[": "]", "{": "}" };
  const stack: string[] = [];
  for (let i = open; i < s.length; i++) {
    const c = s[i];
    if (c === "=" && s[i + 1] === ">") {
      i++;
      continue;
    }
    if (pairs[c]) stack.push(pairs[c]);
    else if (c === stack[stack.length - 1]) {
      stack.pop();
      if (!stack.length) return i + 1;
    }
  }
  return -1;
}

/** Replaces what is inside each first-level `<…>` with "…": `Promise<Result>` → `Promise<…>`. */
export function cutGenerics(text: string): string {
  let out = "";
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "<") {
      const end = closing(text, i);
      if (end > 0) {
        out += "<…>";
        i = end - 1;
        continue;
      }
    }
    out += text[i];
  }
  return out;
}

/** `(params: A, b?: B) => R` → `{ params: ["params", "b"], ret: "R" }`; null if not a function type. */
export function functionType(text: string): { params: string[]; ret: string } | null {
  const t = text.trim().replace(/^<[^>]*>\s*/, "");
  if (!t.startsWith("(")) return null;
  const end = closing(t, 0);
  if (end < 0) return null;
  const rest = t.slice(end).trim();
  if (!rest.startsWith("=>")) return null;
  const inner = t.slice(1, end - 1);
  const params: string[] = [];
  let depth = 0;
  let cur = "";
  for (const c of inner) {
    if ("(<[{".includes(c)) depth++;
    if (")>]}".includes(c)) depth--;
    if (c === "," && depth === 0) {
      params.push(cur);
      cur = "";
    } else cur += c;
  }
  if (cur.trim()) params.push(cur);
  return {
    params: params.map((p) => p.trim().split(/[?:=\s]/)[0]).filter(Boolean),
    ret: rest.slice(2).trim(),
  };
}

/** Cuts text longer than `max` characters at a token boundary and adds "…". */
export function cutText(text: string, max: number): string {
  if (text.length <= max) return text;
  let cut = text.slice(0, max - 1);
  const m = cut.match(/^(.*[^\w$])[\w$]*$/);
  if (m && m[1].length > max / 2) cut = m[1];
  return cut.trimEnd() + "…";
}

export const TYPE_MAX = 40;
export const DEFAULT_MAX = 32;

/** A type's shape at a detail level, before cutting: function types show param names only. */
export function shapeType(text: string, detail: Detail): string {
  const t = text.replace(/\s+/g, " ").trim();
  if (detail === "dig") return t;
  const fn = functionType(t);
  return fn ? `(${fn.params.join(", ")}) => ${cutGenerics(fn.ret)}` : t;
}

/** How a type shows in a signature at a detail level, cut at 40 characters below Dig. */
export function displayType(text: string, detail: Detail): string {
  const shaped = shapeType(text, detail);
  return detail === "dig" ? shaped : cutText(shaped, TYPE_MAX);
}

/**
 * Cuts tokens to `max` characters. A name cut part-way keeps its origin and target, so a long
 * repo type still shows as a repo type ("AgentHarnessSettledTurn…" on ext-soft).
 */
export function cutTokens(toks: TypeTok[], max: number): TypeTok[] {
  const total = toks.reduce((n, t) => n + t.text.length, 0);
  if (total <= max) return toks;
  const out: TypeTok[] = [];
  let used = 0;
  for (const t of toks) {
    const room = max - 1 - used;
    if (t.text.length <= room) {
      out.push(t);
      used += t.text.length;
      continue;
    }
    if (t.kind === "name" && room >= 4) out.push({ ...t, text: t.text.slice(0, room) + "…" });
    else out.push({ text: "…", kind: "punct" });
    break;
  }
  return out;
}

export interface ParamView {
  /** As shown: `{…}` for destructured names. */
  name: string;
  /** The type's shape, uncut (the renderer cuts tokens so cut names keep their origin). */
  typeFull: string | null;
  rest: boolean;
  optional: boolean;
  destructured: boolean;
  type: string | null;
  /** `…` at Read, the literal (cut at 32) at Dig. */
  def: string | null;
  param: ParamInfo;
}

export interface SigView {
  mode: "inline" | "stacked";
  params: ParamView[];
  /** Params left out at Read ("+ 3 more params"). */
  hidden: number;
  returns: string | null;
  /** The return type's shape, uncut. */
  returnsFull: string | null;
}

export const INLINE_MAX_PARAMS = 3;
export const INLINE_MAX_CHARS = 56;
export const READ_MAX_PARAMS = 4;

function paramView(p: ParamInfo, detail: Detail): ParamView {
  const destructured = /^[{[]/.test(p.name.trim());
  return {
    name: destructured ? (p.name.trim()[0] === "{" ? "{…}" : "[…]") : p.name.replace(/^\.\.\.|^\*{1,2}/, ""),
    rest: p.rest,
    optional: p.optional,
    destructured,
    type: p.typeText ? displayType(p.typeText, detail) : null,
    typeFull: p.typeText ? shapeType(p.typeText, detail) : null,
    def: p.default === null ? null : detail === "dig" ? cutText(p.default.replace(/\s+/g, " ").trim(), DEFAULT_MAX) : "…",
    param: p,
  };
}

/** The text an inline signature renders, used for the 56-character rule. */
export function inlineText(params: ParamView[], returns: string | null): string {
  const ps = params.map((p) => `${p.rest ? "..." : ""}${p.name}${p.optional ? "?" : ""}${p.type ? `: ${p.type}` : ""}${p.def !== null ? ` = ${p.def}` : ""}`);
  return `(${ps.join(", ")})${returns ? ` → ${returns}` : ""}`;
}

/**
 * Layout rules: inline when there are 3 or fewer params and the text is 56 characters or
 * fewer; otherwise one param per line. Read shows 4 params then "+ N more params"; Dig shows all.
 * The return is omitted when it is null or void, except that Dig shows "→ void".
 */
export function layoutSig(sig: SigInfo, detail: Detail, opts: { all?: boolean } = {}): SigView {
  const all = sig.params.map((p) => paramView(p, detail));
  const r = sig.returns?.trim() || null;
  const returns = r === null || r === "void" ? (detail === "dig" && r === "void" ? "void" : null) : displayType(r, detail === "dig" ? "dig" : "read");
  const returnsFull = returns === null ? null : returns === "void" ? "void" : shapeType(r!, detail === "dig" ? "dig" : "read");
  const inline = all.length <= INLINE_MAX_PARAMS && inlineText(all, returns).length <= INLINE_MAX_CHARS;
  const limit = detail === "dig" || inline || opts.all ? all.length : READ_MAX_PARAMS;
  const shown = all.slice(0, limit);
  return { mode: inline ? "inline" : "stacked", params: shown, hidden: all.length - shown.length, returns, returnsFull };
}

/** Generic parameters of a declaration (`<T extends X>`), from the signature text; shown at Dig. */
export function genericsOf(signature: string, name: string): string | null {
  const short = name.slice(name.lastIndexOf(".") + 1);
  const i = signature.indexOf(short);
  if (i < 0) return null;
  const after = signature.slice(i + short.length).trimStart();
  if (!after.startsWith("<")) return null;
  const end = closing(after, 0);
  return end > 0 ? after.slice(0, end) : null;
}
