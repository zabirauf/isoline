// Call-flow rendering model: step numbers in execution order, stats, built-ins filtering and the
// wording of each step's meta line. Ported from the prototype's renderItems / flowStats / metaFor.
import type { FlowItem } from "../ipc/bindings/FlowItem";
import type { FlowTarget } from "../ipc/bindings/FlowTarget";
import type { FlowUnit } from "../ipc/bindings/FlowUnit";
import { shortPath } from "./format";

export type CallItem = Extract<FlowItem, { t: "call" }>;
export type BlockItem = Extract<FlowItem, { t: "block" }>;
export type ExitItem = Extract<FlowItem, { t: "exit" }>;

export type FlowNode =
  | { t: "step"; n: number; item: CallItem; inner: FlowNode[] }
  /** A hidden built-in call whose inner calls still show, without a number of their own. */
  | { t: "hidden"; item: CallItem; inner: FlowNode[] }
  | { t: "block"; item: BlockItem; children: FlowNode[] }
  | { t: "exit"; item: ExitItem };

export interface NumberedFlow {
  nodes: FlowNode[];
  /** Visible calls in execution order; index i is step number i + 1. */
  steps: CallItem[];
}

/**
 * Numbers the calls of a flow tree in execution order. Arguments come before the call that
 * receives them because the analyzer already orders them that way; a call's `inner` items
 * (callbacks it runs) are numbered after it. Built-in calls are skipped unless `builtins`.
 */
export function numberFlow(items: FlowItem[], builtins: boolean): NumberedFlow {
  const steps: CallItem[] = [];
  const walk = (list: FlowItem[]): FlowNode[] =>
    list.map((it): FlowNode => {
      if (it.t === "call") {
        if (!builtins && it.target.kind === "builtin") return { t: "hidden", item: it, inner: walk(it.inner ?? []) };
        steps.push(it);
        const n = steps.length;
        return { t: "step", n, item: it, inner: walk(it.inner ?? []) };
      }
      if (it.t === "block") return { t: "block", item: it, children: walk(it.children) };
      return { t: "exit", item: it };
    });
  return { nodes: walk(items), steps };
}

export interface FlowStats {
  calls: number;
  branches: number;
  tries: number;
  loops: number;
  callbacks: number;
  awaits: number;
}

export function flowStats(items: FlowItem[], builtins: boolean, s: FlowStats = { calls: 0, branches: 0, tries: 0, loops: 0, callbacks: 0, awaits: 0 }): FlowStats {
  items.forEach((it) => {
    if (it.t === "call") {
      if (builtins || it.target.kind !== "builtin") s.calls++;
      if (it.await) s.awaits++;
      flowStats(it.inner ?? [], builtins, s);
    } else if (it.t === "block") {
      if (it.kind === "cond" && it.kw !== "else") s.branches++;
      if (it.kind === "try") s.tries++;
      if (it.kind === "loop") s.loops++;
      if (it.kind === "callback") s.callbacks++;
      flowStats(it.children, builtins, s);
    }
  });
  return s;
}

export const STAT_LABELS: [keyof FlowStats, string][] = [
  ["calls", "call"],
  ["branches", "conditional"],
  ["tries", "try block"],
  ["loops", "loop"],
  ["callbacks", "callback"],
  ["awaits", "await"],
];

export function hasLoops(units: FlowUnit[]): boolean {
  const any = (items: FlowItem[]): boolean =>
    items.some((it) => (it.t === "block" ? it.kind === "loop" || any(it.children) : it.t === "call" ? any(it.inner ?? []) : false));
  return units.some((u) => any(u.tree));
}

/** Step color class by target kind. */
export const KIND_CLASS: Record<FlowTarget["kind"], string> = {
  fn: "iso-k-fn",
  ext: "iso-k-ext",
  method: "iso-k-method",
  local: "iso-k-method",
  param: "iso-k-param",
  builtin: "iso-k-param",
  other: "iso-k-param",
};

/** One run of a meta line: prose in the body face, paths, identifiers and lines in mono. */
export interface MetaPart {
  text: string;
  mono: boolean;
}

/** The meta line under a step, as parts: origin, then argument context, then the line. */
export function metaParts(it: CallItem): MetaPart[][] {
  const t = it.target;
  const prose = (text: string): MetaPart[] => [{ text, mono: false }];
  const bits: MetaPart[][] = [];
  if (t.kind === "fn") bits.push(prose("this file"));
  else if (t.kind === "ext") bits.push(t.module ? [{ text: shortPath(t.module), mono: true }] : prose("external import"));
  else if (t.kind === "method") bits.push(prose(t.unit ? "controller method" : (t.note ?? "method")));
  else if (t.kind === "local") bits.push(prose("local helper"));
  else if (t.kind === "param") bits.push(prose("parameter"));
  else if (t.kind === "builtin") bits.push(prose("built-in"));
  if (it.argOf)
    bits.push(
      it.argOf === "(condition)"
        ? prose("evaluated in the condition")
        : [
            { text: "argument to ", mono: false },
            { text: `${it.argOf}()`, mono: true },
          ],
    );
  if (it.optional) bits.push(prose("optional call"));
  bits.push([{ text: "L" + it.line, mono: true }]);
  return bits;
}

/** The meta line as plain text. */
export function metaFor(it: CallItem): string {
  return metaParts(it)
    .map((b) => b.map((p) => p.text).join(""))
    .join(" · ");
}

/** Unit id to drill into from a step, when the callee's own flow can be shown. */
export function drillOf(it: CallItem, units: ReadonlySet<string>): string | null {
  const t = it.target;
  if (t.kind === "fn" && t.id && units.has(t.id)) return t.id;
  if (t.kind === "method" && t.unit && units.has(t.unit)) return t.unit;
  return null;
}

/** Map node a step flies in from: "f:<card>" or "s:<imported symbol>". */
export function keyOf(t: FlowTarget): string {
  if (t.kind === "fn" && t.id) return "f:" + t.id;
  if (t.kind === "ext" && t.id) return "s:" + t.id;
  if (t.kind === "method" && t.of) return "f:" + t.of;
  return "";
}

export function exitKeyword(it: ExitItem): string {
  return it.kind === "throw" ? "throw" : it.early ? "early return" : "return";
}

export function unitTitle(u: Pick<FlowUnit, "id" | "method">): string {
  return u.method ? `${u.method}()` : u.id;
}

/** Stagger between steps so a whole flow lands in about one second. */
export function staggerMs(stepCount: number): number {
  return Math.max(12, Math.min(40, 480 / Math.max(1, stepCount)));
}

export interface SpinePoint {
  x: number;
  y1: number;
  y2: number;
}
export interface Box {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/**
 * Path of one spine segment from the bottom of step a's number to the top of step b's.
 * It drops from a, runs across just above b, then down into b. When an empty frame sits in the
 * way of the drop, the segment steps left of that frame and comes down beside it instead.
 */
export function spinePath(a: SpinePoint, b: SpinePoint, empty: Box[]): string {
  const ym = b.y1 - 7;
  const blockers = empty.filter((e) => e.top >= a.y2 - 1 && e.bottom <= ym + 1 && e.left <= a.x && a.x <= e.right);
  if (!blockers.length) return `M${a.x},${a.y2} L${a.x},${ym} L${b.x},${ym} L${b.x},${b.y1}`;
  const top = Math.min(...blockers.map((e) => e.top)) - 4;
  const dx = Math.max(1, Math.min(...blockers.map((e) => e.left)) - 5);
  return `M${a.x},${a.y2} L${a.x},${top} L${dx},${top} L${dx},${ym} L${b.x},${ym} L${b.x},${b.y1}`;
}
