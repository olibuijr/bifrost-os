//! The bifrost 15 volume HUD: a small rounded panel under the status bar
//! at the top right, a speaker glyph beside a bar that fills with the
//! level. It holds for 1.5 s after the last key, then fades out. On
//! headphones the glyph is a headset; plugging or unplugging shows the
//! route's name in a row above the bar.

use super::control_center::draw_speaker_waves;
use super::ui::{Ui, Weight, WHITE};
use crate::gfx::{over, rgba, scale, Surface};
use crate::media::route::Route;
use crate::scene::STATUS_PT;

const W_PT: f32 = 150.0;
const H_PT: f32 = 36.0;
const RADIUS_PT: f32 = 6.0;
/// the panel with the route name row above the bar
const H_ANNOUNCE_PT: f32 = 58.0;
/// inset from the right edge
const RIGHT_PT: f32 = 24.0;
/// gap below the status bar
const GAP_PT: f32 = 8.0;
const BAR_H_PT: f32 = 7.0;
/// the bar starts this far in (after the glyph) and ends this far from the right
const BAR_LEFT_PT: f32 = 44.0;
const BAR_RIGHT_PT: f32 = 14.0;
/// fully visible for this long after `show`
pub const HOLD_MS: f64 = 1500.0;
/// then fades over this long
pub const FADE_MS: f64 = 250.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeHud {
    level: f32,
    /// when it was last shown
    shown: Option<f64>,
    route: Route,
    /// the route name row is on (a jack event, not a key press)
    announce: bool,
}

impl VolumeHud {
    /// Shows (or restarts) the HUD at `level`.
    pub fn show(&mut self, level: f32, t: f64) {
        self.level = level.clamp(0.0, 1.0);
        self.shown = Some(t);
        self.announce = false;
    }

    /// Shows the HUD for a route change: the route's name above the bar.
    pub fn show_route(&mut self, level: f32, route: Route, t: f64) {
        self.route = route;
        self.show(level, t);
        self.announce = true;
    }

    /// The route the glyph shows; no HUD, no timer.
    pub fn set_route(&mut self, route: Route) {
        self.route = route;
    }

    pub fn route(&self) -> Route {
        self.route
    }

    pub fn hide(&mut self) {
        self.shown = None;
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    /// Whether `show` has been called and `hide` not yet (time aside).
    pub fn shown(&self) -> bool {
        self.shown.is_some()
    }

    /// 1 while holding, falling to 0 through the fade, 0 when hidden.
    pub fn alpha(&self, t: f64) -> f32 {
        match self.shown {
            Some(s) => (1.0 - (t - s - HOLD_MS) / FADE_MS).clamp(0.0, 1.0) as f32,
            None => 0.0,
        }
    }

    pub fn visible(&self, t: f64) -> bool {
        self.alpha(t) > 0.0
    }

    /// When the fade starts, if it has not yet.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        self.shown.map(|s| s + HOLD_MS).filter(|&end| end > t)
    }

    /// Advances time: true while a frame is needed (fading, or the frame
    /// that removes it); hides itself once faded.
    pub fn tick(&mut self, t: f64) -> bool {
        let Some(s) = self.shown else { return false };
        if !self.visible(t) {
            self.shown = None;
            return true;
        }
        t >= s + HOLD_MS
    }

    /// (x, y, w, h) in px for a screen `w` wide.
    pub fn rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        let (hw, hh) = (ui.pt(W_PT), ui.pt(H_PT));
        (w as f32 - ui.pt(RIGHT_PT) - hw, ui.pt(STATUS_PT + GAP_PT), hw, hh)
    }

    /// The box to repaint: the tallest panel, so a route announcement
    /// replaced by a key press leaves nothing behind.
    pub fn damage_rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        let (x, y, hw, _) = Self::rect(ui, w);
        (x, y, hw, ui.pt(H_ANNOUNCE_PT))
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, t: f64) {
        let alpha = (self.alpha(t) * 255.0) as u32;
        if alpha == 0 {
            return;
        }
        let a = |c: u32| scale(c, alpha);
        let (x, y, hw, row) = Self::rect(ui, w);
        let hh = if self.announce { ui.pt(H_ANNOUNCE_PT) } else { row };
        s.fill_rrect(x, y, hw, hh, ui.pt(RADIUS_PT), a(rgba(28, 28, 30, 235)));
        if self.announce {
            let label = ui.text(self.route.label(), 13.0, Weight::Semibold, WHITE);
            s.blit(&label, (x + (hw - label.w as f32) / 2.0) as i32, (y + ui.pt(5.0)) as i32, 255, alpha);
        }
        // the glyph and bar sit in the bottom row, the whole panel when there is no label
        let y = y + hh - row;
        let hh = row;
        let waves = match self.level {
            l if l <= 0.0 => 0,
            l if l < 0.34 => 1,
            l if l < 0.67 => 2,
            _ => 3,
        };
        match self.route {
            Route::Speaker => draw_speaker_waves(s, x + ui.pt(21.0), y + hh / 2.0, ui.s * 0.55, a(WHITE), waves),
            Route::Headphones => draw_headphones(s, x + ui.pt(21.0), y + hh / 2.0, ui.s * 0.55, a(WHITE)),
        }
        let (bx, bw, bh) = (x + ui.pt(BAR_LEFT_PT), hw - ui.pt(BAR_LEFT_PT + BAR_RIGHT_PT), ui.pt(BAR_H_PT));
        let by = y + (hh - bh) / 2.0;
        s.fill_rrect(bx, by, bw, bh, ui.pt(2.0), a(rgba(255, 255, 255, 70)));
        if self.level > 0.0 {
            s.fill_rrect(bx, by, (bw * self.level).max(bh), bh, ui.pt(2.0), a(WHITE));
        }
    }
}

/// A headset: a band over two ear cups; `pt` is px per point, as for the speaker.
pub(super) fn draw_headphones(s: &mut Surface, cx: f32, cy: f32, pt: f32, color: u32) {
    let (radius, thick) = (9.0 * pt, 1.8 * pt);
    let ay = cy + 1.0 * pt;
    for py in ((ay - radius - thick).max(0.0) as usize).max(s.y0)..(ay.ceil() as usize).min(s.h) {
        for px in ((cx - radius - thick).max(0.0) as usize)..((cx + radius + thick).ceil() as usize).min(s.w) {
            let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - ay);
            let cov = (thick / 2.0 - ((dx * dx + dy * dy).sqrt() - radius).abs() + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let p = s.at(px, py);
                *p = over(*p, scale(color, (cov * 255.0) as u32));
            }
        }
    }
    for side in [-1.0, 1.0] {
        let x = cx + side * radius - if side < 0.0 { 2.5 * pt } else { 1.5 * pt };
        s.fill_rrect(x, ay, 4.0 * pt, 8.0 * pt, 1.5 * pt, color);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::ui;
    use super::*;

    #[test]
    fn holds_then_fades_then_hides_itself() {
        let mut h = VolumeHud::default();
        assert_eq!(h.alpha(0.0), 0.0);
        assert!(!h.visible(0.0) && !h.tick(0.0));
        h.show(0.5, 1000.0);
        assert_eq!(h.level(), 0.5);
        assert_eq!(h.alpha(1000.0), 1.0);
        assert_eq!(h.alpha(2499.0), 1.0);
        assert_eq!(h.next_wake(1000.0), Some(2500.0));
        assert!(!h.tick(2000.0), "no frames while holding");
        assert_eq!(h.alpha(2625.0), 0.5);
        assert!(h.tick(2625.0), "frames while fading");
        assert_eq!(h.next_wake(2625.0), None);
        assert_eq!(h.alpha(2750.0), 0.0);
        assert!(h.tick(2750.0), "one frame to remove it");
        assert!(!h.shown());
        assert!(!h.tick(2760.0));
        // another key restarts the hold
        h.show(0.75, 2000.0);
        h.show(0.8125, 2400.0);
        assert_eq!(h.alpha(3800.0), 1.0);
        h.hide();
        assert!(!h.visible(2400.0));
    }

    #[test]
    fn route_announcement_adds_a_row_and_a_key_press_clears_it() {
        let mut h = VolumeHud::default();
        h.show_route(0.5, Route::Headphones, 0.0);
        assert!(h.announce && h.route() == Route::Headphones);
        h.show(0.5, 10.0);
        assert!(!h.announce && h.route() == Route::Headphones, "glyph stays, label goes");
        let ui = ui();
        let (_, _, _, rh) = VolumeHud::rect(&ui, 1200);
        let (_, _, _, dh) = VolumeHud::damage_rect(&ui, 1200);
        assert!(dh > rh);
    }

    #[test]
    fn draws_the_headset_glyph_on_headphones() {
        let ui = ui();
        let paint = |route| {
            let mut buf = vec![0u32; 1200 * 300];
            let mut s = Surface { w: 1200, h: 300, stride: 1200, px: &mut buf, y0: 0 };
            let mut h = VolumeHud::default();
            h.set_route(route);
            h.show(0.5, 0.0);
            h.draw(&mut s, &ui, 1200, 0.0);
            buf
        };
        assert_ne!(paint(Route::Speaker), paint(Route::Headphones));
    }

    #[test]
    fn sits_under_the_status_bar_at_the_top_right() {
        let ui = ui();
        let (x, y, w, h) = VolumeHud::rect(&ui, 1200);
        assert_eq!(x + w, 1200.0 - ui.pt(24.0));
        assert_eq!(y, ui.pt(28.0));
        assert_eq!((w, h), (ui.pt(150.0), ui.pt(36.0)));
    }

    #[test]
    fn draws_a_panel_with_a_bar_that_follows_the_level() {
        let ui = ui();
        let mut buf = vec![0u32; 1200 * 300];
        let mut s = Surface { w: 1200, h: 300, stride: 1200, px: &mut buf, y0: 0 };
        let mut h = VolumeHud::default();
        h.draw(&mut s, &ui, 1200, 0.0);
        assert!(buf.iter().all(|&p| p == 0), "hidden draws nothing");
        let mut s = Surface { w: 1200, h: 300, stride: 1200, px: &mut buf, y0: 0 };
        h.show(0.5, 0.0);
        h.draw(&mut s, &ui, 1200, 0.0);
        let (x, y, w, hh) = VolumeHud::rect(&ui, 1200);
        let at = |px: f32, py: f32| buf[py as usize * 1200 + px as usize];
        let bar_y = y + hh / 2.0;
        let bar_x0 = x + ui.pt(BAR_LEFT_PT);
        let bar_x1 = x + w - ui.pt(BAR_RIGHT_PT);
        assert_eq!(at(bar_x0 + 10.0, bar_y) & 0xff, 255, "filled part is white");
        assert_ne!(at(bar_x1 - 10.0, bar_y) & 0xff, 255, "empty part is the track");
        assert_ne!(at(x + ui.pt(RADIUS_PT), y + 2.0), 0, "panel edge past the corner radius is painted");
        assert_eq!(at(x - 5.0, bar_y), 0, "nothing beside the panel");
        assert_eq!(at(x + w / 2.0, y + hh + 5.0), 0, "nothing below the panel");
        let mut buf2 = vec![0u32; 1200 * 300];
        let mut s = Surface { w: 1200, h: 300, stride: 1200, px: &mut buf2, y0: 0 };
        h.show(0.0, 0.0);
        h.draw(&mut s, &ui, 1200, 0.0);
        assert_ne!(buf2[bar_y as usize * 1200 + (bar_x0 + 10.0) as usize] & 0xff, 255, "muted: no fill");
    }
}
