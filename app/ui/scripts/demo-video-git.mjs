// Records the "branches and compare" demo against the real engine (`iso-cli serve`), no mock data.
//
//   ISOLINE_GH=<stand-in gh> iso-cli serve --port 7790 --data <fresh dir> &
//   node scripts/demo-video-git.mjs --repo <abs scratch clone> --out <dir> [--port 7790]
//
// The repo must be a scratch clone that is not indexed yet in the data dir, on `main`, with:
//   - a local branch `retry-budget` a few commits ahead of main (the branch and compare scenes),
//   - origin/main as the default compare base,
//   - pull requests answered by the stand-in named in ISOLINE_GH (the clone has no GitHub remote).
// The recording switches the clone's branch and edits a file in it (put back at the end).
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import { recorder, sleep } from "./demo-kit.mjs";

const argv = process.argv.slice(2);
const opt = (k, d) => {
  const i = argv.indexOf(`--${k}`);
  return i >= 0 ? argv[i + 1] : d;
};
const port = opt("port", "7790");
const repo = opt("repo");
const outDir = resolve(opt("out", "demo-out"));
if (!repo) {
  console.error("usage: --repo <scratch clone> [--out dir]");
  process.exit(2);
}
const W = 1440;
const H = 900;
const base = `http://127.0.0.1:${port}/`;
const url = (r) => `${base}?backend=http&repo=${encodeURIComponent(r)}`;
const git = (...a) => execFileSync("git", a, { cwd: repo, encoding: "utf8" }).trim();
if (git("branch", "--show-current") !== "main" || git("status", "--porcelain")) {
  console.error("the scratch clone must be on a clean main");
  process.exit(2);
}

// Whatever happens, the clone ends up as it started: the edit undone, back on main.
let undoEdit = () => {};
process.on("exit", () => {
  undoEdit();
  try {
    git("switch", "-q", "main");
  } catch {}
});

const R = await recorder({ outDir, W, H, prefs: { detail: "read", mapExternal: false, flowKeySeen: true, hintSeen: true, aiNoticeHidden: true } });
const { page, startCast, pause, resume, caption, chapter, ring, keys, card, glide, glideTo, click, typeSlow, press, finish } = R;
const T0 = Date.now();
const mark = (s) => console.log(`${((Date.now() - T0) / 1000).toFixed(1)}s ${s}`);
const status = page.locator(".appbar .status .indexing").first();
const branchBtn = page.locator(".branch-btn");
const text = async (loc) => ((await loc.count()) ? (await loc.first().innerText()).trim() : "");

/** Waits for a comparison to finish (the summary line is back and nothing is loading). */
async function compared() {
  await sleep(250);
  await page.waitForFunction(() => !document.querySelector(".cmp-loading") && document.querySelector(".cmp-sum .sum-item"), null, { timeout: 60000 });
  await sleep(300);
}
/** One of the impact entries: its symbol and the "N unchanged files call it" line. */
async function impact(symbol) {
  const li = page.locator(".impact > li").filter({ has: page.locator(".cs-name code", { hasText: new RegExp(`^${symbol}$`) }) }).first();
  return { li, callers: (await text(li.locator(".impact-callers > .caption"))).replace(/:$/, "") };
}
async function pickRef(role, filter, rowText) {
  await click(page.locator(`.ref-btn[aria-label^="${role}:"]`), 650);
  await page.locator(".branch-pop .pick-filter").waitFor();
  await sleep(350);
  if (filter) await typeSlow(filter, 65);
  await sleep(500);
  const row = page.locator(".branch-pop .pick-row").filter({ hasText: rowText }).first();
  await row.waitFor({ timeout: 10000 });
  await click(row.locator(".pick-main"), 600);
}

// ---------------------------------------------------------------- scenes
await page.goto(`${base}?backend=http`);
await page.locator("body").waitFor();
await sleep(400);
await startCast();

// 0. Title
await card(`<div class="eyebrow">Isoline · what's new</div><h1>Branches, pull requests, and what they change.</h1>
  <h2>The folder tree the moment a repo opens, a branch menu, and a compare view in the map's terms: modules, dependencies, and the functions that are still called.</h2>
  <div class="tags"><span>openclaw · 64,229 files</span><span>Real engine, nothing mocked</span><span>A scratch clone</span></div>`);
await sleep(5200);

// 1. The folder tree, before indexing
mark("tree");
const t0 = Date.now();
await page.goto(url(repo));
await page
  .locator(".tree .row")
  .first()
  .waitFor({ timeout: 60000 })
  .catch(async (e) => {
    await page.screenshot({ path: join(outDir, "stuck-tree.png") });
    console.error("status:", await text(page.locator(".status")), "| problems:", R.problems.join(" / "));
    throw e;
  });
const treeMs = Date.now() - t0;
await card(null);
await chapter("1 · Folder tree at once");
mark(`tree after ${treeMs} ms`);
await caption(`Folders are listed ${(treeMs / 1000).toFixed(1)} s after opening`, "openclaw has never been indexed here: 64,229 files. The tree comes from the walk, so it doesn't wait for parsing.");
await ring(".sidebar", null, 2);
await sleep(1600);
await ring(".appbar .status .indexing", "parsing has only started", 4, true);
await sleep(3000);
await ring(null);
await caption("Folders start collapsed, with file counts", "Open the ones you need. Files that are still waiting for the parser are marked pending.");
await click(page.locator('.tree [role=treeitem][title="src"]'), 800);
await sleep(900);
await click(page.locator('.tree [role=treeitem][title="src/infra"]'), 800);
await sleep(1200);
await glide(140, 520, 900);
await sleep(2200);
// Fast-forward to the end of the cold index.
pause();
await status.filter({ hasText: /^Indexed/ }).waitFor({ timeout: 240000 });
const indexedText = await text(status);
mark(indexedText);
await page.locator('.tree [role=treeitem][title="src/infra"]').click();
await page.locator('.tree [role=treeitem][title="src"]').click();
await sleep(600);
resume();
await caption(`⏩ ${indexedText.replace(/^Indexed /, "Cold index finished: ")}`, "Isoline remembers which folders you opened, per repo.");
await sleep(3200);

// 2. Branches
mark("branches");
await chapter("2 · Branches");
await ring(branchBtn, "current branch", 5, true);
await caption("The branch you're on, in the top bar", "With commits ahead of or behind its upstream, and how many files have uncommitted changes.");
await glideTo(branchBtn, 800);
await sleep(2600);
await ring(null);
await click(branchBtn, 300);
await page.locator(".branch-pop .pick-row").first().waitFor({ timeout: 15000 });
await caption("Switch, create, fetch or compare from one list", "Local and remote branches, newest first. Type to filter; a name that doesn't exist yet offers to create it.");
await sleep(2200);
await typeSlow("retry", 90);
await sleep(1200);
const rbRow = page.locator(".branch-pop .pick-row").filter({ hasText: "retry-budget" }).first();
await ring(rbRow, null, 3);
await sleep(1200);
await ring(null);
await caption("Switch to retry-budget", "Three commits ahead of main. git does the checkout, and Isoline reindexes only the files that differ.");
await click(rbRow.locator(".pick-main"), 700);
await page.waitForFunction(() => document.querySelector(".branch-btn")?.textContent?.includes("retry-budget"), null, { timeout: 30000 });
await sleep(1600);
const afterSwitch = await text(status);
mark(`after switch: ${afterSwitch}`);
await ring(branchBtn, null, 5, true);
await caption(afterSwitch ? `Switched. ${afterSwitch}` : "Switched", "The watcher would catch the change too, a little later and in pieces. The switch hands over the exact list.");
await sleep(3200);
await ring(null);
await caption("The new file is already in the map", "retry-budget.ts exists on this branch only.");
await press("/", ["/"], 300);
await typeSlow("retry-budget.ts", 70);
await sleep(700);
const hit = page.locator(".search-pop [role=option]").filter({ hasText: "retry-budget.ts" }).first();
await hit.waitFor({ timeout: 15000 });
await click(hit, 650);
await page.locator('.iso-card[data-card="createRetryBudget"]').waitFor({ timeout: 20000 });
await sleep(800);
await ring(page.locator('.iso-card[data-card="createRetryBudget"]'), "new on this branch", 6);
await sleep(3000);
await ring(null);

// 3. Compare a branch with main
mark("compare");
await chapter("3 · Compare");
await caption("Compare: what a branch changes, in the map's terms", "Here retry-budget against origin/main. Nothing is checked out.");
await click(page.getByRole("button", { name: "Compare", exact: true }), 700);
await compared();
const sum = await text(page.locator(".cmp-sum"));
const took = sum.split(/\n|·/).map((x) => x.trim()).filter(Boolean).pop();
mark(`compare: ${sum}`);
await ring(".cmp-sum", null, 4, true);
await caption(`From the merge base, like a pull request on GitHub: compared in ${took}`, "Files, lines, symbols added, deleted or modified, changed signatures and imports.");
await sleep(4200);
await ring(".cg-pane .cg", "change graph", 8, true);
await caption("The change graph: modules this branch touches", "And the dependencies between them that changed. gateway now imports one more file from infra.");
const edge = page.locator(".cg-edge").first();
if (await edge.count()) await glideTo(edge, 900);
await sleep(3800);
await ring(null);
await glide(W * 0.55, H * 0.55, 600);
const fem = await impact("formatErrorMessage");
const enc = await impact("encodeSqliteStringSet");
await ring(".impact-panel", "still called", 6, true);
await caption("Still called: what this change could break elsewhere", `formatErrorMessage changed its signature: ${fem.callers.toLowerCase()}. encodeSqliteStringSet is deleted: ${enc.callers.toLowerCase()}.`);
await sleep(3200);
await ring(null);
await glideTo(fem.li.locator(".cs-chip"), 800);
await page.locator(".iso-peek").waitFor({ timeout: 8000 }).catch(() => {});
await caption("Hover to see the signature before and after", "Callers come from the index of unchanged files, so it is not a text search.");
await sleep(3400);
await glide(W * 0.86, H * 0.2, 600);
await sleep(400);
await caption("Every changed file, grouped by module", "With the functions each one added, deleted or modified, and the calls they gained or lost.");
const files = page.locator(".cmp-files");
await files.evaluate((el) => el.scrollIntoView({ block: "start", behavior: "smooth" }));
await sleep(1000);
await ring(files, null, 4);
await sleep(3000);
await ring(null);
const kys = page.locator(".cf").filter({ hasText: "kysely-sync.ts" }).first();
const symBtn = kys.locator(".cs-name").filter({ hasText: "sqliteStringSet" }).last();
await caption("Click a function to see its diff", "Side by side from the merge base, scrolled to that function.");
await click(symBtn, 800);
await page.locator(".diff-host .monaco-diff-editor").waitFor({ timeout: 20000 });
await sleep(1000);
await glide(W * 0.78, H * 0.5, 800);
await sleep(3800);

// 4. Any two revisions, and uncommitted work
mark("revisions");
await chapter("4 · Any two revisions");
await page.locator(".compare-page .cmp-inner").first().evaluate((el) => el.scrollTo({ top: 0, behavior: "smooth" }));
await caption("Or pick any two branches, tags or commits", "Base: the branch's first commit. Head: the branch tip. Only the last two commits remain.");
await pickRef("Base", "shared retry", "a shared retry budget");
await compared();
mark(`two commits: ${await text(page.locator(".cmp-sum"))}`);
await ring(".cmp-sum", null, 4, true);
await sleep(3800);
await ring(null);

const target = join(repo, "src/infra/retry-budget.ts");
const before = readFileSync(target, "utf8");
undoEdit = () => writeFileSync(target, before);
await caption("Check uncommitted work before you commit", "An edit in the editor: withRetryBudget takes an options object instead of a bare signal.");
await sleep(1500);
writeFileSync(
  target,
  before.replace(
    "export async function withRetryBudget<T>(budget: RetryBudget, run: () => Promise<T>, signal?: AbortSignal): Promise<T> {\n  for (;;) {\n    try {\n      return await run();\n    } catch (err) {\n      await sleepWithAbort(budget.take(err), signal);\n    }",
    "export async function withRetryBudget<T>(\n  budget: RetryBudget,\n  run: () => Promise<T>,\n  opts: { signal?: AbortSignal; onRetry?: (err: unknown, waitMs: number) => void } = {},\n): Promise<T> {\n  for (;;) {\n    try {\n      return await run();\n    } catch (err) {\n      const waitMs = budget.take(err);\n      opts.onRetry?.(err, waitMs);\n      await sleepWithAbort(waitMs, opts.signal);\n    }",
  ),
);
if (readFileSync(target, "utf8") === before) throw new Error("the worktree edit did not apply");
await page.locator(".branch-btn .branch-dirty").waitFor({ timeout: 20000 });
await ring(branchBtn, "1 uncommitted file", 5, true);
await sleep(1800);
await ring(null);
await pickRef("Base", "retry-budget", "retry-budget");
await pickRef("Head", "working", "Working tree");
await compared();
mark(`worktree: ${await text(page.locator(".cmp-sum"))}`);
const wrb = await impact("withRetryBudget");
await ring(".impact-panel", "still called", 6, true);
await caption("retry-budget ← Working tree", `withRetryBudget's signature changed and ${wrb.callers.toLowerCase().replace(/ call it$/, " still call it")}: startup-tasks.ts, which this edit doesn't touch.`);
await sleep(4600);
await ring(null);

// 5. Pull requests
mark("prs");
await chapter("5 · Pull requests");
await caption("Pull requests, through the GitHub CLI", "This scratch clone has no GitHub remote, so a stand-in answers for gh. Fetching the PR and the comparison are real.");
await click(page.locator(".pr-picker .menu-btn"), 700);
await page.locator(".pr-pop .pick-row").first().waitFor({ timeout: 15000 });
await sleep(3000);
const pr = page.locator(".pr-pop .pick-row").filter({ hasText: "#1287" }).first();
await ring(pr, null, 3);
await sleep(1000);
await ring(null);
await click(pr.locator(".pick-main"), 600);
await compared();
mark(`pr: ${await text(page.locator(".cmp-sum"))}`);
const rph = await impact("resolveTailscalePublishedHost");
await caption("#1287 drops a deprecated parameter", `One file in the PR, but ${rph.callers.toLowerCase()}. The reviewer sees them before merging.`);
await ring(".impact-panel", "still called", 6, true);
await sleep(4800);
await ring(null);
const first = rph.li.locator(".impact-callers .filelink").first();
await caption("Open any caller in the reader", "The map, cards and call flow, at the line that calls it.");
await click(first, 800);
await page.locator(".iso-card").first().waitFor({ timeout: 20000 });
await sleep(3800);

// 6. End card
mark("end");
await chapter(null);
await caption(null);
await card(`<div class="eyebrow">Isoline · branches and compare</div><h1>See what a change touches before you merge it.</h1>
  <h2>Folder tree in ${(treeMs / 1000).toFixed(1)} s on 64k files · switch and create branches · compare branches, commits, uncommitted work or pull requests: modules, dependencies, and the functions still called.</h2>
  <div class="tags"><span>git CLI under the hood</span><span>Pull requests via gh</span><span>macOS · Windows · Linux</span></div>`);
await sleep(5500);

await finish("isoline-branches-compare.mp4");
