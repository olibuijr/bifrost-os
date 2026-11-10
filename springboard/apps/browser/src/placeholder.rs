//! What the page area shows while the tab on screen has no frame yet (a tab just selected, a discarded tab coming
//! back): the background its page had the last time it was drawn, so a light page is not preceded by a dark area nor a
//! dark page by a light one. A tab that was never drawn has no known background and gets the window content colour of
//! the current appearance (`drawing.rs`).
//!
//! The background is read from the frame itself, at six points along its edges, each time a frame is drawn. That costs
//! six pixel reads and needs no script in the page.
use std::collections::HashMap;

use bifrost_app::kit::Surface;
use bifrost_app::webview::WebViewId;

/// Distance of the sample points from the frame's edges, pixels: inside a hairline border, outside the content.
const INSET: usize = 2;
/// A colour is the background when at least this many of the six points show it.
const AGREE: usize = 2;

/// The six sample points of a `w` x `h` frame (frame-local pixels): the corners and the middle of the left and right
/// edges, each `INSET` in. The middles outvote a header band across the top; a frame too small to inset has none.
pub(super) fn points(w: usize, h: usize) -> Option<[(usize, usize); 6]> {
    if w <= 2 * INSET || h <= 2 * INSET {
        return None;
    }
    let (l, r, t, m, b) = (INSET, w - 1 - INSET, INSET, h / 2, h - 1 - INSET);
    Some([(l, m), (r, m), (l, b), (r, b), (l, t), (r, t)])
}

/// The background among the sampled pixels: the colour most of them show (the earlier sample wins a tie), opaque.
/// `None` when no colour is shown by `AGREE` points (a picture behind the page).
pub(super) fn background(samples: &[u32]) -> Option<u32> {
    let mut best: Option<(u32, usize)> = None;
    for (i, colour) in samples.iter().enumerate() {
        if samples[..i].contains(colour) {
            continue;
        }
        let count = samples.iter().filter(|other| *other == colour).count();
        if best.is_none_or(|(_, most)| count > most) {
            best = Some((*colour, count));
        }
    }
    best.filter(|(_, count)| *count >= AGREE).map(|(colour, _)| colour | 0xff00_0000)
}

/// The background of the frame drawn at `area` (window pixels) of `s`. `None` when part of it is outside the surface.
pub(super) fn sample(s: &Surface, area: (i32, i32, u32, u32)) -> Option<u32> {
    let (x, y, w, h) = area;
    if x < 0 || y < 0 {
        return None;
    }
    let mut samples = [0u32; 6];
    for (slot, (px, py)) in samples.iter_mut().zip(points(w as usize, h as usize)?) {
        let (px, py) = (x as usize + px, y as usize + py);
        if px >= s.w || py < s.y0 || py >= s.h {
            return None;
        }
        *slot = *s.px.get((py - s.y0) * s.stride + px)?;
    }
    background(&samples)
}

/// The page background last drawn for each tab.
#[derive(Default)]
pub(super) struct PageBackgrounds {
    by_view: HashMap<WebViewId, u32>,
}

impl PageBackgrounds {
    /// Takes the background of the frame of `view` that was just drawn; a frame without one keeps what was known.
    pub(super) fn note(&mut self, view: WebViewId, colour: Option<u32>) {
        if let Some(colour) = colour {
            self.by_view.insert(view, colour);
        }
    }

    pub(super) fn get(&self, view: WebViewId) -> Option<u32> {
        self.by_view.get(&view).copied()
    }

    pub(super) fn forget(&mut self, view: WebViewId) {
        self.by_view.remove(&view);
    }

    /// The engine is gone (its view ids start over) or the appearance changed (a page may follow it).
    pub(super) fn clear(&mut self) {
        self.by_view.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: u32 = 0xffff_ffff;
    const DARK: u32 = 0xff1e_1e1e;
    const BLUE: u32 = 0xffc0_4020;

    #[test]
    fn the_points_are_inside_the_frame_and_too_small_a_frame_has_none() {
        let pts = points(800, 600).unwrap();
        assert!(pts.iter().all(|(x, y)| *x < 800 && *y < 600));
        assert_eq!(pts[0], (INSET, 300));
        assert_eq!(pts[3], (800 - 1 - INSET, 600 - 1 - INSET));
        assert_eq!(points(2 * INSET, 600), None);
        assert_eq!(points(800, 0), None);
        assert!(points(2 * INSET + 1, 2 * INSET + 1).is_some());
    }

    #[test]
    fn a_plain_page_gives_its_colour() {
        assert_eq!(background(&[WHITE; 6]), Some(WHITE));
        assert_eq!(background(&[DARK; 6]), Some(DARK));
    }

    #[test]
    fn a_header_band_does_not_outvote_the_page() {
        // the two top corners are the header's colour
        assert_eq!(background(&[WHITE, WHITE, WHITE, WHITE, BLUE, BLUE]), Some(WHITE));
    }

    #[test]
    fn a_tie_goes_to_the_earlier_sample_and_a_picture_has_no_background() {
        // a sidebar down the left: the left points first
        assert_eq!(background(&[BLUE, WHITE, BLUE, WHITE, BLUE, WHITE]), Some(BLUE));
        assert_eq!(background(&[1, 2, 3, 4, 5, 6]), None);
        assert_eq!(background(&[]), None);
    }

    #[test]
    fn the_background_is_read_from_the_frame_on_the_surface() {
        let (w, h) = (40usize, 30usize);
        let mut px = vec![DARK; w * h];
        // the toolbar is dark, the frame under it (rows 10 and down) is a white page
        for p in &mut px[10 * w..] {
            *p = WHITE;
        }
        let s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        assert_eq!(sample(&s, (0, 10, 40, 20)), Some(WHITE));
        assert_eq!(sample(&s, (0, 0, 40, 9)), Some(DARK));
        assert_eq!(sample(&s, (0, 10, 40, 40)), None, "a frame that reaches past the surface is not read");
        assert_eq!(sample(&s, (-1, 10, 40, 20)), None);
    }

    #[test]
    fn the_colour_is_opaque() {
        assert_eq!(background(&[0x00ff_ffff; 6]), Some(WHITE));
    }

    #[test]
    fn a_tab_keeps_its_background_until_it_is_forgotten() {
        let mut known = PageBackgrounds::default();
        assert_eq!(known.get(1), None, "a tab that was never drawn has none");
        known.note(1, Some(WHITE));
        known.note(2, Some(DARK));
        // a frame without a clear background (a picture) keeps the last one
        known.note(1, None);
        assert_eq!((known.get(1), known.get(2)), (Some(WHITE), Some(DARK)));
        known.note(1, Some(DARK));
        assert_eq!(known.get(1), Some(DARK));
        known.forget(1);
        assert_eq!((known.get(1), known.get(2)), (None, Some(DARK)));
        known.clear();
        assert_eq!(known.get(2), None);
    }
}
