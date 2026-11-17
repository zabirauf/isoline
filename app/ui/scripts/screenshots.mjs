// Loads the built app (dist/, mock backend) in Chromium at 1440x900, walks through every view
// and saves screenshots to screenshots/. Also checks that the app makes no network requests
// outside its own origin and measures canvas frame times while panning a 5,000-module map.
// Run after `npm run build`:  node scripts/screenshots.mjs
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { preview } from "vite";
import { chromium } from "playwright";

const here = dirname(fileURLToPath(import.meta.url));
const ui = resolve(here, "..");
const out = resolve(ui, "screenshots");
mkdirSync(out, { recursive: true });

const server = await preview({ root: ui, preview: { port: 4317, strictPort: true }, logLevel: "error" });
const base = "http://localhost:4317/";
const browser = await chromium.launch();
const problems = [];
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function newPage(scheme, query = "") {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on("request", (r) => {
    if (!r.url().startsWith(base) && !r.url().startsWith("data:") && !r.url().startsWith("blob:")) problems.push(`network request: ${r.url()}`);
  });
  page.on("pageerror", (e) => problems.push(`page error (${scheme}): ${e.message}`));
  page.on("console", (m) => m.type() === "error" && problems.push(`console error (${scheme}): ${m.text()}`));
  await page.goto(base + query);
  return page;
}

async function openLifecycle(page) {
  await page.getByRole("button", { name: "~/code/openclaw" }).click();
  await page.getByRole("tree", { name: "Files" }).waitFor();
  await page.keyboard.press("/");
  await page.keyboard.type("lifecycle.ts");
  await page.locator("#search-results li", { hasText: "src/agents/harness/lifecycle.ts" }).first().click();
  await page.locator(".dmap .node").first().waitFor();
  await wait(600);
}

async function focusAndStep(page) {
  await page.locator('.card-h[aria-label="Focus runAgentHarnessLifecycleAttempt in the graph"]').click();
  await page.locator(".ftree .iso-step").first().waitFor();
  await wait(1400); // the flow lands in about one second
  for (let i = 0; i < 3; i++) {
    await page.keyboard.press("ArrowRight");
    await wait(150);
  }
  await page.locator(".monaco-editor .mx-step-line").first().waitFor();
  await wait(900);
}

for (const scheme of ["light", "dark"]) {
  const page = await newPage(scheme);
  await page.getByText("Recent repos").waitFor();
  await page.screenshot({ path: `${out}/start-${scheme}.png` });
  await page.getByRole("button", { name: "~/code/openclaw" }).click();
  await page.locator(".mmap-canvas").waitFor();
  await wait(3200); // simulated indexing finishes
  await page.screenshot({ path: `${out}/overview-${scheme}.png` });
  await page.close();

  const p2 = await newPage(scheme);
  await openLifecycle(p2);
  await p2.screenshot({ path: `${out}/map-${scheme}.png` });
  await focusAndStep(p2);
  const pos = await p2.locator(".fpos").textContent();
  if (!/^3 \/ \d+$/.test((pos ?? "").trim())) problems.push(`expected step 3, saw "${pos}"`);
  await p2.screenshot({ path: `${out}/flow-${scheme}.png` });
  // Drill into the controller method of step 3.
  await p2.locator('.iso-step[data-i="2"] .iso-btn-drill').click();
  await p2.mouse.move(5, 895);
  await wait(1300);
  const drillPos = (await p2.locator(".fpos").textContent())?.trim();
  const curSteps = await p2.locator(".iso-step-cur").count();
  if (drillPos?.startsWith("–") && curSteps) problems.push(`after drill the counter says "${drillPos}" but ${curSteps} step is highlighted`);
  await p2.screenshot({ path: `${out}/flow-drill-${scheme}.png` });
  await p2.close();
}

// Outline tier (Go) and text tier (Markdown).
{
  const page = await newPage("light", "?open=tools/relay/main.go");
  await page.locator(".outline-item").first().waitFor();
  await page.locator(".monaco-editor").first().waitFor();
  await page.locator(".outline-item", { hasText: "post" }).click();
  await wait(700);
  await page.screenshot({ path: `${out}/outline-tier-light.png` });
  await page.goto(base + "?open=README.md");
  await page.locator(".monaco-editor").first().waitFor();
  await wait(700);
  await page.screenshot({ path: `${out}/text-tier-light.png` });
  await page.close();
}

// Scale: 12,000 files in the tree, 5,000 modules on the canvas map.
{
  const page = await newPage("light", "?files=12000&modules=5000&index=0");
  await page.getByRole("button", { name: "~/code/openclaw" }).click();
  await page.locator(".mmap-canvas").waitFor();
  await wait(800);
  const rows = await page.locator(".tree .row").count();
  if (rows > 200) problems.push(`sidebar renders ${rows} rows; it should be virtualized`);
  const canvas = page.locator(".mmap-canvas");
  const box = await canvas.boundingBox();
  // Fitted: the zoomed-out level of detail (one block per depth column, counts, bundles).
  await page.screenshot({ path: `${out}/scale-5000-modules-light.png` });
  // Zoom in on the middle until names show, then pan for a while and read the frame times.
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 9; i++) {
    await page.mouse.wheel(0, -240);
    await wait(30);
  }
  await wait(200);
  await page.screenshot({ path: `${out}/scale-5000-modules-zoomed-light.png` });
  await page.mouse.down();
  const t0 = Date.now();
  for (let i = 0; i < 60; i++) await page.mouse.move(box.x + box.width / 2 - i * 4, box.y + box.height / 2 - i * 2);
  await page.mouse.up();
  const dt = Date.now() - t0;
  const stats = await page.evaluate(() => window.__isolineMapStats?.() ?? null);
  console.log(`scale: pan of 60 moves took ${dt} ms; canvas frame stats ${JSON.stringify(stats)}`);
  if (stats && stats.worstMs > 16) problems.push(`slowest module-map frame took ${stats.worstMs} ms`);
  await page.close();
}

await browser.close();
await new Promise((r) => server.httpServer.close(r));
if (problems.length) {
  console.error(problems.join("\n"));
  process.exitCode = 1;
} else console.log(`screenshots saved to ${out}; no network requests outside the app`);
