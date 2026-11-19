//! One terminal's engine, on its own thread: libghostty-vt parses the shell's output here, off
//! the UI thread, and the UI gets frames of the rows that changed, at most one per
//! [`FRAME`] and never more than it has drawn (each frame waits for the previous one's
//! acknowledgement). Input is encoded here with Ghostty's key, mouse and paste encoders, so
//! modes such as application cursor keys, bracketed paste, mouse tracking and the Kitty
//! keyboard protocol behave as they do in Ghostty.
//!
//! The libghostty-vt handles are not `Send`: [`run`] creates them on the engine thread and
//! everything reaches it as a [`Msg`].

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use ghostty_vt::key::{self, Mods};
use ghostty_vt::mouse;
use ghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator};
use ghostty_vt::screen::CellWide;
use ghostty_vt::selection::FormatOptions;
use ghostty_vt::selection::gesture::{DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent};
use ghostty_vt::style::{Palette, RgbColor, Underline};
use ghostty_vt::terminal::{
    Event, Mode, Options, Point, PointCoordinate, ScrollViewport, Terminal,
};
use serde::{Deserialize, Serialize};

use crate::keys::key_from_code;

/// The shortest time between two frames: 60 per second.
pub const FRAME: Duration = Duration::from_millis(16);
/// A frame the UI has not acknowledged by then is taken as lost, and the next one is sent.
const ACK_TIMEOUT: Duration = Duration::from_millis(250);

/// Colors as `#rrggbb`. `palette` holds the 16 ANSI colors; the rest of the 256 stay
/// Ghostty's defaults.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub foreground: String,
    pub background: String,
    pub cursor: String,
    #[serde(default)]
    pub palette: Vec<String>,
}

/// Input from the UI. Positions are in cells, as fractions (column 3.5 is the middle of
/// the fourth cell).
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Input {
    /// A key press. `key` is `KeyboardEvent.key`; `code` is `KeyboardEvent.code`.
    #[serde(rename_all = "camelCase")]
    Key {
        code: String,
        key: String,
        mods: u8,
        repeat: bool,
    },
    /// Text from an input method or a dead key: written as typed.
    Text {
        text: String,
    },
    Paste {
        text: String,
    },
    /// A mouse button or move, for a program that tracks the mouse.
    Mouse {
        action: MouseAction,
        button: Option<u8>,
        x: f32,
        y: f32,
        mods: u8,
    },
    /// The wheel: scrolls the history, or goes to a program that tracks the mouse, or
    /// becomes arrow keys on the alternate screen (`less`, `man`).
    Wheel {
        x: f32,
        y: f32,
        lines: i32,
        mods: u8,
    },
    /// Selection by pointer: press, drag and release. `ms` times double and triple clicks.
    Select {
        phase: SelectPhase,
        x: f32,
        y: f32,
        ms: u64,
    },
    SelectAll,
    ClearSelection,
    /// Scroll the history: lines (negative is up), or to the top or the bottom.
    Scroll {
        lines: Option<i32>,
        to: Option<String>,
        /// An absolute row of the history (the scrollbar).
        row: Option<u64>,
    },
    /// Clear the screen and the history (⌘K); the shell redraws its prompt.
    Clear,
    #[serde(rename_all = "camelCase")]
    Resize {
        cols: u16,
        rows: u16,
        cell_w: u32,
        cell_h: u32,
    },
    Theme(Theme),
    Focus {
        on: bool,
    },
    /// Send every row again (the UI lost its picture, or just opened).
    Refresh,
    /// The UI drew frame `seq`.
    Ack {
        seq: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MouseAction {
    Press,
    Release,
    Move,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectPhase {
    Press,
    Drag,
    Release,
}

pub enum Msg {
    Output(Vec<u8>),
    Input(Input),
    /// The selection as text, for copy.
    Copy(Sender<Option<String>>),
    Stop,
}

/// Mods from the UI: shift 1, alt 2, ctrl 4, meta 8.
fn mods(bits: u8) -> Mods {
    let mut m = Mods::empty();
    if bits & 1 != 0 {
        m |= Mods::SHIFT;
    }
    if bits & 2 != 0 {
        m |= Mods::ALT;
    }
    if bits & 4 != 0 {
        m |= Mods::CTRL;
    }
    if bits & 8 != 0 {
        m |= Mods::SUPER;
    }
    m
}

/// `#rrggbb` or `#rgb` (minified CSS writes `#fff`).
fn hex(s: &str) -> Option<RgbColor> {
    let s = s.trim().trim_start_matches('#');
    let s: String = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect(),
        6 => s.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&s, 16).ok()?;
    Some(RgbColor {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    })
}

fn rgb(c: RgbColor) -> u32 {
    (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}

/// One row of a frame: runs of cells that share a style.
#[derive(Serialize, Debug, PartialEq)]
pub struct Row {
    pub y: u16,
    /// `[column, text, style index, width in cells]`. A run is either plain narrow text,
    /// drawn as one string, or a single wide or uncommon grapheme drawn in its own cells.
    pub r: Vec<(u16, String, u16, u16)>,
    /// Selected columns, inclusive.
    pub sel: Option<(u16, u16)>,
}

#[derive(Serialize, Debug)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    /// "block", "bar", "underline" or "hollow".
    pub style: &'static str,
    pub blink: bool,
    pub wide: bool,
}

#[derive(Serialize, Debug, Default)]
pub struct Frame {
    pub seq: u64,
    /// Every row is in `rows`; the UI clears first.
    pub full: bool,
    pub cols: u16,
    pub rows_n: u16,
    pub rows: Vec<Row>,
    /// `[foreground, background, flags]`; colors are 0xRRGGBB or -1 for the default.
    /// Flags: bold 1, italic 2, faint 4, blink 8, inverse 16, invisible 32,
    /// strikethrough 64, overline 128, underline style << 8 (1 single ... 5 dashed).
    pub styles: Vec<(i64, i64, u32)>,
    pub cursor: Option<Cursor>,
    /// Default colors, on full frames.
    pub colors: Option<(u32, u32, u32)>,
    /// History: total rows, the viewport's first row, visible rows.
    pub scroll: (u64, u64, u64),
    pub title: Option<String>,
    pub pwd: Option<String>,
    pub bell: bool,
    pub clipboard: Option<String>,
    /// A program tracks the mouse: pointer events go to it (Shift selects instead).
    pub mouse: bool,
}

/// Plain text a run can hold: drawn as one string, it must have the font's own advance.
fn runnable(c: char) -> bool {
    matches!(c as u32, 0x20..=0x7e | 0xa0..=0x17f | 0x2500..=0x259f)
}

struct Engine {
    t: Terminal,
    rs: RenderState,
    rows_it: RowIterator,
    cells_it: CellIterator,
    keys: key::Encoder,
    key_ev: key::Event,
    mouse: mouse::Encoder,
    mouse_ev: mouse::Event,
    gesture: Gesture,
    press: PressEvent,
    drag: DragEvent,
    release: ReleaseEvent,
    cell_w: u32,
    cell_h: u32,
    seq: u64,
    full: bool,
    title: Option<String>,
    pwd: Option<String>,
    bell: bool,
    clipboard: Option<String>,
    /// Bytes for the shell.
    to_pty: Sender<Vec<u8>>,
    base_palette: Palette,
}

impl Engine {
    fn new(
        cols: u16,
        rows: u16,
        cell_w: u32,
        cell_h: u32,
        to_pty: Sender<Vec<u8>>,
    ) -> ghostty_vt::error::Result<Self> {
        let mut t = Terminal::new(Options {
            cols: cols.max(2),
            rows: rows.max(2),
            max_scrollback: 10_000_000,
        })?;
        t.resize(cols.max(2), rows.max(2), cell_w.max(1), cell_h.max(1))?;
        let base_palette = t.default_color_palette()?;
        Ok(Engine {
            base_palette,
            t,
            rs: RenderState::new()?,
            rows_it: RowIterator::new()?,
            cells_it: CellIterator::new()?,
            keys: key::Encoder::new()?,
            key_ev: key::Event::new()?,
            mouse: mouse::Encoder::new()?,
            mouse_ev: mouse::Event::new()?,
            gesture: Gesture::new()?,
            press: PressEvent::new()?,
            drag: DragEvent::new()?,
            release: ReleaseEvent::new()?,
            cell_w: cell_w.max(1),
            cell_h: cell_h.max(1),
            seq: 0,
            full: true,
            title: None,
            pwd: None,
            bell: false,
            clipboard: None,
            to_pty,
        })
    }

    fn send(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.to_pty.send(bytes);
        }
    }

    /// Parses output and handles what the terminal asks for (query answers go to the shell).
    fn output(&mut self, data: &[u8]) {
        self.t.vt_write(data);
        self.drain_events();
    }

    fn drain_events(&mut self) {
        let events: Vec<Event> = self.t.events().collect();
        for e in events {
            match e {
                Event::PtyWrite(b) => self.send(b),
                Event::Bell => self.bell = true,
                Event::TitleChanged(s) => self.title = Some(s),
                Event::PwdChanged(s) => self.pwd = Some(s),
                Event::ClipboardWrite(c) => {
                    // OSC 52 from a program (an editor over ssh, tmux): plain text only.
                    if let Some(s) = c
                        .contents
                        .iter()
                        .find(|x| x.mime.starts_with("text/plain"))
                        .and_then(|x| String::from_utf8(x.data.clone()).ok())
                    {
                        self.clipboard = Some(s);
                    }
                }
                _ => {}
            }
        }
    }

    fn theme(&mut self, th: &Theme) {
        let _ = self.t.set_default_fg_color(hex(&th.foreground));
        let _ = self.t.set_default_bg_color(hex(&th.background));
        let _ = self.t.set_default_cursor_color(hex(&th.cursor));
        // Ghostty's 256 colors, with the first 16 from the app theme.
        let mut p = self.base_palette;
        for (i, c) in th.palette.iter().take(16).enumerate() {
            if let Some(c) = hex(c) {
                p.0[i] = c;
            }
        }
        let _ = self.t.set_default_color_palette(Some(p));
        self.full = true;
    }

    fn key(&mut self, code: &str, text: &str, m: u8, repeat: bool) {
        let k = key_from_code(code);
        let mut chars = text.chars();
        let printable = match (chars.next(), chars.next()) {
            (Some(c), None) => !c.is_control(),
            _ => false,
        };
        let m = mods(m);
        let unshifted = match k {
            key::Key::Unidentified => None,
            _ => code
                .strip_prefix("Key")
                .and_then(|l| l.chars().next())
                .map(|c| c.to_ascii_lowercase())
                .or_else(|| code.strip_prefix("Digit").and_then(|d| d.chars().next())),
        };
        self.key_ev
            .set_action(if repeat {
                key::Action::Repeat
            } else {
                key::Action::Press
            })
            .set_key(k)
            .set_mods(m)
            .set_composing(false)
            .set_utf8(printable.then(|| text.to_string()));
        let shifted = printable
            && m.contains(Mods::SHIFT)
            && unshifted.is_some_and(|u| text != u.to_string());
        self.key_ev
            .set_consumed_mods(if shifted { Mods::SHIFT } else { Mods::empty() });
        if let Some(u) = unshifted {
            self.key_ev.set_unshifted_codepoint(u);
        }
        self.keys.set_options_from_terminal(&self.t);
        let mut out = Vec::new();
        if self.keys.encode_to_vec(&self.key_ev, &mut out).is_ok()
            && out.is_empty()
            && printable
            && k == key::Key::Unidentified
        {
            // A key Ghostty does not know (some layouts' extra keys): send its text.
            out.extend_from_slice(text.as_bytes());
        }
        if !out.is_empty() {
            // Typing goes back to the live screen, as in every terminal.
            self.t.scroll_viewport(ScrollViewport::Bottom);
            self.send(out);
        }
    }

    fn paste(&mut self, text: &str) {
        let bracketed = self.t.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
        let mut data = text.as_bytes().to_vec();
        let mut buf = vec![0u8; data.len() * 2 + 16];
        match ghostty_vt::paste::encode(&mut data, bracketed, &mut buf) {
            Ok(n) => {
                buf.truncate(n);
                self.send(buf);
            }
            Err(_) => self.send(text.as_bytes().to_vec()),
        }
        self.t.scroll_viewport(ScrollViewport::Bottom);
    }

    fn mouse_size(&mut self) {
        let cols = self.t.cols().unwrap_or(80) as u32;
        let rows = self.t.rows().unwrap_or(24) as u32;
        self.mouse.set_size(mouse::EncoderSize {
            screen_width: cols * self.cell_w,
            screen_height: rows * self.cell_h,
            cell_width: self.cell_w,
            cell_height: self.cell_h,
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        });
    }

    fn mouse_event(
        &mut self,
        action: mouse::Action,
        button: Option<mouse::Button>,
        x: f32,
        y: f32,
        m: u8,
    ) {
        self.mouse.set_options_from_terminal(&self.t);
        self.mouse_size();
        self.mouse_ev
            .set_action(action)
            .set_button(button)
            .set_mods(mods(m))
            .set_position(mouse::Position {
                x: x * self.cell_w as f32,
                y: y * self.cell_h as f32,
            });
        let mut out = Vec::new();
        if self.mouse.encode_to_vec(&self.mouse_ev, &mut out).is_ok() {
            self.send(out);
        }
    }

    fn button(b: Option<u8>) -> Option<mouse::Button> {
        match b {
            Some(0) => Some(mouse::Button::Left),
            Some(1) => Some(mouse::Button::Middle),
            Some(2) => Some(mouse::Button::Right),
            _ => None,
        }
    }

    fn cell_at(&self, x: f32, y: f32) -> PointCoordinate {
        let cols = self.t.cols().unwrap_or(80);
        let rows = self.t.rows().unwrap_or(24);
        PointCoordinate {
            x: (x.max(0.0) as u16).min(cols.saturating_sub(1)),
            y: (y.max(0.0) as u32).min(rows.saturating_sub(1) as u32),
        }
    }

    fn select(&mut self, phase: SelectPhase, x: f32, y: f32, ms: u64) {
        let p = self.cell_at(x, y);
        let (px, py) = (x as f64 * self.cell_w as f64, y as f64 * self.cell_h as f64);
        let Ok(gref) = self.t.grid_ref(Point::Viewport(p)) else {
            return;
        };
        match phase {
            SelectPhase::Press => {
                let _ = self.press.set_position(px, py);
                let _ = self.press.set_time(Duration::from_millis(ms));
                let _ = self.press.set_repeat_interval(Duration::from_millis(500));
                let _ = self.press.set_repeat_distance(self.cell_w as f64 * 1.5);
                if let Ok(sel) = self.press.apply(&mut self.gesture, &self.t, gref) {
                    let _ = self.t.set_selection(sel.as_ref());
                }
            }
            SelectPhase::Drag => {
                let _ = self.drag.set_position(px, py);
                let geo = Geometry {
                    columns: self.t.cols().unwrap_or(80) as u32,
                    cell_width: self.cell_w,
                    padding_left: 0,
                    screen_height: self.t.rows().unwrap_or(24) as u32 * self.cell_h,
                };
                if let Ok(sel) = self.drag.apply(&mut self.gesture, &self.t, gref, geo) {
                    let _ = self.t.set_selection(sel.as_ref());
                }
            }
            SelectPhase::Release => {
                let _ = self.release.apply(&mut self.gesture, &self.t, Some(gref));
            }
        }
    }

    fn input(&mut self, i: Input) {
        match i {
            Input::Key {
                code,
                key,
                mods,
                repeat,
            } => self.key(&code, &key, mods, repeat),
            Input::Text { text } => {
                self.t.scroll_viewport(ScrollViewport::Bottom);
                self.send(text.into_bytes());
            }
            Input::Paste { text } => self.paste(&text),
            Input::Mouse {
                action,
                button,
                x,
                y,
                mods,
            } => {
                let a = match action {
                    MouseAction::Press => mouse::Action::Press,
                    MouseAction::Release => mouse::Action::Release,
                    MouseAction::Move => mouse::Action::Motion,
                };
                self.mouse_event(a, Self::button(button), x, y, mods);
            }
            Input::Wheel { x, y, lines, mods } => {
                if lines == 0 {
                    return;
                }
                if self.t.is_mouse_tracking().unwrap_or(false) && mods & 1 == 0 {
                    let b = if lines < 0 {
                        mouse::Button::Four
                    } else {
                        mouse::Button::Five
                    };
                    for _ in 0..lines.unsigned_abs().min(10) {
                        self.mouse_event(mouse::Action::Press, Some(b), x, y, mods);
                    }
                } else if self.t.mode(Mode::ALT_SCREEN_SAVE).unwrap_or(false)
                    || self.t.mode(Mode::ALT_SCREEN).unwrap_or(false)
                {
                    // Alternate scroll: the wheel is arrow keys in full-screen programs.
                    let code = if lines < 0 { "ArrowUp" } else { "ArrowDown" };
                    for _ in 0..lines.unsigned_abs().min(10) {
                        self.key(code, "", 0, false);
                    }
                } else {
                    self.t
                        .scroll_viewport(ScrollViewport::Delta(lines as isize));
                }
            }
            Input::Select { phase, x, y, ms } => self.select(phase, x, y, ms),
            Input::SelectAll => {
                if let Ok(sel) = self.t.select_all() {
                    let _ = self.t.set_selection(sel.as_ref());
                }
            }
            Input::ClearSelection => {
                let _ = self.t.set_selection(None);
            }
            Input::Scroll { lines, to, row } => match (lines, to.as_deref()) {
                _ if row.is_some() => self
                    .t
                    .scroll_viewport(ScrollViewport::Row(row.unwrap_or(0) as usize)),
                (_, Some("top")) => self.t.scroll_viewport(ScrollViewport::Top),
                (_, Some("bottom")) => self.t.scroll_viewport(ScrollViewport::Bottom),
                (Some(n), _) => self.t.scroll_viewport(ScrollViewport::Delta(n as isize)),
                _ => {}
            },
            Input::Clear => {
                // Screen and history go; Ctrl+L makes the shell draw its prompt again.
                self.t.vt_write(b"\x1b[H\x1b[2J\x1b[3J");
                self.drain_events();
                self.send(vec![0x0c]);
                self.full = true;
            }
            Input::Resize {
                cols,
                rows,
                cell_w,
                cell_h,
            } => {
                self.cell_w = cell_w.max(1);
                self.cell_h = cell_h.max(1);
                let _ = self
                    .t
                    .resize(cols.max(2), rows.max(2), self.cell_w, self.cell_h);
                self.drain_events();
                self.full = true;
            }
            Input::Theme(th) => self.theme(&th),
            Input::Focus { on } => {
                if self.t.mode(Mode::FOCUS_EVENT).unwrap_or(false) {
                    let mut buf = [0u8; 8];
                    let ev = if on {
                        ghostty_vt::focus::Event::Gained
                    } else {
                        ghostty_vt::focus::Event::Lost
                    };
                    if let Ok(n) = ev.encode(&mut buf) {
                        self.send(buf[..n].to_vec());
                    }
                }
            }
            Input::Refresh => self.full = true,
            Input::Ack { .. } => {}
        }
    }

    fn copy(&self) -> Option<String> {
        let b = self
            .t
            .format_selection_alloc(FormatOptions::new().with_unwrap(true).with_trim(true))
            .ok()??;
        Some(String::from_utf8_lossy(&b).into_owned())
    }

    /// The rows that changed since the last frame, or `None` when nothing did.
    fn frame(&mut self) -> Option<Frame> {
        let Engine {
            t,
            rs,
            rows_it,
            cells_it,
            ..
        } = self;
        let snap = rs.update(t).ok()?;
        let dirty = snap.dirty().ok()?;
        let full = self.full || dirty == Dirty::Full;
        let meta =
            self.title.is_some() || self.pwd.is_some() || self.bell || self.clipboard.is_some();
        if !full && dirty == Dirty::Clean && !meta {
            return None;
        }
        let mut f = Frame {
            full,
            cols: snap.cols().unwrap_or(0),
            rows_n: snap.rows().unwrap_or(0),
            ..Default::default()
        };
        let mut style_ix: std::collections::HashMap<(i64, i64, u32), u16> = Default::default();
        let mut it = rows_it.update(&snap).ok()?;
        let mut y: u16 = 0;
        let mut text = String::new();
        while let Some(row) = it.next() {
            if full || row.dirty().unwrap_or(true) {
                let mut out = Row {
                    y,
                    r: Vec::new(),
                    sel: row.selection().ok().flatten().map(|s| (s.start_x, s.end_x)),
                };
                if let Ok(mut cells) = cells_it.update(row) {
                    let mut x: u16 = 0;
                    while let Some(cell) = cells.next() {
                        let col = x;
                        x += 1;
                        let wide = cell
                            .raw_cell()
                            .and_then(|c| c.wide())
                            .unwrap_or(CellWide::Narrow);
                        if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
                            continue;
                        }
                        let st = cell.style().ok();
                        let fg = cell.fg_color().ok().flatten().map_or(-1, |c| rgb(c) as i64);
                        let bg = cell.bg_color().ok().flatten().map_or(-1, |c| rgb(c) as i64);
                        let flags = st.map_or(0, |s| {
                            (s.bold as u32)
                                | (s.italic as u32) << 1
                                | (s.faint as u32) << 2
                                | (s.blink as u32) << 3
                                | (s.inverse as u32) << 4
                                | (s.invisible as u32) << 5
                                | (s.strikethrough as u32) << 6
                                | (s.overline as u32) << 7
                                | (match s.underline {
                                    Underline::None => 0,
                                    Underline::Single => 1,
                                    Underline::Double => 2,
                                    Underline::Curly => 3,
                                    Underline::Dotted => 4,
                                    Underline::Dashed => 5,
                                    _ => 1,
                                }) << 8
                        });
                        text.clear();
                        let _ = cell.graphemes_utf8(&mut text);
                        if text.is_empty() {
                            text.push(' ');
                        }
                        let key = (fg, bg, flags);
                        let si = match style_ix.get(&key) {
                            Some(&i) => i,
                            None => {
                                let i = f.styles.len() as u16;
                                f.styles.push(key);
                                style_ix.insert(key, i);
                                i
                            }
                        };
                        let w = if wide == CellWide::Wide { 2 } else { 1 };
                        let mut cs = text.chars();
                        let plain = w == 1
                            && matches!((cs.next(), cs.next()), (Some(c), None) if runnable(c));
                        match out.r.last_mut() {
                            Some(last)
                                if plain
                                    && last.2 == si
                                    && last.0 + last.3 == col
                                    && last.1.chars().all(runnable) =>
                            {
                                last.1.push_str(&text);
                                last.3 += 1;
                            }
                            _ => out.r.push((col, text.clone(), si, w)),
                        }
                    }
                }
                // Trailing blanks in the default style draw nothing.
                while out.r.last().is_some_and(|l| {
                    f.styles[l.2 as usize] == (-1, -1, 0) && l.1.trim_end().is_empty()
                }) {
                    out.r.pop();
                }
                if let Some(l) = out.r.last_mut()
                    && f.styles[l.2 as usize] == (-1, -1, 0)
                    && l.1.chars().all(runnable)
                {
                    let n = l.1.trim_end().chars().count() as u16;
                    l.1.truncate(l.1.trim_end().len());
                    l.3 = n;
                }
                f.rows.push(out);
                let _ = row.set_dirty(false);
            }
            y += 1;
        }
        if let Ok(c) = snap.cursor()
            && c.visible
            && let Some(v) = c.viewport
        {
            f.cursor = Some(Cursor {
                x: v.x,
                y: v.y,
                style: match c.visual_style {
                    CursorVisualStyle::Bar => "bar",
                    CursorVisualStyle::Underline => "underline",
                    CursorVisualStyle::BlockHollow => "hollow",
                    _ => "block",
                },
                blink: c.blinking,
                wide: v.at_wide_tail,
            });
        }
        if full && let Ok(c) = snap.colors() {
            f.colors = Some((
                rgb(c.foreground),
                rgb(c.background),
                rgb(c.cursor.unwrap_or(c.foreground)),
            ));
        }
        let _ = snap.set_dirty(Dirty::Clean);
        if let Ok(s) = self.t.scrollbar() {
            f.scroll = (s.total, s.offset, s.len);
        }
        f.mouse = self.t.is_mouse_tracking().unwrap_or(false);
        f.title = self.title.take();
        f.pwd = self.pwd.take();
        f.bell = std::mem::take(&mut self.bell);
        f.clipboard = self.clipboard.take();
        self.full = false;
        self.seq += 1;
        f.seq = self.seq;
        Some(f)
    }
}

#[allow(clippy::too_many_arguments)]
/// Runs a terminal until [`Msg::Stop`] or until every sender is gone. `emit` gets each frame
/// and returns false when the UI is gone.
pub fn run(
    cols: u16,
    rows: u16,
    cell_w: u32,
    cell_h: u32,
    theme: Option<Theme>,
    rx: Receiver<Msg>,
    to_pty: Sender<Vec<u8>>,
    mut emit: impl FnMut(&Frame) -> bool,
) {
    let Ok(mut e) = Engine::new(cols, rows, cell_w, cell_h, to_pty) else {
        return;
    };
    if let Some(th) = theme {
        e.theme(&th);
    }
    let mut last_frame = Instant::now() - FRAME;
    // The frame the UI has not drawn yet, and when it was sent.
    let mut unacked: Option<(u64, Instant)> = None;
    let mut pending = true;
    loop {
        // Wait for input; with a frame due, only until it is due.
        let wait = if pending {
            let due = last_frame + FRAME;
            let ack_due = unacked.map(|(_, at)| at + ACK_TIMEOUT);
            let until = ack_due.map_or(due, |a| a.max(due));
            until.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        let first = match rx.recv_timeout(wait) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Take everything queued: under a flood, many chunks parse into one frame.
        let started = Instant::now();
        let mut next = first;
        while let Some(m) = next.take() {
            match m {
                Msg::Output(b) => {
                    e.output(&b);
                    pending = true;
                }
                Msg::Input(Input::Ack { seq }) => {
                    if unacked.is_some_and(|(s, _)| s <= seq) {
                        unacked = None;
                    }
                }
                Msg::Input(i) => {
                    e.input(i);
                    pending = true;
                }
                Msg::Copy(reply) => {
                    let _ = reply.send(e.copy());
                }
                Msg::Stop => return,
            }
            // Stay responsive to frames even while output keeps coming.
            if started.elapsed() < FRAME {
                next = rx.try_recv().ok();
            }
        }
        let now = Instant::now();
        let ack_ok = unacked.is_none_or(|(_, at)| now >= at + ACK_TIMEOUT);
        if pending && ack_ok && now >= last_frame + FRAME {
            pending = false;
            if let Some(f) = e.frame() {
                if !emit(&f) {
                    return;
                }
                unacked = Some((f.seq, now));
                last_frame = now;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn engine() -> (Engine, Receiver<Vec<u8>>) {
        let (tx, rx) = channel();
        (Engine::new(40, 6, 8, 16, tx).unwrap(), rx)
    }

    fn text(f: &Frame, y: u16) -> String {
        let row = f.rows.iter().find(|r| r.y == y).unwrap();
        let mut s = String::new();
        for (x, t, _, _) in &row.r {
            while s.chars().count() < *x as usize {
                s.push(' ');
            }
            s.push_str(t);
        }
        s
    }

    #[test]
    fn frames_carry_changed_rows_styles_and_the_cursor() {
        let (mut e, _rx) = engine();
        e.output(b"hello \x1b[1;31mred\x1b[0m\r\nsecond");
        let f = e.frame().unwrap();
        assert!(f.full);
        assert_eq!(f.rows.len(), 6);
        assert_eq!(text(&f, 0), "hello red");
        let red = f.rows[0].r.iter().find(|r| r.1 == "red").unwrap();
        let (fg, _, flags) = f.styles[red.2 as usize];
        assert!(fg > 0 && flags & 1 == 1, "{:?}", f.styles);
        assert_eq!(f.cursor.as_ref().map(|c| (c.x, c.y)), Some((6, 1)));
        // Only the changed row next time.
        e.output(b"!");
        let g = e.frame().unwrap();
        assert!(!g.full);
        assert_eq!(g.rows.iter().map(|r| r.y).collect::<Vec<_>>(), vec![1]);
        assert_eq!(text(&g, 1), "second!");
        assert!(e.frame().is_none());
    }

    #[test]
    fn wide_characters_take_two_cells_in_their_own_run() {
        let (mut e, _rx) = engine();
        e.output("ab日本c".as_bytes());
        let f = e.frame().unwrap();
        let r = &f.rows[0].r;
        assert_eq!(r[0], (0, "ab".into(), r[0].2, 2));
        assert_eq!((r[1].0, r[1].1.as_str(), r[1].3), (2, "日", 2));
        assert_eq!((r[2].0, r[2].1.as_str(), r[2].3), (4, "本", 2));
        assert_eq!((r[3].0, r[3].1.as_str()), (6, "c"));
    }

    #[test]
    fn keys_follow_terminal_modes() {
        let (mut e, rx) = engine();
        e.key("KeyA", "a", 0, false);
        assert_eq!(rx.try_recv().unwrap(), b"a");
        e.key("KeyC", "c", 4, false);
        assert_eq!(rx.try_recv().unwrap(), vec![3]);
        e.key("ArrowUp", "ArrowUp", 0, false);
        assert_eq!(rx.try_recv().unwrap(), b"\x1b[A");
        // Application cursor keys (vim, less) change the arrows.
        e.output(b"\x1b[?1h");
        e.key("ArrowUp", "ArrowUp", 0, false);
        assert_eq!(rx.try_recv().unwrap(), b"\x1bOA");
        e.key("Enter", "Enter", 0, false);
        assert_eq!(rx.try_recv().unwrap(), b"\r");
    }

    #[test]
    fn paste_is_bracketed_when_the_program_asks() {
        let (mut e, rx) = engine();
        e.paste("ls");
        assert_eq!(rx.try_recv().unwrap(), b"ls");
        e.output(b"\x1b[?2004h");
        e.paste("ls");
        assert_eq!(rx.try_recv().unwrap(), b"\x1b[200~ls\x1b[201~");
    }

    #[test]
    fn queries_are_answered_to_the_shell() {
        let (mut e, rx) = engine();
        // Device status report: where is the cursor?
        e.output(b"abc\x1b[6n");
        assert_eq!(rx.try_recv().unwrap(), b"\x1b[1;4R");
    }

    #[test]
    fn selection_copies_and_shows_in_frames() {
        let (mut e, _rx) = engine();
        e.output(b"one two three");
        e.frame();
        e.select(SelectPhase::Press, 4.2, 0.5, 1000);
        e.select(SelectPhase::Drag, 6.9, 0.5, 1010);
        e.select(SelectPhase::Release, 6.9, 0.5, 1020);
        assert_eq!(e.copy().as_deref(), Some("two"));
        let f = e.frame().unwrap();
        assert_eq!(f.rows.iter().find(|r| r.y == 0).unwrap().sel, Some((4, 6)));
        // Double-click selects the word.
        e.select(SelectPhase::Press, 9.0, 0.5, 5000);
        e.select(SelectPhase::Release, 9.0, 0.5, 5010);
        e.select(SelectPhase::Press, 9.0, 0.5, 5100);
        e.select(SelectPhase::Release, 9.0, 0.5, 5110);
        assert_eq!(e.copy().as_deref(), Some("three"));
    }

    #[test]
    fn history_scrolls_and_typing_returns_to_the_bottom() {
        let (mut e, _rx) = engine();
        for i in 0..30 {
            e.output(format!("line {i}\r\n").as_bytes());
        }
        e.frame();
        e.input(Input::Wheel {
            x: 1.0,
            y: 1.0,
            lines: -5,
            mods: 0,
        });
        let f = e.frame().unwrap();
        assert!(f.scroll.1 < f.scroll.0 - f.scroll.2, "{:?}", f.scroll);
        e.key("KeyX", "x", 0, false);
        let g = e.frame().unwrap();
        assert_eq!(g.scroll.1, g.scroll.0 - g.scroll.2);
    }

    #[test]
    fn run_coalesces_a_flood_into_few_frames() {
        let (tx, rx) = channel();
        let (pty_tx, _pty_rx) = channel();
        for i in 0..2000 {
            tx.send(Msg::Output(
                format!("\x1b[3{}mline {i}\x1b[0m\r\n", i % 7).into_bytes(),
            ))
            .unwrap();
        }
        let tx2 = tx.clone();
        let frames = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let fr = frames.clone();
        let h = std::thread::spawn(move || {
            run(80, 24, 8, 16, None, rx, pty_tx, move |f| {
                fr.lock().unwrap().push((f.seq, f.rows.len()));
                // The UI draws it and acknowledges.
                let _ = tx2.send(Msg::Input(Input::Ack { seq: f.seq }));
                true
            })
        });
        std::thread::sleep(Duration::from_millis(150));
        tx.send(Msg::Stop).unwrap();
        h.join().unwrap();
        let n = frames.lock().unwrap().len();
        assert!((1..=12).contains(&n), "{n} frames for 2000 lines");
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn theme_sets_the_default_colors_of_frames() {
        let (tx, _rx) = channel();
        let mut e = Engine::new(20, 4, 8, 16, tx).unwrap();
        e.theme(&Theme {
            foreground: "#17202c".into(),
            background: "#fff".into(),
            cursor: "#2753c4".into(),
            palette: vec!["#000000".into(), "#c62f3a".into()],
        });
        e.output(b"\x1b[31mx");
        let f = e.frame().unwrap();
        assert_eq!(f.colors, Some((0x17202c, 0xffffff, 0x2753c4)));
        let x = f.rows[0].r.iter().find(|r| r.1 == "x").unwrap();
        assert_eq!(f.styles[x.2 as usize].0, 0xc62f3a);
    }
}
