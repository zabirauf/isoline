// Side-by-side diff of two versions of a file, read-only, for the compare view. Same fonts, theme
// and quiet chrome as EditorHost; Monaco's own diff computation runs in the editor worker.
import { monaco } from "./monaco";
import { effectiveTheme, onThemeChange } from "../lib/theme";

export interface DiffHostOptions {
  container: HTMLElement;
  before: string;
  after: string;
  language: string;
  ariaLabel: string;
  /** Collapse unchanged stretches to a few lines of context. */
  changesOnly: boolean;
}

let seq = 0;
const strip = (t: string) => (t.endsWith("\n") ? t.slice(0, -1) : t);

export class DiffHost {
  private editor: monaco.editor.IStandaloneDiffEditor;
  private original: monaco.editor.ITextModel;
  private modified: monaco.editor.ITextModel;
  private deco: { o: monaco.editor.IEditorDecorationsCollection; m: monaco.editor.IEditorDecorationsCollection };
  private offTheme: () => void;

  constructor(opts: DiffHostOptions) {
    const n = ++seq;
    this.original = monaco.editor.createModel(strip(opts.before), opts.language, monaco.Uri.from({ scheme: "isoline-diff", path: `/base/${n}` }));
    this.modified = monaco.editor.createModel(strip(opts.after), opts.language, monaco.Uri.from({ scheme: "isoline-diff", path: `/head/${n}` }));
    for (const m of [this.original, this.modified]) m.updateOptions({ bracketColorizationOptions: { enabled: false, independentColorPoolPerBracketType: false } });
    this.editor = monaco.editor.createDiffEditor(opts.container, {
      readOnly: true,
      domReadOnly: true,
      originalEditable: false,
      ariaLabel: opts.ariaLabel,
      theme: effectiveTheme() === "dark" ? "isoline-dark" : "isoline-light",
      fontFamily: '"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      fontSize: 12,
      lineHeight: 19,
      fontLigatures: false,
      renderSideBySide: true,
      useInlineViewWhenSpaceIsLimited: true,
      renderSideBySideInlineBreakpoint: 760,
      renderIndicators: true,
      renderMarginRevertIcon: false,
      enableSplitViewResizing: true,
      ignoreTrimWhitespace: false,
      hideUnchangedRegions: { enabled: opts.changesOnly, contextLineCount: 3, minimumLineCount: 4, revealLineCount: 20 },
      lineNumbersMinChars: 4,
      glyphMargin: false,
      folding: false,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      renderLineHighlight: "none",
      renderOverviewRuler: true,
      overviewRulerBorder: false,
      occurrencesHighlight: "off",
      selectionHighlight: false,
      matchBrackets: "never",
      bracketPairColorization: { enabled: false },
      guides: { indentation: false, bracketPairs: false, bracketPairsHorizontal: false, highlightActiveBracketPair: false },
      stickyScroll: { enabled: false },
      contextmenu: false,
      links: false,
      hover: { enabled: "off" },
      automaticLayout: true,
      fixedOverflowWidgets: true,
      padding: { top: 6, bottom: 6 },
      unicodeHighlight: { ambiguousCharacters: false, invisibleCharacters: false },
      scrollbar: { verticalScrollbarSize: 10, horizontalScrollbarSize: 10, useShadows: false },
    });
    this.editor.setModel({ original: this.original, modified: this.modified });
    this.deco = { o: this.editor.getOriginalEditor().createDecorationsCollection(), m: this.editor.getModifiedEditor().createDecorationsCollection() };
    this.offTheme = onThemeChange((t) => monaco.editor.setTheme(t === "dark" ? "isoline-dark" : "isoline-light"));
  }

  setChangesOnly(on: boolean) {
    this.editor.updateOptions({ hideUnchangedRegions: { enabled: on, contextLineCount: 3, minimumLineCount: 4, revealLineCount: 20 } });
  }

  /** Scroll a symbol's lines into view on one side and mark them with the selection bar. */
  reveal(side: "base" | "head", start: number, end: number) {
    const ed = side === "base" ? this.editor.getOriginalEditor() : this.editor.getModifiedEditor();
    const other = side === "base" ? this.deco.m : this.deco.o;
    const mine = side === "base" ? this.deco.o : this.deco.m;
    other.clear();
    mine.set([{ range: new monaco.Range(start, 1, end, 1), options: { isWholeLine: true, className: "mx-sym-range", linesDecorationsClassName: "mx-sym-bar" } }]);
    ed.revealLinesNearTop(start, end > start + 40 ? start + 40 : end, monaco.editor.ScrollType.Smooth);
  }

  /** Changed stretches, for "n changes" and next/previous. */
  changeCount(): number {
    return this.editor.getLineChanges()?.length ?? 0;
  }

  onReady(f: () => void): () => void {
    const d = this.editor.onDidUpdateDiff(f);
    return () => d.dispose();
  }

  dispose() {
    this.offTheme();
    this.editor.dispose();
    this.original.dispose();
    this.modified.dispose();
  }
}
