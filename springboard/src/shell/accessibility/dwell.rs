//! Dwell Control: a pointer that rests inside a small area for long enough
//! is clicked without a button press, for users who cannot press one.
//!
//! Pure model. The shell feeds each pointer sample (a timestamp in
//! milliseconds and a position in points) into `DwellClick::feed`, and the
//! model answers with a click position once the pointer has stayed inside
//! `radius` of its anchor for `delay_ms`. Moving farther than `radius` starts
//! a new dwell at the new position. One dwell produces at most one click, so
//! a resting pointer does not click again until it leaves the area.

/// Dwell delays offered in settings, in the same shape as `HOLD_MS`.
pub const DELAYS_MS: [(&str, u64); 4] = [("Off", 0), ("0.5 s", 500), ("1 s", 1000), ("2 s", 2000)];

/// Distance in points a pointer may drift from its anchor and still dwell.
pub const DEFAULT_RADIUS: f32 = 6.0;

/// One pointer position with the time it was observed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub t_ms: u64,
    pub x: f32,
    pub y: f32,
}

/// Dwell state: the anchor of the current dwell and whether it has clicked.
#[derive(Clone, Debug, PartialEq)]
pub struct DwellClick {
    delay_ms: u64,
    radius: f32,
    anchor: Option<Sample>,
    fired: bool,
}

impl DwellClick {
    /// A model that clicks after `delay_ms`; zero turns Dwell Control off.
    pub fn new(delay_ms: u64, radius: f32) -> Self {
        Self { delay_ms, radius: radius.max(0.0), anchor: None, fired: false }
    }

    /// Whether dwell clicks are on at all.
    pub fn enabled(&self) -> bool {
        self.delay_ms > 0
    }

    pub fn delay_ms(&self) -> u64 {
        self.delay_ms
    }

    /// Changes the delay and starts over, so a stale anchor cannot click at once.
    pub fn set_delay_ms(&mut self, delay_ms: u64) {
        self.delay_ms = delay_ms;
        self.reset();
    }

    /// Forgets the current dwell, for example when the pointer leaves the screen.
    pub fn reset(&mut self) {
        self.anchor = None;
        self.fired = false;
    }

    /// Feeds one pointer sample. Returns the position to click when this sample
    /// completes a dwell, and `None` otherwise.
    pub fn feed(&mut self, s: Sample) -> Option<(f32, f32)> {
        if !self.enabled() {
            return None;
        }
        let inside = match self.anchor {
            Some(a) => distance(a, s) <= self.radius,
            None => false,
        };
        if !inside {
            self.anchor = Some(s);
            self.fired = false;
            return None;
        }
        let anchor = self.anchor?;
        // a clock that runs backwards counts as no time passed
        let held = s.t_ms.saturating_sub(anchor.t_ms);
        if self.fired || held < self.delay_ms {
            return None;
        }
        self.fired = true;
        Some((s.x, s.y))
    }
}

/// Dwell delay for a settings index, clamped to the last option.
pub fn delay_for_index(index: usize) -> u64 {
    DELAYS_MS[index.min(DELAYS_MS.len() - 1)].1
}

fn distance(a: Sample, b: Sample) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t_ms: u64, x: f32, y: f32) -> Sample {
        Sample { t_ms, x, y }
    }

    #[test]
    fn rests_long_enough_then_clicks_once() {
        let mut d = DwellClick::new(1000, DEFAULT_RADIUS);
        assert_eq!(d.feed(at(0, 100.0, 100.0)), None);
        assert_eq!(d.feed(at(500, 101.0, 100.0)), None);
        assert_eq!(d.feed(at(1000, 101.0, 100.0)), Some((101.0, 100.0)));
        // still resting: no second click
        assert_eq!(d.feed(at(2000, 101.0, 100.0)), None);
        assert_eq!(d.feed(at(3000, 101.0, 100.0)), None);
    }

    #[test]
    fn moving_out_of_the_radius_restarts_the_dwell() {
        let mut d = DwellClick::new(1000, DEFAULT_RADIUS);
        d.feed(at(0, 0.0, 0.0));
        assert_eq!(d.feed(at(900, 50.0, 0.0)), None);
        // the pointer dwells at the new spot, so the timer starts at 900
        assert_eq!(d.feed(at(1500, 50.0, 0.0)), None);
        assert_eq!(d.feed(at(1900, 50.0, 0.0)), Some((50.0, 0.0)));
    }

    #[test]
    fn leaving_after_a_click_allows_the_next_one() {
        let mut d = DwellClick::new(100, DEFAULT_RADIUS);
        d.feed(at(0, 10.0, 10.0));
        assert!(d.feed(at(100, 10.0, 10.0)).is_some());
        assert_eq!(d.feed(at(200, 400.0, 10.0)), None);
        // the new dwell started at 200, so it completes at 300
        assert_eq!(d.feed(at(300, 400.0, 10.0)), Some((400.0, 10.0)));
        assert_eq!(d.feed(at(400, 400.0, 10.0)), None);
    }

    #[test]
    fn zero_delay_turns_dwell_off() {
        let mut d = DwellClick::new(0, DEFAULT_RADIUS);
        assert!(!d.enabled());
        d.feed(at(0, 1.0, 1.0));
        assert_eq!(d.feed(at(10_000, 1.0, 1.0)), None);
    }

    #[test]
    fn backwards_time_does_not_click_or_panic() {
        let mut d = DwellClick::new(100, DEFAULT_RADIUS);
        d.feed(at(500, 1.0, 1.0));
        assert_eq!(d.feed(at(100, 1.0, 1.0)), None);
    }

    #[test]
    fn changing_the_delay_drops_the_anchor() {
        let mut d = DwellClick::new(100, DEFAULT_RADIUS);
        d.feed(at(0, 1.0, 1.0));
        d.set_delay_ms(1000);
        assert_eq!(d.feed(at(500, 1.0, 1.0)), None);
        assert_eq!(d.feed(at(1500, 1.0, 1.0)), Some((1.0, 1.0)));
    }

    #[test]
    fn settings_indices_map_to_delays() {
        assert_eq!(delay_for_index(0), 0);
        assert_eq!(delay_for_index(2), 1000);
        assert_eq!(delay_for_index(99), 2000);
    }
}
