//! Capture modes for the screenshot toolbar (Shift-Cmd-5) and for `screencapture -i -w` through the control port: a
//! region dragged over the screen, a window picked by its frame (with its shadow), the whole screen, a countdown timer
//! and the pointer. The shell owns the session and does the saving (`Shell::finish_capture`); this module holds the rules,
//! the toolbar layout and the drawing, and has no state of its own outside a `Session`.

use super::ui::{Ui, Weight, CARD_BG, SECONDARY, TINT, WHITE};
use crate::gfx::{pack, Image, Surface};
use crate::layout::Rect;

/// A region with a side shorter than this many points is a click, not a selection.
const MIN_SIDE_PT: f32 = 4.0;
/// A window capture takes this much of the shadow around the frame, in points.
const SHADOW_PT: f32 = 24.0;
/// The toolbar's height and its distance from the bottom edge, in points.
const BAR_PT: f32 = 44.0;
const BOTTOM_PT: f32 = 28.0;
/// Button label size in points.
const LABEL_PT: f32 = 15.0;
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

/// A countdown that started at `start` (seconds on the shell's clock) and runs for `secs`.
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
        let elapsed = (t - self.start).max(0.0);
        (self.secs as f64 - elapsed).ceil().max(0.0) as u32
    }

    /// When the countdown ends.
    pub fn end(&self) -> f64 {
        self.start + self.secs as f64
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

/// Where a recording is saved: a folder in the home directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaveTo {
    #[default]
    Desktop,
    Documents,
}

impl SaveTo {
    /// The folder name under the home directory.
    pub fn folder(self) -> &'static str {
        match self {
            SaveTo::Desktop => "Desktop",
            SaveTo::Documents => "Documents",
        }
    }
}

/// The choices of the toolbar's Options menu. They last across capture sessions: the shell keeps them and each session
/// starts from them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    /// the folder a recording is saved to
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
            OptionRow::SaveTo => {
                self.save_to = match self.save_to {
                    SaveTo::Desktop => SaveTo::Documents,
                    SaveTo::Documents => SaveTo::Desktop,
                }
            }
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
        Session { pick: Pick::Region, prefs, at: (0.0, 0.0), drag: Drag::default(), pressed: None, pending: None, record: false, menu: false }
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
        match self.pick {
            Pick::Region => {
                // the release point is the drag's last corner, even when no move came in before it
                self.drag.move_to(x, y);
                self.drag.finish(w, h, min)
            }
            Pick::Window => window_at(frames, x, y).map(|r| with_shadow(r, s, w, h)),
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

    /// The button labels and the toolbar's rectangle for a `w` by `h` screen, as `(bar, buttons)`. The buttons come
    /// with their labels, laid out bottom centre.
    pub fn toolbar(&self, ui: &Ui, w: i32, h: i32) -> (Rect, Vec<(Tool, Rect, Image)>) {
        let s = ui.s;
        let mut items: Vec<(Tool, Image)> = Vec::new();
        for p in [Pick::Region, Pick::Window, Pick::Screen] {
            let color = if self.pick == p && !self.record { WHITE } else { SECONDARY };
            items.push((Tool::Pick(p), ui.text(p.label(), LABEL_PT, Weight::Medium, color)));
        }
        for p in [Pick::Region, Pick::Screen] {
            let color = if self.pick == p && self.record { WHITE } else { SECONDARY };
            let label = if p == Pick::Region { "Record Selection" } else { "Record Screen" };
            items.push((Tool::Record(p), ui.text(label, LABEL_PT, Weight::Medium, color)));
        }
        let options = if self.menu { WHITE } else { SECONDARY };
        items.push((Tool::Options, ui.text("Options", LABEL_PT, Weight::Medium, options)));
        items.push((Tool::Cancel, ui.text("Cancel", LABEL_PT, Weight::Medium, TINT)));

        let pad = (8.0 * s).round() as i32;
        let gap = (6.0 * s).round() as i32;
        let side = (16.0 * s).round() as i32;
        let bar_h = (BAR_PT * s).round() as i32;
        let btn_h = bar_h - 2 * pad;
        let widths: Vec<i32> = items.iter().map(|(_, img)| img.w as i32 + 2 * side).collect();
        let inner = widths.iter().sum::<i32>() + gap * (widths.len() as i32 - 1);
        let bar_w = inner + 2 * pad;
        let bar = Rect { x: (w - bar_w) / 2, y: h - (BOTTOM_PT * s).round() as i32 - bar_h, w: bar_w, h: bar_h };
        let mut x = bar.x + pad;
        let mut buttons = Vec::with_capacity(items.len());
        for ((tool, img), bw) in items.into_iter().zip(widths) {
            buttons.push((tool, Rect { x, y: bar.y + pad, w: bw, h: btn_h }, img));
            x += bw + gap;
        }
        (bar, buttons)
    }

    /// The Options menu's card and its rows (with their text), when the menu is open. The card sits above the bar,
    /// centred, one bar-height row per option.
    pub fn menu_rows(&self, ui: &Ui, w: i32, h: i32) -> Option<(Rect, Vec<(OptionRow, Rect, Image)>)> {
        if !self.menu {
            return None;
        }
        let s = ui.s;
        let (bar, _) = self.toolbar(ui, w, h);
        let texts: Vec<(OptionRow, Image)> = OPTION_ROWS.iter().map(|&row| (row, ui.text(&self.prefs.text(row), LABEL_PT, Weight::Medium, WHITE))).collect();
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
        s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, bar.h as f32 / 2.0, CARD_BG.now());
        for (tool, r, img) in buttons {
            let on = match tool {
                Tool::Pick(p) => self.pick == p && !self.record,
                Tool::Record(p) => self.pick == p && self.record,
                Tool::Options => self.menu,
                _ => false,
            };
            if on {
                s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, r.h as f32 / 2.0, TINT);
            }
            s.blit(&img, r.x + (r.w - img.w as i32) / 2, r.y + (r.h - img.h as i32) / 2, 255, 255);
        }
        if let Some((card, rows)) = self.menu_rows(ui, w, h) {
            s.fill_rrect(card.x as f32, card.y as f32, card.w as f32, card.h as f32, 10.0 * k, CARD_BG.now());
            for (_, r, img) in rows {
                s.blit(&img, r.x + (16.0 * k).round() as i32, r.y + (r.h - img.h as i32) / 2, 255, 255);
            }
        }
    }
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
        let c = Countdown::new(100.0, 5);
        assert_eq!(c.left(100.0), 5);
        assert_eq!(c.left(100.5), 5);
        assert_eq!(c.left(101.0), 4);
        assert_eq!(c.left(104.2), 1);
        assert!(!c.done(104.9));
        assert!(c.done(105.0));
        assert_eq!(c.left(200.0), 0);
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
        assert_eq!(p.save_to, SaveTo::Desktop, "recordings go to the Desktop first");
        p.choose(OptionRow::SaveTo);
        assert_eq!(p.save_to, SaveTo::Documents);
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
        s.pending = Some((rect(0, 0, 10, 10), Countdown::new(50.0, 5)));
        assert!(!s.due(54.0));
        assert!(s.due(55.0));
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
}
