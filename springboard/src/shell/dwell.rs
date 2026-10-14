//! Dwell click: a pointer that stays inside a small radius for the dwell time produces one click at the anchor.
//!
//! Pure state machine. The caller feeds pointer positions with a monotonic time in seconds and performs the click
//! it returns. Nothing here reads the pointer or dispatches events, so the wiring into the cursor service is a
//! separate step.

/// Dwell time in seconds before the click fires.
pub const DEFAULT_DWELL_SECS: f64 = 1.0;
/// Radius in pixels inside which the pointer still counts as holding still.
pub const DEFAULT_RADIUS: f64 = 6.0;

/// What one pointer sample produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Dwell is off, or the pointer is between anchors with nothing to report.
    Idle,
    /// The pointer is holding still and the dwell time has not elapsed yet.
    Pending,
    /// The dwell time elapsed: click at this anchor. Fires once per anchor.
    Click { x: f64, y: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    x: f64,
    y: f64,
    since: f64,
    fired: bool,
}

/// The dwell-click state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dwell {
    enabled: bool,
    secs: f64,
    radius: f64,
    anchor: Option<Anchor>,
}

impl Default for Dwell {
    fn default() -> Self {
        Self::new(false, DEFAULT_DWELL_SECS, DEFAULT_RADIUS)
    }
}

impl Dwell {
    /// A dwell state. Non-positive or non-finite times and radii fall back to the defaults.
    pub fn new(enabled: bool, secs: f64, radius: f64) -> Self {
        Self {
            enabled,
            secs: if secs.is_finite() && secs > 0.0 { secs } else { DEFAULT_DWELL_SECS },
            radius: if radius.is_finite() && radius >= 0.0 { radius } else { DEFAULT_RADIUS },
            anchor: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turning dwell off drops the anchor, so a later enable starts from a clean dwell.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.anchor = None;
        }
    }

    /// One pointer sample at `(x, y)` taken at time `now` (seconds).
    pub fn sample(&mut self, x: f64, y: f64, now: f64) -> Step {
        if !self.enabled {
            return Step::Idle;
        }
        match self.anchor {
            Some(mut a) if distance(a.x, a.y, x, y) <= self.radius => {
                if a.fired {
                    return Step::Idle;
                }
                if now - a.since >= self.secs {
                    a.fired = true;
                    self.anchor = Some(a);
                    return Step::Click { x: a.x, y: a.y };
                }
                Step::Pending
            }
            _ => {
                self.anchor = Some(Anchor { x, y, since: now, fired: false });
                Step::Pending
            }
        }
    }
}

fn distance(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_never_clicks() {
        let mut d = Dwell::new(false, 1.0, 6.0);
        assert_eq!(d.sample(10.0, 10.0, 0.0), Step::Idle);
        assert_eq!(d.sample(10.0, 10.0, 5.0), Step::Idle);
    }

    #[test]
    fn still_pointer_clicks_once_after_dwell() {
        let mut d = Dwell::new(true, 1.0, 6.0);
        assert_eq!(d.sample(100.0, 50.0, 0.0), Step::Pending);
        assert_eq!(d.sample(102.0, 51.0, 0.5), Step::Pending);
        assert_eq!(d.sample(101.0, 50.0, 1.0), Step::Click { x: 100.0, y: 50.0 });
        assert_eq!(d.sample(100.0, 50.0, 2.0), Step::Idle);
        assert_eq!(d.sample(100.0, 50.0, 3.0), Step::Idle);
    }

    #[test]
    fn moving_out_of_radius_restarts_the_dwell() {
        let mut d = Dwell::new(true, 1.0, 6.0);
        d.sample(0.0, 0.0, 0.0);
        assert_eq!(d.sample(50.0, 0.0, 0.9), Step::Pending);
        assert_eq!(d.sample(50.0, 0.0, 1.5), Step::Pending);
        assert_eq!(d.sample(50.0, 0.0, 2.6), Step::Click { x: 50.0, y: 0.0 });
    }

    #[test]
    fn disabling_clears_the_anchor() {
        let mut d = Dwell::new(true, 1.0, 6.0);
        d.sample(5.0, 5.0, 0.0);
        d.set_enabled(false);
        d.set_enabled(true);
        assert_eq!(d.sample(5.0, 5.0, 1.2), Step::Pending);
    }

    #[test]
    fn bad_parameters_fall_back_to_defaults() {
        let d = Dwell::new(true, -1.0, f64::NAN);
        assert_eq!(d.secs, DEFAULT_DWELL_SECS);
        assert_eq!(d.radius, DEFAULT_RADIUS);
    }
}
