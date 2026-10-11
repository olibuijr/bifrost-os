//! Music outside the app: the lock-screen Now Playing card and the Control Center media
//! tile controls, drawn by the shell for whatever the audio service plays.

use super::nowplaying::{self, clock, draw_slider, LockCard, R};
use super::icons;
use crate::gfx::{rgba, Image, Surface};
use crate::layout::Rect;
use crate::shell::switcher::draw_scaled;
use crate::shell::ui::{Ui, Weight, WHITE};

/// Everything the lock screen needs to draw the card, borrowed from the app
/// so worker threads can paint it.
pub struct LockCardView<'a> {
    card: LockCard,
    art: Option<&'a Image>,
    title: &'a Image,
    artist: &'a Image,
    playing: bool,
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
    /// The card for what the audio service plays: no artwork, the labels come from the shell.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn service(ui: &Ui, w: i32, title: &'a Image, artist: &'a Image, snap: &crate::svc::audio::Snapshot, scrub: Option<f32>, pressed: Option<nowplaying::Target>) -> Self {
        LockCardView { card: LockCard::new(ui, w), art: None, title, artist, playing: snap.playing, live: snap.live.is_some() || snap.track.as_ref().is_some_and(|t| t.url.is_some()), position_ms: snap.position_ms, duration_ms: snap.duration_ms, scrub, pressed, route: String::new() }
    }

    /// Paints the card `dy` px above its resting place (the lock screen lifts).
    pub fn draw(&self, s: &mut Surface, ui: &Ui, dy: i32) {
        let c = self.card;
        let y = |v: f32| v + dy as f32;
        s.fill_rrect(c.card.x, y(c.card.y), c.card.w, c.card.h, ui.pt(20.0), rgba(255, 255, 255, 46));
        let r = Rect { x: c.art.x as i32, y: y(c.art.y) as i32, w: c.art.w as i32, h: c.art.h as i32 };
        match self.art {
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
