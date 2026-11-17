// EditorHost: the one wrapper around Monaco. Every code view in the app goes through it.
// It starts read-only. Later features (editing and saving, diff review, inline AI notes,
// language servers) attach here and nowhere else.
//
// What it adds on top of Monaco:
// - real line numbers for slices (a card's code starts at its own first line, never at 1);
// - the current call-flow step line: `user-soft` fill and a 3px `user` bar;
// - links on names the Isoline index knows: a card in this file is a solid `accent` link, an
//   imported name a dotted `ext` link. Click jumps to the card or focuses the symbol;
// - hover and go to definition answered from the FileView, not from a language service;
// - sizing to content for card code, or filling its container for whole-file views.
import { monaco } from "./monaco";
import { effectiveTheme, onThemeChange } from "../lib/theme";
import { lineRange } from "../lib/format";

export interface CardRef {
  name: string;
  start: number;
  end: number;
  desc: string | null;
}
export interface SymRefInfo {
  name: string;
  label: string;
  from: string;
  /** Reached through a parameter (`harness.classify`) rather than imported by name. */
  member: boolean;
  receiver: string | null;
  typeOnly: boolean;
}
export interface TypeRefInfo {
  name: string;
  start: number;
  end: number;
  desc: string | null;
}

/** What the index knows about the file shown, and what to do when the reader follows a name. */
export interface CodeContext {
  path: string;
  cards: Map<string, CardRef>;
  syms: Map<string, SymRefInfo>;
  types: Map<string, TypeRefInfo>;
  /** Outline symbols for tier-2 files: go to definition stays inside the file. */
  outline?: Map<string, { start: number; end: number; kind: string }>;
  /** The card whose code this is: its name is set bold, not linked. */
  self?: string;
  onJumpToCard?(name: string): void;
  onSymbol?(name: string): void;
}

export interface EditorHostOptions {
  container: HTMLElement;
  text: string;
  language: string;
  /** File line number of the first line of `text`. */
  firstLine: number;
  /** true: grow to the content's height (card code). false: fill the container (whole file). */
  fit: boolean;
  context: CodeContext;
  ariaLabel: string;
}

type LinkKind = "fn" | "ext" | "type" | "def" | "outline";
interface Link {
  line: number;
  startCol: number;
  endCol: number;
  kind: LinkKind;
  name: string;
}

const LINE_HEIGHT = 19;
const CARD_SCHEME = "isoline-card";
const hosts = new Map<string, EditorHost>();
const providersFor = new Set<string>();
let modelSeq = 0;
let openerRegistered = false;

let fontsReady: Promise<void> | null = null;
function waitForFonts(): Promise<void> {
  if (!fontsReady) {
    fontsReady = (async () => {
      try {
        await Promise.all([document.fonts.load('12px "IBM Plex Mono"'), document.fonts.load('italic 12px "IBM Plex Mono"'), document.fonts.load('600 12px "IBM Plex Mono"')]);
      } catch {
        /* fall back to the system mono */
      }
      monaco.editor.remeasureFonts();
    })();
  }
  return fontsReady;
}

function setGlobalTheme() {
  monaco.editor.setTheme(effectiveTheme() === "dark" ? "isoline-dark" : "isoline-light");
}
setGlobalTheme();
onThemeChange(setGlobalTheme);

function registerOpener() {
  if (openerRegistered) return;
  openerRegistered = true;
  monaco.editor.registerEditorOpener({
    openCodeEditor(source, resource) {
      if (resource.scheme !== CARD_SCHEME) return false;
      const host = hosts.get(source.getModel()?.uri.toString() ?? "");
      const name = decodeURIComponent(resource.path.replace(/^\//, ""));
      host?.context.onJumpToCard?.(name);
      return true;
    },
  });
}

/** Tiny read-only models that stand for a card, so "peek definition" shows its summary. */
function cardModel(c: CardRef): monaco.Uri {
  const uri = monaco.Uri.from({ scheme: CARD_SCHEME, path: "/" + encodeURIComponent(c.name) });
  if (!monaco.editor.getModel(uri)) monaco.editor.createModel(`${c.name}  ${lineRange(c.start, c.end)}\n${c.desc ?? ""}`, "plaintext", uri);
  return uri;
}

function registerProviders(language: string) {
  if (providersFor.has(language)) return;
  providersFor.add(language);
  monaco.languages.registerHoverProvider(language, {
    provideHover(model, pos) {
      const host = hosts.get(model.uri.toString());
      const link = host?.linkAt(pos.lineNumber, pos.column);
      if (!host || !link) return null;
      const range = new monaco.Range(link.line, link.startCol, link.line, link.endCol);
      const ctx = host.context;
      let md: string | null = null;
      if (link.kind === "fn" || link.kind === "def") {
        const c = ctx.cards.get(link.name)!;
        md = `**\`${c.name}\`** · ${lineRange(c.start, c.end)}\n\n${c.desc ?? "No summary yet."}${link.kind === "fn" ? "\n\nClick to open its card." : ""}`;
      } else if (link.kind === "ext") {
        const s = ctx.syms.get(link.name)!;
        md = `**\`${s.label}\`**${s.typeOnly ? " (type)" : ""}\n\n${s.member ? `Reached through the \`${s.receiver}\` parameter.` : `Imported from \`${s.from}\`.`}`;
      } else if (link.kind === "type") {
        const t = ctx.types.get(link.name)!;
        md = `**\`${t.name}\`** · local type · ${lineRange(t.start, t.end)}${t.desc ? `\n\n${t.desc}` : ""}`;
      } else if (link.kind === "outline") {
        const o = ctx.outline!.get(link.name)!;
        md = `**\`${link.name}\`** · ${o.kind} · ${lineRange(o.start, o.end)}`;
      }
      return md ? { range, contents: [{ value: md }] } : null;
    },
  });
  monaco.languages.registerDefinitionProvider(language, {
    provideDefinition(model, pos) {
      const host = hosts.get(model.uri.toString());
      const link = host?.linkAt(pos.lineNumber, pos.column);
      if (!host || !link) return null;
      const ctx = host.context;
      const inFile = (start: number) => {
        const l = start - host.firstLine + 1;
        return l >= 1 && l <= model.getLineCount() ? { uri: model.uri, range: new monaco.Range(l, 1, l, 1) } : null;
      };
      if (link.kind === "fn" || link.kind === "def") {
        const c = ctx.cards.get(link.name)!;
        return host.fit ? { uri: cardModel(c), range: new monaco.Range(1, 1, 1, 1) } : (inFile(c.start) ?? { uri: cardModel(c), range: new monaco.Range(1, 1, 1, 1) });
      }
      if (link.kind === "type") return inFile(ctx.types.get(link.name)!.start);
      if (link.kind === "outline") return inFile(ctx.outline!.get(link.name)!.start);
      return null;
    },
  });
}

const IDENT = /^(identifier|type\.identifier|type)(\.|$)/;

export class EditorHost {
  readonly editor: monaco.editor.IStandaloneCodeEditor;
  readonly fit: boolean;
  firstLine: number;
  context: CodeContext;
  private model: monaco.editor.ITextModel;
  private links: Link[][] = [];
  private stepDeco: monaco.editor.IEditorDecorationsCollection;
  private linkDeco: monaco.editor.IEditorDecorationsCollection;
  private heightListeners = new Set<(h: number) => void>();
  private disposed = false;
  private disposables: monaco.IDisposable[] = [];

  /** Resolves once fonts are measured; create hosts after this so widths are right. */
  static ready(): Promise<void> {
    return waitForFonts();
  }

  constructor(opts: EditorHostOptions) {
    registerOpener();
    registerProviders(opts.language);
    this.fit = opts.fit;
    this.firstLine = opts.firstLine;
    this.context = opts.context;
    const uri = monaco.Uri.from({ scheme: "isoline", path: "/" + opts.context.path, query: `n=${++modelSeq}` });
    // A trailing newline ends the last line; it does not start another (the backend counts lines that way).
    this.model = monaco.editor.createModel(opts.text.endsWith("\n") ? opts.text.slice(0, -1) : opts.text, opts.language, uri);
    hosts.set(uri.toString(), this);
    this.editor = monaco.editor.create(opts.container, {
      model: this.model,
      readOnly: true,
      domReadOnly: true,
      ariaLabel: opts.ariaLabel,
      theme: effectiveTheme() === "dark" ? "isoline-dark" : "isoline-light",
      fontFamily: '"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      fontSize: 12,
      lineHeight: LINE_HEIGHT,
      fontLigatures: false,
      lineNumbers: (n) => String(n + this.firstLine - 1),
      lineNumbersMinChars: 4,
      lineDecorationsWidth: 12,
      glyphMargin: false,
      folding: !opts.fit,
      showFoldingControls: "mouseover",
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      renderLineHighlight: "none",
      overviewRulerLanes: 0,
      overviewRulerBorder: false,
      hideCursorInOverviewRuler: true,
      occurrencesHighlight: "off",
      selectionHighlight: false,
      matchBrackets: "never",
      // Brackets stay in ink, as in the prototype; color is reserved for meaning.
      bracketPairColorization: { enabled: false },
      guides: { indentation: false, bracketPairs: false, bracketPairsHorizontal: false, highlightActiveBracketPair: false },
      stickyScroll: { enabled: false },
      contextmenu: false,
      links: false,
      wordWrap: "off",
      automaticLayout: true,
      fixedOverflowWidgets: true,
      cursorBlinking: "solid",
      padding: { top: 8, bottom: 8 },
      unicodeHighlight: { ambiguousCharacters: false, invisibleCharacters: false },
      scrollbar: opts.fit
        ? { vertical: "hidden", horizontal: "auto", horizontalScrollbarSize: 8, alwaysConsumeMouseWheel: false, useShadows: false }
        : { verticalScrollbarSize: 10, horizontalScrollbarSize: 10, useShadows: false },
    });
    this.stepDeco = this.editor.createDecorationsCollection();
    this.linkDeco = this.editor.createDecorationsCollection();
    this.disposables.push(
      this.editor.onDidContentSizeChange(() => this.emitHeight()),
      this.editor.onDidLayoutChange(() => this.emitHeight()),
      this.editor.onMouseDown((e) => {
        if (e.event.rightButton || !e.target.position) return;
        if (e.target.type !== monaco.editor.MouseTargetType.CONTENT_TEXT) return;
        const link = this.linkAt(e.target.position.lineNumber, e.target.position.column);
        if (!link) return;
        if (link.kind === "fn") this.context.onJumpToCard?.(link.name);
        else if (link.kind === "ext") this.context.onSymbol?.(link.name);
        else if (link.kind === "type" || link.kind === "outline") {
          const start = link.kind === "type" ? this.context.types.get(link.name)!.start : this.context.outline!.get(link.name)!.start;
          this.revealLine(start);
        }
      }),
    );
    void this.computeLinks();
  }

  get height(): number {
    const info = this.editor.getLayoutInfo();
    const hScroll = this.editor.getScrollWidth() > info.width ? 8 : 0;
    return this.editor.getContentHeight() + hScroll;
  }

  private emitHeight() {
    if (this.disposed) return;
    const h = this.height;
    this.heightListeners.forEach((f) => f(h));
  }

  /** Called with the content height whenever it changes (fit mode sizes its container with it). */
  onHeight(f: (h: number) => void): () => void {
    this.heightListeners.add(f);
    f(this.height);
    return () => this.heightListeners.delete(f);
  }

  setContext(ctx: CodeContext) {
    this.context = ctx;
    void this.computeLinks();
  }

  /** Marks the current call-flow step's line (a file line number), or clears it with null. */
  setStepLine(line: number | null) {
    if (line === null) {
      this.stepDeco.clear();
      return;
    }
    const l = line - this.firstLine + 1;
    if (l < 1 || l > this.model.getLineCount()) {
      this.stepDeco.clear();
      return;
    }
    this.stepDeco.set([
      {
        range: new monaco.Range(l, 1, l, 1),
        options: { isWholeLine: true, className: "mx-step-line", marginClassName: "mx-step-margin" },
      },
    ]);
  }

  /** Scrolls a file line into the middle of the editor (fill mode). */
  revealLine(line: number) {
    const l = line - this.firstLine + 1;
    if (l < 1 || l > this.model.getLineCount()) return;
    this.editor.revealLineInCenter(l, monaco.editor.ScrollType.Smooth);
  }

  /** Pixel offset of a file line from the top of the editor (fit mode scrolls the page with it). */
  lineTop(line: number): number {
    return this.editor.getTopForLineNumber(line - this.firstLine + 1);
  }

  linkAt(line: number, col: number): Link | null {
    const row = this.links[line];
    if (!row) return null;
    return row.find((l) => col >= l.startCol && col < l.endCol) ?? null;
  }

  private async computeLinks() {
    const lang = this.model.getLanguageId();
    // colorize waits for the lazily loaded grammar; tokenize is synchronous once it is there.
    await monaco.editor.colorize("", lang, {});
    if (this.disposed) return;
    const text = this.model.getValue();
    const lines = text.split("\n");
    const tokens = monaco.editor.tokenize(text, lang);
    const ctx = this.context;
    const links: Link[][] = [];
    const decos: monaco.editor.IModelDeltaDecoration[] = [];
    tokens.forEach((row, i) => {
      const src = lines[i] ?? "";
      row.forEach((tok, j) => {
        if (!IDENT.test(tok.type) && !(tok.type === "" && /[A-Za-z_$]/.test(src[tok.offset] ?? ""))) return;
        const end = j + 1 < row.length ? row[j + 1].offset : src.length;
        const word = src.slice(tok.offset, end);
        const m = word.match(/^[A-Za-z_$][\w$]*/);
        if (!m) return;
        const name = m[0];
        let k = tok.offset - 1;
        while (k >= 0 && src[k] === " ") k--;
        const afterDot = src[k] === ".";
        let receiver = "";
        if (afterDot) {
          const r = src.slice(0, k).match(/([A-Za-z_$][\w$]*)\s*$/);
          receiver = r ? r[1] : "";
        }
        let kind: LinkKind | null = null;
        if (!afterDot && name === ctx.self && ctx.cards.has(name)) kind = "def";
        else if (!afterDot && ctx.cards.has(name)) kind = "fn";
        else if (ctx.syms.has(name)) {
          const s = ctx.syms.get(name)!;
          if (s.member ? afterDot && receiver === s.receiver : !afterDot) kind = "ext";
        } else if (!afterDot && ctx.types.has(name)) kind = "type";
        else if (!afterDot && ctx.outline?.has(name)) kind = "outline";
        if (!kind) return;
        const link: Link = { line: i + 1, startCol: tok.offset + 1, endCol: tok.offset + 1 + name.length, kind, name };
        (links[i + 1] ??= []).push(link);
        const cls = kind === "fn" ? "mx-fn" : kind === "ext" ? "mx-ext" : kind === "def" ? "mx-def" : "mx-type";
        decos.push({ range: new monaco.Range(link.line, link.startCol, link.line, link.endCol), options: { inlineClassName: cls } });
      });
    });
    this.links = links;
    this.linkDeco.set(decos);
  }

  dispose() {
    this.disposed = true;
    hosts.delete(this.model.uri.toString());
    this.disposables.forEach((d) => d.dispose());
    this.heightListeners.clear();
    this.editor.dispose();
    this.model.dispose();
  }
}
