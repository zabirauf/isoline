// Monaco setup, imported only by EditorHost. Loads the editor core, the few editor features
// Isoline uses (hover, go to definition, find, folding, clipboard) and the Monarch grammars for
// highlighting. The language services (TypeScript worker and friends) are deliberately left out:
// definitions and hovers come from the Isoline index, never from Monaco's own analysis.
import * as monaco from "monaco-editor/editor/editor.api.js";
import "monaco-editor/features/codicon/register.js";
import "monaco-editor/features/hover/register.js";
import "monaco-editor/features/gotoSymbol/register.js";
import "monaco-editor/features/find/register.js";
import "monaco-editor/features/folding/register.js";
import "monaco-editor/features/clipboard/register.js";
import "monaco-editor/features/bracketMatching/register.js";
import "monaco-editor/languages/definitions/register.all.js";
import EditorWorker from "monaco-editor/editor/editor.worker.js?worker";
import { DARK, LIGHT, type ThemeColors } from "../styles/tokens.gen";

declare global {
  interface Window {
    MonacoEnvironment?: monaco.Environment;
  }
}

// The worker is bundled by Vite (`?worker`), so nothing is fetched from a CDN.
self.MonacoEnvironment = { getWorker: () => new EditorWorker() };

const hex = (c: string) => c.replace("#", "");

function theme(c: ThemeColors, base: "vs" | "vs-dark"): monaco.editor.IStandaloneThemeData {
  return {
    base,
    // Not inherited: every token the grammars emit falls back to `ink` unless listed here,
    // so no colors outside the design tokens can appear.
    inherit: false,
    rules: [
      { token: "", foreground: hex(c.ink), background: hex(c["code-bg"]) },
      { token: "keyword", foreground: hex(c["tok-kw"]) },
      { token: "tag", foreground: hex(c["tok-kw"]) },
      { token: "metatag", foreground: hex(c["tok-kw"]) },
      { token: "string", foreground: hex(c["tok-str"]) },
      { token: "regexp", foreground: hex(c["tok-str"]) },
      { token: "attribute.value", foreground: hex(c["tok-str"]) },
      { token: "number", foreground: hex(c["tok-num"]) },
      { token: "type", foreground: hex(c["tok-type"]) },
      { token: "type.identifier", foreground: hex(c["tok-type"]) },
      { token: "attribute.name", foreground: hex(c["tok-type"]) },
      { token: "comment", foreground: hex(c["tok-com"]), fontStyle: "italic" },
      { token: "identifier", foreground: hex(c.ink) },
      { token: "delimiter", foreground: hex(c.ink) },
      { token: "operator", foreground: hex(c.ink) },
    ],
    colors: {
      "editor.background": c["code-bg"],
      "editor.foreground": c.ink,
      "editorGutter.background": c["code-bg"],
      "editorLineNumber.foreground": c["tok-com"],
      "editorLineNumber.activeForeground": c["tok-com"],
      "editorCursor.foreground": c.accent,
      "editor.selectionBackground": c["accent-soft"],
      "editor.inactiveSelectionBackground": c["accent-soft"],
      "editor.selectionHighlightBackground": "#00000000",
      "editor.wordHighlightBackground": "#00000000",
      "editor.lineHighlightBackground": "#00000000",
      "editor.lineHighlightBorder": "#00000000",
      "editor.findMatchBackground": c["user-soft"],
      "editor.findMatchHighlightBackground": c["user-soft"],
      "editorBracketMatch.background": "#00000000",
      "editorBracketMatch.border": c.line,
      "editorWidget.background": c.surface,
      "editorWidget.foreground": c.ink,
      "editorWidget.border": c.line,
      "editorHoverWidget.background": c.surface,
      "editorHoverWidget.foreground": c.ink,
      "editorHoverWidget.border": c.line,
      "editorHoverWidget.statusBarBackground": c.bg,
      "textLink.foreground": c.accent,
      "textLink.activeForeground": c.accent,
      "textCodeBlock.background": c["code-bg"],
      "focusBorder": c.accent,
      "input.background": c.surface,
      "input.foreground": c.ink,
      "input.border": c.line,
      "scrollbar.shadow": "#00000000",
      "scrollbarSlider.background": c.muted + "40",
      "scrollbarSlider.hoverBackground": c.muted + "66",
      "scrollbarSlider.activeBackground": c.muted + "88",
      "editorIndentGuide.background1": "#00000000",
      "editorIndentGuide.activeBackground1": "#00000000",
      "editorGutter.foldingControlForeground": c.muted,
      "peekView.border": c.accent,
      "peekViewEditor.background": c["code-bg"],
      "peekViewResult.background": c.surface,
      "peekViewTitle.background": c.surface,
    },
  };
}

monaco.editor.defineTheme("isoline-light", theme(LIGHT, "vs"));
monaco.editor.defineTheme("isoline-dark", theme(DARK, "vs-dark"));

export { monaco };
