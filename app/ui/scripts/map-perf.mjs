// Zooms, pans and hovers a dense generated module map in the built UI (mock backend) and prints
// canvas frame times per phase. Run after `npm run build`:
//   node scripts/map-perf.mjs . "?files=2000&modules=3000&fanout=15&index=0" [dpr] [screenshot.png]
import { preview } from "vite";
import { chromium } from "playwright";
const ui = process.argv[2];
const q = process.argv[3] ?? "?files=2000&modules=3000&fanout=15&index=0";
const dpr = Number(process.argv[4] ?? 2);
const server = await preview({ root: ui, preview: { port: 4319, strictPort: true }, logLevel: "error" });
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr });
await ctx.addInitScript(() => {
  localStorage.setItem("isoline.prefs.v1", JSON.stringify({ hintSeen: true, flowKeySeen: true }));
  // Main-thread tasks over 50 ms (React renders, layout, canvas work) per phase.
  window.__long = [];
  new PerformanceObserver((l) => window.__long.push(...l.getEntries().map((e) => Math.round(e.duration)))).observe({ type: "longtask", buffered: true });
});
const page = await ctx.newPage();
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await page.goto("http://localhost:4319/" + q);
await page.getByRole("button", { name: "~/code/openclaw" }).click();
const canvas = page.locator(".mmap-canvas");
await canvas.waitFor();
await wait(800);
console.log("open", JSON.stringify(await page.evaluate(() => window.__long)));
await canvas.scrollIntoViewIfNeeded();
const box = await canvas.boundingBox();
console.log("canvas", JSON.stringify(box));
if (process.argv[5]) await page.screenshot({ path: process.argv[5].replace(".png", "-open.png") });
const cx = box.x + box.width / 2, cy = box.y + box.height / 2;
const reset = () => page.evaluate(() => (window.__isolineMapReset?.(), (window.__long = [])));
async function phase(name, fn) {
  await reset();
  const t0 = Date.now();
  await fn();
  const dt = Date.now() - t0;
  const s = await page.evaluate(() => ({ ...window.__isolineMapStats?.(), longTasks: window.__long }));
  console.log(name.padEnd(10), `${dt} ms wall`, JSON.stringify(s, (k, v) => (typeof v === "number" ? Math.round(v * 10) / 10 : v)));
}
await page.mouse.move(cx, cy);
// PROFILE=1: CPU profile of the zoom phase, top functions by self time.
const cdp = process.env.PROFILE ? await ctx.newCDPSession(page) : null;
if (process.env.TRACE) await browser.startTracing(page, { categories: ["devtools.timeline", "disabled-by-default-devtools.timeline"] });
if (cdp) await cdp.send("Profiler.enable"), await cdp.send("Profiler.start");
await phase("zoom-in", async () => { for (let i = 0; i < 12; i++) { await page.mouse.wheel(0, -200); await wait(16); } await wait(300); });
if (process.env.TRACE) {
  const buf = await browser.stopTracing();
  const evs = JSON.parse(buf.toString()).traceEvents.filter((e) => e.ph === "X" && e.dur);
  const tot = new Map();
  for (const e of evs) tot.set(e.name, (tot.get(e.name) ?? 0) + e.dur / 1000);
  console.log([...tot].sort((a, b) => b[1] - a[1]).slice(0, 14).map(([k, v]) => `${v.toFixed(1).padStart(8)} ms  ${k}`).join("\n"));
}
if (cdp) {
  const { profile } = await cdp.send("Profiler.stop");
  const self = new Map();
  const dt = profile.timeDeltas;
  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  profile.samples.forEach((id, i) => {
    const f = byId.get(id).callFrame;
    const key = `${f.functionName || "(anon)"} ${f.url.split("/").pop()}:${f.lineNumber}`;
    self.set(key, (self.get(key) ?? 0) + (dt[i] ?? 0) / 1000);
  });
  console.log([...self].sort((a, b) => b[1] - a[1]).slice(0, 15).map(([k, v]) => `${v.toFixed(1).padStart(8)} ms  ${k}`).join("\n"));
}
await phase("pan", async () => { await page.mouse.down(); for (let i = 0; i < 80; i++) await page.mouse.move(cx - i * 6, cy - i * 3); await page.mouse.up(); await wait(300); });
await phase("hover", async () => { for (let i = 0; i < 80; i++) await page.mouse.move(box.x + 100 + i * 12, cy + ((i * 37) % 200) - 100); await wait(300); });
await phase("zoom-out", async () => { for (let i = 0; i < 12; i++) { await page.mouse.wheel(0, 200); await wait(16); } await wait(300); });
const n = await page.evaluate(() => window.__isolineMapStats?.());
if (process.argv[5]) await page.screenshot({ path: process.argv[5] });
await browser.close();
await new Promise((r) => server.httpServer.close(r));
