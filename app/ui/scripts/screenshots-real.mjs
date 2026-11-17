// Screenshots of the UI driven by the real engine through `iso-cli serve` (no mock data).
//
//   iso-cli serve --port 7777 &
//   node scripts/screenshots-real.mjs --repo <abs path> --file <repo-relative path> \
//        [--fn <function name>] [--steps 3] [--name prefix] [--port 7777] [--scheme light|dark|both]
//
// Fails on page errors, console errors and any request outside 127.0.0.1:<port>.
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const argv = process.argv.slice(2);
const opt = (k, d) => {
  const i = argv.indexOf(`--${k}`);
  return i >= 0 ? argv[i + 1] : d;
};
const repo = opt("repo");
const file = opt("file");
const fn = opt("fn");
const steps = Number(opt("steps", "3"));
const name = opt("name", "real");
const port = opt("port", "7777");
const schemes = opt("scheme", "both") === "both" ? ["light", "dark"] : [opt("scheme")];
if (!repo || !file) {
  console.error("usage: --repo <path> --file <rel path> [--fn name]");
  process.exit(2);
}
const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(here, "..", "screenshots");
mkdirSync(out, { recursive: true });
const base = `http://127.0.0.1:${port}/`;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const problems = [];
const browser = await chromium.launch();

for (const scheme of schemes) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on("request", (r) => {
    const u = r.url();
    if (!u.startsWith(base) && !u.startsWith("data:") && !u.startsWith("blob:")) problems.push(`network request: ${u}`);
  });
  page.on("pageerror", (e) => problems.push(`page error (${scheme}): ${e.message}`));
  page.on("console", (m) => m.type() === "error" && problems.push(`console error (${scheme}): ${m.text()}`));
  const t0 = Date.now();
  await page.goto(`${base}?backend=http&repo=${encodeURIComponent(repo)}&open=${encodeURIComponent(file)}`);
  await page.locator(".workbench").first().waitFor({ timeout: 120000 });
  await page.locator(".dmap .node, .plain-file, .iso-card").first().waitFor({ timeout: 120000 }).catch(() => {});
  await wait(1200);
  console.log(`${scheme}: file view ready in ${Date.now() - t0} ms`);
  await page.screenshot({ path: `${out}/${name}-map-${scheme}.png` });
  if (fn) {
    const head = page.locator(`.card-h[aria-label="Focus ${fn} in the graph"]`);
    if (await head.count()) {
      await head.first().click();
      await page.locator(".ftree .iso-step").first().waitFor({ timeout: 30000 });
      await wait(1400);
      for (let i = 0; i < steps; i++) {
        await page.keyboard.press("ArrowRight");
        await wait(150);
      }
      await wait(900);
      await page.screenshot({ path: `${out}/${name}-flow-${scheme}.png` });
    } else {
      problems.push(`no card for ${fn}`);
    }
  }
  await ctx.close();
}
await browser.close();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`saved ${name}-*.png in ${out}`);
