// Choosing and describing git revisions for the compare view.
import type { GitRefs } from "../ipc/bindings/GitRefs";
import type { GitStatus } from "../ipc/bindings/GitStatus";

export const WORKTREE = "worktree";

const DEFAULT_BASES = ["origin/main", "main", "origin/master", "master", "origin/develop", "develop", "origin/trunk", "trunk"];

/** The branch a comparison starts from: the usual default branch, else the first one listed. */
export function defaultBase(refs: GitRefs, status: GitStatus | null): string | null {
  const names = new Set(refs.branches.map((b) => b.name));
  // A non-default upstream (origin/release/x) is not a good base; the default branch is.
  for (const n of DEFAULT_BASES) if (names.has(n)) return n;
  const other = refs.branches.find((b) => b.name !== status?.branch);
  return other?.name ?? refs.branches[0]?.name ?? null;
}

/** What to compare the base with: the current branch, or the working tree when HEAD is the base. */
export function defaultHead(refs: GitRefs, status: GitStatus | null, base: string | null): string {
  const cur = status?.branch ?? status?.head ?? null;
  if (!cur) return WORKTREE;
  if (cur === base) return WORKTREE;
  const b = refs.branches.find((x) => x.name === base);
  const c = refs.branches.find((x) => x.name === cur && !x.remote);
  if (b && c && b.sha === c.sha) return WORKTREE;
  return cur;
}

/** Display text for a revision spec. */
export function refLabel(spec: string, refs: GitRefs | null): string {
  if (spec === WORKTREE) return "Working tree";
  const c = refs?.commits.find((x) => x.sha === spec || x.short === spec);
  if (c && !refs?.branches.some((b) => b.name === spec)) return c.short;
  return spec;
}

/** A name git would accept for a new branch (a light check; git has the last word). */
export function validBranchName(name: string): boolean {
  const n = name.trim();
  if (!n || n.startsWith("-") || n.startsWith("/") || n.endsWith("/") || n.endsWith(".") || n.endsWith(".lock")) return false;
  if (/[\s~^:?*[\\\x00-\x1f\x7f]|\.\.|@\{|\/\//.test(n)) return false;
  return n !== "@";
}
