//! Terminal app: an the terminal core emulator core on a pty running the login shell,
//! drawn with SF Mono into a private canvas that only repaints damaged rows.

pub(crate) mod identity;
pub mod control;
mod pty;
mod render;
mod sessions;
pub mod process;
pub use sessions::Sessions;

use crate::font::Font;
use crate::gfx::{rgba, Image, Surface};
use termcore::cell::{attr, Cell};
use termcore::damage::{row_hash, with_shape, RowCache};
use termcore::keys::{encode, Key, Mods};
pub use termcore::select::Selection;
use termcore::term::{CursorShape, Term};
use pty::Pty;
use render::{paint_row, px, Cursor, GlyphCache, Metrics, THEME};
use std::ffi::c_int;
use std::io;

pub const SHELL: &[&str] = &["/bin/bash", "-l"];

const HISTORY: usize = 10000;
/// Most bytes one `pump` call feeds the emulator, so `yes` or `cat bigfile` cannot starve a frame;
/// the rest stays in the kernel pty buffer (which also applies back-pressure to the writer) and poll()
/// reports the fd readable again on the next loop.
const PUMP_BUDGET: usize = 256 * 1024;

/// Scrollback lines per tab: `BIFROST_TERM_HISTORY` (lines, 100..=200000), default `HISTORY`.
fn history_lines() -> usize {
    std::env::var("BIFROST_TERM_HISTORY").ok().and_then(|v| v.trim().parse::<usize>().ok()).map_or(HISTORY, |n| n.clamp(100, 200_000))
}

/// Maps an on-screen key name to an emulator key.
fn named_key(name: &str) -> Option<Key> {
    Some(match name {
        "Enter" => Key::Enter,
        "Backspace" => Key::Backspace,
        "Tab" => Key::Tab,
        "Escape" => Key::Escape,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Delete" => Key::Delete,
        _ => return None,
    })
}

/// Bytes for a named key, honouring application cursor mode; empty if unknown.
fn key_bytes(name: &str, app_cursor: bool) -> Vec<u8> {
    named_key(name).map(|k| encode(k, Mods::default(), app_cursor)).unwrap_or_default()
}

/// Pasted text as shell input: newlines become CR, and the whole is framed
/// as a bracketed paste when the application asked for it (mode 2004).
/// Control characters other than tab and newlines are dropped, so an ESC in
/// the text cannot end the bracketed paste early and run the rest as typing.
fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let clean: String = text.chars().filter(|&c| matches!(c, '\t' | '\n' | '\r') || !c.is_control()).collect();
    let body = clean.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        [b"\x1b[200~", body.as_bytes(), b"\x1b[201~"].concat()
    } else {
        body.into_bytes()
    }
}

/// The selection highlight: the bifrost's selection blue at 20%.
const HIGHLIGHT: (u8, u8, u8, u8) = (70, 147, 255, 51);
/// Other find matches on screen: systemYellow (ui.rs `SYSTEM_YELLOW`) at 30%.
const MATCH: (u8, u8, u8, u8) = (255, 204, 0, 77);

/// Lower-cased character, as the search compares them.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Columns `start..end` of each case-insensitive occurrence of `query` in one grid row.
fn spans(row: &[Cell], query: &[char]) -> Vec<(usize, usize)> {
    let cells: Vec<(usize, char)> = row
        .iter()
        .enumerate()
        .filter(|(_, c)| c.attrs & attr::SPACER == 0)
        .map(|(x, c)| (x, fold(if c.ch == '\0' { ' ' } else { c.ch })))
        .collect();
    let mut out = Vec::new();
    if query.is_empty() || cells.len() < query.len() {
        return out;
    }
    for i in 0..=cells.len() - query.len() {
        if cells[i..i + query.len()].iter().zip(query).all(|((_, a), b)| a == b) {
            let last = cells[i + query.len() - 1].0;
            let wide = row.get(last).is_some_and(|c| c.attrs & attr::WIDE != 0);
            out.push((cells[i].0, last + if wide { 2 } else { 1 }));
        }
    }
    out
}

fn shape_code(s: CursorShape) -> u8 {
    match s {
        CursorShape::Block => 0,
        CursorShape::Underline => 1,
        CursorShape::Bar => 2,
    }
}

/// The terminal background pixel, for chrome around the grid.
pub fn background() -> u32 {
    px(THEME.bg)
}

/// The background pixel of theme `i`, for chrome around the grid.
pub fn theme_background(i: usize) -> u32 {
    px(termcore::palette::THEMES[i.min(termcore::palette::THEMES.len() - 1)].bg)
}

/// Background-filled canvas for a terminal area.
fn blank(area: (i32, i32, i32, i32)) -> Image {
    let mut canvas = Image::new(area.2.max(1) as usize, area.3.max(1) as usize);
    canvas.px.fill(background());
    canvas
}

pub struct Terminal {
    pty: Pty,
    font_data: &'static [u8],
    theme: usize,
    pub unread: bool,
    /// the program rang the bell while this tab was not on screen (cleared when it is)
    pub bell: bool,
    search_match: Option<termcore::search::Match>,
    /// the folded find query whose visible matches are highlighted (empty: none)
    find: Vec<char>,
    term: Term,
    glyphs: GlyphCache<'static>,
    metrics: Metrics,
    area: (i32, i32, i32, i32),
    canvas: Image,
    damage: RowCache,
    outbox: Vec<u8>,
    /// cells picked by a long-press drag, in visible-row coordinates
    selection: Option<Selection>,
    /// the profile's caret shape: drawn in place of the default block (a program's own shape wins)
    cursor: Option<CursorShape>,
    /// the profile blinks the caret; `blink_on` is the phase the shell last set
    blink: bool,
    blink_on: bool,
    /// a title the user typed (Edit Title); it replaces the program's title and the job name
    custom_title: Option<String>,
    /// View > Allow Mouse Reporting: when off, programs get no mouse or wheel reports
    mouse_reporting: bool,
}

/// Half a blink period of the caret, in milliseconds (on for one half, off for the next).
pub const BLINK_MS: f64 = 500.0;

/// Whether a blinking caret is on at time `t` (ms).
pub fn blink_phase(t: f64) -> bool {
    ((t / BLINK_MS).floor() as i64).rem_euclid(2) == 0
}

/// The next time after `t` (ms) at which the blinking caret changes phase.
pub fn next_blink(t: f64) -> f64 {
    ((t / BLINK_MS).floor() + 1.0) * BLINK_MS
}

impl Terminal {
    /// font_data: SFUIMono.ttf bytes; area: px rect the terminal occupies (x, y, w, h); px: font size in pixels
    pub fn new(font_data: &'static [u8], area: (i32, i32, i32, i32), px: f32) -> io::Result<Self> {
        Self::spawn(font_data, area, px, SHELL)
    }

    /// Like `new` but runs `argv` instead of the login shell.
    pub fn spawn(font_data: &'static [u8], area: (i32, i32, i32, i32), px_size: f32, argv: &[&str]) -> io::Result<Self> {
        let font = Font::parse(font_data).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad terminal font"))?;
        let metrics = Metrics::new(&font, px_size);
        let (cols, rows) = metrics.grid(area.2, area.3);
        let login = identity::current()?;
        let env = identity::env(login.as_ref(), &|k| std::env::var(k).ok());
        let env: Vec<&str> = env.iter().map(String::as_str).collect();
        // the default command is the user's own login shell
        let own = login.as_ref().map(|l| [l.shell.as_str(), "-l"]);
        let argv: &[&str] = match (&own, argv == SHELL) {
            (Some(o), true) => o,
            _ => argv,
        };
        let pty = Pty::spawn(argv, &env, cols as u16, rows as u16, login.as_ref())?;
        let mut term = Term::new(cols, rows, history_lines());
        term.set_default_colors(THEME.fg, THEME.bg);
        Ok(Terminal { pty, font_data, theme: 0, unread: false, bell: false, search_match: None, find: Vec::new(), term, glyphs: GlyphCache::new(font, px_size), metrics, area, canvas: blank(area), damage: RowCache::new(), outbox: Vec::new(), selection: None, cursor: None, blink: false, blink_on: true, custom_title: None, mouse_reporting: true })
    }

    /// The px rect the terminal occupies.
    pub fn area(&self) -> (i32, i32, i32, i32) {
        self.area
    }

    pub fn cols(&self) -> usize {
        self.term.cols()
    }

    /// The cell (column, row) under screen px (x, y), clamped to the grid.
    pub fn cell_at(&self, x: f32, y: f32) -> (usize, usize) {
        let (ox, oy) = self.metrics.cell_origin(0, 0);
        let col = ((x - (self.area.0 + ox) as f32) / self.metrics.cw as f32).floor().max(0.0) as usize;
        let row = ((y - (self.area.1 + oy) as f32) / self.metrics.ch as f32).floor().max(0.0) as usize;
        (col.min(self.term.cols() - 1), row.min(self.term.rows() - 1))
    }

    /// The screen px rect of cell (`col`, `row`).
    pub fn cell_rect(&self, col: usize, row: usize) -> (f32, f32, f32, f32) {
        let (x, y) = self.metrics.cell_origin(col, row);
        ((self.area.0 + x) as f32, (self.area.1 + y) as f32, self.metrics.cw as f32, self.metrics.ch as f32)
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    /// Selects a range of visible cells (None clears).
    pub fn select(&mut self, selection: Option<Selection>) {
        self.selection = selection;
    }

    /// The selected cells as text: rows joined by newlines, trailing blanks
    /// trimmed, wide characters once.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        Some(termcore::select::extract(&sel, self.term.cols(), |y| self.term.visible_row(y)))
    }

    /// Selects the word under cell (`col`, `row`), as a double tap does; a blank selects its own cell.
    pub fn select_word(&mut self, (col, row): (usize, usize)) {
        let (start, end) = termcore::select::word_at(self.term.visible_row(row), col);
        self.selection = Some(Selection { anchor: (start, row), head: (end.max(start + 1) - 1, row) });
    }

    /// Selects the whole visible row `row`, as a triple tap does.
    pub fn select_line(&mut self, row: usize) {
        self.selection = Some(Selection::rows(row, row, self.term.cols()));
    }

    /// Sends an xterm mouse report for `event` at cell `cell` when the program asked for it (?1000/?1002/?1003, ?1006);
    /// true when sent, so the caller skips its own selection handling.
    pub fn report_mouse(&mut self, event: termcore::mouse::MouseEvent, cell: (usize, usize)) -> bool {
        if !self.mouse_reporting || !self.term.mouse_mode().reports(event) { return false; }
        let bytes = termcore::mouse::encode(event, cell.0, cell.1, self.term.mouse_sgr());
        self.write(&bytes);
        true
    }

    /// Types `text` into the shell as a paste (bracketed when enabled).
    pub fn paste(&mut self, text: &str) {
        let bytes = paste_bytes(text, self.term.bracketed_paste());
        self.write(&bytes);
    }

    /// Moves the terminal to a new px rect, re-gridding the emulator and the pty.
    pub fn resize(&mut self, area: (i32, i32, i32, i32)) {
        let (cols, rows) = self.metrics.grid(area.2, area.3);
        self.term.resize(cols, rows);
        // a failed TIOCSWINSZ only leaves the child with the old size
        let _ = self.pty.resize(cols as u16, rows as u16);
        self.canvas = blank(area);
        let background = self.background();
        self.canvas.px.fill(background);
        self.selection = None;
        self.damage.invalidate_all(rows);
        self.area = area;
    }

    /// Pixel height of one text row.
    pub fn line_px(&self) -> f32 {
        self.metrics.ch as f32
    }

    /// Pty master, for poll().
    pub fn fd(&self) -> c_int {
        self.pty.fd()
    }

    /// Drains pty output into the emulator; true if anything changed.
    pub fn pump(&mut self) -> bool {
        let mut buf = [0u8; 16384];
        let mut changed = false;
        // a burst (cat of a huge file) must not starve the UI: take at most PUMP_BUDGET per pump
        let mut fed = 0;
        while fed < PUMP_BUDGET {
            match self.pty.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.term.advance(&buf[..n]);
                    fed += n;
                    changed = true;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        if self.term.take_bell() {
            self.bell = true;
        }
        let reply = self.term.take_response();
        self.send(&reply);
        changed
    }

    /// Keyboard input to the shell; jumps back to the live screen and drops
    /// any selection.
    pub fn write(&mut self, bytes: &[u8]) {
        self.term.reset_view();
        self.selection = None;
        self.send(bytes);
    }

    /// Named keys ("Enter", "Backspace", "Tab", "Escape", arrows, ...) as escape sequences.
    pub fn key(&mut self, name: &str) {
        let bytes = key_bytes(name, self.term.app_cursor());
        self.write(&bytes);
    }

    /// Scrolls the view `lines` back into history (negative = towards live);
    /// the selection, which is tied to the visible rows, is dropped.
    pub fn scroll(&mut self, lines: i32) {
        if lines != 0 {
            self.selection = None;
        }
        // A program that asked for mouse reports (htop, vim, tmux with `mouse on`) gets wheel events, not arrow keys.
        if self.mouse_reporting && self.term.mouse_mode().reports(termcore::mouse::MouseEvent::WheelUp) {
            let ev = if lines > 0 { termcore::mouse::MouseEvent::WheelUp } else { termcore::mouse::MouseEvent::WheelDown };
            let (cols, rows) = (self.term.cols(), self.term.rows());
            for _ in 0..lines.unsigned_abs().min(8) {
                let bytes = termcore::mouse::encode(ev, cols / 2, rows / 2, self.term.mouse_sgr());
                self.write(&bytes);
            }
            return;
        }
        if self.term.alt_screen_active() {
            self.write(&termcore::input::alt_scroll_keys(lines as isize, self.term.app_cursor()));
        } else {
            self.term.scroll_view(lines as isize);
        }
    }

    /// Child still running.
    pub fn alive(&self) -> bool {
        self.pty.alive()
    }

    /// Repaints the rows that changed into the canvas and returns them, without touching the
    /// screen: the shell copies just those rows when nothing around the grid changed.
    pub fn repaint(&mut self) -> Vec<usize> {
        let (cx, cy) = self.term.cursor();
        let (show, shape) = self.cursor_state();
        let cols = self.term.cols();
        let mut changed = Vec::new();
        for y in 0..self.term.rows() {
            let cells = self.term.visible_row(y);
            let cursor = (show && y == cy).then(|| Cursor { col: cx.min(cols - 1), shape });
            let hash = with_shape(row_hash(cells, cursor.map(|c| c.col)), shape_code(shape));
            if self.damage.update(y, hash) {
                paint_row(&mut self.canvas, &mut self.glyphs, (&self.metrics, &termcore::palette::THEMES[self.theme]), y, cells, cols, cursor);
                changed.push(y);
            }
        }
        changed
    }

    /// Screen rows (y, height) of grid row `row` in the canvas, padding included at the edges.
    pub fn row_span(&self, row: usize) -> (i32, i32) {
        let m = &self.metrics;
        let top = if row == 0 { 0 } else { m.pad + row as i32 * m.ch };
        let bottom = if row + 1 == self.term.rows() { self.canvas.h as i32 } else { m.pad + (row as i32 + 1) * m.ch };
        (self.area.1 + top, bottom - top)
    }

    /// Copies canvas rows `y`..`y + h` (screen coordinates) into `s`.
    pub fn copy_rows(&self, s: &mut Surface, y: i32, h: i32) {
        let top = (y - self.area.1).clamp(0, self.canvas.h as i32) as usize;
        let bottom = (y - self.area.1 + h).clamp(0, self.canvas.h as i32) as usize;
        if bottom > top {
            let w = self.canvas.w;
            let rows = crate::gfx::View { w, h: bottom - top, px: &self.canvas.px[top * w..bottom * w] };
            s.copy(rows, self.area.0, self.area.1 + top as i32);
        }
    }

    /// Feeds bytes as if the child wrote them.
    #[cfg(any(test, feature = "testkit"))]
    pub fn feed(&mut self, bytes: &[u8]) {
        self.term.advance(bytes);
    }

    /// Sets the profile caret: `shape` replaces the default block (None keeps the program's choice) and `blink`
    /// hides the caret in the off phase of `blink_phase`.
    pub fn set_cursor(&mut self, shape: Option<CursorShape>, blink: bool) {
        self.cursor = shape;
        self.blink = blink;
    }

    /// Moves the caret blink to phase `on`; true when that changes what is drawn.
    pub fn set_blink_phase(&mut self, on: bool) -> bool {
        let changed = self.blink && self.blink_on != on;
        self.blink_on = on;
        changed
    }

    /// Whether the caret is drawn now, and its shape: the profile shape replaces only the program's default block.
    fn cursor_state(&self) -> (bool, CursorShape) {
        let shape = match (self.term.cursor_shape(), self.cursor) {
            (CursorShape::Block, Some(profile)) => profile,
            (own, _) => own,
        };
        (self.term.cursor_visible() && (!self.blink || self.blink_on), shape)
    }

    /// Nothing is drawn over the grid (no selection, no find highlights).
    pub fn plain(&self) -> bool {
        self.selection.is_none() && self.find.is_empty()
    }

    /// Repaints damaged rows into the canvas and copies it into the area.
    pub fn draw(&mut self, s: &mut Surface) {
        let (cx, cy) = self.term.cursor();
        let (show, shape) = self.cursor_state();
        let cols = self.term.cols();
        for y in 0..self.term.rows() {
            let cells = self.term.visible_row(y);
            let cursor = (show && y == cy).then(|| Cursor { col: cx.min(cols - 1), shape });
            let hash = with_shape(row_hash(cells, cursor.map(|c| c.col)), shape_code(shape));
            if self.damage.update(y, hash) {
                paint_row(&mut self.canvas, &mut self.glyphs, (&self.metrics, &termcore::palette::THEMES[self.theme]), y, cells, cols, cursor);
            }
        }
        s.copy(self.canvas.view(), self.area.0, self.area.1);
        if !self.find.is_empty() {
            let (r, g, b, a) = MATCH;
            for y in 0..self.term.rows() {
                for (start, end) in spans(self.term.visible_row(y), &self.find) {
                    let (x0, y0, cw, ch) = self.cell_rect(start, y);
                    s.fill_rrect(x0, y0, (end - start) as f32 * cw, ch, 0.0, rgba(r, g, b, a));
                }
            }
        }
        if let Some(sel) = self.selection {
            let (r, g, b, a) = HIGHLIGHT;
            for y in 0..self.term.rows() {
                if let Some((start, end)) = sel.span_in_row(y, cols) {
                    let (x0, y0, cw, ch) = self.cell_rect(start, y);
                    s.fill_rrect(x0, y0, (end - start) as f32 * cw, ch, 0.0, rgba(r, g, b, a));
                }
            }
        }
    }

    pub fn background(&self) -> u32 { px(termcore::palette::THEMES[self.theme].bg) }

    /// Tab caption: the title the program set, else the running job's name, else `shell N`.
    pub fn title(&self, index: usize) -> String {
        if let Some(title) = &self.custom_title {
            return title.clone();
        }
        termcore::tabs::label(self.term.title(), self.job().as_deref(), index as u32 + 1)
    }

    /// Sets the tab title the user typed; blank text goes back to the automatic title.
    pub fn set_custom_title(&mut self, title: &str) {
        let title = title.trim();
        self.custom_title = (!title.is_empty()).then(|| title.chars().take(64).collect());
    }

    pub fn has_custom_title(&self) -> bool {
        self.custom_title.is_some()
    }

    /// Turns mouse and wheel reports to the running program on or off.
    pub fn set_mouse_reporting(&mut self, on: bool) {
        self.mouse_reporting = on;
    }

    /// The job running in the foreground ("vim"), None at the prompt.
    pub fn job(&self) -> Option<String> {
        self.pty.job()
    }

    pub fn appearance(&mut self, theme: usize, px_size: f32) {
        self.theme = theme.min(termcore::palette::THEMES.len() - 1);
        let theme = &termcore::palette::THEMES[self.theme];
        self.term.set_default_colors(theme.fg, theme.bg);
        let font = Font::parse(self.font_data).expect("validated terminal font");
        self.metrics = Metrics::new(&font, px_size);
        self.glyphs = GlyphCache::new(font, px_size);
        self.resize(self.area);
    }

    pub fn find(&mut self, query: &str, dir: termcore::search::Direction, restart: bool) -> bool {
        self.find = query.chars().map(fold).collect();
        let hit = termcore::search::find(self.term.line_count(), |i| self.term.line(i), query,
            if restart { None } else { self.search_match }, dir);
        self.search_match = hit;
        self.selection = hit.map(|m| {
            let y = self.term.show_line(m.line);
            Selection { anchor: (m.start, y), head: (m.end.saturating_sub(1), y) }
        });
        hit.is_some()
    }

    /// (position of the current match, matches in all) of `query`; (0, 0) when there are none. Capped at 999.
    pub fn match_position(&self, query: &str) -> (usize, usize) {
        let find = |from| termcore::search::find(self.term.line_count(), |i| self.term.line(i), query, from, termcore::search::Direction::Newer);
        let Some(first) = find(None) else { return (0, 0) };
        let (mut total, mut at, mut m) = (1, 0, first);
        if Some(m) == self.search_match {
            at = 1;
        }
        while total < 999 {
            match find(Some(m)) {
                Some(next) if next != first => {
                    total += 1;
                    m = next;
                    if Some(m) == self.search_match {
                        at = total;
                    }
                }
                _ => break,
            }
        }
        (at, total)
    }

    /// Drops the highlights of the other matches; the current one stays selected.
    pub fn clear_find(&mut self) { self.find.clear(); }
    pub fn clear_history(&mut self) { self.term.clear_history(); self.selection = None; self.search_match = None; self.find.clear(); }
    pub fn reset(&mut self) { self.term.advance(b"\x1bc"); self.clear_history(); self.damage.invalidate_all(self.term.rows()); }
    pub fn select_all(&mut self) {
        self.selection = Some(Selection { anchor: (0, 0), head: (self.term.cols() - 1, self.term.rows() - 1) });
    }
    pub fn transcript(&self) -> String {
        (0..self.term.line_count()).map(|i| termcore::select::extract(
            &Selection { anchor: (0, 0), head: (self.term.cols() - 1, 0) }, self.term.cols(), |_| self.term.line(i)) + "\n").collect()
    }
    pub fn take_clipboard(&mut self) -> Option<String> { self.term.take_clipboard() }
    pub fn cwd(&self) -> Option<String> { self.pty.cwd() }

    /// The visible row `row` as text, trailing blanks dropped (for link detection on a tap).
    pub fn row_text(&self, row: usize) -> String {
        self.term.visible_row(row).iter().map(|c| c.ch).collect::<String>().trim_end().to_string()
    }

    /// The last `n` non-blank visible lines, trailing spaces trimmed (for
    /// the Today View widget).
    pub fn tail(&self, n: usize) -> Vec<String> {
        let lines: Vec<String> = (0..self.term.rows()).map(|y| self.term.visible_row(y).iter().map(|c| c.ch).collect::<String>().trim_end().to_string()).collect();
        let end = lines.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1);
        lines[end.saturating_sub(n)..end].to_vec()
    }

    /// The visible (possibly scrolled-back) rows as text, one line each.
    #[cfg(test)]
    pub(crate) fn viewport_text(&self) -> String {
        (0..self.term.rows()).map(|y| self.term.visible_row(y).iter().map(|c| c.ch).collect::<String>() + "\n").collect()
    }

    /// Queues `bytes` for the pty and flushes what it takes without blocking.
    fn send(&mut self, bytes: &[u8]) {
        self.outbox.extend_from_slice(bytes);
        while !self.outbox.is_empty() {
            match self.pty.write(&self.outbox) {
                Ok(n) => drop(self.outbox.drain(..n)),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_follow_xterm() {
        assert_eq!(key_bytes("Enter", false), b"\r");
        assert_eq!(key_bytes("Backspace", false), b"\x7f");
        assert_eq!(key_bytes("Tab", false), b"\t");
        assert_eq!(key_bytes("Escape", false), b"\x1b");
        assert_eq!(key_bytes("Up", false), b"\x1b[A");
        assert_eq!(key_bytes("Down", false), b"\x1b[B");
        assert_eq!(key_bytes("Right", false), b"\x1b[C");
        assert_eq!(key_bytes("Left", false), b"\x1b[D");
        assert_eq!(key_bytes("Left", true), b"\x1bOD");
        assert_eq!(key_bytes("Delete", false), b"\x1b[3~");
        assert!(key_bytes("Hyper", false).is_empty());
    }

    #[test]
    fn paste_turns_newlines_into_cr_and_brackets_on_request() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("x", true), b"\x1b[200~x\x1b[201~");
        assert_eq!(paste_bytes("", true), b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn paste_drops_control_characters_that_could_end_the_frame() {
        assert_eq!(paste_bytes("a\x1b[201~b", true), b"\x1b[200~a[201~b\x1b[201~");
        assert_eq!(paste_bytes("x\x07\x7fy\tz", false), "xy\tz".as_bytes());
    }

    #[test]
    fn shapes_get_distinct_damage_codes() {
        let codes = [CursorShape::Block, CursorShape::Underline, CursorShape::Bar].map(shape_code);
        assert!(codes[0] != codes[1] && codes[1] != codes[2] && codes[0] != codes[2]);
    }

    fn font() -> &'static [u8] {
        Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice())
    }

    fn pump_until(t: &mut Terminal, want: &str) -> String {
        let start = std::time::Instant::now();
        while !t.viewport_text().contains(want) && start.elapsed().as_secs() < 5 {
            crate::bare::sys::wait(&[t.fd()], 50);
            t.pump();
        }
        t.viewport_text()
    }

    #[test]
    fn shell_output_lands_on_screen_and_draws() {
        let mut t = Terminal::spawn(font(), (10, 20, 400, 300), 20.0, &["/bin/sh", "-c", "printf hello"]).unwrap();
        assert!(pump_until(&mut t, "hello").lines().next().unwrap().starts_with("hello"));
        let mut buf = vec![0u32; 420 * 330];
        t.draw(&mut Surface { w: 420, h: 330, stride: 420, px: &mut buf, y0: 0 });
        let area: Vec<u32> = (20..320).flat_map(|y| buf[y * 420 + 10..y * 420 + 410].to_vec()).collect();
        assert!(area.iter().any(|&p| p == px(THEME.fg)), "text drawn");
        assert_eq!(buf[0], 0, "outside the area untouched");
        let start = std::time::Instant::now();
        while t.alive() && start.elapsed().as_secs() < 5 {
            t.pump();
        }
        assert!(!t.alive());
    }

    #[test]
    fn find_tints_every_visible_match_and_keeps_the_current_one_selected() {
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "printf 'Needle one\\nneedle two\\n'; read x"]).unwrap();
        pump_until(&mut t, "two");
        assert!(t.find("NEEDLE", termcore::search::Direction::Older, true));
        assert!(!t.plain(), "highlights keep the grid off the quiet repaint path");
        let mut buf = vec![0u32; 420 * 330];
        t.draw(&mut Surface { w: 420, h: 330, stride: 420, px: &mut buf, y0: 0 });
        // one line holds the current match (blue selection), the other the tinted one
        let at = |row| {
            let (x, y, ..) = t.cell_rect(0, row);
            buf[(y as usize + 2) * 420 + x as usize + 2]
        };
        let (a, b) = (at(0), at(1));
        assert!(a != px(THEME.bg) && b != px(THEME.bg), "both matches are drawn");
        assert_ne!(a, b, "the current match (selection blue) and the other match (yellow tint) differ");
        t.clear_find();
        assert!(t.selection().is_some() && !t.plain(), "the current match stays selected");
        t.select(None);
        assert!(t.plain());
    }

    #[test]
    fn a_bell_is_remembered_until_the_tab_is_shown() {
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "printf '\\a'; read x"]).unwrap();
        let start = std::time::Instant::now();
        while !t.bell && start.elapsed().as_secs() < 5 {
            crate::bare::sys::wait(&[t.fd()], 50);
            t.pump();
        }
        assert!(t.bell, "the BEL byte sets the flag");
        assert!(!t.viewport_text().contains('\u{7}'), "a bell is not printed");
    }

    #[test]
    fn caret_blinks_on_half_periods_and_reports_phase_changes() {
        assert!(blink_phase(0.0) && blink_phase(499.0));
        assert!(!blink_phase(500.0) && !blink_phase(999.0));
        assert!(blink_phase(1000.0));
        assert_eq!(next_blink(0.0), 500.0);
        assert_eq!(next_blink(499.9), 500.0);
        assert_eq!(next_blink(500.0), 1000.0);
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "read x"]).unwrap();
        // the caret starts in phase "on", so the steady probe uses `true`; `false` here would move the phase before the blink is set
        assert!(!t.set_blink_phase(true), "a steady caret has no phase to change");
        t.set_cursor(None, true);
        assert!(t.set_blink_phase(false), "the blinking caret changes phase");
        assert!(!t.set_blink_phase(false), "same phase, nothing to draw");
        assert!(t.set_blink_phase(true));
    }

    #[test]
    fn pump_is_bounded_per_call_under_a_flood() {
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "yes"]).unwrap();
        // wait until `yes` has written something (a loaded machine can take longer than a fixed sleep)
        crate::bare::sys::wait(&[t.fd()], 5000);
        let t0 = std::time::Instant::now();
        assert!(t.pump());
        assert!(t0.elapsed() < std::time::Duration::from_millis(500), "one pump call must return promptly");
    }

    #[test]
    #[ignore]
    fn bench_term_flood_pump() {
        let mut t = Terminal::spawn(font(), (0, 0, 800, 600), 16.0, &["/bin/sh", "-c", "yes 'the quick brown fox jumps over the lazy dog'"]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let t0 = std::time::Instant::now();
        for _ in 0..20 { t.pump(); }
        println!("BENCH term_pump_256k {:.3} ms", t0.elapsed().as_secs_f64() * 1000.0 / 20.0);
        let t1 = std::time::Instant::now();
        for _ in 0..20 { t.pump(); t.repaint(); }
        println!("BENCH term_pump_repaint {:.3} ms", t1.elapsed().as_secs_f64() * 1000.0 / 20.0);
    }

    #[test]
    #[ignore]
    fn bench_term_repaint_idle() {
        let mut t = Terminal::spawn(font(), (0, 0, 800, 600), 16.0, &["/bin/sh", "-c", "read x"]).unwrap();
        t.feed(b"hello\r\nworld\r\n");
        t.repaint();
        let t0 = std::time::Instant::now();
        for _ in 0..1000 { std::hint::black_box(t.repaint()); }
        println!("BENCH term_repaint_undamaged {:.4} ms", t0.elapsed().as_secs_f64());
    }

    #[test]
    fn a_typed_title_replaces_the_automatic_one_until_cleared() {
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "read x"]).unwrap();
        t.set_custom_title("  build  ");
        assert_eq!(t.title(0), "build");
        assert!(t.has_custom_title());
        t.set_custom_title("   ");
        assert!(!t.has_custom_title());
        assert_eq!(t.title(1), "shell 2");
    }

    #[test]
    fn mouse_reports_stop_when_reporting_is_off() {
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "read x"]).unwrap();
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        t.set_mouse_reporting(false);
        assert!(!t.report_mouse(termcore::mouse::MouseEvent::WheelUp, (1, 1)));
        t.set_mouse_reporting(true);
        assert!(t.report_mouse(termcore::mouse::MouseEvent::WheelUp, (1, 1)));
    }

    #[test]
    fn an_idle_shell_is_titled_by_its_number() {
        let t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "read x"]).unwrap();
        assert_eq!(t.job(), None, "the shell itself is in the foreground");
        assert_eq!(t.title(1), "shell 2");
    }

    #[test]
    fn keys_reach_the_shell_and_scroll_is_clamped() {
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "read x; echo got:$x"]).unwrap();
        t.write(b"hi");
        t.key("Enter");
        pump_until(&mut t, "got:hi");
        assert!(t.viewport_text().contains("got:hi"));
        t.scroll(5);
        assert_eq!(t.term.view_offset(), 0, "no history yet");
    }

    #[test]
    fn selection_maps_pixels_to_cells_and_extracts_trimmed_text() {
        let mut t = Terminal::spawn(font(), (10, 20, 400, 300), 20.0, &["/bin/sh", "-c", "printf 'hello   \\nwo\\xe4\\xb8\\x96rld'; read x"]).unwrap();
        pump_until(&mut t, "rld");
        let (x0, y0, cw, ch) = t.cell_rect(0, 0);
        assert_eq!((x0, y0), (10.0 + t.metrics.pad as f32, 20.0 + t.metrics.pad as f32));
        assert_eq!(t.cell_at(x0 + cw * 2.5, y0 + ch * 1.5), (2, 1));
        assert_eq!(t.cell_at(-100.0, -100.0), (0, 0), "clamped at the origin");
        assert_eq!(t.cell_at(1e6, 1e6), (t.term.cols() - 1, t.term.rows() - 1), "clamped at the far corner");
        assert_eq!(t.selected_text(), None);
        t.select(Some(Selection { anchor: (0, 0), head: (9, 1) }));
        assert_eq!(t.selected_text().as_deref(), Some("hello\nwo世rld"), "trailing blanks trimmed, the wide char once");
        let mut buf = vec![0u32; 420 * 330];
        t.draw(&mut Surface { w: 420, h: 330, stride: 420, px: &mut buf, y0: 0 });
        let (hx, hy, ..) = t.cell_rect(7, 0);
        let tinted = buf[(hy as usize + 2) * 420 + hx as usize + 2];
        assert_ne!(tinted, px(THEME.bg), "a blank selected cell is tinted");
        assert!((tinted >> 16) & 0xff > tinted & 0xff, "towards blue");
        t.scroll(0);
        assert!(t.selection().is_some(), "a zero scroll keeps it");
        t.write(b"");
        assert_eq!(t.selection(), None, "typing drops it");
    }

    #[test]
    fn paste_is_bracketed_when_the_application_asks() {
        let mut t = Terminal::spawn(font(), (0, 0, 600, 300), 20.0, &["/bin/sh", "-c", "/bin/stty raw -echo; printf '\\033[?2004h'; echo ready; /bin/head -c 15 | /bin/od -An -c; read x"]).unwrap();
        pump_until(&mut t, "ready");
        assert!(t.term.bracketed_paste());
        t.paste("a\nb");
        let out = pump_until(&mut t, "1   ~");
        let tokens: Vec<&str> = out.split_whitespace().skip_while(|&w| w != "033").collect();
        assert_eq!(tokens, ["033", "[", "2", "0", "0", "~", "a", "\\r", "b", "033", "[", "2", "0", "1", "~"], "{out}");
    }

    #[test]
    fn resize_regrids_and_tells_the_child() {
        let mut t = Terminal::spawn(font(), (0, 0, 400, 300), 20.0, &["/bin/sh", "-c", "read x; /bin/stty size"]).unwrap();
        let (cols, rows) = (t.term.cols(), t.term.rows());
        t.resize((0, 50, 400, 600));
        assert_eq!((t.term.cols(), t.term.rows()), (cols, t.metrics.grid(400, 600).1));
        assert!(t.term.rows() > rows);
        assert_eq!((t.canvas.w, t.canvas.h), (400, 600));
        t.key("Enter");
        let want = format!("{} {}", t.term.rows(), cols);
        let screen = pump_until(&mut t, &want);
        assert!(screen.contains(&want), "{want} {screen:?}");
        let mut buf = vec![0u32; 400 * 650];
        t.draw(&mut Surface { w: 400, h: 650, stride: 400, px: &mut buf, y0: 0 });
        assert_eq!(buf[0], 0, "drawn at the new origin");
        assert_eq!(buf[60 * 400], px(THEME.bg));
    }
}
