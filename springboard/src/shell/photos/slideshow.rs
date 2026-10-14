//! The slideshow: pictures of an album or a selection, one after another
//! with a cross-fade and a slow Ken Burns drift. Pure timing and geometry
//! (times in ms); `mod.rs` owns the decoded pictures and the drawing.

use super::library::Photo;

/// How long one picture stays, fade included.
pub const SLIDE_MS: f64 = 4000.0;
/// The cross-fade at the end of a slide.
pub const FADE_MS: f64 = 1000.0;
/// The Ken Burns end magnification.
const DRIFT_ZOOM: f32 = 1.12;
/// Controls hide this long after the last tap while playing.
pub const CONTROLS_MS: f64 = 3000.0;

#[derive(Debug)]
pub struct Slideshow {
    pub photos: Vec<Photo>,
    pub index: usize,
    pub paused: bool,
    pub controls: bool,
    /// ms into the current slide, before the fade-in it already had
    elapsed: f64,
    last: Option<f64>,
    hide_at: f64,
    /// how long one picture stays, fade included (ms)
    slide_ms: f64,
    /// the next picture replaces this one at once instead of fading in
    cut: bool,
    /// the pictures stay still instead of drifting
    still: bool,
}

impl Slideshow {
    /// Starts at `index` of `photos` (clamped); None when there are none.
    pub fn new(photos: Vec<Photo>, index: usize) -> Option<Self> {
        if photos.is_empty() {
            return None;
        }
        let index = index.min(photos.len() - 1);
        Some(Slideshow { photos, index, paused: false, controls: false, elapsed: 0.0, last: None, hide_at: 0.0, slide_ms: SLIDE_MS, cut: false, still: false })
    }

    /// Shows each picture for `seconds` (at least two, so the cross-fade fits).
    pub fn with_seconds(mut self, seconds: u32) -> Self {
        self.slide_ms = f64::from(seconds.max(2)) * 1000.0;
        self
    }

    /// The transition between pictures (a cross-fade, or a cut) and whether the pictures drift.
    pub fn with_style(mut self, fade: bool, motion: bool) -> Self {
        self.cut = !fade;
        self.still = !motion;
        self
    }

    pub fn current(&self) -> &Photo {
        &self.photos[self.index]
    }

    pub fn next_index(&self) -> usize {
        (self.index + 1) % self.photos.len()
    }

    /// Advances the clock; true when something on screen changed.
    pub fn tick(&mut self, t: f64) -> bool {
        let dt = self.last.map_or(0.0, |l| (t - l).clamp(0.0, 250.0));
        self.last = Some(t);
        if self.hide_at == 0.0 {
            self.hide_at = t + CONTROLS_MS;
        }
        if self.controls && !self.paused && t >= self.hide_at {
            self.controls = false;
        }
        if self.paused {
            return false;
        }
        self.elapsed += dt;
        while self.elapsed >= self.slide_ms {
            self.elapsed -= self.slide_ms;
            self.index = self.next_index();
        }
        true
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.controls = true;
    }

    pub fn show_controls(&mut self, t: f64) {
        self.controls = true;
        self.hide_at = t + CONTROLS_MS;
    }

    pub fn toggle_controls(&mut self, t: f64) {
        if self.controls {
            self.controls = false;
        } else {
            self.show_controls(t);
        }
    }

    /// Jumps one picture forward (`+1`) or back (`-1`), restarting its clock.
    pub fn step(&mut self, dir: i32, t: f64) {
        let n = self.photos.len();
        self.index = if dir >= 0 { (self.index + 1) % n } else { (self.index + n - 1) % n };
        self.elapsed = 0.0;
        self.show_controls(t);
    }

    /// The pictures to draw as (photo index, progress 0..1 of its drift,
    /// opacity 0..255): the current one, then the next while it fades in.
    pub fn layers(&self) -> Vec<(usize, f32, u32)> {
        let total = self.slide_ms + FADE_MS;
        let drift = |p: f32| if self.still { 0.0 } else { p };
        let mut out = vec![(self.index, drift(((self.elapsed + FADE_MS) / total) as f32), 255)];
        let into = self.elapsed - (self.slide_ms - FADE_MS);
        if into > 0.0 && self.photos.len() > 1 && !self.cut {
            out.push((self.next_index(), drift((into / total) as f32), ((into / FADE_MS).min(1.0) * 255.0) as u32));
        }
        out
    }
}

/// Where a picture of `fit` px draws in a `view` px screen at drift
/// `progress`: it grows to `DRIFT_ZOOM` and slides toward a corner that
/// depends on `index`, never exposing more than its own edges.
pub fn ken_burns(index: usize, progress: f32, fit: (f32, f32), view: (f32, f32)) -> (f32, f32, f32, f32) {
    let p = progress.clamp(0.0, 1.0);
    let scale = 1.0 + (DRIFT_ZOOM - 1.0) * p;
    let (w, h) = (fit.0 * scale, fit.1 * scale);
    let (dx, dy) = match index % 4 {
        0 => (1.0, 1.0),
        1 => (-1.0, 1.0),
        2 => (-1.0, -1.0),
        _ => (1.0, -1.0),
    };
    let (ox, oy) = (dx * (w - fit.0) / 2.0, dy * (h - fit.1) / 2.0);
    ((view.0 - w) / 2.0 + ox, (view.1 - h) / 2.0 + oy, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn show(n: usize) -> Slideshow {
        let photos = (0..n).map(|i| Photo { path: PathBuf::from(format!("/p/{i}.jpg")), mtime: i as i64, size: 1 }).collect();
        Slideshow::new(photos, 0).unwrap()
    }

    #[test]
    fn empty_has_no_show() {
        assert!(Slideshow::new(vec![], 0).is_none());
    }

    #[test]
    fn advances_and_wraps() {
        let mut s = show(2);
        s.tick(0.0);
        for k in 1..=17 {
            s.tick(k as f64 * 250.0);
        }
        assert_eq!(s.index, 1);
        for k in 18..=33 {
            s.tick(k as f64 * 250.0);
        }
        assert_eq!(s.index, 0);
    }

    #[test]
    fn slide_length_follows_the_chosen_seconds() {
        let mut s = show(2).with_seconds(2);
        s.tick(0.0);
        for k in 1..=9 {
            s.tick(k as f64 * 250.0);
        }
        assert_eq!(s.index, 1, "two seconds later the second picture is up");
        let long = show(2).with_seconds(10);
        assert_eq!(long.slide_ms, 10_000.0);
        assert_eq!(show(2).with_seconds(0).slide_ms, 2000.0, "never shorter than the fade allows");
    }

    #[test]
    fn pause_freezes_and_fade_overlaps() {
        let mut s = show(3);
        s.tick(0.0);
        for k in 1..=13 {
            s.tick(k as f64 * 250.0);
        }
        assert_eq!(s.layers().len(), 2);
        assert!(s.layers()[1].2 > 0);
        s.toggle_pause();
        let before = s.layers();
        assert!(!s.tick(9999.0));
        assert_eq!(s.layers(), before);
    }

    #[test]
    fn a_cut_shows_one_picture_and_still_pictures_do_not_drift() {
        let mut s = show(3).with_style(false, false);
        s.tick(0.0);
        for k in 1..=15 {
            s.tick(k as f64 * 250.0);
        }
        let layers = s.layers();
        assert_eq!(layers.len(), 1, "no cross-fade in a cut");
        assert_eq!(layers[0].1, 0.0, "no drift when still");
        let mut f = show(3);
        f.tick(0.0);
        for k in 1..=15 {
            f.tick(k as f64 * 250.0);
        }
        assert_eq!(f.layers().len(), 2);
    }

    #[test]
    fn drift_stays_over_the_picture_edges() {
        let (x, y, w, h) = ken_burns(0, 1.0, (800.0, 600.0), (1200.0, 1920.0));
        assert!(w > 800.0 && h > 600.0);
        assert!((x - (1200.0 - 800.0) / 2.0).abs() < 1.0 || x >= 200.0 - 1.0);
        assert!(y >= (1920.0 - 600.0) / 2.0 - 1.0);
    }
}
