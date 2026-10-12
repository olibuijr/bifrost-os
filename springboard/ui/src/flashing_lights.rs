//! Flashing light detection and dimming as a pure model. The model reads a series of timed luminance samples of the
//! picture (0.0 is black, 1.0 is white), finds each reversal of the luminance that swings by at least `SWING`, and
//! counts a flash for every reversal after the first. When more than `MAX_FLASHES_PER_SECOND` flashes fall in any
//! one-second window, the sequence is flagged and, with dimming on, the output luminance is scaled down by
//! `DIM_FACTOR`. This is the SDK side of the flashing-lights setting (the enabled flag) and of the processor that
//! reads a framebuffer. Gaps: no framebuffer capture (the caller supplies the samples), no change notification, and
//! the flash count is a simplified reversal count, not the full pair-of-transitions rule of the photosensitivity
//! guidelines.

/// The smallest luminance swing (0.0 to 1.0 scale) that counts as one change of state.
pub const SWING: f64 = 0.10;

/// The most flashes allowed in any one-second window before the sequence is flagged.
pub const MAX_FLASHES_PER_SECOND: usize = 3;

/// The luminance multiplier used for flagged content when dimming is on.
pub const DIM_FACTOR: f64 = 0.6;

/// One timed luminance sample of the picture. The luminance is clamped to 0.0 to 1.0 when it is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub time_ms: u64,
    pub luminance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leg {
    Up,
    Down,
}

/// The times, in milliseconds, at which the luminance reverses after a swing of at least `SWING`. A reversal is
/// timed at the extreme it turns at. The samples must be in ascending time order.
pub fn reversals(samples: &[Sample]) -> Vec<u64> {
    let mut turns = Vec::new();
    let first = match samples.first() {
        Some(s) => s,
        None => return turns,
    };
    let mut pivot = first.luminance;
    let mut leg: Option<Leg> = None;
    let mut ext = first.luminance;
    let mut ext_t = first.time_ms;
    for s in &samples[1..] {
        let v = s.luminance;
        match leg {
            None => {
                if v - pivot >= SWING {
                    leg = Some(Leg::Up);
                    ext = v;
                    ext_t = s.time_ms;
                } else if pivot - v >= SWING {
                    leg = Some(Leg::Down);
                    ext = v;
                    ext_t = s.time_ms;
                }
            }
            Some(Leg::Up) => {
                if v > ext {
                    ext = v;
                    ext_t = s.time_ms;
                } else if ext - v >= SWING {
                    turns.push(ext_t);
                    pivot = ext;
                    leg = Some(Leg::Down);
                    ext = v;
                    ext_t = s.time_ms;
                }
            }
            Some(Leg::Down) => {
                if v < ext {
                    ext = v;
                    ext_t = s.time_ms;
                } else if v - ext >= SWING {
                    turns.push(ext_t);
                    pivot = ext;
                    leg = Some(Leg::Up);
                    ext = v;
                    ext_t = s.time_ms;
                }
            }
        }
    }
    turns
}

/// The flash times of a sample series: every reversal after the first one.
pub fn flashes(samples: &[Sample]) -> Vec<u64> {
    reversals(samples).into_iter().skip(1).collect()
}

/// Whether more than `MAX_FLASHES_PER_SECOND` flashes fall in some window of one second. The times must be in
/// ascending order. Two flashes exactly one second apart are not in the same window.
pub fn exceeds_limit(flash_times: &[u64]) -> bool {
    for (i, &start) in flash_times.iter().enumerate() {
        let count = flash_times[i..].iter().take_while(|&&t| t - start < 1000).count();
        if count > MAX_FLASHES_PER_SECOND {
            return true;
        }
    }
    false
}

/// The flashing light setting and the state of the last analysed sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlashingLights {
    dimming_enabled: bool,
    flagged: bool,
}

impl FlashingLights {
    /// Dimming off and no sequence flagged.
    pub fn new() -> Self {
        Self::default()
    }

    /// Turns the dimming setting on or off, as the system setting does.
    pub fn set_dimming_enabled(&mut self, on: bool) {
        self.dimming_enabled = on;
    }

    /// Whether the dimming setting is on.
    pub fn dimming_enabled(&self) -> bool {
        self.dimming_enabled
    }

    /// Analyses a sample series, stores whether it is flagged and returns that flag.
    pub fn analyse(&mut self, samples: &[Sample]) -> bool {
        self.flagged = exceeds_limit(&flashes(samples));
        self.flagged
    }

    /// Whether the last analysed sequence was flagged.
    pub fn is_flagged(&self) -> bool {
        self.flagged
    }

    /// The output luminance for one input luminance: scaled by `DIM_FACTOR` only while dimming is on and the sequence
    /// is flagged. The input is clamped to 0.0 to 1.0 first.
    pub fn output(&self, luminance: f64) -> f64 {
        let v = luminance.clamp(0.0, 1.0);
        if self.dimming_enabled && self.flagged {
            v * DIM_FACTOR
        } else {
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(time_ms: u64, luminance: f64) -> Sample {
        Sample { time_ms, luminance }
    }

    #[test]
    fn a_steady_picture_has_no_reversals() {
        let samples = [s(0, 0.5), s(100, 0.55), s(200, 0.52), s(300, 0.5)];
        assert!(reversals(&samples).is_empty());
        assert!(flashes(&samples).is_empty());
    }

    #[test]
    fn a_swing_below_the_threshold_is_not_a_reversal() {
        // The rise from 0.0 to 0.05 is below SWING, so nothing starts; then 0.05 to 0.0 is also below SWING.
        let samples = [s(0, 0.0), s(100, 0.05), s(200, 0.0)];
        assert!(reversals(&samples).is_empty());
    }

    #[test]
    fn reversals_are_timed_at_their_extremes_and_the_first_one_is_not_a_flash() {
        // 0 -> 1 -> 0 -> 1 -> 0: peaks and troughs turn at 100, 200 and 300.
        let samples = [s(0, 0.0), s(100, 1.0), s(200, 0.0), s(300, 1.0), s(400, 0.0)];
        assert_eq!(reversals(&samples), vec![100, 200, 300]);
        assert_eq!(flashes(&samples), vec![200, 300]);
    }

    #[test]
    fn a_square_wave_flashing_every_100_ms_is_flagged() {
        // Samples every 100 ms from 0 to 1000, alternating black and white: reversals at 100..=900, flashes at 200..=900.
        let samples: Vec<Sample> = (0..=10).map(|i| s(i * 100, if i % 2 == 0 { 0.0 } else { 1.0 })).collect();
        let f = flashes(&samples);
        assert_eq!(f, vec![200, 300, 400, 500, 600, 700, 800, 900]);
        assert!(exceeds_limit(&f));
    }

    #[test]
    fn three_flashes_in_a_second_are_allowed_and_four_are_flagged() {
        assert!(!exceeds_limit(&[0, 300, 600]));
        assert!(exceeds_limit(&[0, 300, 600, 900]));
    }

    #[test]
    fn flashes_exactly_one_second_apart_do_not_share_a_window() {
        assert!(!exceeds_limit(&[0, 1000, 2000, 3000, 4000]));
        assert!(exceeds_limit(&[0, 999, 1000, 1001, 1002]));
    }

    #[test]
    fn dimming_scales_only_flagged_content_while_the_setting_is_on() {
        let mut lights = FlashingLights::new();
        let square: Vec<Sample> = (0..=10).map(|i| s(i * 100, if i % 2 == 0 { 0.0 } else { 1.0 })).collect();
        assert!(lights.analyse(&square));
        assert!(lights.is_flagged());
        assert_eq!(lights.output(1.0), 1.0, "dimming is off, so the picture is unchanged");
        lights.set_dimming_enabled(true);
        assert!((lights.output(1.0) - 0.6).abs() < 1e-9);
        assert!((lights.output(2.0) - 0.6).abs() < 1e-9, "the input is clamped to 1.0 first");
        assert_eq!(lights.output(-0.5), 0.0);
        let calm = [s(0, 0.2), s(100, 0.25)];
        assert!(!lights.analyse(&calm));
        assert!((lights.output(1.0) - 1.0).abs() < 1e-9, "a calm sequence is not dimmed");
    }
}
