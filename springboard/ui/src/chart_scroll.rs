//! Scroll targets for charts that scroll along a value axis. A scroll that ends snaps to values spaced at a regular
//! unit, so a chart always rests on whole values (one week, one day, one hour) instead of a fraction of one.
//! Values are data values: the caller converts a scroll offset with `LinearScale::invert` and maps the target back
//! with `LinearScale::map`. Pure arithmetic, no state.

/// How far one gesture may move the target away from where it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// The target lands on the aligned value nearest the gesture's end, however far that is.
    Never,
    /// The target moves at most one unit from its start, however far the gesture went.
    Always,
}

/// A value-aligned scroll behaviour: targets snap to multiples of `unit`. When `major` is set and the gesture ends
/// within half a unit of a multiple of `major`, the target snaps to that coarser value instead (for example every
/// seventh day inside a chart whose unit is one day).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueAligned {
    pub unit: f64,
    pub major: Option<f64>,
    pub limit: Limit,
}

impl ValueAligned {
    pub fn new(unit: f64, limit: Limit) -> Self {
        Self { unit, major: None, limit }
    }

    /// Adds a coarser alignment step. The major value is ignored when it is not a finite positive number.
    pub fn with_major(mut self, major: f64) -> Self {
        self.major = Some(major);
        self
    }

    /// The scroll target for a gesture that started at `from` and would end at `proposed`.
    /// Returns `from` unchanged when the unit is not a finite positive number or a value is not finite.
    pub fn target(&self, from: f64, proposed: f64) -> f64 {
        let u = self.unit;
        if !(u.is_finite() && u > 0.0) || !from.is_finite() || !proposed.is_finite() {
            return from;
        }
        let unit_near = nearest(proposed, u);
        let aligned = match self.major {
            Some(m) if m.is_finite() && m > 0.0 => {
                let major_near = nearest(proposed, m);
                if (major_near - proposed).abs() <= u / 2.0 {
                    major_near
                } else {
                    unit_near
                }
            }
            _ => unit_near,
        };
        match self.limit {
            Limit::Never => aligned,
            Limit::Always => {
                // Only unit multiples inside `from - unit..=from + unit` are allowed. Two such multiples always exist.
                let (lo, hi) = (from - u, from + u);
                if aligned > hi {
                    (hi / u).floor() * u
                } else if aligned < lo {
                    (lo / u).ceil() * u
                } else {
                    aligned
                }
            }
        }
    }
}

/// The multiple of `unit` nearest to `x`. Halves round away from zero.
fn nearest(x: f64, unit: f64) -> f64 {
    (x / unit).round() * unit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn target_snaps_to_the_nearest_unit() {
        let s = ValueAligned::new(7.0, Limit::Never);
        assert!(close(s.target(0.0, 10.0), 7.0), "10 is nearer to 7 than to 14");
        assert!(close(s.target(0.0, 11.0), 14.0), "11 is nearer to 14 than to 7");
        let days = ValueAligned::new(1.0, Limit::Never);
        assert!(close(days.target(0.0, 3.4), 3.0));
        assert!(close(days.target(0.0, -2.6), -3.0), "negative values snap the same way");
        assert!(close(days.target(0.0, 2.5), 3.0), "a half unit rounds away from zero");
    }

    #[test]
    fn major_alignment_wins_only_within_half_a_unit() {
        let s = ValueAligned::new(1.0, Limit::Never).with_major(7.0);
        assert!(close(s.target(0.0, 6.6), 7.0), "6.6 is 0.4 from the major value 7, inside half a unit");
        assert!(close(s.target(0.0, 5.0), 5.0), "5 is 2 from 7, so the plain unit wins");
        let bad = ValueAligned::new(1.0, Limit::Never).with_major(0.0);
        assert!(close(bad.target(0.0, 6.6), 7.0), "a zero major value is ignored");
    }

    #[test]
    fn always_limit_moves_at_most_one_unit() {
        let s = ValueAligned::new(1.0, Limit::Always);
        assert!(close(s.target(0.0, 10.0), 1.0), "a long swipe forward moves one unit");
        assert!(close(s.target(0.0, -10.0), -1.0), "a long swipe back moves one unit");
        assert!(close(s.target(2.0, 2.4), 2.0), "a short swipe still snaps to the nearest unit");
        let off_grid = ValueAligned::new(1.0, Limit::Always);
        assert!(close(off_grid.target(0.5, 9.0), 1.0), "from 0.5 the reachable units are 0 and 1, and 1 is the furthest forward");
    }

    #[test]
    fn never_limit_follows_the_gesture() {
        let s = ValueAligned::new(2.0, Limit::Never);
        assert!(close(s.target(0.0, 10.0), 10.0));
        assert!(close(s.target(0.0, 40.0), 40.0));
    }

    #[test]
    fn bad_input_keeps_the_start() {
        assert!(close(ValueAligned::new(0.0, Limit::Never).target(4.0, 9.0), 4.0), "a zero unit cannot align");
        assert!(close(ValueAligned::new(f64::NAN, Limit::Never).target(4.0, 9.0), 4.0));
        assert!(close(ValueAligned::new(1.0, Limit::Always).target(4.0, f64::INFINITY), 4.0));
        assert!(ValueAligned::new(1.0, Limit::Never).target(f64::NAN, 9.0).is_nan(), "a non-finite start is returned as it is");
    }
}
