//! On-screen display for one level: the volume or brightness bar a shell shows for a moment after a key press.
//!
//! [`Osd`] holds the latest level and when it was shown. It stays fully visible for [`HOLD_MS`], then fades out over
//! [`FADE_MS`]. A new [`Osd::show`] replaces the level and restarts the hold. The model reads no clock and draws
//! nothing: the caller passes `now` in milliseconds and draws the bar with [`Osd::segments`] and [`Osd::opacity`].

/// How long a level stays fully visible, in milliseconds.
pub const HOLD_MS: u64 = 1500;
/// How long the display takes to fade out after the hold, in milliseconds.
pub const FADE_MS: u64 = 300;
/// Segments in the level bar when a shell does not pick its own count.
pub const DEFAULT_SEGMENTS: u32 = 16;

/// The level the display shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Volume,
    Brightness,
    KeyboardBrightness,
}

impl Level {
    /// The English label a shell shows beside the bar.
    pub fn label(self) -> &'static str {
        match self {
            Level::Volume => "Volume",
            Level::Brightness => "Brightness",
            Level::KeyboardBrightness => "Keyboard Brightness",
        }
    }
}

/// The on-screen display state: the last level shown and when it was shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Osd {
    shown: Option<(Level, f32, u64)>,
}

impl Osd {
    pub fn new() -> Osd {
        Osd::default()
    }

    /// Shows `value` for `level` at `now`. The value is clamped to 0.0..=1.0; a NaN value counts as 0.0.
    pub fn show(&mut self, level: Level, value: f32, now: u64) {
        let value = if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) };
        self.shown = Some((level, value, now));
    }

    /// The level and value on screen at `now`, or None once the display has faded out.
    pub fn current(&self, now: u64) -> Option<(Level, f32)> {
        let (level, value, _) = self.shown?;
        (self.opacity(now) > 0.0).then_some((level, value))
    }

    /// How opaque the display is at `now`: 1.0 during the hold, a straight fade to 0.0 over [`FADE_MS`], then 0.0.
    pub fn opacity(&self, now: u64) -> f32 {
        let Some((_, _, at)) = self.shown else {
            return 0.0;
        };
        let age = now.saturating_sub(at);
        if age < HOLD_MS {
            1.0
        } else if age < HOLD_MS + FADE_MS {
            1.0 - (age - HOLD_MS) as f32 / FADE_MS as f32
        } else {
            0.0
        }
    }

    /// The number of filled segments in a bar of `count` segments for `value`, rounded to the nearest segment.
    pub fn segments(value: f32, count: u32) -> u32 {
        let v = if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) };
        (v * count as f32).round() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_display_shows_nothing() {
        let osd = Osd::new();
        assert_eq!(osd.current(0), None);
        assert_eq!(osd.opacity(0), 0.0);
    }

    #[test]
    fn a_shown_level_stays_fully_visible_for_the_hold() {
        let mut osd = Osd::new();
        osd.show(Level::Volume, 0.4, 1000);
        assert_eq!(osd.current(1000), Some((Level::Volume, 0.4)));
        assert_eq!(osd.opacity(1000 + HOLD_MS - 1), 1.0);
    }

    #[test]
    fn the_display_fades_out_after_the_hold_and_then_clears() {
        let mut osd = Osd::new();
        osd.show(Level::Brightness, 0.5, 0);
        let mid = HOLD_MS + FADE_MS / 2;
        assert!((osd.opacity(mid) - 0.5).abs() < 1e-6);
        assert!(osd.current(mid).is_some());
        assert_eq!(osd.opacity(HOLD_MS + FADE_MS), 0.0);
        assert_eq!(osd.current(HOLD_MS + FADE_MS), None);
    }

    #[test]
    fn a_new_level_replaces_the_old_one_and_restarts_the_hold() {
        let mut osd = Osd::new();
        osd.show(Level::Volume, 0.2, 0);
        osd.show(Level::KeyboardBrightness, 0.9, HOLD_MS);
        assert_eq!(osd.current(HOLD_MS + 10), Some((Level::KeyboardBrightness, 0.9)));
        assert_eq!(osd.opacity(HOLD_MS + HOLD_MS - 1), 1.0);
    }

    #[test]
    fn values_are_clamped_and_nan_counts_as_zero() {
        let mut osd = Osd::new();
        osd.show(Level::Volume, 3.0, 0);
        assert_eq!(osd.current(0), Some((Level::Volume, 1.0)));
        osd.show(Level::Volume, -2.0, 0);
        assert_eq!(osd.current(0), Some((Level::Volume, 0.0)));
        osd.show(Level::Volume, f32::NAN, 0);
        assert_eq!(osd.current(0), Some((Level::Volume, 0.0)));
    }

    #[test]
    fn the_bar_fills_whole_segments_and_rounds_to_the_nearest() {
        assert_eq!(Osd::segments(0.0, DEFAULT_SEGMENTS), 0);
        assert_eq!(Osd::segments(0.5, DEFAULT_SEGMENTS), 8);
        assert_eq!(Osd::segments(1.0, DEFAULT_SEGMENTS), 16);
        assert_eq!(Osd::segments(0.51, 10), 5);
        assert_eq!(Osd::segments(f32::NAN, 10), 0);
    }

    #[test]
    fn every_level_has_a_label() {
        assert_eq!(Level::Volume.label(), "Volume");
        assert_eq!(Level::Brightness.label(), "Brightness");
        assert_eq!(Level::KeyboardBrightness.label(), "Keyboard Brightness");
    }
}
