//! The slideshow's state: which capture is up, when the next one comes, and the stepping rules. The window shows it
//! full size over the viewfinder; this file knows nothing about drawing.

/// How long each capture stays up, in ms.
pub const SLIDE_MS: f64 = 3000.0;
/// How long the capture that leaves fades out over the one that comes, in ms.
pub const FADE_MS: f64 = 400.0;

/// The opacity (0 to 255) of the capture that is leaving, `elapsed_ms` after the change: fully opaque at the change and
/// gone once the fade has run its course.
pub fn fade_alpha(elapsed_ms: f64) -> u32 {
    if elapsed_ms <= 0.0 {
        return 255;
    }
    if elapsed_ms >= FADE_MS {
        return 0;
    }
    (255.0 * (1.0 - elapsed_ms / FADE_MS)).round() as u32
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slideshow {
    /// the capture on screen, as an index into the session's captures
    pub index: usize,
    /// when the next capture comes, in ms on the app clock; `None` while paused
    next_at: Option<f64>,
}

impl Slideshow {
    /// A slideshow that starts at capture `index` at time `t`.
    pub fn start(index: usize, t: f64) -> Slideshow {
        Slideshow { index, next_at: Some(t + SLIDE_MS) }
    }

    pub fn paused(&self) -> bool {
        self.next_at.is_none()
    }

    /// Pauses or resumes; resuming gives the capture on screen a full slide time again.
    pub fn toggle_pause(&mut self, t: f64) {
        self.next_at = if self.paused() { Some(t + SLIDE_MS) } else { None };
    }

    /// Moves `delta` captures on or back through `count` captures, wrapping around, and restarts the slide time.
    pub fn step(&mut self, delta: i32, count: usize, t: f64) {
        if count == 0 {
            return;
        }
        let n = count as i64;
        self.index = (self.index as i64 + i64::from(delta)).rem_euclid(n) as usize;
        if !self.paused() {
            self.next_at = Some(t + SLIDE_MS);
        }
    }

    /// Advances when the slide time is up; true when the capture on screen changed.
    pub fn tick(&mut self, count: usize, t: f64) -> bool {
        match self.next_at {
            Some(at) if t >= at && count > 0 => {
                let before = self.index;
                self.step(1, count, t);
                self.index != before
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slideshow_advances_after_the_slide_time() {
        let mut show = Slideshow::start(0, 1000.0);
        assert!(!show.tick(3, 1000.0 + SLIDE_MS - 1.0));
        assert_eq!(show.index, 0);
        assert!(show.tick(3, 1000.0 + SLIDE_MS));
        assert_eq!(show.index, 1);
        // the next slide gets a full slide time from the moment of the change
        assert!(!show.tick(3, 1000.0 + SLIDE_MS + 10.0));
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut show = Slideshow::start(2, 0.0);
        show.step(1, 3, 10.0);
        assert_eq!(show.index, 0);
        show.step(-1, 3, 20.0);
        assert_eq!(show.index, 2);
        show.step(-7, 3, 30.0);
        assert_eq!(show.index, 1);
    }

    #[test]
    fn stepping_restarts_the_slide_time() {
        let mut show = Slideshow::start(0, 0.0);
        show.step(1, 4, 2900.0);
        assert!(!show.tick(4, 3000.0));
        assert!(show.tick(4, 2900.0 + SLIDE_MS));
    }

    #[test]
    fn a_paused_slideshow_stays_on_its_capture_until_it_is_stepped_by_hand() {
        let mut show = Slideshow::start(1, 0.0);
        show.toggle_pause(500.0);
        assert!(show.paused());
        assert!(!show.tick(3, 1_000_000.0));
        assert_eq!(show.next_at, None);
        show.step(1, 3, 600.0);
        assert_eq!(show.index, 2);
        assert!(show.paused());
        show.toggle_pause(700.0);
        assert!(!show.paused());
        assert_eq!(show.next_at, Some(700.0 + SLIDE_MS));
    }

    #[test]
    fn one_capture_never_changes() {
        let mut show = Slideshow::start(0, 0.0);
        assert!(!show.tick(1, SLIDE_MS * 2.0));
        assert_eq!(show.index, 0);
    }

    #[test]
    fn the_leaving_capture_fades_from_opaque_to_gone() {
        assert_eq!(fade_alpha(0.0), 255);
        assert_eq!(fade_alpha(FADE_MS / 2.0), 128);
        assert_eq!(fade_alpha(FADE_MS), 0);
        assert_eq!(fade_alpha(FADE_MS * 3.0), 0);
        let steps: Vec<u32> = (0..=10).map(|k| fade_alpha(FADE_MS * f64::from(k) / 10.0)).collect();
        assert!(steps.windows(2).all(|pair| pair[1] <= pair[0]), "{steps:?}");
    }

    #[test]
    fn nothing_to_show_changes_nothing() {
        let mut show = Slideshow::start(0, 0.0);
        assert!(!show.tick(0, SLIDE_MS * 2.0));
        show.step(1, 0, 0.0);
        assert_eq!(show.index, 0);
    }
}
