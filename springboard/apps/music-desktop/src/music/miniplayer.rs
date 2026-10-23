//! The MiniPlayer window (`Window > MiniPlayer`): a small extra window of the Music app with the artwork, the title and
//! artist, the transport, a scrubber and the volume. The geometry is pure (`layout`, `Layout::hit`, `Layout::fraction`)
//! so the touch targets are tested without a surface; `draw` paints the window with the shared kit colours and text.

use bifrost_app::kit::ui::{Ui, Weight, CONTENT_BG, LABEL, LABEL_2, LABEL_3, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, SEPARATOR_C, SPACE_XL, SPACE_XS, SPACE_XXL};
use springboard::gfx::{Image, Surface};
use springboard::shell::music::icons;
use springboard::shell::music::nowplaying::R;
use springboard::shell::ui::WHITE;

/// The window's width and height in points.
pub const WIDTH_PT: f32 = 320.0;
pub const HEIGHT_PT: f32 = 184.0;

/// The window's size in pixels at `scale` pixels per point.
pub fn size_px(scale: f32) -> (f32, f32) {
    (WIDTH_PT * scale, HEIGHT_PT * scale)
}

/// What a finger can land on in the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Prev,
    PlayPause,
    Next,
    Scrubber,
    Volume,
}

/// The window's geometry in pixels, for a window `w` pixels wide at `scale` pixels per point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// the artwork, top left
    pub art: R,
    /// where the title starts; the artist sits under it
    pub title_x: f32,
    pub title_y: f32,
    /// the song's position bar
    pub scrub: R,
    /// the volume bar
    pub volume: R,
    pub prev: (f32, f32),
    pub play: (f32, f32),
    pub next: (f32, f32),
    /// the touch reach around a transport button, in pixels
    pub reach: f32,
    /// the touch reach above and below a bar, in pixels
    pub bar_reach: f32,
}

/// The geometry of a window `w` pixels wide at `scale` pixels per point.
pub fn layout(w: f32, scale: f32) -> Layout {
    let pt = |v: f32| v * scale;
    let pad = pt(16.0);
    let art = pt(80.0);
    let mid = w / 2.0;
    let bar_w = (w - 2.0 * pad).max(0.0);
    Layout {
        art: R { x: pad, y: pad, w: art, h: art },
        title_x: pad + art + pt(12.0),
        title_y: pad,
        scrub: R { x: pad, y: pt(112.0), w: bar_w, h: pt(4.0) },
        volume: R { x: pad, y: pt(168.0), w: bar_w, h: pt(4.0) },
        prev: (mid - pt(56.0), pt(140.0)),
        play: (mid, pt(140.0)),
        next: (mid + pt(56.0), pt(140.0)),
        reach: pt(22.0),
        bar_reach: pt(10.0),
    }
}

impl Layout {
    /// The part under the pixel (x, y), or None. The transport is checked before the bars, so a touch near a button
    /// is that button.
    pub fn hit(&self, x: f32, y: f32) -> Option<Part> {
        let near = |c: (f32, f32)| (x - c.0).hypot(y - c.1) <= self.reach;
        if near(self.play) {
            Some(Part::PlayPause)
        } else if near(self.prev) {
            Some(Part::Prev)
        } else if near(self.next) {
            Some(Part::Next)
        } else if within(self.scrub, self.bar_reach, x, y) {
            Some(Part::Scrubber)
        } else if within(self.volume, self.bar_reach, x, y) {
            Some(Part::Volume)
        } else {
            None
        }
    }

    /// The fraction along `bar` that the pixel `x` is at: 0 at the left end, 1 at the right end, clamped.
    pub fn fraction(bar: R, x: f32) -> f32 {
        if bar.w <= 0.0 {
            0.0
        } else {
            ((x - bar.x) / bar.w).clamp(0.0, 1.0)
        }
    }
}

fn within(r: R, reach: f32, x: f32, y: f32) -> bool {
    x >= r.x - reach && x <= r.x + r.w + reach && y >= r.y - reach && y <= r.y + r.h + reach
}

/// What the window shows of the current song, read from the Music app.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// the position as a fraction of the song, 0 to 1
    pub progress: f32,
    pub volume: f32,
    /// a live station: no length to scrub and no skipping
    pub live: bool,
}

/// Paints the window into `s` (`lay` is its layout for that surface). `art` is the artwork when the cache has it, and
/// `pressed` the part a finger is on.
pub fn draw(s: &mut Surface, ui: &Ui, lay: &Layout, info: &Info, art: Option<&Image>, pressed: Option<Part>) {
    let (w, h) = (s.w, s.h);
    s.fill_rect(0, 0, w, h, CONTENT_BG.now());
    match art {
        Some(img) => s.blit(img, lay.art.x as i32, lay.art.y as i32, 255, 255),
        None => s.blit(&icons::placeholder(lay.art.w as usize, ui.pt(SPACE_XS)), lay.art.x as i32, lay.art.y as i32, 255, 255),
    }
    let text_w = (w as f32 - lay.title_x - lay.art.x).max(0.0);
    let title = ui.text(&ui.fit(&info.title, MAC_SUBHEAD_PT, text_w), MAC_SUBHEAD_PT, Weight::Semibold, WHITE);
    let artist = ui.text(&ui.fit(&info.artist, MAC_FOOTNOTE_PT, text_w), MAC_FOOTNOTE_PT, Weight::Regular, WHITE);
    s.blit_colored(&title, lay.title_x as i32, lay.title_y as i32, LABEL.now(), 255);
    let gap = ui.pt(4.0);
    s.blit_colored(&artist, lay.title_x as i32, (lay.title_y + title.h as f32 + gap) as i32, LABEL_2.now(), 255);
    bar(s, lay.scrub, if info.live { 0.0 } else { info.progress });
    bar(s, lay.volume, info.volume);
    let dim = |p: Part| if pressed == Some(p) { LABEL_3.now() } else { LABEL.now() };
    let skip_color = |p: Part| if info.live { LABEL_3.now() } else { dim(p) };
    icons::skip(s, lay.prev, ui.pt(SPACE_XL), false, skip_color(Part::Prev));
    icons::play_pause(s, lay.play, ui.pt(SPACE_XXL), info.playing, dim(Part::PlayPause));
    icons::skip(s, lay.next, ui.pt(SPACE_XL), true, skip_color(Part::Next));
}

/// A bar filled to `frac` (0 to 1) with its track behind it.
fn bar(s: &mut Surface, r: R, frac: f32) {
    let radius = r.h / 2.0;
    s.fill_rrect(r.x, r.y, r.w, r.h, radius, SEPARATOR_C.now());
    let fill = r.w * frac.clamp(0.0, 1.0);
    if fill > 0.0 {
        s.fill_rrect(r.x, r.y, fill, r.h, radius, LABEL.now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_touch_in_points_hits_play_only_once_scaled_to_px() {
        // the shell sends the MiniPlayer's touches in window points; the layout is in px (`App::window_event` scales)
        let k = 1200.0 / 768.0;
        let lay = layout(size_px(k).0, k);
        let play_pt = (WIDTH_PT / 2.0, 140.0);
        assert_eq!(lay.hit(play_pt.0 * k, play_pt.1 * k), Some(Part::PlayPause));
        assert_ne!(lay.hit(play_pt.0, play_pt.1), Some(Part::PlayPause), "points read as px miss the button at 1.5625");
    }

    #[test]
    fn every_part_sits_inside_the_window_at_each_scale() {
        for scale in [1.0_f32, 1.5625, 2.0] {
            let (w, h) = size_px(scale);
            let lay = layout(w, scale);
            for (x, y) in [lay.play, lay.prev, lay.next, (lay.art.x, lay.art.y), (lay.scrub.x, lay.scrub.y), (lay.volume.x + lay.volume.w, lay.volume.y)] {
                assert!(x >= 0.0 && x <= w && y >= 0.0 && y <= h, "({x}, {y}) outside {w}x{h} at scale {scale}");
            }
            assert!(lay.art.x + lay.art.w < lay.title_x + 1.0, "the title starts after the artwork");
        }
    }

    #[test]
    fn a_touch_on_a_button_is_that_button() {
        let lay = layout(size_px(1.0).0, 1.0);
        assert_eq!(lay.hit(lay.play.0, lay.play.1), Some(Part::PlayPause));
        assert_eq!(lay.hit(lay.prev.0, lay.prev.1), Some(Part::Prev));
        assert_eq!(lay.hit(lay.next.0, lay.next.1), Some(Part::Next));
    }

    #[test]
    fn a_touch_on_a_bar_is_that_bar_and_the_artwork_is_nothing() {
        let lay = layout(size_px(1.0).0, 1.0);
        assert_eq!(lay.hit(lay.scrub.x + lay.scrub.w / 2.0, lay.scrub.y), Some(Part::Scrubber));
        assert_eq!(lay.hit(lay.volume.x + lay.volume.w / 2.0, lay.volume.y), Some(Part::Volume));
        assert_eq!(lay.hit(lay.art.x + 10.0, lay.art.y + 10.0), None);
    }

    #[test]
    fn a_bar_fraction_clamps_to_its_ends() {
        let bar = R { x: 16.0, y: 0.0, w: 200.0, h: 4.0 };
        assert_eq!(Layout::fraction(bar, 16.0), 0.0);
        assert_eq!(Layout::fraction(bar, 116.0), 0.5);
        assert_eq!(Layout::fraction(bar, 400.0), 1.0, "past the right end is full");
        assert_eq!(Layout::fraction(bar, -50.0), 0.0, "past the left end is empty");
    }
}
