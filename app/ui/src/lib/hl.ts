// A small highlighter for the definition peek (the prototype's hlLine, made multi-line aware).
// The peek shows a few lines of another file; loading Monaco for that would be heavy, and the
// coloring only needs keywords, strings, numbers, comments and names that are links.
export type HlKind = "kw" | "str" | "num" | "com" | "type" | "name" | "plain";

export interface HlTok {
  text: string;
  kind: HlKind;
}

const KW = new Set(
  "import export from type interface function const let var return if else async await try catch finally throw new typeof instanceof in of as true false null undefined void unknown string number boolean extends keyof class enum implements readonly private public protected static declare namespace module default fn pub struct impl trait use mod match where self def lambda yield".split(
    " ",
  ),
);

const TOK = /(\/\/.*$|#.*$|\/\*.*?(?:\*\/|$)|"(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?|`(?:[^`\\]|\\.)*`?|\b\d+(?:\.\d+)?\b|[A-Za-z_$][\w$]*|\s+|.)/g;

/**
 * Splits lines into tokens. A block comment that is still open at the end of a line carries over
 * to the next one. `hash` treats `#` as a line comment (Python, shell).
 */
export function highlightLines(lines: string[], opts: { hash?: boolean } = {}): HlTok[][] {
  let inBlock = false;
  return lines.map((line) => {
    const out: HlTok[] = [];
    let rest = line;
    if (inBlock) {
      const end = rest.indexOf("*/");
      if (end < 0) return [{ text: rest, kind: "com" }];
      out.push({ text: rest.slice(0, end + 2), kind: "com" });
      rest = rest.slice(end + 2);
      inBlock = false;
    }
    TOK.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = TOK.exec(rest)) && m[0] !== "") {
      const t = m[0];
      let kind: HlKind = "plain";
      if (t.startsWith("//") || (opts.hash && t.startsWith("#"))) kind = "com";
      else if (t.startsWith("#")) {
        // Not a comment in this language: hand the rest back to the tokenizer one char at a time.
        out.push({ text: "#", kind: "plain" });
        TOK.lastIndex = m.index + 1;
        continue;
      } else if (t.startsWith("/*")) {
        kind = "com";
        if (!t.endsWith("*/") || t.length < 4) inBlock = true;
      } else if (/^["'`]/.test(t)) kind = "str";
      else if (/^\d/.test(t)) kind = "num";
      else if (/^[A-Za-z_$]/.test(t)) kind = KW.has(t) ? "kw" : /^[A-Z]/.test(t) ? "type" : "name";
      out.push({ text: t, kind });
    }
    return out;
  });
}
