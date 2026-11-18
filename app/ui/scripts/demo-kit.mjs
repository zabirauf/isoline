// Shared by the demo recorders: an overlay injected into the page (captions, highlight rings, a
// cursor that becomes a hand over links, key caps, full-screen cards), a CDP screencast recorder
// with its own frame timestamps, and the ffmpeg encode.
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { chromium } from "playwright";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- overlay (runs in the page)
export const OVERLAY = () => {
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
/** Opens a browser page with the overlay and a paused screencast; returns the scene helpers. */
export async function recorder({ outDir, W = 1440, H = 900, SCALE = 1.5, prefs, colorScheme = "light" }) {
  mkdirSync(outDir, { recursive: true });
  const framesDir = join(outDir, "frames");
  rmSync(framesDir, { recursive: true, force: true });
  mkdirSync(framesDir, { recursive: true });
  const frames = []; // { file, t, cut }
  let paused = false;
  let cutPending = false;

  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: W, height: H }, deviceScaleFactor: SCALE, colorScheme });
  if (prefs) await ctx.addInitScript((p) => localStorage.setItem("isoline.prefs.v1", JSON.stringify(p)), prefs);
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

  // A heartbeat keeps the screencast emitting frames while nothing moves (it only sends on paint,
  // so the pixel needs a color for its opacity change to repaint anything).
  await ctx.addInitScript(() => {
    const tick = () => {
      let d = document.getElementById("demo-heartbeat");
      if (!d && document.body) {
        d = document.createElement("div");
        d.id = "demo-heartbeat";
        d.style.cssText = "position:fixed;right:0;bottom:0;width:1px;height:1px;z-index:2147483647;pointer-events:none;background:#808080";
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

  /** Stops recording and encodes: each frame lasts until the next; frames before a cut get 1/30 s. */
  async function finish(name) {
    const stoppedAt = Date.now() / 1000;
    await cdp.send("Page.stopScreencast");
    await browser.close();
    let list = "ffconcat version 1.0\n";
    for (let i = 0; i < frames.length; i++) {
      const next = frames[i + 1];
      // The last frame holds until recording stopped; a gap longer than 10 s is a stall, not a scene.
      let d = next ? next.t - frames[i].t : stoppedAt - frames[i].t;
      if (next?.cut) d = 1 / 30;
      d = Math.min(Math.max(d, 0.001), 10);
      list += `file '${frames[i].file}'\nduration ${d.toFixed(4)}\n`;
    }
    list += `file '${frames[frames.length - 1].file}'\n`;
    const listFile = join(outDir, "frames.ffconcat");
    writeFileSync(listFile, list);
    const mp4 = join(outDir, name);
    execFileSync(
      "ffmpeg",
      ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", listFile, "-vf", "fps=30,scale=1920:-2:flags=lanczos,format=yuv420p", "-c:v", "libx264", "-preset", "slow", "-crf", "20", "-movflags", "+faststart", mp4],
      { stdio: "inherit" },
    );
    console.log(`${frames.length} frames -> ${mp4}`);
    if (problems.length) console.error(problems.join("\n"));
    return mp4;
  }

  return { page, ctx, startCast, pause, resume, caption, chapter, ring, keys, card, glide, glideTo, center, click, typeSlow, press, finish, problems };
}
