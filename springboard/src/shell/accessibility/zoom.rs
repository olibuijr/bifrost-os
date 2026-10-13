//! Screen Zoom: the magnified view of the display that the GPU presenter draws as a final-frame transform.
//!
//! Two modes, as in Settings > Accessibility > Zoom: Full Screen magnifies the whole display and keeps the
//! pointer in view; Picture in Picture shows a lens (a fixed-size rectangle) that follows the pointer and
//! magnifies what is under it. The model is pure geometry: it returns the source rectangle of the screen to
//! sample, and the normalised texture coordinates the presenter draws it with.

/// Smallest and largest magnification; 1.0 is no zoom.
pub const MIN_FACTOR: f32 = 1.0;
pub const MAX_FACTOR: f32 = 8.0;
/// One zoom-in or zoom-out step (Option+Cmd+= and Option+Cmd+-).
const STEP: f32 = 1.25;
/// Picture in Picture lens size, screen px at 1x (the shell scales it with the interface).
pub const LENS_W: f32 = 360.0;
pub const LENS_H: f32 = 240.0;
/// Gap between the pointer and the lens.
const LENS_GAP: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// the whole display magnified around the pointer
    Full,
    /// a lens that follows the pointer, magnifying what lies under it
    Lens,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenZoom {
    pub on: bool,
    pub factor: f32,
    pub mode: Mode,
}

impl Default for ScreenZoom {
    fn default() -> Self {
        ScreenZoom { on: false, factor: 2.0, mode: Mode::Full }
    }
}

impl ScreenZoom {
    /// Option+Cmd+8: zoom on or off.
    pub fn toggle(&mut self) {
        self.on = !self.on;
    }

    /// Option+Cmd+= (`bigger`) and Option+Cmd+-: one step in or out. Zooming in turns zoom on; zooming out
    /// to 1x turns it off again.
    pub fn step(&mut self, bigger: bool) {
        if bigger {
            self.factor = (self.factor * STEP).min(MAX_FACTOR);
            self.on = true;
        } else {
            self.factor = (self.factor / STEP).max(MIN_FACTOR);
            if self.factor <= MIN_FACTOR {
                self.on = false;
            }
        }
    }

    /// The screen rectangle the presenter samples, `[x, y, w, h]` in screen px, or None while zoom is off.
    /// Full Screen returns the whole display shrunk by the factor around the pointer; a lens returns the
    /// lens's content rectangle at the same factor.
    pub fn source_rect(&self, pointer: (f32, f32), screen: (f32, f32)) -> Option<[f32; 4]> {
        if !self.on || self.factor <= MIN_FACTOR {
            return None;
        }
        let (sw, sh) = screen;
        let (w, h) = match self.mode {
            Mode::Full => (sw, sh),
            Mode::Lens => (LENS_W.min(sw), LENS_H.min(sh)),
        };
        let (w, h) = (w / self.factor, h / self.factor);
        let x = clamp(pointer.0 - w / 2.0, 0.0, sw - w);
        let y = clamp(pointer.1 - h / 2.0, 0.0, sh - h);
        Some([x, y, w, h])
    }

    /// Picture in Picture only: where the lens is drawn on the screen, `[x, y, w, h]` in screen px. It sits
    /// below and to the right of the pointer and flips to the other side near the screen edges.
    pub fn lens_rect(&self, pointer: (f32, f32), screen: (f32, f32)) -> Option<[f32; 4]> {
        if !self.on || self.mode != Mode::Lens {
            return None;
        }
        let (sw, sh) = screen;
        let (w, h) = (LENS_W.min(sw), LENS_H.min(sh));
        let right = pointer.0 + LENS_GAP;
        let x = if right + w <= sw { right } else { pointer.0 - LENS_GAP - w };
        let below = pointer.1 + LENS_GAP;
        let y = if below + h <= sh { below } else { pointer.1 - LENS_GAP - h };
        Some([clamp(x, 0.0, sw - w), clamp(y, 0.0, sh - h), w, h])
    }
}

/// Normalised texture coordinates `[u0, v0, u1, v1]` of a screen rectangle, for the presenter's final pass.
pub fn uv(rect: [f32; 4], screen: (f32, f32)) -> [f32; 4] {
    let [x, y, w, h] = rect;
    let (sw, sh) = screen;
    [x / sw, y / sh, (x + w) / sw, (y + h) / sh]
}

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if hi < lo { lo } else { v.max(lo).min(hi) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (f32, f32) = (1920.0, 1080.0);

    #[test]
    fn off_by_default_and_no_rect() {
        let z = ScreenZoom::default();
        assert!(!z.on);
        assert_eq!(z.source_rect((10.0, 10.0), SCREEN), None);
    }

    #[test]
    fn toggle_flips_on_and_off() {
        let mut z = ScreenZoom::default();
        z.toggle();
        assert!(z.on);
        z.toggle();
        assert!(!z.on);
    }

    #[test]
    fn step_in_turns_on_and_caps_at_max() {
        let mut z = ScreenZoom::default();
        for _ in 0..50 {
            z.step(true);
        }
        assert!(z.on);
        assert_eq!(z.factor, MAX_FACTOR);
    }

    #[test]
    fn step_out_to_one_turns_off() {
        let mut z = ScreenZoom { on: true, factor: 1.25, mode: Mode::Full };
        z.step(false);
        assert_eq!(z.factor, MIN_FACTOR);
        assert!(!z.on);
        z.step(false);
        assert_eq!(z.factor, MIN_FACTOR);
    }

    #[test]
    fn full_screen_source_is_a_fraction_centred_on_the_pointer() {
        let z = ScreenZoom { on: true, factor: 2.0, mode: Mode::Full };
        assert_eq!(z.source_rect((960.0, 540.0), SCREEN), Some([480.0, 270.0, 960.0, 540.0]));
    }

    #[test]
    fn full_screen_source_stays_on_the_screen() {
        let z = ScreenZoom { on: true, factor: 4.0, mode: Mode::Full };
        let [x, y, w, h] = z.source_rect((0.0, 1079.0), SCREEN).unwrap();
        assert_eq!((x, y), (0.0, 1080.0 - h));
        assert_eq!((w, h), (480.0, 270.0));
        let [x, _, _, _] = z.source_rect((1919.0, 0.0), SCREEN).unwrap();
        assert_eq!(x, 1920.0 - 480.0);
    }

    #[test]
    fn lens_samples_a_lens_sized_area_and_has_a_placement() {
        let z = ScreenZoom { on: true, factor: 2.0, mode: Mode::Lens };
        let src = z.source_rect((500.0, 400.0), SCREEN).unwrap();
        assert_eq!((src[2], src[3]), (LENS_W / 2.0, LENS_H / 2.0));
        let lens = z.lens_rect((500.0, 400.0), SCREEN).unwrap();
        assert_eq!((lens[0], lens[1]), (500.0 + LENS_GAP, 400.0 + LENS_GAP));
        assert_eq!((lens[2], lens[3]), (LENS_W, LENS_H));
    }

    #[test]
    fn lens_flips_near_the_right_and_bottom_edges() {
        let z = ScreenZoom { on: true, factor: 2.0, mode: Mode::Lens };
        let lens = z.lens_rect((1900.0, 1060.0), SCREEN).unwrap();
        assert!(lens[0] + lens[2] <= 1900.0);
        assert!(lens[1] + lens[3] <= 1060.0);
        assert!(lens[0] >= 0.0 && lens[1] >= 0.0);
    }

    #[test]
    fn full_screen_has_no_lens() {
        let z = ScreenZoom { on: true, factor: 2.0, mode: Mode::Full };
        assert_eq!(z.lens_rect((10.0, 10.0), SCREEN), None);
    }

    #[test]
    fn uv_is_normalised() {
        assert_eq!(uv([480.0, 270.0, 960.0, 540.0], SCREEN), [0.25, 0.25, 0.75, 0.75]);
    }
}
