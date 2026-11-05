//! Music outside the app: the lock-screen Now Playing card and the Control Center media
//! tile controls, drawn by the shell for whatever the audio service plays.

use super::nowplaying::{self, clock, draw_slider, LockCard, R};
use super::icons;
use crate::shell::services::now_playing::Feedback;
use crate::gfx::{rgba, Image, Surface};
use crate::layout::Rect;
use crate::shell::switcher::draw_scaled;
use crate::shell::ui::{Ui, Weight, WHITE};
use std::sync::Arc;

/// The feedback icons' size in pt.
pub const FEEDBACK_ICON_PT: f32 = 20.0;

/// The feedback toggle a card target stands for, in `Feedback::toggles` order.
pub fn toggle_target(slot: usize) -> Option<nowplaying::Target> {
    [nowplaying::Target::Like, nowplaying::Target::Dislike, nowplaying::Target::Bookmark].get(slot).copied()
}

/// The fraction of a rating scale `value` sits at; 0 when the scale has no width.
pub fn rating_fraction(min: f64, max: f64, value: f64) -> f32 {
    if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

/// Everything the lock screen needs to draw the card, borrowed from the app
/// so worker threads can paint it.
pub struct LockCardView<'a> {
    card: LockCard,
    /// the cover the owner published; None draws the glyph
    art: Option<Arc<Image>>,
    title: &'a Image,
    artist: &'a Image,
    playing: bool,
    /// the like, dislike, bookmark and rating the owner published; None hides the row
    pub(super) feedback: Option<Feedback>,
    /// a live stream: no timeline, "LIVE" instead
    pub(super) live: bool,
    position_ms: f64,
    duration_ms: Option<f64>,
    /// a finger on the scrubber: the fraction under it
    scrub: Option<f32>,
    pressed: Option<nowplaying::Target>,
    /// output route shown under the artist ("Speakers", a Bluetooth sink)
    pub route: String,
}

impl<'a> LockCardView<'a> {
    /// The card for what the audio service plays: the labels come from the shell, the cover from the shown Now Playing
    /// session (the glyph without one).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn service(ui: &Ui, w: i32, title: &'a Image, artist: &'a Image, snap: &crate::svc::audio::Snapshot, scrub: Option<f32>, pressed: Option<nowplaying::Target>) -> Self {
        LockCardView { card: LockCard::new(ui, w), art: crate::shell::services::now_playing::artwork(), title, artist, playing: snap.playing, feedback: crate::shell::services::now_playing::feedback(), live: snap.live.is_some() || snap.track.as_ref().is_some_and(|t| t.url.is_some()), position_ms: snap.position_ms, duration_ms: snap.duration_ms, scrub, pressed, route: String::new() }
    }

    /// Paints the card `dy` px above its resting place (the lock screen lifts).
    pub fn draw(&self, s: &mut Surface, ui: &Ui, dy: i32) {
        let c = self.card;
        let y = |v: f32| v + dy as f32;
        s.fill_rrect(c.card.x, y(c.card.y), c.card.w, c.card.h, ui.pt(20.0), rgba(255, 255, 255, 46));
        let r = Rect { x: c.art.x as i32, y: y(c.art.y) as i32, w: c.art.w as i32, h: c.art.h as i32 };
        match &self.art {
            Some(img) => draw_scaled(s, img, r, ui.pt(6.0), 255),
            None => draw_scaled(s, &icons::placeholder(64, 0.0), r, ui.pt(6.0), 255),
        }
        s.blit(self.title, c.title_x as i32, y(c.art.y + ui.pt(10.0)) as i32, 255, 255);
        s.blit(self.artist, c.title_x as i32, y(c.art.y + ui.pt(34.0)) as i32, 255, 255);
        let dim = rgba(255, 255, 255, 150);
        if !self.route.is_empty() {
            let route = ui.text(&self.route, 12.0, Weight::Medium, dim);
            s.blit(&route, c.title_x as i32, y(c.art.y + ui.pt(52.0)) as i32, 255, 255);
        }
        if self.live {
            let live = ui.text("LIVE", 11.0, Weight::Semibold, dim);
            s.blit(&live, (c.scrub.x + (c.scrub.w - live.w as f32) / 2.0) as i32, y(c.scrub.y + ui.pt(2.0)) as i32, 255, 255);
        } else {
            self.draw_timeline(s, ui, dy);
        }
        let ink = |t: nowplaying::Target| if self.pressed == Some(t) { rgba(255, 255, 255, 140) } else { WHITE };
        icons::skip(s, (c.prev.0, y(c.prev.1)), ui.pt(22.0), false, ink(nowplaying::Target::Prev));
        icons::play_pause(s, (c.play.0, y(c.play.1)), ui.pt(30.0), self.playing, ink(nowplaying::Target::Play));
        icons::skip(s, (c.next.0, y(c.next.1)), ui.pt(22.0), true, ink(nowplaying::Target::Next));
        self.draw_feedback(s, ui, dy);
    }

    /// The like, dislike and bookmark icons, and the rating track when the item has a rating scale. Lit toggles are
    /// white; the others are dim. Nothing is drawn when the owner published no feedback.
    fn draw_feedback(&self, s: &mut Surface, ui: &Ui, dy: i32) {
        let Some(fb) = self.feedback else { return };
        let c = self.card;
        let y = |v: f32| v + dy as f32;
        let size = ui.pt(FEEDBACK_ICON_PT);
        for (slot, &(cx, cy)) in c.toggles.iter().enumerate() {
            let pressed = toggle_target(slot).is_some_and(|t| self.pressed == Some(t));
            let color = match (fb.toggles[slot], pressed) {
                (true, false) => WHITE,
                (true, true) => rgba(255, 255, 255, 140),
                (false, false) => rgba(255, 255, 255, 110),
                (false, true) => rgba(255, 255, 255, 60),
            };
            let at = (cx, y(cy));
            match slot {
                0 => icons::heart(s, at, size, color),
                1 => icons::thumb(s, at, size, false, color),
                _ => icons::bookmark(s, at, size, color),
            }
        }
        if let Some(rating) = fb.rating {
            let frac = rating_fraction(rating.min, rating.max, rating.value);
            draw_slider(s, ui, R { y: y(c.rating.y), ..c.rating }, frac, self.pressed == Some(nowplaying::Target::Rating));
        }
    }

    /// The scrubber and the elapsed / remaining times of a song.
    fn draw_timeline(&self, s: &mut Surface, ui: &Ui, dy: i32) {
        let c = self.card;
        let y = |v: f32| v + dy as f32;
        let dur = self.duration_ms.unwrap_or(0.0);
        let frac = self.scrub.unwrap_or(if dur > 0.0 { (self.position_ms / dur) as f32 } else { 0.0 });
        draw_slider(s, ui, R { y: y(c.scrub.y), ..c.scrub }, frac, self.scrub.is_some());
        let shown = self.scrub.map_or(self.position_ms, |f| f as f64 * dur);
        let dim = rgba(255, 255, 255, 150);
        let elapsed = ui.text(&clock(shown), 11.0, Weight::Medium, dim);
        s.blit(&elapsed, c.scrub.x as i32, y(c.scrub.y + ui.pt(8.0)) as i32, 255, 255);
        let left = ui.text(&if dur > 0.0 { format!("-{}", clock(dur - shown)) } else { "-:--".into() }, 11.0, Weight::Medium, dim);
        s.blit(&left, (c.scrub.x + c.scrub.w - left.w as f32) as i32, y(c.scrub.y + ui.pt(8.0)) as i32, 255, 255);
    }
}

/// The previous / play-pause / next row of the Control Center media tile.
pub fn draw_cc_controls(s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), playing: bool, color: u32, pressed: Option<nowplaying::Target>) {
    let cy = y + h * 0.72;
    let dim = |t: nowplaying::Target| if pressed == Some(t) { crate::gfx::scale(color, 140) } else { color };
    icons::skip(s, (x + w * 0.22, cy), ui.pt(20.0), false, dim(nowplaying::Target::Prev));
    icons::play_pause(s, (x + w * 0.5, cy), ui.pt(28.0), playing, dim(nowplaying::Target::Play));
    icons::skip(s, (x + w * 0.78, cy), ui.pt(20.0), true, dim(nowplaying::Target::Next));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_toggles_map_to_their_slots() {
        assert_eq!(toggle_target(0), Some(nowplaying::Target::Like));
        assert_eq!(toggle_target(1), Some(nowplaying::Target::Dislike));
        assert_eq!(toggle_target(2), Some(nowplaying::Target::Bookmark));
        assert_eq!(toggle_target(3), None);
    }

    #[test]
    fn rating_fraction_clamps_and_guards_an_empty_scale() {
        assert_eq!(rating_fraction(0.0, 5.0, 2.5), 0.5);
        assert_eq!(rating_fraction(0.0, 5.0, 9.0), 1.0);
        assert_eq!(rating_fraction(1.0, 5.0, -3.0), 0.0);
        assert_eq!(rating_fraction(2.0, 2.0, 2.0), 0.0, "a zero-width scale draws empty");
    }
}
