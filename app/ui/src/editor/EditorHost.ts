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
  /**
   * Where every name used in the file is defined (FileView.targets): any name with a target
   * that is not a built-in becomes a link, colored by origin.
   */
  targets?: Map<string, { origin: "local" | "imported" | "external" | "builtin"; kind: string | null }>;
  /** Go to definition (click, F12). Falls back to onJumpToCard for functions in this file. */
  onGoTo?(name: string): void;
  onJumpToCard?(name: string): void;
  onSymbol?(name: string): void;
  /** The pointer rests on a link (rect in viewport coordinates), or left it (null). Drives the peek. */
  onHoverName?(name: string | null, rect: DOMRect | null): void;
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
/** How far the pointer may move between press and release for a click to count as a jump. */
const CLICK_SLOP = 4;
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
      if (host?.context.onGoTo) host.context.onGoTo(name);
      else host?.context.onJumpToCard?.(name);
      return true;
    },
  });
}

/** Tiny read-only models that stand for a definition elsewhere; the opener turns them into go to definition. */
function nameModel(name: string, text: string): monaco.Uri {
  const uri = monaco.Uri.from({ scheme: CARD_SCHEME, path: "/" + encodeURIComponent(name) });
  if (!monaco.editor.getModel(uri)) monaco.editor.createModel(text, "plaintext", uri);
  return uri;
}

function registerProviders(language: string) {
  if (providersFor.has(language)) return;
  providersFor.add(language);
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
      if (link.kind === "outline") return inFile(ctx.outline!.get(link.name)!.start);
      // Everything else goes through the app's go to definition, which knows cards and files.
      const c = ctx.cards.get(link.name);
      return { uri: nameModel(link.name, c ? `${c.name}  ${lineRange(c.start, c.end)}\n${c.desc ?? ""}` : link.name), range: new monaco.Range(1, 1, 1, 1) };
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
  private press: { x: number; y: number } | null = null;
  private hoverKey = "";

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
    this.model.updateOptions({ bracketColorizationOptions: { enabled: false, independentColorPoolPerBracketType: false } });
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
      // One peek for cards and code: Monaco's hover is off and the iso-peek is driven from here.
      hover: { enabled: "off" },
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
        this.press = e.event.rightButton ? null : { x: e.event.posx, y: e.event.posy };
      }),
      // Jump on mouse-up, and only when the pointer barely moved, so drag-select still works.
      this.editor.onMouseUp((e) => {
        const press = this.press;
        this.press = null;
        if (!press || !e.target.position) return;
        if (Math.abs(e.event.posx - press.x) > CLICK_SLOP || Math.abs(e.event.posy - press.y) > CLICK_SLOP) return;
        if (e.target.type !== monaco.editor.MouseTargetType.CONTENT_TEXT) return;
        const link = this.linkAt(e.target.position.lineNumber, e.target.position.column);
        if (!link || link.kind === "def") return;
        this.follow(link);
      }),
      this.editor.onMouseMove((e) => {
        const pos = e.target.position;
        const link = pos && e.target.type === monaco.editor.MouseTargetType.CONTENT_TEXT ? this.linkAt(pos.lineNumber, pos.column) : null;
        const key = link ? `${link.line}:${link.startCol}` : "";
        if (key === this.hoverKey) return;
        this.hoverKey = key;
        if (!link) {
          this.context.onHoverName?.(null, null);
          return;
        }
        this.context.onHoverName?.(link.name, this.linkRect(link));
      }),
      this.editor.onMouseLeave(() => {
        this.hoverKey = "";
        this.context.onHoverName?.(null, null);
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

  /** Viewport rectangle of a link's text, to anchor the peek under it. */
  private linkRect(link: Link): DOMRect {
    const box = this.editor.getDomNode()!.getBoundingClientRect();
    const a = this.editor.getScrolledVisiblePosition({ lineNumber: link.line, column: link.startCol });
    const b = this.editor.getScrolledVisiblePosition({ lineNumber: link.line, column: link.endCol });
    if (!a || !b) return box;
    return new DOMRect(box.left + a.left, box.top + a.top, Math.max(4, b.left - a.left), a.height);
  }

  /** Go to definition for a link: the app decides (card, local type, other file, peek). */
  private follow(link: Link) {
    const ctx = this.context;
    if (link.kind === "outline") {
      this.revealLine(ctx.outline!.get(link.name)!.start);
      return;
    }
    if (ctx.onGoTo) {
      ctx.onGoTo(link.name);
      return;
    }
    if (link.kind === "fn") ctx.onJumpToCard?.(link.name);
    else if (link.kind === "ext") ctx.onSymbol?.(link.name);
    else if (link.kind === "type") {
      const t = ctx.types.get(link.name);
      if (t) this.revealLine(t.start);
    }
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
        else if (!afterDot && ctx.targets?.has(name)) {
          const t = ctx.targets.get(name)!;
          if (t.origin === "local") kind = t.kind === "function" ? "fn" : "type";
          else if (t.origin !== "builtin") kind = "ext";
        }
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
