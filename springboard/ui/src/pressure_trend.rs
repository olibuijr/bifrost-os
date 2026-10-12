//! Pressure trend (`PressureTrend`): whether the air pressure is rising, falling or steady over a recent window. The
//! caller feeds pressure readings with their Unix time in seconds (UTC) and the current time; nothing here reads the
//! clock. The enum mirrors the official weather framework's `PressureTrend` cases (`rising`, `falling`, `steady`) and
//! its localized description wording. The window and the steady band are BifrOSt choices: the official
//! documentation does not publish the thresholds it uses.

/// The change in air pressure over the trend window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PressureTrend {
    Falling,
    Rising,
    Steady,
}

impl PressureTrend {
    /// The trend in words, as the official framework describes each case.
    pub fn description(self) -> &'static str {
        match self {
            PressureTrend::Falling => "The pressure is falling.",
            PressureTrend::Rising => "The pressure is rising.",
            PressureTrend::Steady => "The pressure is not changing.",
        }
    }
}

/// The default trend window: three hours, the span forecasters usually compare pressure over.
pub const DEFAULT_WINDOW_SECS: u64 = 3 * 60 * 60;

/// The default steady band: a change of at most one hectopascal over the window counts as steady.
pub const DEFAULT_BAND_HPA: f64 = 1.0;

/// Classifies the pressure trend from readings of `(unix_seconds, hectopascal)`.
///
/// Only finite readings from the last `window_secs` up to `now` are used. The change is taken from the earliest to the
/// latest of those readings, so input order does not matter. A change whose absolute size is at most `band_hpa` is
/// `Steady`; a larger rise is `Rising` and a larger fall is `Falling`. Returns `None` when the readings cover less than
/// half the window, because a short run of readings says little about the trend.
pub fn classify(samples: &[(u64, f64)], now: u64, window_secs: u64, band_hpa: f64) -> Option<PressureTrend> {
    let start = now.saturating_sub(window_secs);
    let mut inside: Vec<(u64, f64)> = samples
        .iter()
        .copied()
        .filter(|&(t, p)| p.is_finite() && t >= start && t <= now)
        .collect();
    if inside.is_empty() {
        return None;
    }
    inside.sort_by_key(|&(t, _)| t);
    let (first_t, first_p) = inside[0];
    let (last_t, last_p) = inside[inside.len() - 1];
    if last_t == first_t || last_t - first_t < window_secs / 2 {
        return None;
    }
    let band = band_hpa.max(0.0);
    let change = last_p - first_p;
    if change.abs() <= band {
        Some(PressureTrend::Steady)
    } else if change > 0.0 {
        Some(PressureTrend::Rising)
    } else {
        Some(PressureTrend::Falling)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3600;

    #[test]
    fn a_rise_above_the_band_is_rising() {
        // 1000 hPa three hours ago, 1003 hPa now: +3.0 hPa over a 3 h window is more than the 1.0 hPa band.
        let samples = [(0, 1000.0), (3 * HOUR, 1003.0)];
        assert_eq!(classify(&samples, 3 * HOUR, 3 * HOUR, 1.0), Some(PressureTrend::Rising));
    }

    #[test]
    fn a_fall_above_the_band_is_falling() {
        // 1010.0 to 1007.5 hPa is -2.5 hPa, larger than the band in the falling direction.
        let samples = [(0, 1010.0), (3 * HOUR, 1007.5)];
        assert_eq!(classify(&samples, 3 * HOUR, 3 * HOUR, 1.0), Some(PressureTrend::Falling));
    }

    #[test]
    fn a_small_change_is_steady() {
        // +0.6 hPa is inside the 1.0 hPa band.
        let samples = [(0, 1013.0), (3 * HOUR, 1013.6)];
        assert_eq!(classify(&samples, 3 * HOUR, 3 * HOUR, 1.0), Some(PressureTrend::Steady));
    }

    #[test]
    fn a_change_equal_to_the_band_is_steady() {
        // Exactly +1.0 hPa is at the edge of the band and stays steady; the band is inclusive.
        let samples = [(0, 1000.0), (3 * HOUR, 1001.0)];
        assert_eq!(classify(&samples, 3 * HOUR, 3 * HOUR, 1.0), Some(PressureTrend::Steady));
    }

    #[test]
    fn readings_covering_less_than_half_the_window_give_none() {
        // The readings span 30 minutes of a 3 h window, which is under half the window.
        let samples = [(9000, 1000.0), (10800, 1005.0)];
        assert_eq!(classify(&samples, 10800, 3 * HOUR, 1.0), None);
    }

    #[test]
    fn readings_outside_the_window_and_in_the_future_are_ignored() {
        // With now = 20000 and a 10800 s window the window starts at 9200. The reading at 0 is too old, the one at
        // 30000 is in the future, and the in-window pair is +2.0 hPa, which is rising.
        let samples = [(0, 900.0), (10000, 1000.0), (20000, 1002.0), (30000, 50.0)];
        assert_eq!(classify(&samples, 20000, 3 * HOUR, 1.0), Some(PressureTrend::Rising));
    }

    #[test]
    fn non_finite_readings_are_dropped_and_input_order_does_not_matter() {
        // The NaN reading is dropped, leaving one reading, so there is no trend.
        let only_one = [(10000, f64::NAN), (20000, 1002.0)];
        assert_eq!(classify(&only_one, 20000, 3 * HOUR, 1.0), None);
        // The same two readings in reverse order still give a rise of +2.0 hPa.
        let reversed = [(20000, 1002.0), (10000, 1000.0)];
        assert_eq!(classify(&reversed, 20000, 3 * HOUR, 1.0), Some(PressureTrend::Rising));
    }

    #[test]
    fn descriptions_match_the_official_wording() {
        assert_eq!(PressureTrend::Rising.description(), "The pressure is rising.");
        assert_eq!(PressureTrend::Falling.description(), "The pressure is falling.");
        assert_eq!(PressureTrend::Steady.description(), "The pressure is not changing.");
    }
}
