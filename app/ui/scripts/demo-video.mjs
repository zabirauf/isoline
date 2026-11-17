// Records the annotated demo video against the real engine (`iso-cli serve`), no mock data.
//
//   iso-cli serve --port 7788 --data <fresh dir> &
//   node scripts/demo-video.mjs --big <abs repo> --big-file <rel> --edit-repo <abs scratch copy> \
//        --edit-file <rel> --out <dir> [--port 7788]
//
// The big repo is opened cold (its index must not exist in the data dir) and is only read.
// The edit repo must be a scratch copy: the live-edit scene appends a function to a file in it.
// Frames come from the CDP screencast (lossless-ish JPEG at device scale 1.5) with their own
// timestamps; `ffmpeg` turns them into an MP4. Annotations are drawn by an overlay injected
// into the page (captions, highlight rings, a cursor that becomes a hand over links, key caps).
import { mkdirSync, writeFileSync, appendFileSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import { chromium } from "playwright";

const argv = process.argv.slice(2);
const opt = (k, d) => {
  const i = argv.indexOf(`--${k}`);
  return i >= 0 ? argv[i + 1] : d;
};
const port = opt("port", "7788");
const big = opt("big");
const bigFile = opt("big-file", "src/agents/harness/lifecycle.ts");
const editRepo = opt("edit-repo");
const editFile = opt("edit-file", "app/ui/src/lib/format.ts");
const outDir = resolve(opt("out", "demo-out"));
const only = opt("only"); // run one scene while iterating
if (!big || !editRepo) {
  console.error("usage: --big <repo> --edit-repo <scratch copy> [--out dir]");
  process.exit(2);
}
const base = `http://127.0.0.1:${port}/`;
const W = 1440;
const H = 900;
const SCALE = 1.5;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- overlay (runs in the page)
const OVERLAY = () => {
  const css = `
  #demo-root{position:fixed;inset:0;pointer-events:none;z-index:2147483647;font-family:-apple-system,BlinkMacSystemFont,"Inter","Segoe UI",sans-serif}
  #demo-cap{position:absolute;left:50%;bottom:34px;transform:translate(-50%,16px);opacity:0;transition:opacity .35s,transform .35s;
    background:rgba(17,20,28,.92);color:#fff;border-radius:14px;padding:14px 22px 15px;max-width:980px;box-shadow:0 12px 40px rgba(0,0,0,.35);
    border:1px solid rgba(255,255,255,.08);backdrop-filter:blur(8px)}
  #demo-cap.on{opacity:1;transform:translate(-50%,0)}
  #demo-cap .t{font-size:21px;font-weight:650;letter-spacing:-.01em;line-height:1.25}
  #demo-cap .s{font-size:15px;color:#b9c2d3;margin-top:5px;line-height:1.4}
  #demo-cap .e{font-size:12px;font-weight:750;letter-spacing:.1em;text-transform:uppercase;color:#7dd3fc;margin-bottom:6px}
  #demo-cap .n{display:inline-block;font-size:12px;font-weight:700;color:#0b0e14;background:#7dd3fc;border-radius:6px;padding:2px 7px;margin-right:10px;vertical-align:3px}
  .demo-ring{position:absolute;border:3px solid #f59e0b;border-radius:10px;box-shadow:0 0 0 4px rgba(245,158,11,.25),0 0 30px rgba(245,158,11,.35);
    transition:all .35s ease;opacity:0}
  .demo-ring.on{opacity:1}
  .demo-ring .l{position:absolute;top:-30px;left:-3px;background:#f59e0b;color:#1a1204;font-size:13px;font-weight:700;padding:3px 9px;border-radius:7px;white-space:nowrap}
  .demo-ring.below .l{top:auto;bottom:-30px}
  #demo-cursor{position:absolute;left:0;top:0;width:28px;height:28px;transform:translate(-200px,-200px);transition:none}
  #demo-cursor svg{position:absolute;left:0;top:0;filter:drop-shadow(0 2px 3px rgba(0,0,0,.35))}
  #demo-cursor .hand{display:none}
  #demo-cursor.pointer .arrow{display:none}
  #demo-cursor.pointer .hand{display:block}
  .demo-click{position:absolute;width:44px;height:44px;margin:-22px 0 0 -22px;border-radius:50%;border:3px solid #f59e0b;animation:demo-click .5s ease-out forwards}
  @keyframes demo-click{from{transform:scale(.3);opacity:1}to{transform:scale(1.4);opacity:0}}
  #demo-keys{position:absolute;right:28px;bottom:34px;display:flex;gap:8px;opacity:0;transition:opacity .2s}
  #demo-keys.on{opacity:1}
  #demo-keys kbd{font:600 22px -apple-system,BlinkMacSystemFont,sans-serif;color:#fff;background:rgba(17,20,28,.92);border:1px solid rgba(255,255,255,.18);
    border-bottom-width:3px;border-radius:10px;padding:8px 14px;min-width:24px;text-align:center}
  #demo-card{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;opacity:0;transition:opacity .5s;
    background:radial-gradient(1200px 700px at 30% 20%,#1d2b4a 0%,#0b0e14 60%);color:#fff}
  #demo-card.on{opacity:1}
  #demo-card .box{max-width:1100px;padding:0 60px}
  #demo-card h1{font-size:64px;font-weight:750;letter-spacing:-.03em;margin:0 0 14px}
  #demo-card h2{font-size:28px;font-weight:500;color:#b9c2d3;margin:0 0 28px;letter-spacing:-.01em}
  #demo-card .tags{display:flex;flex-wrap:wrap;gap:10px}
  #demo-card .tags span{font-size:16px;color:#dbe4f3;border:1px solid rgba(255,255,255,.18);border-radius:999px;padding:6px 14px}
  #demo-card .eyebrow{font-size:15px;font-weight:700;letter-spacing:.12em;text-transform:uppercase;color:#7dd3fc;margin-bottom:16px}
  #demo-card table{border-collapse:collapse;width:100%;font-size:19px}
  #demo-card td{padding:9px 14px;border-bottom:1px solid rgba(255,255,255,.08)}
  #demo-card td.m{color:#b9c2d3}
  #demo-card td.v{font-weight:700;text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}
  #demo-card td.g{color:#8a96ab;font-size:15px;text-align:right;white-space:nowrap}
  #demo-card td.x{color:#4ade80;font-weight:700;font-size:15px;white-space:nowrap}
  #demo-card .cols{display:grid;grid-template-columns:1.25fr 1fr;gap:48px;align-items:start}
  #demo-card .stat{font-size:44px;font-weight:750;letter-spacing:-.02em}
  #demo-card .lab{font-size:15px;color:#b9c2d3;margin-bottom:22px}
  #demo-chapter{position:absolute;left:36px;top:76px;opacity:0;transition:opacity .4s;background:#7dd3fc;color:#06222f;font-weight:750;font-size:14px;
    letter-spacing:.08em;text-transform:uppercase;padding:7px 12px;border-radius:8px}
  #demo-chapter.on{opacity:1}
  `;
  const install = () => {
    if (document.getElementById("demo-root")) return;
    const st = document.createElement("style");
    st.textContent = css;
    document.head.appendChild(st);
    const root = document.createElement("div");
    root.id = "demo-root";
    root.innerHTML = `<div id="demo-card"></div><div id="demo-chapter"></div><div id="demo-cap"></div><div id="demo-keys"></div>
      <div id="demo-cursor"><svg class="arrow" width="28" height="28" viewBox="0 0 28 28"><path d="M5 3 L5 22 L10 17.5 L13.5 25 L17 23.5 L13.6 16.2 L20 16 Z" fill="#fff" stroke="#111" stroke-width="1.6" stroke-linejoin="round"/></svg>
      <svg class="hand" width="30" height="30" viewBox="0 0 30 30" style="left:-9px;top:-2px"><path d="M11 3.5c1.1 0 2 .9 2 2V13l.6-.1c.3-1 1.2-1.6 2.2-1.4.8.2 1.4.8 1.5 1.6.4-.7 1.2-1.1 2-.9.9.2 1.5 1 1.5 1.9.4-.4 1-.6 1.6-.5 1 .2 1.6 1.1 1.6 2.1v5c0 4-3.2 7.3-7.2 7.3h-2.4c-2.4 0-4.6-1.3-5.8-3.4L5 16.6c-.6-1-.3-2.2.7-2.8.9-.5 2-.3 2.6.5L9 15.7V5.5c0-1.1.9-2 2-2z" fill="#fff" stroke="#111" stroke-width="1.5" stroke-linejoin="round"/></svg></div>`;
    document.body.appendChild(root);
    const cur = root.querySelector("#demo-cursor");
    const track = (e) => {
      cur.style.transform = `translate(${e.clientX}px,${e.clientY}px)`;
      const el = document.elementFromPoint(e.clientX, e.clientY);
      const c = el ? getComputedStyle(el).cursor : "auto";
      cur.classList.toggle("pointer", c === "pointer");
    };
    document.addEventListener("mousemove", track, true);
    document.addEventListener(
      "mousedown",
      (e) => {
        const r = document.createElement("div");
        r.className = "demo-click";
        r.style.left = `${e.clientX}px`;
        r.style.top = `${e.clientY}px`;
        root.appendChild(r);
        setTimeout(() => r.remove(), 600);
      },
      true,
    );
  };
  const $ = (id) => document.getElementById(id);
  window.__demo = {
    install,
    caption(t, s, n) {
      install();
      const c = $("demo-cap");
      if (!t) return c.classList.remove("on");
      const e = window.__demoChapter ? `<div class="e">${window.__demoChapter}</div>` : "";
      c.innerHTML = `${e}<div class="t">${n ? `<span class="n">${n}</span>` : ""}${t}</div>${s ? `<div class="s">${s}</div>` : ""}`;
      c.classList.add("on");
    },
    chapter(t) {
      window.__demoChapter = t;
    },
    ring(sel, label, pad = 6, below = false) {
      install();
      document.querySelectorAll(".demo-ring").forEach((r) => r.remove());
      if (!sel) return false;
      const el = typeof sel === "string" ? document.querySelector(sel) : null;
      const b = sel.rect ?? el?.getBoundingClientRect();
      if (!b) return false;
      const r = document.createElement("div");
      r.className = `demo-ring${below ? " below" : ""}`;
      Object.assign(r.style, { left: `${b.left - pad}px`, top: `${b.top - pad}px`, width: `${b.width + 2 * pad}px`, height: `${b.height + 2 * pad}px` });
      if (label) r.innerHTML = `<span class="l">${label}</span>`;
      $("demo-root").appendChild(r);
      requestAnimationFrame(() => r.classList.add("on"));
      return true;
    },
    keys(list) {
      install();
      const k = $("demo-keys");
      if (!list || !list.length) return k.classList.remove("on");
      k.innerHTML = list.map((x) => `<kbd>${x}</kbd>`).join("");
      k.classList.add("on");
    },
    card(html) {
      install();
      const c = $("demo-card");
      if (!html) return c.classList.remove("on");
      c.innerHTML = `<div class="box">${html}</div>`;
      c.classList.add("on");
    },
  };
  if (document.body) install();
  else document.addEventListener("DOMContentLoaded", install);
};

// ---------------------------------------------------------------- recorder
mkdirSync(outDir, { recursive: true });
const framesDir = join(outDir, "frames");
rmSync(framesDir, { recursive: true, force: true });
mkdirSync(framesDir, { recursive: true });
const frames = []; // { file, t, cut }
let paused = false;
let cutPending = false;

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: W, height: H }, deviceScaleFactor: SCALE, colorScheme: "light" });
await ctx.addInitScript(() => localStorage.setItem("isoline.prefs.v1", JSON.stringify({ detail: "read", mapExternal: false, flowKeySeen: true, hintSeen: true, aiNoticeHidden: true })));
await ctx.addInitScript(OVERLAY);
const page = await ctx.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
page.on("console", (m) => m.type() === "error" && problems.push(`console: ${m.text()}`));

const cdp = await ctx.newCDPSession(page);
cdp.on("Page.screencastFrame", async (f) => {
  try {
    await cdp.send("Page.screencastFrameAck", { sessionId: f.sessionId });
  } catch {}
  if (paused) return;
  const file = join(framesDir, `f${String(frames.length).padStart(6, "0")}.jpg`);
  writeFileSync(file, Buffer.from(f.data, "base64"));
  frames.push({ file, t: f.metadata.timestamp, cut: cutPending });
  cutPending = false;
});
const startCast = () => cdp.send("Page.startScreencast", { format: "jpeg", quality: 92, maxWidth: W * SCALE, maxHeight: H * SCALE, everyNthFrame: 1 });
const pause = () => (paused = true);
const resume = () => ((paused = false), (cutPending = true));

// A heartbeat keeps the screencast emitting frames while nothing moves (it only sends on paint).
await ctx.addInitScript(() => {
  const tick = () => {
    let d = document.getElementById("demo-heartbeat");
    if (!d && document.body) {
      d = document.createElement("div");
      d.id = "demo-heartbeat";
      d.style.cssText = "position:fixed;right:0;bottom:0;width:1px;height:1px;z-index:2147483647;pointer-events:none";
      document.body.appendChild(d);
    }
    if (d) d.style.opacity = d.style.opacity === "0.01" ? "0.02" : "0.01";
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});

const demo = (fn, ...args) => page.evaluate(([fn, args]) => window.__demo[fn](...args), [fn, args]);
const caption = (t, s, n) => demo("caption", t ?? null, s ?? null, n ?? null);
const chapter = (t) => demo("chapter", t ?? null);
async function ring(sel, label, pad, below) {
  let arg = sel ?? null;
  if (arg && typeof arg !== "string") {
    const b = await arg.boundingBox();
    arg = b ? { rect: { left: b.x, top: b.y, width: b.width, height: b.height } } : null;
  }
  return demo("ring", arg, label ?? null, pad ?? 6, below ?? false);
}
const keys = (list) => demo("keys", list ?? []);
const card = (html) => demo("card", html ?? null);
let mx = W / 2;
let my = H / 2;
async function glide(x, y, ms = 650) {
  const n = Math.max(8, Math.round(ms / 16));
  const x0 = mx;
  const y0 = my;
  for (let i = 1; i <= n; i++) {
    const t = i / n;
    const e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    await page.mouse.move(x0 + (x - x0) * e, y0 + (y - y0) * e);
    await sleep(ms / n);
  }
  mx = x;
  my = y;
}
async function center(loc) {
  const b = await loc.boundingBox();
  if (!b) throw new Error("no box");
  return [b.x + b.width / 2, b.y + b.height / 2];
}
async function glideTo(loc, ms, dx = 0, dy = 0) {
  const [x, y] = await center(loc);
  await glide(x + dx, y + dy, ms);
}
async function click(loc, ms = 600) {
  await glideTo(loc, ms);
  await sleep(180);
  await page.mouse.down();
  await sleep(70);
  await page.mouse.up();
}
async function typeSlow(text, delay = 70) {
  for (const ch of text) {
    await page.keyboard.type(ch);
    await sleep(delay);
  }
}
async function press(key, shown, hold = 500) {
  if (shown) await keys(shown);
  await page.keyboard.press(key);
  await sleep(hold);
  if (shown) await keys([]);
}
const cmdKey = process.platform === "darwin" ? "Meta" : "Control";
const url = (repo, open) => `${base}?backend=http&repo=${encodeURIComponent(repo)}${open ? `&open=${encodeURIComponent(open)}` : ""}`;
const fnName = "runAgentHarnessLifecycleAttempt";

// ---------------------------------------------------------------- scenes
await page.goto(`${base}?backend=http`);
await page.locator("body").waitFor();
await sleep(400);
await startCast();
const T0 = Date.now();
const mark = (s) => console.log(`${((Date.now() - T0) / 1000).toFixed(1)}s ${s}`);

// 0. Title
await card(`<div class="eyebrow">Isoline · desktop code map</div><h1>Understand code without reading every line.</h1>
  <h2>Function cards, a dependency map and call flow for any repo, built from a real index: tree-sitter, a module resolver and SQLite, in Rust.</h2>
  <div class="tags"><span>This demo: openclaw · 64,228 files · 14.6M lines</span><span>Real engine, nothing mocked</span><span>Apple silicon laptop</span></div>`);
await sleep(4800);

// 1. Cold index of a big repo, and opening a file while it runs
mark("cold index");
await page.goto(url(big));
await page.locator(".workbench, .repo-page").first().waitFor({ timeout: 60000 });
await card(null);
await chapter("1 · Open a big repo");
await caption("Opening openclaw for the first time", "64,228 files, 14.6M lines. No index exists yet: every file is walked, hashed and parsed with tree-sitter.");
await ring(".appbar .status .indexing", "live progress", 4, true);
await sleep(4200);
await ring(null);
await caption("You don't have to wait for it", "Search works mid-index, and the file you open is indexed ahead of the queue.");
await sleep(900);
await page.keyboard.press("/");
await keys(["/"]);
await sleep(500);
await keys([]);
await typeSlow("lifecycle.ts", 85);
await sleep(900);
const hit = page.locator(".search-pop [role=option]").filter({ hasText: "harness/lifecycle.ts" }).first();
await hit.waitFor({ timeout: 20000 });
await click(hit, 700);
await page.locator(".iso-card").first().waitFor({ timeout: 30000 });
await sleep(600);
await ring(".appbar .status .indexing", "still indexing", 4, true);
await caption("lifecycle.ts opened while 64k files are still indexing", "Cards, signatures, the dependency map and call flow, for this file, right now.");
await sleep(4200);
await ring(null);
// Fast-forward to the end of the index.
pause();
await page.locator(".appbar .status .indexing").filter({ hasText: /^Indexed/ }).first().waitFor({ timeout: 240000 });
const indexedText = (await page.locator(".appbar .status .indexing").first().innerText()).trim();
mark(indexedText);
resume();
await caption(`⏩ ${indexedText.replace(/^Indexed /, "Cold index finished: ")}`, "About 21 million lines a minute on 12 cores. Reopening the repo later takes 370 ms.");
await ring(".appbar .status .indexing", null, 4, true);
await sleep(3800);
await ring(null);

// 2. Repo overview: module map
mark("overview");
await chapter("2 · Repo overview");
await click(page.locator(".repo-menu button").first(), 700);
await sleep(400);
await click(page.getByRole("menuitem", { name: "Repo overview" }).first(), 500);
await page.locator(".repo-page").waitFor();
await sleep(1200);
await caption("Modules, clustered from folders and imports", "Colors are stable across runs; rename a module and it sticks. Languages, hot spots and entry points sit below.");
await glide(W * 0.55, H * 0.45, 900);
await sleep(1200);
await glide(W * 0.42, H * 0.38, 900);
await sleep(2400);

// 3. Search for a symbol, cards at three detail levels
mark("cards");
await chapter("3 · Function cards");
await caption("Search files and symbols", "One index for 64k files: results while you type.");
await press("/", ["/"], 300);
await typeSlow("runAgentHarnessLife", 70);
await sleep(700);
const symHit = page.locator(".search-pop [role=option]").filter({ hasText: fnName }).first();
await symHit.waitFor({ timeout: 10000 });
await click(symHit, 600);
await page.locator(`.iso-card[data-card="${fnName}"]`).waitFor({ timeout: 20000 });
await sleep(1200);
// The search pick focuses the function; clear it so the map shows the whole file.
await page.locator(".pane-left").click({ position: { x: 4, y: 4 } });
await page.keyboard.press("Escape");
await page.getByRole("tab", { name: "Dependency map" }).click();
await sleep(800);
await caption("Every function as a card", "Description, typed signature, what it calls and who calls it. Exported entry points come first.");
await sleep(3500);
await page.locator(".pane-left").click({ position: { x: 4, y: 4 } });
await caption("Glance", "Progressive disclosure: one row per function when you want the shape of a file.", "1");
await ring(page.locator('.iso-seg[aria-label="Detail"]'), null, 4);
await press("1", ["1"], 2600);
await caption("Read", "The default: signatures and relations, details folded away.", "2");
await press("2", ["2"], 2200);
await caption("Dig", "Everything: every parameter with defaults, layers, all relation rows open.", "3");
await press("3", ["3"], 2600);
await press("2", null, 600);
await ring(null);

// 4. Typed signatures: origin colors, peek, cmd-hold, jump to definition, back
mark("types");
await chapter("4 · Types and links");
const fnCard = page.locator(`.iso-card[data-card="${fnName}"]`);
await fnCard.scrollIntoViewIfNeeded();
await sleep(400);
const sig = fnCard.locator(".iso-sig").first();
await ring(sig, "typed signature", 6, true);
await caption("Parameter types are colored by where they come from", "This file, another file in the repo, a package, or the language itself.");
await glideTo(sig, 800, -40, 0);
await sleep(2600);
const ty = fnCard.locator(".iso-sig button.ty").first();
await ring(null);
await caption("Hover a type to peek at its definition", "The cursor becomes a hand over anything you can open. Nothing is underlined at rest.");
await glideTo(ty, 700);
await page.locator(".iso-peek").waitFor({ timeout: 8000 });
await sleep(3400);
// Rest the pointer on the empty map so the peek closes before the ⌘ hold.
await glide(W * 0.86, H * 0.28, 700);
await sleep(700);
await caption("Hold ⌘ to reveal every link at once", "Types, calls, imports and callers. Let go and the page is quiet again.");
await keys(["⌘"]);
await page.keyboard.down(cmdKey);
await sleep(2800);
await page.keyboard.up(cmdKey);
await keys([]);
await sleep(300);
await caption("Click to jump to the code", "Definitions open in another file at the right line, in Monaco.");
await click(ty, 700);
await sleep(2600);
await caption("⌘[ and ⌘] go back and forward", "Like an editor, with scroll and selection restored.");
await keys(["⌘", "["]);
await page.keyboard.press(`${cmdKey}+BracketLeft`);
await sleep(700);
await keys([]);
await page.locator(`.iso-card[data-card="${fnName}"]`).waitFor({ timeout: 10000 });
await glide(W * 0.86, H * 0.28, 600);
await sleep(1500);

// 5. Dependency map
mark("map");
await chapter("5 · Dependency map");
const mapTab = page.getByRole("tab", { name: "Dependency map" });
await click(mapTab, 700);
await sleep(1200);
await caption("Dependency map", "This file's functions in layers by call depth, with what they import, grouped by module color.");
const ext = page.getByRole("button", { name: "External imports" });
if (await ext.count()) {
  await click(ext, 700);
  await sleep(1400);
  await caption("Show what the file imports", "Imported modules in their module colors, with the symbols each function uses.");
  await sleep(2600);
}
const node = page.locator(".dmap .node").first();
if (await node.count()) {
  await glideTo(node, 900);
  await sleep(2200);
  const node2 = page.locator(".dmap .node").nth(2);
  if (await node2.count()) await glideTo(node2, 800);
  await sleep(2000);
} else {
  await glide(W * 0.78, H * 0.42, 900);
  await sleep(3000);
}

// 6. Call flow
mark("flow");
await chapter("6 · Call flow");
const head = page.locator(`.card-h[aria-label="Focus ${fnName} in the graph"]`).first();
await caption("Call flow: every call in order", "try blocks, callbacks, awaits and conditionals, the same output as the prototype's analyzer.");
await click(head, 800);
await page.locator(".ftree .iso-step").first().waitFor({ timeout: 30000 });
await sleep(2200);
await caption("Step through it; the code follows", "Each step highlights its line in the card's code.");
for (let i = 0; i < 5; i++) await press("ArrowRight", ["→"], 750);
await sleep(1600);

// 7. Dark theme
mark("dark");
await chapter("7 · Light and dark");
await caption("Light and dark themes", "From the design system's tokens, including the editor.");
await click(page.getByRole("button", { name: "Theme" }), 700);
await sleep(400);
await page.locator(".theme-pop").waitFor({ timeout: 5000 });
await click(page.locator(".theme-pop button").filter({ hasText: "Dark" }).first(), 500);
await page.locator('html[data-theme="dark"]').waitFor({ timeout: 5000 });
await glide(W * 0.86, H * 0.3, 700);
await sleep(3200);

// 8. Live edit on a scratch copy
mark("edit");
pause();
await page.goto(url(editRepo, editFile));
await page.locator(".iso-card").first().waitFor({ timeout: 60000 });
await sleep(1500);
resume();
await chapter("8 · Incremental updates");
await caption("Edit a file in your editor and save", "Here: a scratch copy of Isoline's own repo. We append one exported function to format.ts.");
await sleep(3200);
const target = join(editRepo, editFile);
const before = readFileSync(target, "utf8");
appendFileSync(
  target,
  `\n/** Index speed for the status line: "21.2M lines/min". */\nexport function fmtRate(lines: number, ms: number): string {\n  const perMin = (lines / Math.max(1, ms)) * 60_000;\n  return perMin >= 1e6 ? \`\${(perMin / 1e6).toFixed(1)}M lines/min\` : \`\${fmtInt(perMin)} lines/min\`;\n}\n`,
);
mark("saved");
await caption("Saved. One file re-hashed and re-parsed, nothing else", "The watcher sees the change, the Merkle tree finds it, and the open view refreshes.");
await page.locator(`.iso-card[data-card="fmtRate"]`).waitFor({ timeout: 15000 });
await sleep(300);
const st = page.locator(".appbar .status .indexing").first();
const took = (await st.count()) ? (await st.innerText()).trim() : "";
mark(took);
await ring(".appbar .status .indexing", null, 4, true);
if (took) await caption(`Saved → ${took}`, "Only this file is re-parsed. A new export means its importers re-link their edges; nothing else is touched.");
await sleep(2200);
const newCard = page.locator(`.iso-card[data-card="fmtRate"]`);
await newCard.evaluate((el) => el.scrollIntoView({ block: "center", behavior: "smooth" }));
await sleep(900);
await ring(newCard, "new card", 6);
await sleep(3800);
await ring(null);
writeFileSync(target, before);

// 9. Numbers
mark("numbers");
await chapter(null);
await caption(null);
await card(`<div class="cols"><div><div class="eyebrow">Measured on openclaw · 64,228 files · 14.6M lines</div>
  <table>
  <tr><td class="m">Cold index</td><td class="v">21.2M lines/min</td><td class="g">target 1M</td><td class="x">21×</td></tr>
  <tr><td class="m">Reopen an indexed repo</td><td class="v">370 ms</td><td class="g">under 1 s</td><td class="x">✓</td></tr>
  <tr><td class="m">Open a file, p95</td><td class="v">6.1 ms</td><td class="g">under 100 ms</td><td class="x">✓</td></tr>
  <tr><td class="m">Edit to refreshed view, p95</td><td class="v">29 ms</td><td class="g">under 50 ms</td><td class="x">✓</td></tr>
  <tr><td class="m">Body edit: files reparsed</td><td class="v">1, 0 dependents</td><td class="g">exactly 1</td><td class="x">✓</td></tr>
  <tr><td class="m">Unresolved imports</td><td class="v">0.05%</td><td class="g">under 5%</td><td class="x">✓</td></tr>
  <tr><td class="m">Call edges confirmed by SCIP</td><td class="v">100%</td><td class="g">90% or better</td><td class="x">✓</td></tr>
  <tr><td class="m">Call flow equal to the prototype</td><td class="v">2,026 / 2,026</td><td class="g">units</td><td class="x">✓</td></tr>
  </table></div>
  <div><div class="eyebrow">Replaying git history</div>
  <div class="stat">32 / 32</div><div class="lab">commit steps did exactly the work <code>git diff</code> implies</div>
  <div class="stat">6–27 of 2,148</div><div class="lab">folders the Merkle diff visits per commit</div>
  <div class="stat">0 reparsed</div><div class="lab">jumping back 30 commits and forward again (content-hash cache)</div>
  <div class="stat">36 / 36</div><div class="lab">steps identical to a fresh cold index (Isoline's own history)</div>
  </div></div>`);
await sleep(9500);

// 10. End card
await card(`<div class="eyebrow">Isoline 0.1.0</div><h1>Read the shape first. Dig when you need to.</h1>
  <h2>Rust engine · tree-sitter · oxc resolver · SQLite · Tauri 2 · React · Monaco. AI summaries via GitHub Copilot, off until you turn them on.</h2>
  <div class="tags"><span>macOS · arm64 + x64</span><span>Windows · arm64 + x64</span><span>Linux · arm64 + x64</span><span>Nix flake</span><span>iso-cli</span></div>`);
await sleep(5000);
mark("end");

await cdp.send("Page.stopScreencast");
await browser.close();

// ---------------------------------------------------------------- encode
// Each frame lasts until the next one; frames right before a cut get 1/30 s.
let list = "ffconcat version 1.0\n";
for (let i = 0; i < frames.length; i++) {
  const next = frames[i + 1];
  let d = next ? next.t - frames[i].t : 0.5;
  if (next?.cut) d = 1 / 30;
  d = Math.min(Math.max(d, 0.001), 2);
  list += `file '${frames[i].file}'\nduration ${d.toFixed(4)}\n`;
}
list += `file '${frames[frames.length - 1].file}'\n`;
const listFile = join(outDir, "frames.ffconcat");
writeFileSync(listFile, list);
const mp4 = join(outDir, "isoline-demo.mp4");
execFileSync(
  "ffmpeg",
  ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", listFile, "-vf", "fps=30,scale=1920:-2:flags=lanczos,format=yuv420p", "-c:v", "libx264", "-preset", "slow", "-crf", "20", "-movflags", "+faststart", mp4],
  { stdio: "inherit" },
);
console.log(`${frames.length} frames -> ${mp4}`);
if (problems.length) console.error(problems.join("\n"));
