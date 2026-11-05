//! The arrangement of the displays the desktop spans: where each output sits in the shell's pixel space, which one is
//! the main display, and the visible frame of each (the part windows may use).
//!
//! The reference behaviour: the menu bar and the Dock belong to one display, the main one. Every other display has its
//! whole frame as its visible frame, with no menu bar or Dock reservation. A window is placed, zoomed (Fill), tiled
//! and kept on screen against the visible frame of the display it is on, the one that shows most of it. `Metrics`
//! (`desk.rs`) describe the main display: its size and the menu bar and Dock it reserves.
//!
//! An empty or single-output arrangement is the one-display desktop the shell always had: every question is answered
//! with `Desk::area(m)`, so a machine with one panel behaves exactly as before. The backend reports the outputs with
//! `Shell::set_outputs`.
use super::desk::{Desk, Metrics, Rect};

/// One output: a name and its frame in the shell's pixel space (the main display's top-left is the origin when the
/// backend reports it that way; other outputs may sit at negative coordinates).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    pub frame: Rect,
}

impl Output {
    pub fn new(name: &str, frame: Rect) -> Self {
        Output { name: name.into(), frame }
    }
}

/// The outputs and which of them is main (the one that holds the menu bar and the Dock).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arrangement {
    outputs: Vec<Output>,
    main: usize,
}

fn overlap(a: Rect, b: Rect) -> i64 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if w > 0 && h > 0 { w as i64 * h as i64 } else { 0 }
}

fn centre_distance(a: Rect, b: Rect) -> i64 {
    let dx = (a.x as i64 * 2 + a.w as i64) - (b.x as i64 * 2 + b.w as i64);
    let dy = (a.y as i64 * 2 + a.h as i64) - (b.y as i64 * 2 + b.h as i64);
    dx * dx + dy * dy
}

impl Arrangement {
    /// The outputs, with `main` as the main display (an index past the end reads as the first).
    pub fn new(outputs: Vec<Output>, main: usize) -> Self {
        let main = if main < outputs.len() { main } else { 0 };
        Arrangement { outputs, main }
    }

    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    pub fn main_index(&self) -> usize {
        self.main
    }

    /// True when there is more than one display; a single one is answered by `Desk::area`.
    pub fn is_multi(&self) -> bool {
        self.outputs.len() > 1
    }

    /// The display that shows most of `r`. A rectangle on no display belongs to the nearest one.
    pub fn index_for(&self, r: Rect) -> usize {
        let mut best: Option<(usize, i64)> = None;
        for (i, o) in self.outputs.iter().enumerate() {
            let a = overlap(o.frame, r);
            if a > 0 && best.map_or(true, |(_, b)| a > b) {
                best = Some((i, a));
            }
        }
        if let Some((i, _)) = best {
            return i;
        }
        let nearest = self.outputs.iter().enumerate().min_by_key(|(_, o)| centre_distance(o.frame, r));
        nearest.map_or(0, |(i, _)| i)
    }

    /// The frame of display `i` (a full-screen window covers it). The main display's size is `m`'s when none are known.
    pub fn frame(&self, i: usize, m: &Metrics) -> Rect {
        self.outputs.get(i).map_or(Rect { x: 0, y: 0, w: m.w, h: m.h }, |o| o.frame)
    }

    /// The visible frame of display `i`: the main one is inset by the menu bar and the Dock's edge, the others are
    /// whole.
    pub fn visible(&self, i: usize, m: &Metrics) -> Rect {
        let Some(o) = self.outputs.get(i) else { return Desk::area(m) };
        let f = o.frame;
        if i == self.main {
            Rect { x: f.x + m.dock_left, y: f.y + m.menubar, w: (f.w - m.dock_left - m.dock_right).max(1), h: (f.h - m.menubar - m.dock_bottom).max(1) }
        } else {
            Rect { x: f.x, y: f.y, w: f.w.max(1), h: f.h.max(1) }
        }
    }

    /// The visible frame of the display `r` is on.
    pub fn visible_for(&self, r: Rect, m: &Metrics) -> Rect {
        self.visible(self.index_for(r), m)
    }

    /// The frame of the display `r` is on.
    pub fn frame_for(&self, r: Rect, m: &Metrics) -> Rect {
        self.frame(self.index_for(r), m)
    }

    /// The smallest rectangle holding every display; None without outputs.
    pub fn bounds(&self) -> Option<Rect> {
        let mut it = self.outputs.iter().map(|o| o.frame);
        let first = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
        for f in it {
            x0 = x0.min(f.x);
            y0 = y0.min(f.y);
            x1 = x1.max(f.x + f.w);
            y1 = y1.max(f.y + f.h);
        }
        Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }

    /// True when `r` touches no display.
    pub fn is_off_screen(&self, r: Rect) -> bool {
        self.outputs.iter().all(|o| overlap(o.frame, r) == 0)
    }
}

impl super::Shell {
    /// The displays when the desktop spans more than one, else None (the one display is `Desk::area`).
    pub(super) fn multi_displays(&self) -> Option<&Arrangement> {
        self.desk.as_ref().map(|d| &d.displays).filter(|a| a.is_multi())
    }

    /// The backend reports the outputs (`main` is the one with the menu bar and the Dock). Windows that no longer touch
    /// any display are brought onto the nearest one, and filled windows refit the visible frame of their display.
    pub fn set_outputs(&mut self, outputs: Vec<Output>, main: usize) {
        let arrangement = Arrangement::new(outputs, main);
        let m = self.desk_metrics();
        let Some(d) = self.desk.as_mut() else { return };
        if d.displays == arrangement {
            return;
        }
        d.displays = arrangement;
        let lost: Vec<usize> = (0..d.wins.len()).filter(|&i| d.displays.is_multi() && d.displays.is_off_screen(d.wins[i].r)).collect();
        for &i in &lost {
            let r = d.wins[i].r;
            d.place(&m, i, r);
        }
        for i in lost {
            self.desk_layout(i);
        }
        self.reflow_filled_windows();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::desk::{Tile, Win};
    use super::super::AppId;
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// A 1920x1080 main display with a 28 px menu bar and a 100 px bottom Dock.
    fn metrics() -> Metrics {
        Metrics { w: 1920, h: 1080, title: 30, menubar: 28, min_w: 320, min_h: 220, edge: 6, light: 12, light_gap: 8, light_x: 12, dock_left: 0, dock_right: 0, dock_bottom: 100, pt: 1.0 }
    }

    /// The main display on the left and a larger second display to its right.
    fn two() -> Arrangement {
        Arrangement::new(vec![Output::new("Main", r(0, 0, 1920, 1080)), Output::new("Side", r(1920, 0, 2560, 1440))], 0)
    }

    fn win(x: i32) -> Win {
        Win { app: AppId::Files, r: r(x, 200, 900, 600), maximized: false, minimized: false, full: false, restore: r(x, 200, 900, 600), full_size: false }
    }

    #[test]
    fn the_main_display_keeps_the_menu_bar_and_the_dock_and_the_others_are_whole() {
        let a = two();
        let m = metrics();
        assert_eq!(a.visible(0, &m), r(0, 28, 1920, 1080 - 28 - 100));
        assert_eq!(a.visible(1, &m), r(1920, 0, 2560, 1440));
    }

    #[test]
    fn the_dock_edge_is_reserved_on_the_main_display_only() {
        let a = two();
        let m = Metrics { dock_bottom: 0, dock_left: 90, ..metrics() };
        assert_eq!(a.visible(0, &m), r(90, 28, 1920 - 90, 1080 - 28));
        assert_eq!(a.visible(1, &m), r(1920, 0, 2560, 1440));
        let m = Metrics { dock_bottom: 0, dock_right: 90, ..metrics() };
        assert_eq!(a.visible(0, &m).w, 1920 - 90);
        assert_eq!(a.visible(1, &m).w, 2560);
    }

    #[test]
    fn a_dock_that_autohides_leaves_the_main_display_the_menu_bar_inset_only() {
        let m = Metrics { dock_bottom: 0, ..metrics() };
        assert_eq!(two().visible(0, &m), r(0, 28, 1920, 1080 - 28));
    }

    #[test]
    fn a_rectangle_belongs_to_the_display_that_shows_most_of_it() {
        let a = two();
        let m = metrics();
        // 300 px on the main display, 700 on the side one
        assert_eq!(a.index_for(r(1620, 200, 1000, 400)), 1);
        assert_eq!(a.visible_for(r(1620, 200, 1000, 400), &m), a.visible(1, &m));
        assert_eq!(a.index_for(r(100, 100, 500, 500)), 0);
        // on no display: the nearest one
        assert_eq!(a.index_for(r(9000, 100, 200, 200)), 1);
        assert_eq!(a.index_for(r(-5000, 100, 200, 200)), 0);
        assert!(a.is_off_screen(r(9000, 100, 200, 200)));
        assert!(!a.is_off_screen(r(1900, 100, 200, 200)));
    }

    #[test]
    fn an_empty_or_single_arrangement_is_the_one_display_desktop() {
        let m = metrics();
        assert!(!Arrangement::default().is_multi());
        assert_eq!(Arrangement::default().visible_for(r(5, 5, 10, 10), &m), Desk::area(&m));
        let one = Arrangement::new(vec![Output::new("Main", r(0, 0, 1920, 1080))], 0);
        assert!(!one.is_multi());
        assert_eq!(one.visible(0, &m), Desk::area(&m));
        assert_eq!(one.frame_for(r(5, 5, 10, 10), &m), r(0, 0, 1920, 1080));
    }

    #[test]
    fn main_can_be_any_output_and_a_stray_index_reads_as_the_first() {
        let a = Arrangement::new(vec![Output::new("A", r(-1280, 0, 1280, 800)), Output::new("B", r(0, 0, 1920, 1080))], 1);
        let m = metrics();
        assert_eq!(a.main_index(), 1);
        assert_eq!(a.visible(1, &m), r(0, 28, 1920, 1080 - 28 - 100));
        assert_eq!(a.visible(0, &m), r(-1280, 0, 1280, 800));
        assert_eq!(Arrangement::new(a.outputs().to_vec(), 7).main_index(), 0);
    }

    #[test]
    fn the_bounds_span_every_display() {
        let a = Arrangement::new(vec![Output::new("A", r(-1280, 100, 1280, 800)), Output::new("B", r(0, 0, 1920, 1080))], 1);
        assert_eq!(a.bounds(), Some(r(-1280, 0, 3200, 1080)));
        assert_eq!(Arrangement::default().bounds(), None);
    }

    #[test]
    fn zoom_fills_the_visible_frame_of_the_display_the_window_is_on() {
        let m = metrics();
        let mut d = Desk::new();
        d.displays = two();
        d.wins.push(win(100));
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, r(0, 28, 1920, 1080 - 28 - 100), "main: under the bar, above the Dock");
        d.wins[0] = win(2200);
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, r(1920, 0, 2560, 1440), "side: the whole display");
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, r(2200, 200, 900, 600), "and back");
    }

    #[test]
    fn tiling_and_full_screen_use_the_display_the_window_is_on() {
        let m = metrics();
        let mut d = Desk::new();
        d.displays = two();
        d.wins.push(win(2200));
        d.tile(&m, 0, Tile::Left);
        assert_eq!(d.wins[0].r.x, 1920);
        assert_eq!(d.wins[0].r.y, 0);
        assert_eq!(d.wins[0].r.h, 1440);
        assert!(d.set_full_screen(&m, 0, true));
        assert_eq!(d.wins[0].r, r(1920, 0, 2560, 1440));
    }

    #[test]
    fn restoring_a_window_keeps_it_on_the_display_it_was_put_on() {
        let m = metrics();
        let mut d = Desk::new();
        d.displays = two();
        d.wins.push(win(100));
        d.place(&m, 0, r(2000, 1300, 900, 600));
        assert_eq!(d.wins[0].r, r(2000, 840, 900, 600), "held inside the side display, which has no Dock");
    }

    #[test]
    fn a_window_can_be_dragged_onto_another_display_and_stays_on_the_desktop() {
        let m = metrics();
        let mut d = Desk::new();
        d.displays = two();
        d.wins.push(win(100));
        d.move_to(&m, 0, 2500, 10);
        assert_eq!(d.wins[0].r.x, 2500, "past the main display's right edge");
        assert_eq!(d.wins[0].r.y, 10, "the side display has no menu bar");
        d.move_to(&m, 0, 9000, 5000);
        assert_eq!(d.wins[0].r.x, 1920 + 2560 - m.title * 2, "held inside the desktop");
        assert_eq!(d.wins[0].r.y, 1440 - m.title);
        d.move_to(&m, 0, 100, 0);
        assert_eq!(d.wins[0].r.y, m.menubar, "back on the main display: under the bar");
    }

    #[test]
    fn one_display_moves_windows_as_before() {
        let m = metrics();
        let mut d = Desk::new();
        d.wins.push(win(100));
        d.move_to(&m, 0, 9000, 5000);
        assert_eq!((d.wins[0].r.x, d.wins[0].r.y), (m.w - m.title * 2, m.h - m.title));
        d.move_to(&m, 0, -9000, 0);
        assert_eq!((d.wins[0].r.x, d.wins[0].r.y), (m.title * 2 - 900, m.menubar));
    }
}
