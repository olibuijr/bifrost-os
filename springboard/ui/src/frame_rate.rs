//! Frame-rate ranges an app asks for (`CAFrameRateRange`, `CADisplayLink.preferredFrameRateRange`) as a pure model.
//! A range resolves against a display's refresh rate; this module reads no display.

/// The frame rates an app allows and prefers for display updates, in frames per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRateRange {
    pub minimum: f32,
    pub maximum: f32,
    /// The rate the app would like. Zero means no preference: the highest allowed rate is wanted.
    pub preferred: f32,
}

impl FrameRateRange {
    /// No request, so the system chooses (`CAFrameRateRange.default`). Represented as all zero.
    pub const DEFAULT: FrameRateRange = FrameRateRange { minimum: 0.0, maximum: 0.0, preferred: 0.0 };

    /// Builds a range with consistent values: negative or NaN numbers become 0, the maximum is at least the minimum and
    /// the preferred rate lies inside the range.
    pub fn new(minimum: f32, maximum: f32, preferred: f32) -> Self {
        let minimum = minimum.max(0.0);
        let maximum = maximum.max(0.0).max(minimum);
        let preferred = preferred.max(0.0).clamp(minimum, maximum);
        FrameRateRange { minimum, maximum, preferred }
    }

    pub fn is_default(self) -> bool {
        self == Self::DEFAULT
    }

    /// The rate to target on a display that refreshes at `display_max` Hz: the preferred rate, limited to the range and
    /// to the display. A zero preference targets the highest allowed rate. `None` for the default range, or when the
    /// display reports no usable rate.
    pub fn resolve(self, display_max: f32) -> Option<f32> {
        if self.is_default() || display_max.is_nan() || display_max <= 0.0 {
            return None;
        }
        let cap = if self.maximum > 0.0 { self.maximum.min(display_max) } else { display_max };
        let floor = self.minimum.min(cap);
        let want = if self.preferred > 0.0 { self.preferred } else { cap };
        Some(want.clamp(floor, cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_range_resolves_to_nothing() {
        assert!(FrameRateRange::DEFAULT.is_default());
        assert!(FrameRateRange::new(0.0, 0.0, 0.0).is_default(), "all zero is the default");
        assert_eq!(FrameRateRange::DEFAULT.resolve(120.0), None);
    }

    #[test]
    fn the_preferred_rate_is_used_inside_the_range() {
        assert_eq!(FrameRateRange::new(30.0, 120.0, 60.0).resolve(120.0), Some(60.0));
    }

    #[test]
    fn a_display_caps_the_range() {
        assert_eq!(FrameRateRange::new(30.0, 120.0, 120.0).resolve(60.0), Some(60.0), "a 60 Hz display cannot be asked for 120");
    }

    #[test]
    fn a_zero_preference_targets_the_highest_allowed_rate() {
        assert_eq!(FrameRateRange::new(0.0, 90.0, 0.0).resolve(120.0), Some(90.0));
        assert_eq!(FrameRateRange::new(0.0, 0.0, 0.0).resolve(120.0), None, "no maximum and no preference is the default");
    }

    #[test]
    fn an_inverted_range_is_made_consistent() {
        assert_eq!(FrameRateRange::new(60.0, 30.0, 20.0), FrameRateRange { minimum: 60.0, maximum: 60.0, preferred: 60.0 });
    }

    #[test]
    fn a_minimum_above_the_display_falls_back_to_the_display_rate() {
        assert_eq!(FrameRateRange::new(90.0, 120.0, 0.0).resolve(60.0), Some(60.0));
    }

    #[test]
    fn negative_and_nan_numbers_are_zeroed_and_bad_displays_resolve_to_nothing() {
        assert!(FrameRateRange::new(-5.0, f32::NAN, 10.0).is_default());
        assert_eq!(FrameRateRange::new(30.0, 120.0, 60.0).resolve(f32::NAN), None);
        assert_eq!(FrameRateRange::new(30.0, 120.0, 60.0).resolve(0.0), None);
    }
}
