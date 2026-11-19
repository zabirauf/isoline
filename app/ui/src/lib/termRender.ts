// Draws a native terminal (frames from libghostty-vt in the backend) on a canvas. Frames
// carry only the rows that changed; the renderer keeps every row and redraws the changed
// ones, plus the rows the cursor left and entered. Runs of plain text are one fillText each,
// at fractional cell positions that match the font's own advance, so text stays on the grid.
import type { TermFrame } from "../ipc/api";

/** A run with its style resolved: column, text, fg, bg (-1: default), flags, width. */
export type Run = [number, string, number, number, number, number];

export interface Line {
  runs: Run[];
  sel: [number, number] | null;
}

export const F = { bold: 1, italic: 2, faint: 4, blink: 8, inverse: 16, invisible: 32, strike: 64, overline: 128 } as const;

/** Resolves a frame's style indexes into the runs, so a row stands alone after the frame. */
export function resolveRows(f: TermFrame): Map<number, Line> {
  const out = new Map<number, Line>();
  for (const row of f.rows) {
    out.set(row.y, {
      sel: row.sel,
      runs: row.r.map(([x, text, s, w]) => {
        const st = f.styles[s] ?? [-1, -1, 0];
        return [x, text, st[0], st[1], st[2], w];
      }),
    });
  }
  return out;
}

const hexCache = new Map<number, string>();
export function css(c: number): string {
  let s = hexCache.get(c);
  if (!s) {
    s = "#" + c.toString(16).padStart(6, "0");
    hexCache.set(c, s);
  }
  return s;
}

export interface Metrics {
  /** Cell width (fractional: the font's advance) and height, in CSS pixels. */
  cw: number;
  ch: number;
  baseline: number;
}

export function measure(ctx: CanvasRenderingContext2D, font: string, size: number): Metrics {
  ctx.font = `${size}px ${font}`;
  const m = ctx.measureText("MMMMMMMMMM");
  const cw = m.width / 10;
  const asc = m.fontBoundingBoxAscent || size * 0.8;
  const desc = m.fontBoundingBoxDescent || size * 0.2;
  const ch = Math.ceil((asc + desc) * 1.12);
  return { cw, ch, baseline: Math.round((ch - (asc + desc)) / 2 + asc) };
}

export class TermRenderer {
  lines: Line[] = [];
  cols = 0;
  rows = 0;
  fg = 0xd4d4d4;
  bg = 0x1e1e1e;
  cursorColor = 0xffffff;
  selColor = "rgba(90,130,230,.35)";
  cursor: TermFrame["cursor"] = null;
  focused = false;
  blinkOn = true;
  scroll: [number, number, number] = [0, 0, 0];
  mouse = false;
  private fontKey = "";

  constructor(
    private ctx: CanvasRenderingContext2D,
    public m: Metrics,
    private font: string,
    private size: number,
    public dpr: number,
  ) {}

  /** Resizing a canvas resets its state (the font among it): set everything again. */
  resetContext() {
    this.fontKey = "";
    this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    this.ctx.textBaseline = "alphabetic";
  }

  /** Merges a frame; returns the rows to redraw (with the cursor's old and new rows). */
  apply(f: TermFrame): Set<number> {
    const dirty = new Set<number>();
    if (this.cursor) dirty.add(this.cursor.y);
    if (f.full || f.rows_n !== this.rows) {
      this.rows = f.rows_n;
      this.lines = Array.from({ length: this.rows }, () => ({ runs: [], sel: null }));
      for (let y = 0; y < this.rows; y++) dirty.add(y);
    }
    this.cols = f.cols;
    if (f.colors) [this.fg, this.bg, this.cursorColor] = f.colors;
    for (const [y, line] of resolveRows(f)) {
      if (y < this.rows) {
        this.lines[y] = line;
        dirty.add(y);
      }
    }
    this.cursor = f.cursor;
    if (this.cursor) dirty.add(this.cursor.y);
    this.scroll = f.scroll;
    this.mouse = f.mouse;
    return dirty;
  }

  private setFont(flags: number) {
    const k = `${flags & F.italic ? "italic " : ""}${flags & F.bold ? "600 " : ""}${this.size}px ${this.font}`;
    if (k !== this.fontKey) {
      this.ctx.font = k;
      this.fontKey = k;
    }
  }

  /** Snaps a CSS x to the device pixel grid, so adjacent backgrounds leave no seam. */
  private snap(x: number) {
    return Math.round(x * this.dpr) / this.dpr;
  }

  drawAll() {
    const { ctx } = this;
    ctx.fillStyle = css(this.bg);
    ctx.fillRect(0, 0, ctx.canvas.width / this.dpr, ctx.canvas.height / this.dpr);
    for (let y = 0; y < this.rows; y++) this.drawRow(y);
  }

  draw(rows: Iterable<number>) {
    for (const y of rows) if (y < this.rows) this.drawRow(y);
  }

  drawRow(y: number) {
    const { ctx, m } = this;
    const top = y * m.ch;
    const width = ctx.canvas.width / this.dpr;
    ctx.fillStyle = css(this.bg);
    ctx.fillRect(0, top, width, m.ch);
    const line = this.lines[y];
    if (!line) return;
    // Backgrounds, then the selection, then text.
    for (const [x, , fg, bg, flags, w] of line.runs) {
      let b = bg;
      if (flags & F.inverse) b = fg < 0 ? this.fg : fg;
      if (b < 0) continue;
      const x0 = this.snap(x * m.cw);
      ctx.fillStyle = css(b);
      ctx.fillRect(x0, top, this.snap((x + w) * m.cw) - x0, m.ch);
    }
    if (line.sel) {
      const x0 = this.snap(line.sel[0] * m.cw);
      ctx.fillStyle = this.selColor;
      ctx.fillRect(x0, top, this.snap((line.sel[1] + 1) * m.cw) - x0, m.ch);
    }
    const cur = this.cursor && this.cursor.y === y && this.cursorShown() ? this.cursor : null;
    for (const [x, text, fg, bg, flags, w] of line.runs) {
      if (flags & F.invisible || !text.trim()) continue;
      let f = fg < 0 ? this.fg : fg;
      if (flags & F.inverse) f = bg < 0 ? this.bg : bg;
      this.setFont(flags);
      ctx.fillStyle = css(f);
      ctx.globalAlpha = flags & F.faint ? 0.6 : 1;
      ctx.fillText(text, x * m.cw, top + m.baseline);
      const ul = (flags >> 8) & 7;
      if (ul || flags & F.strike || flags & F.overline) {
        ctx.fillRect(x * m.cw, top + (ul ? m.ch - 2 : flags & F.strike ? Math.round(m.ch / 2) : 0), w * m.cw, 1);
      }
      ctx.globalAlpha = 1;
    }
    if (cur) this.drawCursor(cur, line);
  }

  private cursorShown() {
    return !this.cursor?.blink || !this.focused || this.blinkOn;
  }

  private drawCursor(c: NonNullable<TermFrame["cursor"]>, line: Line) {
    const { ctx, m } = this;
    const x = c.x * m.cw;
    const top = c.y * m.ch;
    const run = line.runs.find(([rx, , , , , w]) => c.x >= rx && c.x < rx + w);
    // A wide grapheme is one character over two cells; a plain run has one per cell.
    const wide = !!run && run[5] === 2 && [...run[1]].length === 1;
    const cellW = (wide ? 2 : 1) * m.cw;
    ctx.fillStyle = css(this.cursorColor);
    const style = this.focused ? c.style : "hollow";
    if (style === "bar") ctx.fillRect(this.snap(x), top, 2, m.ch);
    else if (style === "underline") ctx.fillRect(x, top + m.ch - 2, cellW, 2);
    else if (style === "hollow") {
      ctx.strokeStyle = css(this.cursorColor);
      ctx.lineWidth = 1;
      ctx.strokeRect(this.snap(x) + 0.5, top + 0.5, cellW - 1, m.ch - 1);
    } else {
      ctx.fillRect(this.snap(x), top, cellW, m.ch);
      // The character under a block cursor, in the background color.
      if (run) {
        const ch = wide ? run[1] : [...run[1]][c.x - run[0]];
        if (ch && ch.trim()) {
          this.setFont(run[4]);
          ctx.fillStyle = css(this.bg);
          ctx.fillText(ch, x, top + m.baseline);
        }
      }
    }
  }
}
