//! Capture modes for the screenshot toolbar (Shift-Cmd-5) and for `screencapture -i -w` through the control port: a
//! region dragged over the screen, a window picked by its frame (with its shadow), the whole screen, a countdown timer
//! and the pointer. The shell owns the session and does the saving (`Shell::finish_capture`); this module holds the rules,
//! the toolbar layout and the drawing, and has no state of its own outside a `Session`.

use super::ui::{Ui, Weight, CARD_BG, SECONDARY, TINT, WHITE};
use crate::gfx::{pack, Image, Surface};
use crate::layout::Rect;
use crate::symbols::Symbol;
use std::path::{Path, PathBuf};

/// A region with a side shorter than this many points is a click, not a selection.
const MIN_SIDE_PT: f32 = 4.0;
/// A window capture takes this much of the shadow around the frame, in points.
pub const SHADOW_PT: f32 = 24.0;
/// The toolbar's height in points, its distance from the bottom edge when there is no Dock, and the gap kept above a Dock.
const BAR_PT: f32 = 44.0;
const BOTTOM_PT: f32 = 28.0;
const DOCK_GAP_PT: f32 = 12.0;
/// A toolbar button's width and its icon's size, in points.
const BUTTON_PT: f32 = 40.0;
const ICON_PT: f32 = 18.0;
/// Text size of the tooltip that names the button under the pointer, and of the Options rows.
const LABEL_PT: f32 = 13.0;
const MENU_PT: f32 = 15.0;
/// The countdown's numerals, in points.
const COUNT_PT: f32 = 72.0;
/// What is not captured is dimmed: black at 40 per cent (premultiplied, so the colour is zero).
const DIM: u32 = pack(0, 0, 0, 102);

/// What a capture takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A rectangle dragged over the screen.
    Region,
    /// The window under the pointer, with its shadow.
    Window,
    /// The whole screen, on a click anywhere.
    Screen,
}

impl Pick {
    /// The button's label in the toolbar.
    pub fn label(self) -> &'static str {
        match self {
            Pick::Region => "Selection",
            Pick::Window => "Window",
            Pick::Screen => "Screen",
        }
    }
}

/// The timer choices, in seconds, in the order the toolbar cycles them.
pub const TIMERS: [u32; 3] = [0, 5, 10];

/// The timer after `secs` in the cycle Off, 5 s, 10 s, then Off again.
pub fn next_timer(secs: u32) -> u32 {
    let i = TIMERS.iter().position(|&s| s == secs).unwrap_or(0);
    TIMERS[(i + 1) % TIMERS.len()]
}

/// A drag from the press point to the current point, in screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drag {
    start: Option<(i32, i32)>,
    at: (i32, i32),
}

fn px(x: f32, y: f32) -> (i32, i32) {
    (x.round() as i32, y.round() as i32)
}

impl Drag {
    /// Whether a press is held (a drag is under way).
    pub fn active(&self) -> bool {
        self.start.is_some()
    }

    pub fn begin(&mut self, x: f32, y: f32) {
        let p = px(x, y);
        self.start = Some(p);
        self.at = p;
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        if self.start.is_some() {
            self.at = px(x, y);
        }
    }

    /// The rectangle the drag covers, clamped to a `w` by `h` screen. None when no drag is open or a side is under `min` pixels.
    pub fn rect(&self, w: i32, h: i32, min: i32) -> Option<Rect> {
        let (x0, y0) = self.start?;
        let (x1, y1) = self.at;
        let (x0, x1) = (x0.clamp(0, w), x1.clamp(0, w));
        let (y0, y1) = (y0.clamp(0, h), y1.clamp(0, h));
        let (rw, rh) = ((x1 - x0).abs(), (y1 - y0).abs());
        (rw >= min && rh >= min).then(|| Rect { x: x0.min(x1), y: y0.min(y1), w: rw, h: rh })
    }

    /// Ends the drag and returns the rectangle it covered.
    pub fn finish(&mut self, w: i32, h: i32, min: i32) -> Option<Rect> {
        let r = self.rect(w, h, min);
        *self = Drag::default();
        r
    }
}

/// The frontmost of `frames` under the point. `frames` are back to front, as the desk keeps its windows.
pub fn window_at(frames: &[Rect], x: f32, y: f32) -> Option<Rect> {
    frames.iter().rev().find(|r| r.contains(x, y)).copied()
}

/// The rectangle a window capture takes: the frame with its shadow around it, clipped to a `w` by `h` screen.
pub fn with_shadow(r: Rect, s: f32, w: i32, h: i32) -> Rect {
    let pad = (SHADOW_PT * s).round() as i32;
    let x0 = (r.x - pad).max(0);
    let y0 = (r.y - pad).max(0);
    let x1 = (r.x + r.w + pad).min(w);
    let y1 = (r.y + r.h + pad).min(h);
    Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
}

/// A countdown that started at `start` (the shell's clock, in milliseconds) and runs for `secs` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Countdown {
    start: f64,
    secs: u32,
}

impl Countdown {
    pub fn new(start: f64, secs: u32) -> Self {
        Countdown { start, secs }
    }

    /// The seconds still to wait at `t`, rounded up: the timer shows 5, 4, ... 1 and the capture follows.
    pub fn left(&self, t: f64) -> u32 {
        let elapsed = (t - self.start).max(0.0) / 1000.0;
        (self.secs as f64 - elapsed).ceil().max(0.0) as u32
    }

    /// When the countdown ends, in the shell's milliseconds.
    pub fn end(&self) -> f64 {
        self.start + self.secs as f64 * 1000.0
    }

    pub fn done(&self, t: f64) -> bool {
        t >= self.end()
    }
}

/// A toolbar button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pick(Pick),
    /// Records the region dragged (`Region`) or the whole screen (`Screen`) instead of taking a picture.
    Record(Pick),
    /// Opens or closes the Options menu above the bar.
    Options,
    Cancel,
}

impl Tool {
    /// The button's name, shown as its tooltip (the buttons themselves are icons).
    pub fn label(self) -> &'static str {
        match self {
            Tool::Pick(Pick::Region) => "Capture Selected Portion",
            Tool::Pick(Pick::Window) => "Capture Selected Window",
            Tool::Pick(Pick::Screen) => "Capture Entire Screen",
            Tool::Record(Pick::Region) => "Record Selected Portion",
            Tool::Record(_) => "Record Entire Screen",
            Tool::Options => "Options",
            Tool::Cancel => "Close",
        }
    }
}

/// The toolbar's buttons, left to right.
pub const TOOLS: [Tool; 7] = [
    Tool::Pick(Pick::Screen),
    Tool::Pick(Pick::Window),
    Tool::Pick(Pick::Region),
    Tool::Record(Pick::Screen),
    Tool::Record(Pick::Region),
    Tool::Options,
    Tool::Cancel,
];

/// Where the toolbar goes: the free strip of a `w` by `h` screen above a bottom Dock (or beside a side Dock), as
/// `(left, right, bottom)` in px. `dock` is the Dock's frame, `s` the points-to-pixels scale. Derived from the frame, so
/// it follows the Dock's size, position and any screen size.
pub fn free_strip(dock: Option<Rect>, w: i32, h: i32, s: f32) -> (i32, i32, i32) {
    let gap = (DOCK_GAP_PT * s).round() as i32;
    let floor = h - (BOTTOM_PT * s).round() as i32;
    let Some(d) = dock.filter(|d| d.w > 0 && d.h > 0) else { return (0, w, floor) };
    if d.w >= d.h {
        // along the bottom edge (or the top): the bar sits above its top edge
        let bottom = if d.y + d.h / 2 >= h / 2 { (d.y - gap).min(floor) } else { floor };
        (0, w, bottom.max(0))
    } else if d.x + d.w / 2 < w / 2 {
        (d.x + d.w + gap, w, floor)
    } else {
        (0, d.x - gap, floor)
    }
}

/// The toolbar's bar for `n` buttons in a screen of `w` by `h`, `s` px per point, centred in the free strip.
pub fn bar_rect(dock: Option<Rect>, w: i32, h: i32, s: f32, n: usize) -> Rect {
    let pad = (8.0 * s).round() as i32;
    let gap = (6.0 * s).round() as i32;
    let button = (BUTTON_PT * s).round() as i32;
    let bar_h = (BAR_PT * s).round() as i32;
    let bar_w = n as i32 * button + (n as i32 - 1).max(0) * gap + 2 * pad;
    let (left, right, bottom) = free_strip(dock, w, h, s);
    let x = (left + (right - left - bar_w) / 2).clamp(0, (w - bar_w).max(0));
    Rect { x, y: (bottom - bar_h).max(0), w: bar_w, h: bar_h }
}

/// Where a picture or a recording is saved: a folder in the home directory. The Desktop is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaveTo {
    #[default]
    Desktop,
    Documents,
    Pictures,
}

impl SaveTo {
    /// The folder name under the home directory.
    pub fn folder(self) -> &'static str {
        match self {
            SaveTo::Desktop => "Desktop",
            SaveTo::Documents => "Documents",
            SaveTo::Pictures => "Pictures",
        }
    }

    /// The folder for the account whose home is `home`.
    pub fn dir(self, home: &Path) -> PathBuf {
        home.join(self.folder())
    }

    /// The next choice in the Options menu's cycle.
    fn next(self) -> SaveTo {
        match self {
            SaveTo::Desktop => SaveTo::Documents,
            SaveTo::Documents => SaveTo::Pictures,
            SaveTo::Pictures => SaveTo::Desktop,
        }
    }
}

/// The choices of the toolbar's Options menu. They last across capture sessions: the shell keeps them and each session
/// starts from them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    /// the folder a picture or a recording is saved to, in the account's home (the Desktop unless Options says otherwise)
    pub save_to: SaveTo,
    /// seconds to wait before a capture: 0, 5 or 10
    pub timer: u32,
    /// whether the pointer is drawn into a picture
    pub pointer: bool,
    /// whether a recording takes the microphone too, through the audio service
    pub microphone: bool,
}

/// A row of the Options menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionRow {
    SaveTo,
    Timer,
    Pointer,
    Microphone,
}

/// The Options menu's rows, top to bottom.
pub const OPTION_ROWS: [OptionRow; 4] = [OptionRow::SaveTo, OptionRow::Timer, OptionRow::Pointer, OptionRow::Microphone];

impl Prefs {
    /// Applies a menu row: Save To and Timer cycle through their choices, the switches flip.
    pub fn choose(&mut self, row: OptionRow) {
        match row {
            OptionRow::SaveTo => self.save_to = self.save_to.next(),
            OptionRow::Timer => self.timer = next_timer(self.timer),
            OptionRow::Pointer => self.pointer = !self.pointer,
            OptionRow::Microphone => self.microphone = !self.microphone,
        }
    }

    /// The row's text, such as `Timer: 5 s`.
    pub fn text(&self, row: OptionRow) -> String {
        let onoff = |on: bool| if on { "On" } else { "Off" };
        match row {
            OptionRow::SaveTo => format!("Save To: {}", self.save_to.folder()),
            OptionRow::Timer => {
                if self.timer == 0 {
                    "Timer: Off".to_string()
                } else {
                    format!("Timer: {} s", self.timer)
                }
            }
            OptionRow::Pointer => format!("Show Pointer: {}", onoff(self.pointer)),
            OptionRow::Microphone => format!("Microphone: {}", onoff(self.microphone)),
        }
    }
}

/// One open capture session: the toolbar's choices, the drag in progress and a capture waiting on its countdown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Session {
    pub pick: Pick,
    /// The Options menu's choices (timer, pointer, save location, microphone).
    pub prefs: Prefs,
    /// The last pointer position in screen pixels.
    pub at: (f32, f32),
    pub drag: Drag,
    /// The toolbar button under a press; it acts when the press is released on it.
    pub pressed: Option<Tool>,
    /// The chosen rectangle and the countdown that ends in its capture.
    pub pending: Option<(Rect, Countdown)>,
    /// Whether the chosen rectangle is recorded as a movie instead of saved as a picture (Record buttons).
    pub record: bool,
    /// The Options menu is open above the bar.
    pub menu: bool,
    /// Whether the toolbar is shown: Shift-Cmd-5 shows it, Shift-Cmd-4 goes straight to the crosshair without it.
    pub bar: bool,
    /// The Dock's frame, which the toolbar keeps clear of (set by the shell from the Dock's own frame; None without a Dock).
    pub dock: Option<Rect>,
    /// The window the pending window capture picked, without its shadow.
    pub window: Option<Rect>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    /// A fresh session: a selection, with the default options.
    pub fn new() -> Self {
        Session::with_prefs(Prefs::default())
    }

    /// A fresh session that starts with the options the shell kept from the last one.
    pub fn with_prefs(prefs: Prefs) -> Self {
        Session { pick: Pick::Region, prefs, at: (0.0, 0.0), drag: Drag::default(), pressed: None, pending: None, record: false, menu: false, bar: true, dock: None, window: None }
    }

    /// A session for Shift-Cmd-4: the crosshair for a selection, no toolbar. Space turns it into a window pick.
    pub fn crosshair(prefs: Prefs) -> Self {
        Session { bar: false, ..Session::with_prefs(prefs) }
    }

    /// Space: a selection becomes a window pick and back. The whole-screen pick stays.
    pub fn toggle_pick(&mut self) {
        self.drag = Drag::default();
        self.record = false;
        self.pick = match self.pick {
            Pick::Region => Pick::Window,
            Pick::Window => Pick::Region,
            Pick::Screen => Pick::Screen,
        };
    }

    /// A press on the screen (not on the toolbar): a selection starts here.
    pub fn press(&mut self, x: f32, y: f32) {
        self.at = (x, y);
        if self.pick == Pick::Region {
            self.drag.begin(x, y);
        }
    }

    /// The pointer moved while it is down or up.
    pub fn moved(&mut self, x: f32, y: f32) {
        self.at = (x, y);
        self.drag.move_to(x, y);
    }

    /// A release on the screen (not on the toolbar). Returns the rectangle the press chose, if it chose one: a region
    /// too small to be one is a click, a window is the frontmost one under the point, the screen is the whole screen.
    pub fn release(&mut self, x: f32, y: f32, frames: &[Rect], w: i32, h: i32, s: f32) -> Option<Rect> {
        self.at = (x, y);
        let min = (MIN_SIDE_PT * s).round().max(1.0) as i32;
        self.window = None;
        match self.pick {
            Pick::Region => {
                // the release point is the drag's last corner, even when no move came in before it
                self.drag.move_to(x, y);
                self.drag.finish(w, h, min)
            }
            Pick::Window => {
                self.window = window_at(frames, x, y);
                self.window.map(|r| with_shadow(r, s, w, h))
            }
            Pick::Screen => Some(Rect { x: 0, y: 0, w, h }),
        }
    }

    /// Applies a toolbar button to the session. Returns true when the button cancels the session.
    pub fn apply(&mut self, tool: Tool) -> bool {
        match tool {
            Tool::Pick(p) => {
                self.pick = p;
                self.record = false;
            }
            Tool::Record(p) => {
                self.pick = p;
                self.record = true;
            }
            Tool::Options => self.menu = !self.menu,
            Tool::Cancel => return true,
        }
        false
    }

    /// True when the countdown has run out and the capture is due.
    pub fn due(&self, t: f64) -> bool {
        self.pending.is_some_and(|(_, c)| c.done(t))
    }

    /// Whether the toolbar button is the one the session is set to (drawn on the tint).
    pub fn is_on(&self, tool: Tool) -> bool {
        match tool {
            Tool::Pick(p) => self.pick == p && !self.record,
            Tool::Record(p) => self.pick == p && self.record,
            Tool::Options => self.menu,
            Tool::Cancel => false,
        }
    }

    /// The toolbar's bar and its icon buttons for a `w` by `h` screen, as `(bar, buttons)`: a row of `TOOLS`, centred above
    /// the Dock (`Session::dock`). Each button comes with its icon; its name is the tooltip (`Tool::label`). With no
    /// toolbar (Shift-Cmd-4) the bar is empty and there are no buttons.
    pub fn toolbar(&self, ui: &Ui, w: i32, h: i32) -> (Rect, Vec<(Tool, Rect, Image)>) {
        if !self.bar {
            return (Rect { x: 0, y: 0, w: 0, h: 0 }, Vec::new());
        }
        let s = ui.s;
        let bar = bar_rect(self.dock, w, h, s, TOOLS.len());
        let pad = (8.0 * s).round() as i32;
        let gap = (6.0 * s).round() as i32;
        let button = (BUTTON_PT * s).round() as i32;
        let buttons = TOOLS
            .into_iter()
            .enumerate()
            .map(|(i, tool)| {
                let color = if self.is_on(tool) {
                    WHITE
                } else if tool == Tool::Cancel {
                    TINT
                } else {
                    SECONDARY
                };
                let r = Rect { x: bar.x + pad + i as i32 * (button + gap), y: bar.y + pad, w: button, h: bar.h - 2 * pad };
                (tool, r, tool_icon(ui, tool, color))
            })
            .collect();
        (bar, buttons)
    }

    /// The Options menu's card and its rows (with their text), when the menu is open. The card sits above the bar,
    /// centred, one bar-height row per option.
    pub fn menu_rows(&self, ui: &Ui, w: i32, h: i32) -> Option<(Rect, Vec<(OptionRow, Rect, Image)>)> {
        if !self.menu || !self.bar {
            return None;
        }
        let s = ui.s;
        let (bar, _) = self.toolbar(ui, w, h);
        let texts: Vec<(OptionRow, Image)> = OPTION_ROWS.iter().map(|&row| (row, ui.text(&self.prefs.text(row), MENU_PT, Weight::Medium, WHITE))).collect();
        let pad = (8.0 * s).round() as i32;
        let gap = (6.0 * s).round() as i32;
        let side = (16.0 * s).round() as i32;
        let row_h = (BAR_PT * s).round() as i32;
        let text_w = texts.iter().map(|(_, img)| img.w as i32).max().unwrap_or(0);
        let card_w = text_w + 2 * side + 2 * pad;
        let card_h = row_h * OPTION_ROWS.len() as i32 + 2 * pad;
        let card = Rect { x: ((w - card_w) / 2).max(0), y: (bar.y - gap - card_h).max(0), w: card_w, h: card_h };
        let rows = texts
            .into_iter()
            .enumerate()
            .map(|(i, (row, img))| (row, Rect { x: card.x + pad, y: card.y + pad + i as i32 * row_h, w: card_w - 2 * pad, h: row_h }, img))
            .collect();
        Some((card, rows))
    }

    /// The Options menu row under the point, if the menu is open and the point is on one.
    pub fn option_at(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<OptionRow> {
        self.menu_rows(ui, w, h)?.1.into_iter().find(|(_, r, _)| r.contains(x, y)).map(|(row, _, _)| row)
    }

    /// True when the point is on the open Options menu's card.
    pub fn on_menu(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> bool {
        self.menu_rows(ui, w, h).is_some_and(|(card, _)| card.contains(x, y))
    }

    /// The toolbar button under the point, if any.
    pub fn tool_at(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Tool> {
        self.toolbar(ui, w, h).1.into_iter().find(|(_, r, _)| r.contains(x, y)).map(|(t, _, _)| t)
    }

    /// True when the point is on the toolbar's bar (its gaps and padding included).
    pub fn on_toolbar(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> bool {
        self.toolbar(ui, w, h).0.contains(x, y)
    }

    /// Draws the session over the screen: the dim outside what the pick would take, the edge and size of a selection, the
    /// countdown numeral and the toolbar. `frames` are the windows back to front.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, frames: &[Rect], w: i32, h: i32, t: f64) {
        let k = ui.s;
        let target = match self.pick {
            Pick::Region => self.drag.rect(w, h, 1),
            Pick::Window => window_at(frames, self.at.0, self.at.1).map(|r| with_shadow(r, k, w, h)),
            Pick::Screen => None,
        };
        if let Some(r) = target {
            dim_outside(s, r, w, h);
            s.stroke_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, 0.0, k, WHITE);
            if self.pick == Pick::Region {
                let size = ui.text(&format!("{} × {}", r.w, r.h), 13.0, Weight::Medium, WHITE);
                let (lx, ly) = (r.x, (r.y - size.h as i32 - (6.0 * k) as i32).max(0));
                s.fill_rrect(lx as f32, ly as f32, (size.w as f32) + 10.0 * k, size.h as f32 + 4.0 * k, 4.0 * k, CARD_BG.now());
                s.blit(&size, lx + (5.0 * k) as i32, ly + (2.0 * k) as i32, 255, 255);
            }
        }
        if let Some((_, c)) = self.pending {
            let n = ui.text(&c.left(t).max(1).to_string(), COUNT_PT, Weight::Semibold, WHITE);
            s.blit(&n, (w - n.w as i32) / 2, (h - n.h as i32) / 2, 255, 255);
        }
        let (bar, buttons) = self.toolbar(ui, w, h);
        if self.bar {
            s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, bar.h as f32 / 2.0, CARD_BG.now());
        }
        let hover = (!self.menu && !self.drag.active()).then(|| buttons.iter().find(|(_, r, _)| r.contains(self.at.0, self.at.1)).map(|(t, r, _)| (*t, *r))).flatten();
        for (tool, r, img) in buttons {
            if self.is_on(tool) {
                s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, r.h as f32 / 2.0, TINT);
            }
            s.blit(&img, r.x + (r.w - img.w as i32) / 2, r.y + (r.h - img.h as i32) / 2, 255, 255);
        }
        if let Some((tool, r)) = hover {
            // the tooltip names the icon under the pointer, above the bar
            let name = ui.text(tool.label(), LABEL_PT, Weight::Medium, WHITE);
            let (px, py) = ((8.0 * k).round() as i32, (4.0 * k).round() as i32);
            let (tw, th) = (name.w as i32 + 2 * px, name.h as i32 + 2 * py);
            let tx = (r.x + r.w / 2 - tw / 2).clamp(0, (w - tw).max(0));
            let ty = (bar.y - (6.0 * k).round() as i32 - th).max(0);
            s.fill_rrect(tx as f32, ty as f32, tw as f32, th as f32, 6.0 * k, CARD_BG.now());
            s.blit(&name, tx + px, ty + py, 255, 255);
        }
        if let Some((card, rows)) = self.menu_rows(ui, w, h) {
            s.fill_rrect(card.x as f32, card.y as f32, card.w as f32, card.h as f32, 10.0 * k, CARD_BG.now());
            for (_, r, img) in rows {
                s.blit(&img, r.x + (16.0 * k).round() as i32, r.y + (r.h - img.h as i32) / 2, 255, 255);
            }
        }
    }
}

/// A dashed rectangle's outline: three dashes along each side, drawn into `s`.
fn dashed_rect(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, thick: f32, color: u32) {
    let (dx, dy) = (w / 5.0, h / 5.0);
    for i in [0.0, 2.0, 4.0] {
        s.fill_rrect(x + i * dx, y, dx, thick, 0.0, color);
        s.fill_rrect(x + i * dx, y + h - thick, dx, thick, 0.0, color);
        s.fill_rrect(x, y + i * dy, thick, dy, 0.0, color);
        s.fill_rrect(x + w - thick, y + i * dy, thick, dy, 0.0, color);
    }
}

/// A filled dot centred at (`cx`, `cy`), the mark of a recording button.
fn record_dot(s: &mut Surface, cx: f32, cy: f32, r: f32, color: u32) {
    s.fill_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, color);
}

/// The icon of a toolbar button in `color`, `ICON_PT` points square: a dashed square for a selection, a window with its
/// title strip, a screen on its stand (each with a dot when it records), and the symbol set's gear and cross for Options
/// and Close. A symbol the font lacks falls back to a drawn mark, so a button is never blank.
pub fn tool_icon(ui: &Ui, tool: Tool, color: u32) -> Image {
    let n = (ICON_PT * ui.s).round().max(8.0) as usize;
    let nf = n as f32;
    let k = (1.5 * ui.s).max(1.0);
    match tool {
        Tool::Options => {
            if let Some(img) = ui.symbols.render(Symbol::GearOutline, nf, color) {
                return img;
            }
        }
        Tool::Cancel => {
            if let Some(img) = ui.symbols.render(Symbol::Xmark, nf * 0.8, color) {
                return img;
            }
        }
        _ => {}
    }
    let mut img = Image::new(n, n);
    let mut s = Surface { w: n, h: n, stride: n, px: &mut img.px, y0: 0 };
    let (half, corner) = (k / 2.0, 2.0 * ui.s);
    match tool {
        Tool::Pick(Pick::Region) => dashed_rect(&mut s, half, half, nf - k, nf - k, k, color),
        Tool::Record(Pick::Region) => {
            dashed_rect(&mut s, half, half, nf - k, nf - k, k, color);
            record_dot(&mut s, nf / 2.0, nf / 2.0, nf * 0.16, color);
        }
        Tool::Pick(Pick::Window) => {
            s.stroke_rrect(half, nf * 0.12, nf - k, nf * 0.76, corner, k, color);
            s.fill_rrect(half, nf * 0.12 + nf * 0.26, nf - k, k, 0.0, color);
        }
        Tool::Pick(Pick::Screen) | Tool::Record(Pick::Screen) => {
            s.stroke_rrect(half, nf * 0.08, nf - k, nf * 0.62, corner, k, color);
            s.fill_rrect(nf * 0.32, nf * 0.84, nf * 0.36, k, k / 2.0, color);
            s.fill_rrect(nf / 2.0 - half, nf * 0.7, k, nf * 0.14, 0.0, color);
            if tool == Tool::Record(Pick::Screen) {
                record_dot(&mut s, nf / 2.0, nf * 0.39, nf * 0.14, color);
            }
        }
        // the fallbacks: three dots for Options, a cross for Close
        Tool::Options => {
            for i in 0..3 {
                record_dot(&mut s, nf * (0.2 + 0.3 * i as f32), nf / 2.0, nf * 0.08, color);
            }
        }
        Tool::Cancel => {
            for i in 0..n {
                let d = i as f32 + 0.5;
                s.fill_rrect(d - half, d - half, k, k, 0.0, color);
                s.fill_rrect(nf - d - half, d - half, k, k, 0.0, color);
            }
        }
        Tool::Record(Pick::Window) => {}
    }
    img
}

/// A window capture's picture from the composed screen `shot`: the window at `win` (screen px) alone on a transparent
/// ground, its corners rounded by `radius` px and its drop shadow around it, `pad` px of room on every side. `scale`
/// converts the shell's coordinates to `shot`'s pixels when the output is not the shell's size.
pub fn window_image(ui: &Ui, shot: &Image, win: Rect, radius: f32, pad: i32, scale: (f32, f32)) -> Image {
    let sw = ((win.w as f32 * scale.0).round() as i32).max(1);
    let sh = ((win.h as f32 * scale.1).round() as i32).max(1);
    let (sx, sy) = ((win.x as f32 * scale.0).round().max(0.0) as usize, (win.y as f32 * scale.1).round().max(0.0) as usize);
    let body = shot.crop(sx, sy, sw as usize, sh as usize);
    window_picture(ui, &body, radius, pad)
}

/// A window screenshot from the window's own picture `body` (what the window server captured of that window alone): on
/// a transparent ground with its corners rounded by `radius` px and its drop shadow around it, `pad` px of room on every
/// side. Nothing that overlaps the window on the screen can be in it.
pub fn window_picture(ui: &Ui, body: &Image, radius: f32, pad: i32) -> Image {
    let (sw, sh) = (body.w.max(1) as i32, body.h.max(1) as i32);
    let (w, h) = (sw as usize + 2 * pad as usize, sh as usize + 2 * pad as usize);
    let mut out = Image::new(w, h);
    let mut s = Surface { w, h, stride: w, px: &mut out.px, y0: 0 };
    super::desk::soft_shadow(&mut s, ui, pad as f32, pad as f32, sw as f32, sh as f32, radius);
    super::switcher::draw_scaled(&mut s, body, Rect { x: pad, y: pad, w: sw, h: sh }, radius, 255);
    out
}

/// A rectangle given in the shell's pixels, in the pixels of a capture of `from` by `to` size (the output the window
/// server composes need not be the shell's size): scaled and clipped to the picture. None when nothing is left.
pub fn region_in(region: Rect, from: (i32, i32), to: (usize, usize)) -> Option<Rect> {
    if from.0 <= 0 || from.1 <= 0 {
        return None;
    }
    let (kx, ky) = (to.0 as f32 / from.0 as f32, to.1 as f32 / from.1 as f32);
    let x0 = ((region.x as f32 * kx).round() as i32).clamp(0, to.0 as i32);
    let y0 = ((region.y as f32 * ky).round() as i32).clamp(0, to.1 as i32);
    let x1 = (((region.x + region.w) as f32 * kx).round() as i32).clamp(0, to.0 as i32);
    let y1 = (((region.y + region.h) as f32 * ky).round() as i32).clamp(0, to.1 as i32);
    (x1 > x0 && y1 > y0).then_some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// Dims everything of a `w` by `h` screen outside `r`.
fn dim_outside(s: &mut Surface, r: Rect, w: i32, h: i32) {
    let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, (r.x + r.w) as f32, (r.y + r.h) as f32);
    let (fw, fh) = (w as f32, h as f32);
    let bands = [(0.0, 0.0, fw, y0), (0.0, y1, fw, fh - y1), (0.0, y0, x0, y1 - y0), (x1, y0, fw - x1, y1 - y0)];
    for (x, y, bw, bh) in bands {
        if bw > 0.0 && bh > 0.0 {
            s.fill_rrect(x, y, bw, bh, 0.0, DIM);
        }
    }
}

/// Draws a pointer arrow into a capture, its tip at (`x`, `y`) in the image's pixels. `s` is the points-to-pixels scale.
pub fn draw_pointer(img: &mut Image, x: f32, y: f32, s: f32) {
    let (tx, ty) = (x.round() as i32, y.round() as i32);
    let size = (16.0 * s).round().max(1.0) as i32;
    let black = pack(0, 0, 0, 255);
    for row in 0..size {
        let width = 1 + row / 2;
        for col in 0..width {
            let (px, py) = (tx + col, ty + row);
            if px < 0 || py < 0 || px >= img.w as i32 || py >= img.h as i32 {
                continue;
            }
            let i = py as usize * img.w + px as usize;
            img.px[i] = if col == width - 1 || row == size - 1 { black } else { WHITE };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn a_drag_covers_its_corners_in_any_direction() {
        let mut d = Drag::default();
        assert_eq!(d.rect(100, 100, 1), None, "no press, no rectangle");
        d.begin(60.0, 70.0);
        d.move_to(20.0, 30.0);
        assert_eq!(d.rect(100, 100, 1), Some(rect(20, 30, 40, 40)));
        assert_eq!(d.finish(100, 100, 1), Some(rect(20, 30, 40, 40)));
        assert!(!d.active(), "finish ends the drag");
    }

    #[test]
    fn a_drag_is_clamped_to_the_screen_and_small_ones_are_clicks() {
        let mut d = Drag::default();
        d.begin(-10.0, 50.0);
        d.move_to(500.0, 90.0);
        assert_eq!(d.rect(200, 100, 4), Some(rect(0, 50, 200, 40)));
        d.move_to(2.0, 52.0);
        assert_eq!(d.rect(200, 100, 4), None, "a click is not a selection");
    }

    #[test]
    fn the_frontmost_window_under_the_point_wins() {
        let frames = [rect(0, 0, 100, 100), rect(50, 50, 100, 100)];
        assert_eq!(window_at(&frames, 60.0, 60.0), Some(rect(50, 50, 100, 100)), "the later one is in front");
        assert_eq!(window_at(&frames, 10.0, 10.0), Some(rect(0, 0, 100, 100)));
        assert_eq!(window_at(&frames, 300.0, 10.0), None);
    }

    #[test]
    fn a_window_capture_takes_its_shadow_clipped_to_the_screen() {
        assert_eq!(with_shadow(rect(100, 100, 50, 50), 1.0, 1000, 1000), rect(76, 76, 98, 98));
        assert_eq!(with_shadow(rect(0, 0, 50, 50), 1.0, 1000, 1000), rect(0, 0, 74, 74), "clipped at the top left");
        assert_eq!(with_shadow(rect(950, 950, 50, 50), 1.0, 1000, 1000), rect(926, 926, 74, 74), "clipped at the bottom right");
    }

    #[test]
    fn the_timer_cycles_off_five_ten() {
        assert_eq!(next_timer(0), 5);
        assert_eq!(next_timer(5), 10);
        assert_eq!(next_timer(10), 0);
        assert_eq!(next_timer(7), 5, "an unknown value starts the cycle again");
    }

    #[test]
    fn the_countdown_shows_the_seconds_left_and_ends_the_capture() {
        let c = Countdown::new(100_000.0, 5);
        assert_eq!(c.left(100_000.0), 5);
        assert_eq!(c.left(100_500.0), 5);
        assert_eq!(c.left(101_000.0), 4);
        assert_eq!(c.left(104_200.0), 1);
        assert!(!c.done(104_900.0), "five seconds are five thousand milliseconds on the shell clock");
        assert!(c.done(105_000.0));
        assert_eq!(c.end(), 105_000.0);
        assert_eq!(c.left(200_000.0), 0);
        assert!(Countdown::new(3.0, 0).done(3.0), "no timer means the capture is due at once");
    }

    #[test]
    fn a_session_releases_the_rectangle_its_pick_chooses() {
        let frames = [rect(10, 10, 200, 100)];
        let mut s = Session::new();
        s.press(20.0, 20.0);
        assert_eq!(s.release(60.0, 50.0, &frames, 800, 600, 1.0), Some(rect(20, 20, 40, 30)));
        s.pick = Pick::Window;
        assert_eq!(s.release(30.0, 30.0, &frames, 800, 600, 1.0), Some(with_shadow(rect(10, 10, 200, 100), 1.0, 800, 600)));
        assert_eq!(s.release(900.0, 30.0, &frames, 800, 600, 1.0), None, "no window under the point");
        s.pick = Pick::Screen;
        assert_eq!(s.release(1.0, 1.0, &frames, 800, 600, 1.0), Some(rect(0, 0, 800, 600)));
    }

    #[test]
    fn a_region_click_takes_nothing() {
        let mut s = Session::new();
        s.press(20.0, 20.0);
        assert_eq!(s.release(21.0, 21.0, &[], 800, 600, 1.0), None);
    }

    #[test]
    fn toolbar_buttons_apply_their_option() {
        let mut s = Session::new();
        assert!(!s.apply(Tool::Pick(Pick::Window)));
        assert_eq!(s.pick, Pick::Window);
        assert!(!s.menu, "the Options menu starts closed");
        s.apply(Tool::Options);
        assert!(s.menu, "Options opens the menu");
        s.apply(Tool::Options);
        assert!(!s.menu, "and a second press closes it");
        assert!(s.apply(Tool::Cancel), "cancel ends the session");
    }

    #[test]
    fn options_rows_change_the_prefs_they_name() {
        let mut p = Prefs::default();
        assert_eq!(p.save_to, SaveTo::Desktop, "pictures and recordings go to the Desktop first");
        p.choose(OptionRow::SaveTo);
        assert_eq!(p.save_to, SaveTo::Documents);
        p.choose(OptionRow::SaveTo);
        assert_eq!(p.save_to, SaveTo::Pictures);
        p.choose(OptionRow::SaveTo);
        assert_eq!(p.save_to, SaveTo::Desktop);
        p.choose(OptionRow::Timer);
        assert_eq!(p.timer, 5);
        assert_eq!(p.text(OptionRow::Timer), "Timer: 5 s");
        p.choose(OptionRow::Pointer);
        assert!(p.pointer);
        assert_eq!(p.text(OptionRow::Pointer), "Show Pointer: On");
        p.choose(OptionRow::Microphone);
        assert!(p.microphone);
        assert_eq!(p.text(OptionRow::Microphone), "Microphone: On");
        assert_eq!(p.text(OptionRow::SaveTo), "Save To: Desktop");
        p.timer = 0;
        assert_eq!(p.text(OptionRow::Timer), "Timer: Off");
    }

    #[test]
    fn a_new_session_starts_from_the_kept_prefs() {
        let prefs = Prefs { save_to: SaveTo::Documents, timer: 10, pointer: true, microphone: false };
        let s = Session::with_prefs(prefs);
        assert_eq!(s.prefs, prefs);
        assert!(!s.menu);
    }

    #[test]
    fn record_buttons_choose_a_movie_of_the_region_or_the_screen() {
        let mut s = Session::new();
        assert!(!s.record, "a fresh session takes pictures");
        assert!(!s.apply(Tool::Record(Pick::Screen)));
        assert!(s.record);
        assert_eq!(s.pick, Pick::Screen);
        s.apply(Tool::Pick(Pick::Region));
        assert!(!s.record, "a capture button goes back to pictures");
        assert_eq!(s.pick, Pick::Region);
    }

    #[test]
    fn a_due_capture_waits_for_its_countdown() {
        let mut s = Session::new();
        s.pending = Some((rect(0, 0, 10, 10), Countdown::new(50_000.0, 5)));
        assert!(!s.due(54_000.0));
        assert!(s.due(55_000.0));
        assert!(!Session::new().due(0.0));
    }

    #[test]
    fn the_pointer_arrow_is_drawn_with_its_tip_at_the_point() {
        let mut img = Image { w: 4, h: 4, px: vec![0; 16] };
        draw_pointer(&mut img, 0.0, 0.0, 1.0);
        assert_eq!(img.px[0], pack(0, 0, 0, 255), "the tip is outlined");
        assert_eq!(img.px[8], WHITE, "the third row is two pixels wide: the fill, then the outline");
        let mut off = Image { w: 2, h: 2, px: vec![0; 4] };
        draw_pointer(&mut off, 5.0, 5.0, 1.0);
        assert_eq!(off.px, vec![0; 4], "a pointer off the image draws nothing");
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn pictures_go_to_the_desktop_of_the_account_by_default() {
        let home = Path::new("/home/ola");
        assert_eq!(Prefs::default().save_to.dir(home), PathBuf::from("/home/ola/Desktop"), "the Desktop is the default");
        assert_eq!(SaveTo::Documents.dir(home), PathBuf::from("/home/ola/Documents"));
        assert_eq!(SaveTo::Pictures.dir(home), PathBuf::from("/home/ola/Pictures"));
        assert!(SaveTo::Desktop.dir(home).starts_with(home), "under the home, never at the filesystem root");
    }

    #[test]
    fn the_toolbar_sits_above_a_bottom_dock_at_any_screen_size() {
        for (w, h, s) in [(2560, 1440, 1.5625), (1200, 1920, 1.5625), (1280, 720, 1.0), (3840, 2160, 2.0)] {
            // a Dock capsule centred at the bottom, as the desk lays it out
            let dock = Rect { x: w / 6, y: h - (80.0 * s) as i32, w: w * 2 / 3, h: (70.0 * s) as i32 };
            let bar = bar_rect(Some(dock), w, h, s, TOOLS.len());
            assert!(!overlaps(bar, dock), "{w}x{h}: the bar {bar:?} is clear of the Dock {dock:?}");
            assert!(bar.y + bar.h <= dock.y, "{w}x{h}: above the Dock");
            assert!(bar.x >= 0 && bar.x + bar.w <= w && bar.y >= 0, "{w}x{h}: on the screen");
            assert!((bar.x + bar.w / 2 - w / 2).abs() <= 1, "{w}x{h}: centred");
        }
    }

    #[test]
    fn the_toolbar_follows_a_dock_at_the_side_or_none_at_all() {
        let (w, h, s) = (2560, 1440, 1.0);
        let left = Rect { x: 0, y: 60, w: 90, h: 900 };
        let bar = bar_rect(Some(left), w, h, s, TOOLS.len());
        assert!(!overlaps(bar, left) && bar.x >= left.w, "beside a Dock on the left: {bar:?}");
        let right = Rect { x: w - 90, y: 60, w: 90, h: 900 };
        let bar = bar_rect(Some(right), w, h, s, TOOLS.len());
        assert!(!overlaps(bar, right) && bar.x + bar.w <= right.x, "beside a Dock on the right: {bar:?}");
        let bare = bar_rect(None, w, h, s, TOOLS.len());
        assert_eq!(bare.y + bare.h, h - BOTTOM_PT as i32, "without a Dock the bar keeps its own distance from the edge");
        // a hidden Dock leaves a thin strip at the edge: the bar does not drop below its own resting place
        let strip = Rect { x: 0, y: h - 3, w, h: 3 };
        assert_eq!(bar_rect(Some(strip), w, h, s, TOOLS.len()), bare);
    }

    #[test]
    fn the_toolbar_is_a_row_of_icon_buttons_with_a_name_each() {
        let ui = crate::shell::tests::ui();
        let (w, h) = (2560, 1440);
        let dock = Rect { x: 600, y: h - 120, w: 1360, h: 100 };
        let mut sess = Session::new();
        sess.dock = Some(dock);
        let (bar, buttons) = sess.toolbar(&ui, w, h);
        assert_eq!(buttons.len(), TOOLS.len());
        assert!(!overlaps(bar, dock), "the bar does not cover the Dock's icons");
        let mut last_right = bar.x;
        for (tool, r, icon) in &buttons {
            assert!(r.x >= last_right && r.x + r.w <= bar.x + bar.w && r.y >= bar.y && r.y + r.h <= bar.y + bar.h, "{tool:?} {r:?} is inside {bar:?}");
            last_right = r.x + r.w;
            assert!(icon.px.iter().any(|p| p >> 24 != 0), "{tool:?} has an icon, not an empty box");
            assert!(icon.w <= r.w as usize && icon.h <= r.h as usize, "{tool:?}'s icon fits its button");
        }
        // text labels live in the tooltips and are all different
        let names: Vec<&str> = TOOLS.iter().map(|t| t.label()).collect();
        for (i, a) in names.iter().enumerate() {
            assert!(!a.is_empty() && !names[i + 1..].contains(a), "tooltip {a:?} names one button");
        }
        // a press on a button finds it, a press on the Dock does not
        let (tool, r, _) = &buttons[0];
        assert_eq!(sess.tool_at(&ui, w, h, (r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32), Some(*tool));
        assert_eq!(sess.tool_at(&ui, w, h, (dock.x + 10) as f32, (dock.y + 10) as f32), None);
    }

    #[test]
    fn a_crosshair_session_has_no_toolbar_and_space_swaps_selection_and_window() {
        let ui = crate::shell::tests::ui();
        let mut s = Session::crosshair(Prefs::default());
        assert!(!s.bar);
        assert_eq!(s.pick, Pick::Region);
        let (bar, buttons) = s.toolbar(&ui, 1000, 800);
        assert!(buttons.is_empty() && bar.w == 0);
        assert_eq!(s.tool_at(&ui, 1000, 800, 500.0, 760.0), None);
        assert!(!s.on_toolbar(&ui, 1000, 800, 500.0, 760.0));
        s.toggle_pick();
        assert_eq!(s.pick, Pick::Window, "Space turns the selection into a window pick");
        s.toggle_pick();
        assert_eq!(s.pick, Pick::Region, "and back");
        s.pick = Pick::Screen;
        s.toggle_pick();
        assert_eq!(s.pick, Pick::Screen, "the whole-screen pick stays");
    }

    #[test]
    fn a_window_pick_remembers_the_window_without_its_shadow() {
        let frames = [rect(100, 100, 300, 200)];
        let mut s = Session::new();
        s.pick = Pick::Window;
        let taken = s.release(150.0, 150.0, &frames, 1000, 800, 1.0).unwrap();
        assert_eq!(s.window, Some(rect(100, 100, 300, 200)));
        assert_eq!(taken, with_shadow(rect(100, 100, 300, 200), 1.0, 1000, 800));
        assert_eq!(s.release(900.0, 700.0, &frames, 1000, 800, 1.0), None);
        assert_eq!(s.window, None, "a pick that found nothing forgets the last window");
    }

    #[test]
    fn a_region_in_the_shells_pixels_maps_onto_the_composed_picture() {
        let r = rect(100, 50, 200, 100);
        assert_eq!(region_in(r, (1000, 500), (2000, 1000)), Some(rect(200, 100, 400, 200)), "an output twice the shell's size");
        assert_eq!(region_in(r, (1000, 500), (1000, 500)), Some(r), "the same size is unchanged");
        assert_eq!(region_in(rect(900, 450, 300, 300), (1000, 500), (1000, 500)), Some(rect(900, 450, 100, 50)), "clipped to the picture");
        assert_eq!(region_in(rect(1200, 0, 10, 10), (1000, 500), (1000, 500)), None, "outside it nothing is left");
        assert_eq!(region_in(r, (0, 0), (10, 10)), None);
    }

    #[test]
    fn a_window_capture_is_the_window_on_a_clear_ground_with_a_shadow() {
        let ui = crate::shell::tests::ui();
        let shot = Image { w: 400, h: 300, px: vec![pack(200, 100, 50, 255); 400 * 300] };
        let (win, pad) = (rect(100, 80, 200, 120), 40);
        let img = window_image(&ui, &shot, win, 12.0, pad, (1.0, 1.0));
        assert_eq!((img.w, img.h), (280, 200), "the window and the shadow's room on every side");
        let at = |x: usize, y: usize| img.px[y * img.w + x];
        assert_eq!(at(140, 100), pack(200, 100, 50, 255), "the window's own pixels");
        assert_eq!(at(0, 0) >> 24, 0, "the far corner of the margin is clear: the picture has a transparent ground");
        assert!(at(pad as usize, pad as usize) >> 24 < 255, "the window's corner is rounded: its square corner is not opaque");
        assert!(img.px.iter().any(|p| p >> 24 != 0 && p >> 24 != 255), "the shadow is partly transparent");
    }
}
