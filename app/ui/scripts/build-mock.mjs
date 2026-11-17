// Builds the mock backend's data from the handoff prototype and the TS fixture.
//   inputs:  docs/handoff/prototype/code-map.html  (embedded <script id="data">: cards, modules, types)
//            fixtures/ts/lifecycle/lifecycle.ts       (source text)
//            fixtures/ts/lifecycle/expected/flows.json (flow units, generic golden)
//   output:  src/ipc/mock/data/mock.json
// Numbers that are u64 on the Rust side (bigint in the bindings) are written as plain numbers;
// the mock converts them when it serves a payload.
// Run with: npm run mock
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve, posix } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const ui = resolve(here, "..");
const repoRoot = resolve(ui, "../..");

// ---------- helpers ----------
const MASK = (1n << 64n) - 1n;
function hash64(s) {
  // FNV-1a 64, the same width as FileId / SymbolId on the wire.
  let h = 0xcbf29ce484222325n;
  for (const b of Buffer.from(s, "utf8")) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & MASK;
  }
  return h.toString(16).padStart(16, "0");
}
const symId = (path, name, kind) => hash64(`${path}\u0000${name}\u0000${kind}`);
const countLines = (text) => (text.endsWith("\n") ? text.slice(0, -1) : text).split("\n").length;

// ---------- the prototype file ----------
const html = readFileSync(resolve(repoRoot, "docs/handoff/prototype/code-map.html"), "utf8");
const m = html.match(/<script type="application\/json" id="data">([\s\S]*?)<\/script>/);
if (!m) throw new Error("no embedded data in code-map.html");
const D = JSON.parse(m[1].replace(/<\\\//g, "</"));
const source = readFileSync(resolve(repoRoot, "fixtures/ts/lifecycle/lifecycle.ts"), "utf8");
const flows = JSON.parse(readFileSync(resolve(repoRoot, "fixtures/ts/lifecycle/expected/flows.json"), "utf8"));
const srcLines = source.split("\n");

const LIFE = "src/agents/harness/lifecycle.ts";
const LIFE_DIR = posix.dirname(LIFE);

// Module colors (light theme), one per cluster. The UI lightens them for dark.
const MODULES = [
  { id: "agents-harness", name: "Agent harness", role: "Runs agent harnesses under diagnostics", dir: "src/agents/harness", color: "#2f6fb0", source: "ai" },
  { id: "agents-runner", name: "Embedded runner", role: "Runs agents in-process", dir: "src/agents/embedded-agent-runner", color: "#7a5bb5", source: "ai" },
  { id: "agents-core", name: "Agent core", role: "Shared agent outcomes and tools", dir: "src/agents", color: "#3d8a7a", source: "cluster" },
  { id: "infra-diagnostics", name: "Diagnostics", role: "Events, traces and content policy", dir: "src/infra", color: "#b5651d", source: "ai" },
  { id: "config", name: "Config", role: "Runtime configuration snapshots", dir: "src/config", color: "#5d7a2a", source: "cluster" },
  { id: "context-engine", name: "Context engine", role: "Context engine host checks", dir: "src/context-engine", color: "#a8467a", source: "ai" },
  { id: "gateway", name: "Gateway", role: "HTTP and WebSocket entry points", dir: "src/gateway", color: "#1f7f99", source: "cluster" },
  { id: "channels", name: "Channels", role: "Chat channel adapters", dir: "src/channels", color: "#9a5b2e", source: "cluster" },
  { id: "plugins", name: "Plugins", role: "Plugin loading and hooks", dir: "src/plugins", color: "#6b6f2a", source: "user" },
  { id: "cli", name: "CLI", role: "Command-line entry points", dir: "src/cli", color: "#80539a", source: "cluster" },
  { id: "relay", name: "Relay (Go)", role: "Event relay service", dir: "tools/relay", color: "#46708f", source: "cluster" },
  { id: "docs", name: "Docs", role: null, dir: "docs", color: "#7d7466", source: "cluster" },
];
const modOf = (path) => {
  let best = null;
  for (const mo of MODULES) if (path === mo.dir || path.startsWith(mo.dir + "/")) if (!best || mo.dir.length > best.dir.length) best = mo;
  return best;
};
const modRef = (path) => {
  const mo = modOf(path);
  return mo ? { id: mo.id, name: mo.name, color: mo.color } : null;
};

const LANGS = {
  ts: ["typescript", "TypeScript", "typescript", "full"],
  js: ["javascript", "JavaScript", "javascript", "full"],
  css: ["css", "CSS", "css", "full"],
  go: ["go", "Go", "go", "outline"],
  md: ["markdown", "Markdown", "markdown", "text"],
  json: ["json", "JSON", "json", "text"],
  yaml: ["yaml", "YAML", "yaml", "text"],
};
const langOf = (path) => {
  if (posix.basename(path) === "LICENSE") return ["other", "Plain text", "plaintext", "text"];
  return LANGS[posix.extname(path).slice(1)] ?? ["other", "Plain text", "plaintext", "text"];
};

// ---------- lifecycle.ts FileView ----------
const fns = D.functions;
const byName = Object.fromEntries(fns.map((f) => [f.name, f]));
const level = {};
const L = (n) => {
  if (n in level) return level[n];
  level[n] = 0;
  return (level[n] = byName[n].calls.length ? 1 + Math.max(...byName[n].calls.map(L)) : 0);
};
fns.forEach((f) => L(f.name));
const calledBy = Object.fromEntries(fns.map((f) => [f.name, []]));
fns.forEach((f) => f.calls.forEach((c) => calledBy[c].push(f.name)));
const maxLevel = Math.max(...fns.map((f) => level[f.name]));

function docOf(code) {
  const d = code.match(/^\s*\/\*\*([\s\S]*?)\*\//);
  if (!d) return null;
  return d[1].split("\n").map((l) => l.replace(/^\s*\*\s?/, "").trim()).filter(Boolean).join(" ");
}
function signatureOf(code) {
  const body = code.replace(/^\s*\/\*\*[\s\S]*?\*\/\s*/, "");
  let depth = 0;
  let out = "";
  for (const ch of body) {
    if (ch === "(" || ch === "<" || ch === "[") depth++;
    else if (ch === ")" || ch === ">" || ch === "]") depth--;
    if (ch === "{" && depth <= 0 && /\)\s*(:\s*[^{]*)?$/.test(out)) break;
    out += ch;
  }
  out = out.replace(/\s+/g, " ").trim();
  return out.length > 200 ? out.slice(0, 199) + "…" : out;
}
function importLine(spec) {
  const i = srcLines.findIndex((l) => l.includes(`from "${spec}"`));
  let j = i;
  while (j > 0 && !srcLines[j].startsWith("import")) j--;
  return j + 1;
}
function resolveSpec(spec) {
  if (!spec.startsWith(".")) return null;
  return posix.normalize(posix.join(LIFE_DIR, spec)).replace(/\.js$/, ".ts");
}

const EXTERNAL_CALLERS = { runAgentHarnessLifecycleAttempt: 3, runAgentHarnessLifecycleFinalization: 1 };
const cards = fns.map((f) => ({
  id: symId(LIFE, f.name, "function"),
  name: f.name,
  qualifiedName: f.name,
  kind: "function",
  start: f.start,
  end: f.end,
  exported: f.exported,
  isAsync: new RegExp(`async\\s+function\\s+${f.name}\\b`).test(f.code),
  signature: signatureOf(f.code),
  doc: docOf(f.code),
  level: level[f.name],
  calls: f.calls,
  calledBy: calledBy[f.name],
  uses: f.uses,
  typeUses: f.typeUses,
  externalCallers: EXTERNAL_CALLERS[f.name] ?? 0,
  summary: { desc: f.desc, algo: f.algo, notes: f.notes, model: "mock", ai: true },
  unit: flows[f.name] ? f.name : null,
}));

const harnessLine = srcLines.findIndex((l) => l.includes("harness.runAttempt")) + 1;
const importModules = D.modules.map((mo) =>
  mo.runtime
    ? { id: mo.path, kind: "static", resolved: null, external: false, symbols: mo.symbols.map((s) => ({ name: s.name, typeOnly: !!s.typeOnly, member: true })), line: harnessLine }
    : { id: mo.id, kind: "static", resolved: resolveSpec(mo.id), external: false, symbols: mo.symbols.map((s) => ({ name: s.name, typeOnly: !!s.typeOnly, member: false })), line: importLine(mo.id) },
);

const OVERVIEW =
  "Wraps every agent-harness execution in diagnostics. Each run gets an outer `harness.run` span and, for any harness other than `openclaw`, a child `run` span. Along the way it normalizes harness results to the canonical terminal-outcome contract and applies the harness's own classification. Prompt and answer text only leave through the private-data channel, and only when the content-capture policy allows it.";

const files = {};
files[LIFE] = {
  source,
  flows,
  view: {
    path: LIFE,
    name: posix.basename(LIFE),
    lang: "typescript",
    langLabel: "TypeScript",
    monacoLang: "typescript",
    tier: "full",
    lines: countLines(source),
    contentHash: hash64(source),
    layers: maxLevel + 1,
    functions: cards,
    types: D.types.map((t) => ({ id: symId(LIFE, t.name, "type"), name: t.name, kind: "type", start: t.start, end: t.end, exported: false, doc: null, summary: t.desc })),
    modules: importModules,
    importsRange: { start: D.imports.start, end: D.imports.end },
    outline: [],
    flowUnits: Object.values(flows).map((u) => ({ id: u.id, owner: u.owner, method: u.method, start: u.start, end: u.end })),
    importers: [
      { path: "src/agents/harness/index.ts", line: 4 },
      { path: "src/agents/embedded-agent-runner/runner.ts", line: 12 },
      { path: "src/gateway/agent-session.ts", line: 9 },
    ],
    overview: { text: OVERVIEW, model: "mock" },
    hasErrors: false,
    module: modRef(LIFE),
  },
};

// ---------- the files lifecycle.ts imports: stubs built from what it takes from them ----------
const importedFiles = {};
for (const mo of importModules) {
  if (!mo.resolved) continue;
  (importedFiles[mo.resolved] ??= []).push(...mo.symbols);
}
for (const [path, syms] of Object.entries(importedFiles)) {
  const lines = [`// ${posix.basename(path)}: mock stub generated from what lifecycle.ts imports.`, ""];
  const fnCards = [];
  const types = [];
  for (const s of syms) {
    const start = lines.length + 1;
    if (s.typeOnly) {
      lines.push(`export type ${s.name} = {`, `  id: string;`, `  createdAt: number;`, `};`, "");
      types.push({ id: symId(path, s.name, "type"), name: s.name, kind: "type", start, end: start + 3, exported: true, doc: null, summary: null });
    } else if (/^[A-Z]/.test(s.name)) {
      lines.push(`export class ${s.name} extends Error {`, `  constructor(readonly result: unknown) {`, `    super("${s.name}");`, `  }`, `}`, "");
      types.push({ id: symId(path, s.name, "class"), name: s.name, kind: "class", start, end: start + 4, exported: true, doc: null, summary: null });
    } else {
      lines.push(`export function ${s.name}(...args: unknown[]): unknown {`, `  return args[0];`, `}`, "");
      fnCards.push({
        id: symId(path, s.name, "function"),
        name: s.name,
        qualifiedName: s.name,
        kind: "function",
        start,
        end: start + 2,
        exported: true,
        isAsync: false,
        signature: `export function ${s.name}(...args: unknown[]): unknown`,
        doc: null,
        level: 0,
        calls: [],
        calledBy: [],
        uses: [],
        typeUses: [],
        externalCallers: 1,
        summary: null,
        unit: s.name,
      });
    }
  }
  const text = lines.join("\n");
  files[path] = {
    source: text,
    flows: Object.fromEntries(
      fnCards.map((c) => [c.name, { id: c.name, owner: c.name, method: null, start: c.start, end: c.end, async: false, params: ["...args"], tree: [{ t: "exit", kind: "return", label: "args[0]", line: c.start + 1, early: false }] }]),
    ),
    view: {
      path,
      name: posix.basename(path),
      lang: "typescript",
      langLabel: "TypeScript",
      monacoLang: "typescript",
      tier: "full",
      lines: countLines(text),
      contentHash: hash64(text),
      layers: fnCards.length ? 1 : 0,
      functions: fnCards,
      types,
      modules: [],
      importsRange: null,
      outline: [],
      flowUnits: fnCards.map((c) => ({ id: c.name, owner: c.name, method: null, start: c.start, end: c.end })),
      importers: [{ path: LIFE, line: importModules.find((x) => x.resolved === path)?.line ?? null }],
      overview: null,
      hasErrors: false,
      module: modRef(path),
    },
  };
}

// ---------- a Go file in the outline tier ----------
const GO = `// Package main runs the event relay: it reads diagnostic events from stdin
// and forwards them to an HTTP collector in batches.
package main

import (
	"bufio"
	"encoding/json"
	"log"
	"net/http"
	"os"
	"time"
)

// Event is one diagnostic event as written by the harness.
type Event struct {
	Type      string          \`json:"type"\`
	RunID     string          \`json:"runId"\`
	Timestamp int64           \`json:"ts"\`
	Payload   json.RawMessage \`json:"payload"\`
}

// Batch collects events until it is full or old enough to flush.
type Batch struct {
	events  []Event
	started time.Time
	limit   int
}

// NewBatch returns an empty batch that flushes at limit events.
func NewBatch(limit int) *Batch {
	return &Batch{limit: limit, started: time.Now()}
}

// Add appends an event and reports whether the batch is now full.
func (b *Batch) Add(e Event) bool {
	b.events = append(b.events, e)
	return len(b.events) >= b.limit
}

// Due reports whether the batch has waited longer than max.
func (b *Batch) Due(max time.Duration) bool {
	return len(b.events) > 0 && time.Since(b.started) > max
}

func post(client *http.Client, url string, events []Event) error {
	body, err := json.Marshal(events)
	if err != nil {
		return err
	}
	resp, err := client.Post(url, "application/json", bytesReader(body))
	if err != nil {
		return err
	}
	return resp.Body.Close()
}

func main() {
	url := os.Getenv("RELAY_URL")
	client := &http.Client{Timeout: 5 * time.Second}
	batch := NewBatch(100)
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		var e Event
		if err := json.Unmarshal(scanner.Bytes(), &e); err != nil {
			log.Printf("skip: %v", err)
			continue
		}
		if batch.Add(e) || batch.Due(2*time.Second) {
			if err := post(client, url, batch.events); err != nil {
				log.Printf("post: %v", err)
			}
			batch = NewBatch(100)
		}
	}
}
`;
const goLines = GO.split("\n");
const goOutline = [];
goLines.forEach((l, i) => {
  let mm;
  if ((mm = l.match(/^type (\w+) struct/))) {
    let e = i;
    while (!goLines[e].startsWith("}")) e++;
    goOutline.push({ name: mm[1], kind: "struct", start: i + 1, end: e + 1, depth: 0 });
  } else if ((mm = l.match(/^func (\([^)]*\) )?(\w+)/))) {
    let e = i;
    while (!goLines[e].startsWith("}")) e++;
    const recv = mm[1] ? mm[1].match(/\*?(\w+)\)/)[1] : null;
    goOutline.push({ name: recv ? `${recv}.${mm[2]}` : mm[2], kind: recv ? "method" : "function", start: i + 1, end: e + 1, depth: recv ? 1 : 0 });
  }
});
const GO_PATH = "tools/relay/main.go";
const goImports = ["bufio", "encoding/json", "log", "net/http", "os", "time"];
files[GO_PATH] = {
  source: GO,
  flows: {},
  view: {
    path: GO_PATH,
    name: "main.go",
    lang: "go",
    langLabel: "Go",
    monacoLang: "go",
    tier: "outline",
    lines: countLines(GO),
    contentHash: hash64(GO),
    layers: 0,
    functions: [],
    types: [],
    modules: goImports.map((id) => ({ id, kind: "static", resolved: null, external: true, symbols: [], line: goLines.findIndex((l) => l.includes(`"${id}"`)) + 1 })),
    importsRange: { start: 5, end: 12 },
    outline: goOutline,
    flowUnits: [],
    importers: [],
    overview: { text: "Reads diagnostic events from stdin and forwards them to an HTTP collector in batches of up to 100, or every 2 seconds.", model: "mock" },
    hasErrors: false,
    module: modRef(GO_PATH),
  },
};

// ---------- a Markdown file in the text tier ----------
const MD = `# openclaw

An agent runtime with pluggable harnesses, chat channels and a diagnostics pipeline.

## Layout

- \`src/agents\`: harnesses, the embedded runner and agent tools.
- \`src/infra\`: diagnostic events, trace contexts and content policy.
- \`src/gateway\`: the HTTP and WebSocket entry points.
- \`src/channels\`: adapters for Slack, Discord and the terminal.
- \`tools/relay\`: a small Go service that forwards diagnostic events.

## Diagnostics

Every harness run is wrapped by \`runAgentHarnessLifecycleAttempt\`, which emits
\`harness.run.started\`, \`run.started\`, \`run.completed\` and \`harness.run.completed\`.
Prompt and answer text are only attached when the content-capture policy allows it.

## Development

\`\`\`sh
pnpm install
pnpm test
\`\`\`
`;
files["README.md"] = {
  source: MD,
  flows: {},
  view: {
    path: "README.md",
    name: "README.md",
    lang: "markdown",
    langLabel: "Markdown",
    monacoLang: "markdown",
    tier: "text",
    lines: countLines(MD),
    contentHash: hash64(MD),
    layers: 0,
    functions: [],
    types: [],
    modules: [],
    importsRange: null,
    outline: [],
    flowUnits: [],
    importers: [],
    overview: null,
    hasErrors: false,
    module: null,
  },
};

// ---------- the fake repo tree ----------
const OTHER_FILES = [
  ["src/agents/harness/index.ts", 38],
  ["src/agents/harness/registry.ts", 142],
  ["src/agents/harness/openclaw-harness.ts", 311],
  ["src/agents/embedded-agent-runner/runner.ts", 412],
  ["src/agents/embedded-agent-runner/run/attempt.ts", 268],
  ["src/agents/tools/index.ts", 54],
  ["src/agents/tools/web-search.ts", 187],
  ["src/infra/diagnostic-sinks.ts", 133],
  ["src/infra/logger.ts", 96],
  ["src/config/schema.ts", 520],
  ["src/config/defaults.ts", 88],
  ["src/context-engine/registry.ts", 176],
  ["src/context-engine/legacy.ts", 205],
  ["src/gateway/server.ts", 344],
  ["src/gateway/agent-session.ts", 298],
  ["src/gateway/ws.ts", 151],
  ["src/channels/slack/adapter.ts", 402],
  ["src/channels/slack/format.ts", 120],
  ["src/channels/discord/adapter.ts", 377],
  ["src/channels/terminal/adapter.ts", 140],
  ["src/plugins/loader.ts", 233],
  ["src/plugins/hooks.ts", 167],
  ["src/cli/run.ts", 189],
  ["src/cli/index.ts", 61],
  ["src/ui/panel.css", 210],
  ["docs/architecture.md", 140],
  ["docs/diagnostics.md", 96],
  ["package.json", 64],
  ["tsconfig.json", 28],
  [".github/workflows/ci.yaml", 52],
  ["LICENSE", 21],
];
const PENDING = new Set(["src/channels/discord/adapter.ts", "src/plugins/hooks.ts", "src/cli/run.ts"]);
const FAILED = new Set(["src/ui/panel.css"]);
const allFiles = [
  ...Object.keys(files).map((p) => [p, files[p].view.lines]),
  ...OTHER_FILES.filter(([p]) => !files[p]),
];

function buildTree(entries) {
  const root = { name: "openclaw", path: "", isDir: true, lang: null, tier: null, module: null, color: null, status: "indexed", lines: 0, children: [] };
  for (const [path, lines] of entries) {
    const parts = path.split("/");
    let node = root;
    for (let i = 0; i < parts.length; i++) {
      const p = parts.slice(0, i + 1).join("/");
      const isLeaf = i === parts.length - 1;
      let child = node.children.find((c) => c.name === parts[i]);
      if (!child) {
        if (isLeaf) {
          const [lang, , , tier] = langOf(p);
          const mo = modOf(p);
          child = { name: parts[i], path: p, isDir: false, lang, tier, module: mo?.id ?? null, color: mo?.color ?? null, status: FAILED.has(p) ? "failed" : PENDING.has(p) ? "pending" : "indexed", lines, children: [] };
        } else {
          const mo = MODULES.find((x) => x.dir === p);
          child = { name: parts[i], path: p, isDir: true, lang: null, tier: null, module: mo?.id ?? null, color: mo?.color ?? null, status: "indexed", lines: 0, children: [] };
        }
        node.children.push(child);
      }
      node = child;
    }
  }
  const finish = (n) => {
    n.children.sort((a, b) => (a.isDir === b.isDir ? a.name.localeCompare(b.name) : a.isDir ? -1 : 1));
    n.children.forEach(finish);
    if (n.isDir) {
      n.lines = n.children.reduce((s, c) => s + c.lines, 0);
      n.status = n.children.some((c) => c.status === "pending") ? "pending" : "indexed";
    }
  };
  finish(root);
  return root;
}
const tree = buildTree(allFiles);

// ---------- repo summary ----------
const leaves = [];
const walk = (n) => (n.isDir ? n.children.forEach(walk) : leaves.push(n));
walk(tree);
const tiers = { full: 0, outline: 0, text: 0 };
const langCounts = {};
for (const f of leaves) {
  tiers[f.tier]++;
  const [lang, label] = langOf(f.path);
  const c = (langCounts[lang] ??= { lang, label, files: 0, lines: 0 });
  c.files++;
  c.lines += f.lines;
}
const repo = {
  root: "~/code/openclaw",
  name: "openclaw",
  files: leaves.length,
  lines: leaves.reduce((s, f) => s + f.lines, 0),
  tiers,
  languages: Object.values(langCounts).sort((a, b) => b.lines - a.lines),
  indexed: leaves.filter((f) => f.status === "indexed").length,
  rootHash: hash64(leaves.map((f) => f.path).join("\n")),
  aiEnabled: false,
};

// ---------- module map ----------
const modStats = Object.fromEntries(MODULES.map((mo) => [mo.id, { files: 0, lines: 0 }]));
for (const f of leaves) if (f.module) { modStats[f.module].files++; modStats[f.module].lines += f.lines; }
const moduleMap = {
  modules: MODULES.map((mo) => ({ id: mo.id, name: mo.name, role: mo.role, color: mo.color, source: mo.source, dir: mo.dir, files: modStats[mo.id].files, lines: modStats[mo.id].lines })),
  edges: [
    ["cli", "gateway", 3], ["cli", "config", 2], ["gateway", "agents-harness", 4], ["gateway", "channels", 5], ["gateway", "config", 2],
    ["channels", "agents-runner", 3], ["channels", "infra-diagnostics", 2], ["agents-runner", "agents-harness", 6], ["agents-runner", "agents-core", 4],
    ["agents-harness", "agents-core", 5], ["agents-harness", "infra-diagnostics", 9], ["agents-harness", "config", 1], ["agents-harness", "context-engine", 1],
    ["agents-core", "infra-diagnostics", 3], ["context-engine", "config", 2], ["plugins", "agents-harness", 2], ["agents-harness", "plugins", 1],
    ["plugins", "config", 1], ["infra-diagnostics", "config", 2], ["relay", "infra-diagnostics", 1],
  ].map(([from, to, weight]) => ({ from, to, weight })),
  cycles: [["agents-harness", "plugins"]],
};

const out = {
  generatedFrom: ["docs/handoff/prototype/code-map.html", "fixtures/ts/lifecycle/lifecycle.ts", "fixtures/ts/lifecycle/expected/flows.json"],
  repo,
  recent: ["~/code/openclaw", "~/code/isoline", "~/work/payments-api"],
  tree,
  moduleMap,
  files,
};
const outPath = resolve(ui, "src/ipc/mock/data/mock.json");
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, JSON.stringify(out, null, 1) + "\n");
console.log(`mock: ${Object.keys(files).length} detailed files, ${leaves.length} files in tree, ${cards.length} cards in ${LIFE}`);
