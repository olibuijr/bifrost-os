//! Dwell click (`com.apple.DwellControl`): a pointer that rests inside a small radius for a set time clicks once, at the
//! spot where it settled. Moving farther than the radius starts a new dwell. Pure model: the caller feeds pointer
//! samples with a timestamp in milliseconds and acts on the click it gets back. Nothing here reads a device.

/// Dwell time and the radius (in points) a pointer may wander inside before the dwell restarts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub dwell_ms: f64,
    pub radius: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config { dwell_ms: 1000.0, radius: 6.0 }
    }
}

/// A click at the point where the pointer settled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DwellClick {
    config: Config,
    /// The settle point and the time it was reached, while a dwell runs.
    anchor: Option<(f32, f32, f64)>,
    /// Set once the current dwell has produced its click, so a long rest clicks only once.
    fired: bool,
}

fn within(ax: f32, ay: f32, x: f32, y: f32, radius: f32) -> bool {
    let (dx, dy) = (x - ax, y - ay);
    dx * dx + dy * dy <= radius * radius
}

impl DwellClick {
    pub fn new(config: Config) -> Self {
        DwellClick { config, anchor: None, fired: false }
    }

    /// Feeds one pointer sample. Returns a click at the settle point the first time the pointer has stayed inside the
    /// radius for `dwell_ms`. Samples outside the radius restart the dwell at the new position.
    pub fn sample(&mut self, x: f32, y: f32, t_ms: f64) -> Option<Click> {
        match self.anchor {
            Some((ax, ay, since)) if within(ax, ay, x, y, self.config.radius) => {
                if self.fired || t_ms - since < self.config.dwell_ms {
                    return None;
                }
                self.fired = true;
                Some(Click { x: ax, y: ay })
            }
            _ => {
                self.anchor = Some((x, y, t_ms));
                self.fired = false;
                None
            }
        }
    }

    /// Drops the current dwell, for example when the pointer leaves the window or dwell clicking is switched off.
    pub fn reset(&mut self) {
        self.anchor = None;
        self.fired = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> DwellClick {
        DwellClick::new(Config::default())
    }

    #[test]
    fn default_config_is_one_second_within_six_points() {
        assert_eq!(Config::default(), Config { dwell_ms: 1000.0, radius: 6.0 });
    }

    #[test]
    fn resting_pointer_clicks_at_its_settle_point_after_the_dwell() {
        let mut m = model();
        assert_eq!(m.sample(100.0, 50.0, 0.0), None);
        assert_eq!(m.sample(100.0, 50.0, 999.0), None);
        assert_eq!(m.sample(100.0, 50.0, 1000.0), Some(Click { x: 100.0, y: 50.0 }));
    }

    #[test]
    fn a_long_rest_clicks_only_once() {
        let mut m = model();
        m.sample(10.0, 10.0, 0.0);
        assert!(m.sample(10.0, 10.0, 1200.0).is_some());
        assert_eq!(m.sample(10.0, 10.0, 2500.0), None);
        assert_eq!(m.sample(11.0, 10.0, 3000.0), None);
    }

    #[test]
    fn jitter_inside_the_radius_keeps_the_original_anchor() {
        let mut m = model();
        m.sample(10.0, 10.0, 0.0);
        assert_eq!(m.sample(12.0, 11.0, 500.0), None);
        // The click lands on the first settle point, not the last jittered sample.
        assert_eq!(m.sample(13.0, 12.0, 1000.0), Some(Click { x: 10.0, y: 10.0 }));
    }

    #[test]
    fn leaving_the_radius_restarts_the_dwell_at_the_new_point() {
        let mut m = model();
        m.sample(10.0, 10.0, 0.0);
        assert_eq!(m.sample(50.0, 10.0, 900.0), None);
        assert_eq!(m.sample(50.0, 10.0, 1500.0), None);
        assert_eq!(m.sample(50.0, 10.0, 1900.0), Some(Click { x: 50.0, y: 10.0 }));
    }

    #[test]
    fn the_radius_boundary_counts_as_inside() {
        let mut m = model();
        m.sample(0.0, 0.0, 0.0);
        assert!(m.sample(6.0, 0.0, 1000.0).is_some());
    }

    #[test]
    fn a_fired_dwell_rearms_after_the_pointer_moves_away() {
        let mut m = model();
        m.sample(0.0, 0.0, 0.0);
        assert!(m.sample(0.0, 0.0, 1000.0).is_some());
        m.sample(100.0, 0.0, 1100.0);
        assert_eq!(m.sample(100.0, 0.0, 2100.0), Some(Click { x: 100.0, y: 0.0 }));
    }

    #[test]
    fn reset_clears_the_running_dwell() {
        let mut m = model();
        m.sample(10.0, 10.0, 0.0);
        m.reset();
        // Without an anchor the next sample starts a fresh dwell, so 1000 ms later it is the first click, not a
        // continuation of the old one.
        assert_eq!(m.sample(10.0, 10.0, 1000.0), None);
        assert_eq!(m.sample(10.0, 10.0, 2000.0), Some(Click { x: 10.0, y: 10.0 }));
    }

    #[test]
    fn a_zero_dwell_clicks_on_the_second_sample() {
        let mut m = DwellClick::new(Config { dwell_ms: 0.0, radius: 6.0 });
        assert_eq!(m.sample(4.0, 4.0, 0.0), None);
        assert_eq!(m.sample(4.0, 4.0, 0.0), Some(Click { x: 4.0, y: 4.0 }));
    }
}
