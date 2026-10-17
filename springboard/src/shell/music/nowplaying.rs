//! Geometry of the Now Playing screen, the mini player and the lock-screen
//! card, with hit testing. Measurements follow bifrost 15 Music in points.

use super::super::ui::{home_zone, Ui, WHITE};
use crate::gfx::{rgba, Surface};

/// "3:07" for a length in ms (hours when needed); whole seconds are truncated and a
/// negative or non-finite length reads "0:00".
pub fn clock(ms: f64) -> String {
    let total = if ms.is_finite() { (ms / 1000.0).max(0.0) as u64 } else { 0 };
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A thin horizontal slider: dim track, bright fill, a knob that grows under a finger.
pub fn draw_slider(s: &mut Surface, ui: &Ui, track: R, frac: f32, dragging: bool) {
    s.fill_rrect(track.x, track.y, track.w, track.h, track.h / 2.0, rgba(255, 255, 255, 70));
    let fx = track.x + track.w * frac.clamp(0.0, 1.0);
    s.fill_rrect(track.x, track.y, fx - track.x, track.h, track.h / 2.0, rgba(255, 255, 255, 200));
    let k = ui.pt(if dragging { KNOB_DRAG_PT } else { KNOB_PT });
    s.fill_rrect(fx - k / 2.0, track.y + track.h / 2.0 - k / 2.0, k, k, k / 2.0, WHITE);
}

/// Something a finger can land on in a player surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Play,
    Prev,
    Next,
    Scrubber,
    Volume,
    Shuffle,
    Repeat,
    /// opens the Up Next list
    Queue,
    /// the grab handle / anywhere that dismisses
    Dismiss,
    /// opens the full screen (mini player)
    Open,
    /// the artwork
    Artwork,
    /// feedback toggles on the lock card: like, dislike, bookmark (slots 0, 1, 2)
    Like,
    Dislike,
    Bookmark,
    /// the rating track on the lock card
    Rating,
}

pub const ART_MARGIN_PT: f32 = 40.0;
pub const ART_RADIUS_PT: f32 = 12.0;
/// how much the artwork shrinks while paused
pub const PAUSED_SCALE: f32 = 0.82;
pub const HANDLE_W_PT: f32 = 36.0;
pub const HANDLE_H_PT: f32 = 5.0;
pub const SCRUB_H_PT: f32 = 3.0;
pub const KNOB_PT: f32 = 7.0;
pub const KNOB_DRAG_PT: f32 = 24.0;
pub const PLAY_PT: f32 = 40.0;
pub const SKIP_PT: f32 = 30.0;
pub const MINI_H_PT: f32 = 64.0;
pub const MINI_ART_PT: f32 = 48.0;
pub const SIDE_PT: f32 = 40.0;
/// finger slop around a control
const HIT_PT: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn inset(&self, d: f32) -> R {
        R { x: self.x - d, y: self.y - d, w: self.w + 2.0 * d, h: self.h + 2.0 * d }
    }
}

/// The full-screen player's parts in px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub handle: R,
    /// the artwork square at full size (playing)
    pub art: R,
    pub title_y: f32,
    pub artist_y: f32,
    pub scrub: R,
    pub times_y: f32,
    pub prev: (f32, f32),
    pub play: (f32, f32),
    pub next: (f32, f32),
    pub volume: R,
    pub shuffle: (f32, f32),
    pub repeat: (f32, f32),
    pub queue: (f32, f32),
    hit: f32,
}

impl Layout {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Layout {
        let (w, h) = (w as f32, h as f32);
        let side = ui.pt(SIDE_PT);
        let handle = R { x: (w - ui.pt(HANDLE_W_PT)) / 2.0, y: ui.pt(8.0), w: ui.pt(HANDLE_W_PT), h: ui.pt(HANDLE_H_PT) };
        let size = (w - 2.0 * ui.pt(ART_MARGIN_PT)).min(h * 0.42);
        let art = R { x: (w - size) / 2.0, y: ui.pt(52.0), w: size, h: size };
        let title_y = art.y + art.h + ui.pt(44.0);
        let artist_y = title_y + ui.pt(27.0);
        let scrub = R { x: side, y: artist_y + ui.pt(60.0), w: w - 2.0 * side, h: ui.pt(SCRUB_H_PT) };
        let times_y = scrub.y + ui.pt(14.0);
        let controls_y = scrub.y + ui.pt(84.0);
        let gap = ui.pt(120.0);
        let volume = R { x: side + ui.pt(32.0), y: controls_y + ui.pt(82.0), w: w - 2.0 * side - ui.pt(64.0), h: ui.pt(SCRUB_H_PT) };
        let bottom_y = (volume.y + ui.pt(84.0)).min(h - home_zone(ui) - ui.pt(30.0));
        Layout {
            handle,
            art,
            title_y,
            artist_y,
            scrub,
            times_y,
            prev: (w / 2.0 - gap, controls_y),
            play: (w / 2.0, controls_y),
            next: (w / 2.0 + gap, controls_y),
            volume,
            shuffle: (w * 0.3, bottom_y),
            repeat: (w * 0.7, bottom_y),
            queue: (w * 0.5, bottom_y),
            hit: ui.pt(HIT_PT),
        }
    }

    /// The artwork rect at the current scale, centred on the full one.
    pub fn art_at(&self, scale: f32) -> R {
        let s = self.art.w * scale;
        R { x: self.art.x + (self.art.w - s) / 2.0, y: self.art.y + (self.art.h - s) / 2.0, w: s, h: s }
    }

    fn near((cx, cy): (f32, f32), r: f32, x: f32, y: f32) -> bool {
        (x - cx).abs() <= r && (y - cy).abs() <= r
    }

    /// What is under (x, y); None on empty background.
    pub fn hit(&self, x: f32, y: f32) -> Option<Target> {
        let r = self.hit;
        if self.scrub.inset(r).contains(x, y) {
            return Some(Target::Scrubber);
        }
        if self.volume.inset(r).contains(x, y) {
            return Some(Target::Volume);
        }
        for (c, t) in [(self.play, Target::Play), (self.prev, Target::Prev), (self.next, Target::Next), (self.shuffle, Target::Shuffle), (self.repeat, Target::Repeat), (self.queue, Target::Queue)] {
            if Self::near(c, r, x, y) {
                return Some(t);
            }
        }
        if y < self.art.y {
            return Some(Target::Dismiss);
        }
        if self.art.contains(x, y) {
            return Some(Target::Artwork);
        }
        None
    }

    /// The fraction along a horizontal slider for a finger at `x`.
    pub fn fraction(track: R, x: f32) -> f32 {
        ((x - track.x) / track.w).clamp(0.0, 1.0)
    }
}

/// The mini player bar at the bottom of the app: (bar, art, play centre, next centre).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mini {
    pub bar: R,
    pub art: R,
    pub play: (f32, f32),
    pub next: (f32, f32),
    hit: f32,
}

impl Mini {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Mini {
        let bar_h = ui.pt(MINI_H_PT);
        let y = h as f32 - home_zone(ui) - bar_h;
        let art_s = ui.pt(MINI_ART_PT);
        let bar = R { x: 0.0, y, w: w as f32, h: bar_h };
        let mid = y + bar_h / 2.0;
        Mini { bar, art: R { x: ui.pt(20.0), y: y + (bar_h - art_s) / 2.0, w: art_s, h: art_s }, play: (w as f32 - ui.pt(100.0), mid), next: (w as f32 - ui.pt(44.0), mid), hit: ui.pt(HIT_PT) }
    }

    /// Where the title text starts.
    pub fn text_x(&self, ui: &Ui) -> f32 {
        self.art.x + self.art.w + ui.pt(14.0)
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if !self.bar.contains(x, y) {
            return None;
        }
        if Layout::near(self.play, self.hit, x, y) {
            return Some(Target::Play);
        }
        if Layout::near(self.next, self.hit, x, y) {
            return Some(Target::Next);
        }
        Some(Target::Open)
    }
}

/// The lock-screen Now Playing card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LockCard {
    pub card: R,
    pub art: R,
    pub title_x: f32,
    pub scrub: R,
    pub prev: (f32, f32),
    pub play: (f32, f32),
    pub next: (f32, f32),
    /// centres of the like, dislike and bookmark icons, in toggle slot order
    pub toggles: [(f32, f32); 3],
    /// the rating track at the end of the feedback row
    pub rating: R,
    hit: f32,
}

pub const LOCK_CARD_W_PT: f32 = 400.0;
pub const LOCK_CARD_H_PT: f32 = 200.0;
/// the distance between the feedback icons, in pt; the icons are drawn at `FEEDBACK_ICON_PT` (`system.rs`)
pub const FEEDBACK_SPACING_PT: f32 = 44.0;
/// right under the lock screen's widget strip
pub const LOCK_CARD_Y_PT: f32 = crate::shell::notify::LOCK_LIST_TOP_PT;
pub const LOCK_ART_PT: f32 = 64.0;

impl LockCard {
    pub fn new(ui: &Ui, w: i32) -> LockCard {
        let cw = ui.pt(LOCK_CARD_W_PT).min(w as f32 - ui.pt(32.0));
        let card = R { x: (w as f32 - cw) / 2.0, y: ui.pt(LOCK_CARD_Y_PT), w: cw, h: ui.pt(LOCK_CARD_H_PT) };
        let pad = ui.pt(16.0);
        let art = R { x: card.x + pad, y: card.y + pad, w: ui.pt(LOCK_ART_PT), h: ui.pt(LOCK_ART_PT) };
        let scrub = R { x: card.x + pad, y: art.y + art.h + ui.pt(22.0), w: cw - 2.0 * pad, h: ui.pt(SCRUB_H_PT) };
        let cy = scrub.y + ui.pt(52.0);
        let (cx, gap) = (card.x + cw / 2.0, ui.pt(90.0));
        let row_y = cy + ui.pt(30.0);
        let first = card.x + pad + ui.pt(10.0);
        let step = ui.pt(FEEDBACK_SPACING_PT);
        let toggles = [(first, row_y), (first + step, row_y), (first + 2.0 * step, row_y)];
        let rating_x = first + 2.0 * step + ui.pt(40.0);
        let rating_end = card.x + cw - pad;
        let track_h = ui.pt(SCRUB_H_PT);
        let rating = R { x: rating_x, y: row_y - track_h / 2.0, w: (rating_end - rating_x).max(0.0), h: track_h };
        LockCard { card, art, title_x: art.x + art.w + ui.pt(14.0), scrub, prev: (cx - gap, cy), play: (cx, cy), next: (cx + gap, cy), toggles, rating, hit: ui.pt(HIT_PT) }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if !self.card.contains(x, y) {
            return None;
        }
        for (c, t) in [(self.play, Target::Play), (self.prev, Target::Prev), (self.next, Target::Next)] {
            if Layout::near(c, self.hit, x, y) {
                return Some(t);
            }
        }
        for (c, t) in self.toggles.iter().zip([Target::Like, Target::Dislike, Target::Bookmark]) {
            if Layout::near(*c, self.hit, x, y) {
                return Some(t);
            }
        }
        if self.rating.w > 0.0 && self.rating.inset(self.hit).contains(x, y) {
            return Some(Target::Rating);
        }
        if self.scrub.inset(self.hit).contains(x, y) {
            return Some(Target::Scrubber);
        }
        Some(Target::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    #[test]
    fn now_playing_parts_stack_top_to_bottom_inside_the_screen() {
        let ui = ui();
        let l = Layout::new(&ui, 1200, 1920);
        assert!(l.handle.y < l.art.y);
        assert_eq!(l.art.w, l.art.h);
        assert!(l.art.x >= ui.pt(ART_MARGIN_PT) && (l.art.x + l.art.w / 2.0 - 600.0).abs() < 0.01, "centred inside the margins");
        assert!(l.art.y + l.art.h < l.title_y && l.title_y < l.artist_y && l.artist_y < l.scrub.y);
        assert!(l.scrub.y < l.play.1 && l.play.1 < l.volume.y && l.volume.y < l.shuffle.1);
        assert!(l.shuffle.1 < 1920.0 - home_zone(&ui));
        assert_eq!(l.prev.1, l.next.1);
        assert!(l.prev.0 < l.play.0 && l.play.0 < l.next.0);
        assert_eq!(l.play.0, 600.0);
        let small = l.art_at(PAUSED_SCALE);
        assert!(small.w < l.art.w);
        assert!((small.centre().0 - l.art.centre().0).abs() < 0.01 && (small.centre().1 - l.art.centre().1).abs() < 0.01, "shrinks about its centre");
    }

    #[test]
    fn now_playing_hit_testing() {
        let ui = ui();
        let l = Layout::new(&ui, 1200, 1920);
        assert_eq!(l.hit(l.play.0, l.play.1), Some(Target::Play));
        assert_eq!(l.hit(l.prev.0 + 10.0, l.prev.1 - 10.0), Some(Target::Prev));
        assert_eq!(l.hit(l.next.0, l.next.1), Some(Target::Next));
        assert_eq!(l.hit(l.scrub.x + 100.0, l.scrub.y + ui.pt(20.0)), Some(Target::Scrubber), "generous slop around the thin track");
        assert_eq!(l.hit(l.volume.x + 5.0, l.volume.y), Some(Target::Volume));
        assert_eq!(l.hit(l.shuffle.0, l.shuffle.1), Some(Target::Shuffle));
        assert_eq!(l.hit(l.repeat.0, l.repeat.1), Some(Target::Repeat));
        assert_eq!(l.hit(l.queue.0, l.queue.1), Some(Target::Queue));
        assert!(l.shuffle.0 < l.queue.0 && l.queue.0 < l.repeat.0 && l.queue.1 == l.shuffle.1);
        assert_eq!(l.hit(600.0, 5.0), Some(Target::Dismiss));
        let (cx, cy) = l.art.centre();
        assert_eq!(l.hit(cx, cy), Some(Target::Artwork));
        assert_eq!(l.hit(5.0, l.title_y), None);
        assert_eq!(Layout::fraction(l.scrub, l.scrub.x - 50.0), 0.0);
        assert_eq!(Layout::fraction(l.scrub, l.scrub.x + l.scrub.w / 4.0), 0.25);
        assert_eq!(Layout::fraction(l.scrub, 5000.0), 1.0);
    }

    #[test]
    fn mini_player_sits_above_the_home_zone_and_hits() {
        let ui = ui();
        let m = Mini::new(&ui, 1200, 1920);
        assert_eq!(m.bar.y + m.bar.h, 1920.0 - home_zone(&ui));
        assert!(m.art.y > m.bar.y && m.art.y + m.art.h < m.bar.y + m.bar.h);
        assert!(m.text_x(&ui) > m.art.x + m.art.w);
        assert_eq!(m.hit(m.play.0, m.play.1), Some(Target::Play));
        assert_eq!(m.hit(m.next.0 - 5.0, m.next.1 + 5.0), Some(Target::Next));
        assert_eq!(m.hit(300.0, m.bar.y + 10.0), Some(Target::Open));
        assert_eq!(m.hit(300.0, m.bar.y - 10.0), None);
    }

    #[test]
    fn lock_card_geometry_and_hits() {
        let ui = ui();
        let c = LockCard::new(&ui, 1200);
        assert_eq!(c.card.w, ui.pt(LOCK_CARD_W_PT));
        assert!((c.card.x + c.card.w / 2.0 - 600.0).abs() < 0.01, "centred");
        assert!(c.art.x > c.card.x && c.scrub.y > c.art.y + c.art.h && c.play.1 > c.scrub.y && c.play.1 < c.card.y + c.card.h);
        assert_eq!(c.hit(c.play.0, c.play.1), Some(Target::Play));
        assert_eq!(c.hit(c.prev.0, c.prev.1), Some(Target::Prev));
        assert_eq!(c.hit(c.next.0, c.next.1), Some(Target::Next));
        assert_eq!(c.hit(c.scrub.x + 10.0, c.scrub.y), Some(Target::Scrubber));
        assert_eq!(c.hit(c.title_x, c.art.y + 5.0), Some(Target::Open));
        assert_eq!(c.hit(10.0, 10.0), None);
        let narrow = LockCard::new(&ui, 500);
        assert_eq!(narrow.card.w, 500.0 - ui.pt(32.0));
    }

    #[test]
    fn lock_card_feedback_row_hits_and_stays_on_the_card() {
        let ui = ui();
        let c = LockCard::new(&ui, 1200);
        let row_y = c.toggles[0].1;
        assert!(row_y + ui.pt(10.0) <= c.card.y + c.card.h, "the row fits inside the card");
        assert_eq!(c.hit(c.toggles[0].0, row_y), Some(Target::Like));
        assert_eq!(c.hit(c.toggles[1].0, row_y), Some(Target::Dislike));
        assert_eq!(c.hit(c.toggles[2].0, row_y), Some(Target::Bookmark));
        assert_eq!(c.hit(c.rating.x + c.rating.w / 2.0, row_y), Some(Target::Rating));
        assert!(c.rating.w > 0.0, "a rating track fits beside the toggles");
        assert_eq!(c.hit(c.card.x + 4.0, c.card.y + 4.0), Some(Target::Open));
        let narrow = LockCard::new(&ui, 200);
        assert_eq!(narrow.rating.w, 0.0, "a narrow card has no room for the rating track");
        // A point past the narrow card's edge is outside the card; a point on the card's own top corner opens it.
        assert_eq!(narrow.hit(narrow.card.x + narrow.card.w - 4.0, narrow.card.y + 4.0), Some(Target::Open));
    }
}
