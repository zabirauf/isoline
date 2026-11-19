// Responsiveness while a repo indexes, against the real engine through `iso-cli serve`.
// Opens the repo with an empty data folder (a cold index) and, until indexing ends and a few
// seconds after, keeps interacting: scrolls the file tree, types in its filter, opens files and
// pans the module map. Reports main-thread long tasks, event-loop lag, input-to-paint latency
// per interaction, backend command latency, and how many events of each kind arrived.
//
//   cargo build --release -p iso-cli && npm run build
//   node scripts/index-perf.mjs /path/to/repo [--warm] [--data DIR] [--dpr 1.5] [--cpu 4]
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const argv = process.argv.slice(2);
const repo = argv.find((a, i) => !a.startsWith("--") && !argv[i - 1]?.startsWith("--"));
if (!repo) throw new Error("usage: index-perf.mjs <repo> [--warm] [--data DIR] [--dpr N]");
const opt = (k, d) => (argv.includes(k) ? argv[argv.indexOf(k) + 1] : d);
const ownData = !argv.includes("--data");
const data = opt("--data", mkdtempSync(join(tmpdir(), "iso-perf-")));
const dpr = Number(opt("--dpr", 1.5));
// --cpu 4: the page runs four times slower, like a modest laptop.
const cpu = Number(opt("--cpu", 1));
// --ui DIR: the built UI to serve (an unminified build gives readable --profile output).
const uiDir = resolve(opt("--ui", join(root, "app/ui/dist")));
// --profile: CPU profile of the page for the whole run, top functions by self time.
const profile = argv.includes("--profile");
const port = 7791;

const pct = (a, p) => (a.length ? [...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor(a.length * p))] : 0);
const r1 = (x) => Math.round(x * 10) / 10;
const summary = (a) => ({ n: a.length, p50: r1(pct(a, 0.5)), p95: r1(pct(a, 0.95)), max: r1(Math.max(0, ...a)) });

async function serve() {
  const p = spawn(join(root, "target/release/iso-cli"), ["--data", data, "serve", "--port", String(port), "--ui", uiDir], { stdio: ["ignore", "ignore", "pipe"] });
  await new Promise((ok, fail) => {
    p.stderr.on("data", (b) => String(b).includes("serving") && ok());
    p.on("exit", (c) => fail(new Error(`serve exited ${c}`)));
  });
  return p;
}

async function run(label) {
  const server = await serve();
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr });
  await ctx.addInitScript(() => {
    localStorage.setItem("isoline.prefs.v1", JSON.stringify({ hintSeen: true, flowKeySeen: true, aiNoticeHidden: true }));
    const w = window;
    w.__long = [];
    w.__lag = [];
    w.__events = {};
    w.__progress = [];
    new PerformanceObserver((l) => w.__long.push(...l.getEntries().map((e) => Math.round(e.duration)))).observe({ type: "longtask", buffered: true });
    // Event-loop lag: how late a 50 ms timer fires.
    let last = performance.now();
    setInterval(() => {
      const now = performance.now();
      w.__lag.push(Math.max(0, now - last - 50));
      last = now;
    }, 50);
    // A second event stream that only counts.
    addEventListener("load", () => {
      const es = new EventSource("./events");
      es.onmessage = (m) => {
        const { event, payload } = JSON.parse(m.data);
        w.__events[event] = (w.__events[event] ?? 0) + 1;
        if (event === "index_progress") w.__progress.push([Math.round(performance.now()), payload.phase, payload.done, payload.total]);
      };
    });
  });
  const page = await ctx.newPage();
  const cdp = await ctx.newCDPSession(page);
  if (cpu > 1) await cdp.send("Emulation.setCPUThrottlingRate", { rate: cpu });
  if (profile) await cdp.send("Profiler.enable"), await cdp.send("Profiler.start");
  page.on("pageerror", (e) => console.log("page error:", e.message));
  const t0 = Date.now();
  await page.goto(`http://127.0.0.1:${port}/?backend=http&repo=${encodeURIComponent(repo)}`);
  await page.getByRole("tree", { name: "Files" }).waitFor({ timeout: 120000 });
  const treeAt = Date.now() - t0;

  const lat = { scroll: [], filter: [], open: [], map: [] };
  const paint = () => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  // Input-to-paint in the page: the action, then two frames.
  const timed = (fn) =>
    page.evaluate(
      (src) =>
        new Promise((r) => {
          const t = performance.now();
          new Function(src)();
          requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now() - t)));
        }),
      fn,
    );
  let indexDone = false;
  let doneAt = 0;
  let round = 0;
  const deadline = Date.now() + 10 * 60_000;
  while (Date.now() < deadline) {
    round++;
    indexDone = await page.evaluate(() => window.__progress.some((p) => p[1] === "done"));
    if (indexDone && !doneAt) doneAt = Date.now() - t0;
    if (doneAt && Date.now() - t0 > doneAt + 6000) break;
    lat.scroll.push(await timed("const t=document.querySelector('.tree'); if(t) t.scrollTop = (t.scrollTop + 900) % Math.max(1, t.scrollHeight - t.clientHeight);"));
    const f = page.locator(".side-filter");
    const t = Date.now();
    const q = round % 2 ? "index" : "";
    await f.fill(q);
    // The list follows the input at lower priority: wait until it shows the filter's results.
    await page.waitForFunction((filtering) => !!document.querySelector(".tree .row .dirhint") === filtering || !document.querySelector(".tree .row"), !!q, { timeout: 10000 }).catch(() => {});
    await paint();
    lat.filter.push(Date.now() - t);
    // Open a file from the tree (or the filter results), wait for its header.
    const row = page.locator(".tree .row.file").nth(round % 7);
    if (await row.count()) {
      const name = (await row.getAttribute("title"))?.split(" · ")[0];
      const t2 = Date.now();
      await row.click();
      await page.waitForFunction((n) => document.querySelector("header.top")?.textContent?.includes(n.split("/").pop()), name, { timeout: 30000 }).catch(() => {});
      lat.open.push(Date.now() - t2);
    }
    if (round % 3 === 0) {
      await f.fill("");
      await page.getByRole("button", { name: /^Isoline$|^Home$/ }).first().click().catch(() => {});
    }
    const canvas = page.locator(".mmap-canvas");
    if (await canvas.count()) {
      const b = await canvas.boundingBox();
      if (b && b.y < 850) {
        const tm = Date.now();
        await page.mouse.move(b.x + b.width / 2, b.y + 60);
        await page.mouse.down();
        for (let i = 0; i < 10; i++) await page.mouse.move(b.x + b.width / 2 - i * 15, b.y + 60);
        await page.mouse.up();
        await paint();
        lat.map.push((Date.now() - tm) / 10);
      }
    }
  }
  const res = await page.evaluate(() => {
    const cmds = {};
    for (const e of performance.getEntriesByType("resource")) {
      const m = e.name.match(/\/invoke\/(\w+)/);
      const eng = e.serverTiming?.find((t) => t.name === "engine");
      if (m) (cmds[m[1]] ??= []).push(eng ? eng.duration : e.duration);
    }
    return { long: window.__long, lag: window.__lag, events: window.__events, cmds, progress: window.__progress };
  });
  console.log(`\n== ${label}: tree shown at ${treeAt} ms, index done at ${doneAt || "not seen"} ms, ${round} rounds`);
  console.log("long tasks  ", JSON.stringify({ ...summary(res.long), totalMs: res.long.reduce((a, b) => a + b, 0) }));
  console.log("loop lag    ", JSON.stringify(summary(res.lag)));
  for (const [k, v] of Object.entries(lat)) console.log(`${k.padEnd(12)}`, JSON.stringify(summary(v)));
  console.log("engine ms   ", JSON.stringify(Object.fromEntries(Object.entries(res.cmds).map(([k, v]) => [k, summary(v)]))));
  console.log("events      ", JSON.stringify(res.events));
  if (profile) {
    const { profile: prof } = await cdp.send("Profiler.stop");
    const self = new Map();
    const byId = new Map(prof.nodes.map((n) => [n.id, n]));
    prof.samples.forEach((id, i) => {
      const f = byId.get(id).callFrame;
      const key = `${f.functionName || "(anon)"} ${f.url.split("/").pop()}:${f.lineNumber + 1}`;
      self.set(key, (self.get(key) ?? 0) + (prof.timeDeltas[i] ?? 0) / 1000);
    });
    console.log([...self].filter(([k]) => !k.startsWith("(idle)")).sort((a, b) => b[1] - a[1]).slice(0, 25).map(([k, v]) => `${v.toFixed(0).padStart(7)} ms  ${k}`).join("\n"));
  }
  await browser.close();
  server.kill();
  await new Promise((r) => server.on("exit", r));
}

try {
  if (!argv.includes("--warm")) await run("cold");
  await run("warm");
} finally {
  if (ownData) rmSync(data, { recursive: true, force: true });
}
