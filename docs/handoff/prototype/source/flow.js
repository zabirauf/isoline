  // ---------- call flow ----------
  const FL = D.flows;
  const METHOD_DESC = D.methodDescs || {};
  const view = { mode: "map", stack: [], cur: -1, builtins: false, gen: 0, timers: [] };
  const stage = $("stage");
  const KIND = { fn: "k-fn", ext: "k-ext", method: "k-method", local: "k-method", param: "k-param", builtin: "k-param", other: "k-param" };
  const hasLoops = Object.values(FL).some(u => JSON.stringify(u.tree).includes('"kind":"loop"'));
  $("flegend").innerHTML = `
    <div class="row"><span><span class="sw" style="--c: var(--accent)"></span>this file</span><span><span class="sw" style="--c: var(--ext)"></span>external import</span><span><span class="sw" style="--c: var(--method)"></span>controller method or local helper</span><span><span class="sw" style="--c: var(--muted)"></span>parameter or built-in</span></div>
    <div class="row"><span><span class="sw" style="--c: var(--cond); --s: var(--cond-soft)"></span>conditional</span><span><span class="sw" style="--c: var(--try); --s: var(--try-soft)"></span>try</span><span><span class="sw" style="--c: var(--warn); --s: var(--warn-soft)"></span>catch</span><span><span class="sw" style="--c: var(--loop); --s: var(--loop-soft)"></span>loop${hasLoops ? "" : " (none in this file)"}</span><span><span class="sw" style="--c: var(--accent); border-style: dashed"></span>runs inside the call above</span></div>
    <div>Numbers are execution order: arguments run before the call that receives them. Click a call to see its line in the code; Open › drills in.</div>`;

  const unitTitle = id => { const u = FL[id]; return u.method ? `${u.method}()` : u.id; };
  function metaFor(it) {
    const t = it.target, bits = [];
    if (t.kind === "fn") bits.push("this file");
    else if (t.kind === "ext") bits.push(t.module === "harness" ? "AgentHarness method" : shortPath(t.module));
    else if (t.kind === "method") bits.push(t.unit ? "controller method" : (t.note || "method"));
    else if (t.kind === "local") bits.push("local helper");
    else if (t.kind === "param") bits.push("parameter");
    else if (t.kind === "builtin") bits.push("built-in");
    if (it.argOf) bits.push(it.argOf === "(condition)" ? "evaluated in the condition" : `argument to ${it.argOf}()`);
    if (it.optional) bits.push("optional call");
    bits.push("L" + it.line);
    return bits.join(" · ");
  }
  const drillOf = it => it.target.kind === "fn" ? it.target.id : it.target.kind === "method" && it.target.unit ? it.target.unit : null;
  const keyOf = t => t.kind === "fn" ? "f:" + t.id : t.kind === "ext" ? "s:" + t.id : t.kind === "method" && t.of ? "f:" + t.of : "";
  let stepNo = 0;
  function renderItems(items) {
    return items.map(it => {
      if (it.t === "call") {
        const innerOf = () => it.inner && it.inner.length ? `<div class="inner">${renderItems(it.inner)}</div>` : "";
        if (!view.builtins && it.target.kind === "builtin") return innerOf();
        const n = ++stepNo, d = drillOf(it), inner = innerOf();
        return `<div class="st ${KIND[it.target.kind]}" data-line="${it.line}" data-key="${keyOf(it.target)}" tabindex="0">
          <span class="num">${n}</span>
          <span class="cl"><code>${esc(it.label)}()</code>${it.await ? '<span class="aw">await</span>' : ""}<span class="mt">${esc(metaFor(it))}</span></span>
          ${d ? `<button class="drill" data-unit="${d}" title="Show the calls inside ${esc(it.label)}()">Open ›</button>` : "<span></span>"}
        </div>${inner}`;
      }
      if (it.t === "block") {
        const body = it.children.length ? renderItems(it.children) : `<div class="nocall">no calls</div>`;
        return `<div class="blk b-${it.kind}" data-line="${it.line}"><div class="bl"><span class="kw">${esc(it.kw)}</span>${it.label ? `<code>${esc(it.label)}</code>` : ""}</div><div class="bc">${body || '<div class="nocall">no calls</div>'}</div></div>`;
      }
      if (it.t === "exit") {
        const kw = it.kind === "throw" ? "throw" : it.early ? "early return" : "return";
        const methods = it.methods ? `<span class="mchips">${it.methods.map(m => m.unit
          ? `<button class="chip fn drill" data-unit="${m.unit}">.${m.name}()</button>`
          : `<span class="chip" title="${m.from ? "function returned by " + m.from + "()" : ""}">.${m.name}()</span>`).join("")}</span>` : "";
        return `<div class="ex x-${it.kind}${it.early ? " early" : ""}" data-line="${it.line}"><span class="kw">${kw}</span>${it.label ? `<code>${esc(it.label)}</code>` : ""}${methods}</div>`;
      }
      return "";
    }).join("");
  }
  function flowStats(items, s = { calls: 0, branches: 0, tries: 0, loops: 0, callbacks: 0, awaits: 0 }) {
    items.forEach(it => {
      if (it.t === "call") { if (view.builtins || it.target.kind !== "builtin") s.calls++; if (it.await) s.awaits++; flowStats(it.inner || [], s); }
      else if (it.t === "block") {
        if (it.kind === "cond" && it.kw !== "else") s.branches++;
        if (it.kind === "try") s.tries++;
        if (it.kind === "loop") s.loops++;
        if (it.kind === "callback") s.callbacks++;
        flowStats(it.children, s);
      }
    });
    return s;
  }
  function renderCrumbs() {
    $("crumbs").innerHTML = view.stack.map((id, i) => (i ? '<span class="sep">›</span>' : "") +
      (i === view.stack.length - 1 ? `<span class="crumb cur">${esc(unitTitle(id))}</span>` : `<button class="crumb" data-depth="${i}">${esc(unitTitle(id))}</button>`)).join("");
  }
  function drawSpine() {
    const body = $("fbody"), sp = $("spine");
    if (!sp) return;
    const br = body.getBoundingClientRect();
    sp.setAttribute("width", body.scrollWidth);
    sp.setAttribute("height", body.scrollHeight);
    const pts = [...body.querySelectorAll(".st .num")].map(n => { const r = n.getBoundingClientRect(); return { x: r.left - br.left + r.width / 2, y1: r.top - br.top, y2: r.bottom - br.top }; });
    let h = "";
    for (let i = 1; i < pts.length; i++) {
      const a = pts[i - 1], b = pts[i], ym = b.y1 - 7;
      h += `<path class="seg" data-i="${i}" pathLength="1" d="M${a.x},${a.y2} L${a.x},${ym} L${b.x},${ym} L${b.x},${b.y1}"/>`;
    }
    sp.innerHTML = h;
  }
  function stopPlay() { view.gen++; view.timers.forEach(clearTimeout); view.timers = []; }
  function revealAll() {
    stopPlay();
    $("fbody").querySelectorAll(".hid, .off, .fly").forEach(e => { e.classList.remove("hid", "off", "fly"); e.style.transform = ""; e.style.transition = ""; });
    $("fbody").querySelectorAll(".seg").forEach(x => x.classList.add("on"));
  }
  function play(src) {
    stopPlay();
    const body = $("fbody"), sc = $("fscroll");
    const els = [...body.querySelectorAll(".ftree .st, .ftree .blk, .ftree .ex, .ftree .nocall")];
    if (reduce) { revealAll(); return; }
    els.forEach(e => e.classList.add(e.classList.contains("blk") ? "off" : "hid"));
    const gen = view.gen;
    // Whole sequence lands in about 0.8s: the stagger shrinks as the list grows.
    const steps = els.filter(e => e.classList.contains("st")).length || 1;
    const gap = Math.max(12, Math.min(40, 480 / steps));
    let t = 30, step = 0;
    els.forEach(e => {
      const isStep = e.classList.contains("st"), my = isStep ? ++step : 0;
      const from = src && isStep && e.dataset.key ? src.get(e.dataset.key) : null;
      view.timers.push(setTimeout(() => {
        if (gen !== view.gen) return;
        if (e.classList.contains("blk")) { e.classList.remove("off"); return; }
        if (from) {
          e.style.transition = "none";
          e.style.transform = "none";
          e.classList.remove("hid");
          const d = e.getBoundingClientRect();
          e.style.transform = `translate(${from.left - d.left}px, ${from.top - d.top}px)`;
          e.classList.add("fly");
          e.getBoundingClientRect();
          e.style.transition = "transform .38s cubic-bezier(.2,.8,.2,1), box-shadow .2s";
          e.style.transform = "";
          view.timers.push(setTimeout(() => e.classList.remove("fly"), 400));
        } else e.classList.remove("hid");
        if (my > 1) { const seg = body.querySelector(`.seg[data-i="${my - 1}"]`); if (seg) seg.classList.add("on"); }
      }, t));
      t += isStep ? gap : gap / 3;
    });
    view.timers.push(setTimeout(() => { if (gen === view.gen) view.timers = []; }, t + 420));
  }
  function renderFlow(src, animate = true) {
    const id = view.stack[view.stack.length - 1], u = FL[id], f = byId[u.owner];
    const desc = u.method ? (METHOD_DESC[u.method] || "") : f.desc;
    stepNo = 0;
    const tree = renderItems(u.tree);
    const s = flowStats(u.tree);
    $("fbody").innerHTML = `
      <header class="fentry">
        <div class="sig"><span class="kw">${u.async ? "async " : ""}${u.method ? "method" : "function"}</span><code>${esc(u.method || u.id)}(${esc(u.params.join(", "))})</code><span class="lines">L${u.start}–${u.end}</span></div>
        ${u.method ? `<div class="mt">on the controller returned by <button class="chip fn drill" data-unit="${u.owner}">${u.owner}</button></div>` : ""}
        <p>${esc(desc)}</p>
        <div class="fsum">${[[s.calls, "call"], [s.branches, "conditional"], [s.tries, "try block"], [s.loops, "loop"], [s.callbacks, "callback"], [s.awaits, "await"]].map(([n, l]) => `<span class="stat"><b>${n}</b> ${l}${n === 1 ? "" : "s"}</span>`).join("")}</div>
      </header>
      <div class="ftree">${tree || '<div class="nocall">This function makes no calls.</div>'}</div>
      <svg class="spine" id="spine" aria-hidden="true"></svg>`;
    renderCrumbs();
    view.cur = -1;
    $("f-pos").textContent = `– / ${stepNo}`;
    $("fscroll").scrollTop = 0;
    drawSpine();
    if (animate) play(src); else revealAll();
  }
  function captureMap() {
    const m = new Map();
    vp.querySelectorAll(".node").forEach(g => m.set("f:" + g.dataset.fn, g.querySelector("rect.box").getBoundingClientRect()));
    vp.querySelectorAll(".sym").forEach(g => m.set("s:" + g.dataset.sym, g.getBoundingClientRect()));
    return m;
  }
  function showView(mode) {
    view.mode = mode;
    stage.classList.toggle("flowmode", mode === "flow");
    $("flowpane").hidden = mode !== "flow";
    $("gctl").hidden = mode === "flow";
    $("fctl").hidden = mode !== "flow";
    $("tab-map").setAttribute("aria-selected", String(mode === "map"));
    $("tab-flow").setAttribute("aria-selected", String(mode === "flow"));
    if (mode === "map") { stopPlay(); hideTip(); }
  }
  function openFlow(id) {
    const src = view.mode === "map" ? captureMap() : null;
    view.stack = [id];
    $("tab-flow").disabled = false;
    $("tab-flow").title = "";
    showView("flow");
    renderFlow(src);
  }
  function drill(unitId) {
    const at = view.stack.indexOf(unitId);
    if (at >= 0) view.stack = view.stack.slice(0, at + 1); else view.stack.push(unitId);
    const owner = FL[unitId].owner;
    state.sel = { kind: "fn", id: owner };
    applyFocus();
    scrollCard(owner);
    renderFlow(null);
  }
  function syncCode(line) {
    const u = FL[view.stack[view.stack.length - 1]], card = cards[u.owner];
    toggleCode(card, true);
    document.querySelectorAll(".code .row.hl").forEach(r => r.classList.remove("hl"));
    const row = [...card.querySelectorAll(".code .row")].find(r => +r.firstChild.textContent === line);
    if (row) { row.classList.add("hl"); row.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" }); }
  }
  function setCur(i) {
    const steps = [...$("fbody").querySelectorAll(".st")];
    if (!steps.length) return;
    if (view.timers.length) revealAll();
    i = Math.max(0, Math.min(steps.length - 1, i));
    view.cur = i;
    steps.forEach((s, j) => s.classList.toggle("cur", j === i));
    steps[i].scrollIntoView({ block: "nearest", behavior: reduce ? "auto" : "smooth" });
    steps[i].focus({ preventScroll: true });
    $("f-pos").textContent = `${i + 1} / ${steps.length}`;
    syncCode(+steps[i].dataset.line);
  }

  $("fbody").addEventListener("click", e => {
    const dr = e.target.closest(".drill");
    if (dr) { drill(dr.dataset.unit); return; }
    const st = e.target.closest(".st");
    if (st) { setCur([...$("fbody").querySelectorAll(".st")].indexOf(st)); return; }
    const other = e.target.closest(".bl, .ex");
    if (other) { if (view.timers.length) revealAll(); syncCode(+other.closest("[data-line]").dataset.line); }
  });
  $("fbody").addEventListener("keydown", e => {
    const st = e.target.closest(".st");
    if (st && e.key === "Enter") { const d = st.querySelector(".drill"); if (d) drill(d.dataset.unit); else st.click(); }
  });
  $("crumbs").addEventListener("click", e => {
    const c = e.target.closest("button.crumb");
    if (!c) return;
    view.stack = view.stack.slice(0, +c.dataset.depth + 1);
    const owner = FL[view.stack[view.stack.length - 1]].owner;
    state.sel = { kind: "fn", id: owner };
    applyFocus();
    scrollCard(owner);
    renderFlow(null);
  });
  $("f-replay").addEventListener("click", () => renderFlow(null));
  $("f-prev").addEventListener("click", () => setCur(view.cur < 0 ? 0 : view.cur - 1));
  $("f-next").addEventListener("click", () => setCur(view.cur + 1));
  $("f-builtins").addEventListener("click", () => {
    view.builtins = !view.builtins;
    $("f-builtins").setAttribute("aria-pressed", String(view.builtins));
    renderFlow(null, false);
  });
  $("tab-map").addEventListener("click", () => showView("map"));
  $("tab-flow").addEventListener("click", () => {
    if (!view.stack.length || view.mode === "flow") return;
    const src = captureMap();
    showView("flow");
    renderFlow(src);
  });
