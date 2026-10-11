//! Motion activity: a coarse label for what the user is doing, from the spread of the acceleration magnitude over a
//! window and the step count the pedometer took in the same window.
//!
//! The rules are fixed thresholds, not a trained model, so a label is a best guess and carries a confidence that
//! grows with the length of the window. A window with no steps and a moderate spread reads as riding in a vehicle;
//! a steady step rate reads as walking or running. Input is the acceleration magnitude in g, one value per sample,
//! and the window length in seconds. The module owns no sensor and no clock.

/// A spread below this (g) is a device at rest.
const STILL_SPREAD_G: f64 = 0.02;
/// A spread below this (g) with no steps is vibration from a vehicle.
const VEHICLE_SPREAD_G: f64 = 0.2;
/// Steps per second from which a window reads as walking.
const WALKING_HZ: f64 = 1.0;
/// Steps per second from which a window reads as running (about 150 steps a minute).
const RUNNING_HZ: f64 = 2.5;
/// Windows shorter than this give a low confidence; from it, a medium one.
const SHORT_WINDOW_S: f64 = 5.0;
/// Windows at least this long give a high confidence.
const LONG_WINDOW_S: f64 = 10.0;

/// What the user is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Stationary,
    Walking,
    Running,
    Automotive,
    Unknown,
}

/// How sure the label is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// One activity estimate for a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activity {
    pub kind: Kind,
    pub confidence: Confidence,
}

/// Classifies a window of acceleration magnitudes (g), the window's length in seconds and the steps counted in it.
///
/// Returns `Unknown` with low confidence for an empty window, a value that is not finite, or a window length that is
/// not positive.
pub fn classify(magnitudes_g: &[f64], window_s: f64, steps: usize) -> Activity {
    let unknown = Activity { kind: Kind::Unknown, confidence: Confidence::Low };
    if magnitudes_g.is_empty() || !window_s.is_finite() || window_s <= 0.0 || magnitudes_g.iter().any(|m| !m.is_finite()) {
        return unknown;
    }
    let n = magnitudes_g.len() as f64;
    let mean = magnitudes_g.iter().sum::<f64>() / n;
    let spread = (magnitudes_g.iter().map(|m| (m - mean) * (m - mean)).sum::<f64>() / n).sqrt();
    let cadence = steps as f64 / window_s;
    let kind = if spread < STILL_SPREAD_G {
        Kind::Stationary
    } else if cadence >= RUNNING_HZ {
        Kind::Running
    } else if cadence >= WALKING_HZ {
        Kind::Walking
    } else if spread < VEHICLE_SPREAD_G {
        Kind::Automotive
    } else {
        Kind::Unknown
    };
    if kind == Kind::Unknown {
        return unknown;
    }
    let confidence = if window_s >= LONG_WINDOW_S {
        Confidence::High
    } else if window_s >= SHORT_WINDOW_S {
        Confidence::Medium
    } else {
        Confidence::Low
    };
    Activity { kind, confidence }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magnitudes that alternate between `high` and `low`, so the spread is half their difference.
    fn alternating(high: f64, low: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| if i % 2 == 0 { high } else { low }).collect()
    }

    #[test]
    fn a_device_at_rest_is_stationary_with_high_confidence_after_a_long_window() {
        let still = vec![1.0; 120];
        assert_eq!(classify(&still, 12.0, 0), Activity { kind: Kind::Stationary, confidence: Confidence::High });
    }

    #[test]
    fn a_steady_step_rate_is_walking_and_a_fast_one_is_running() {
        // 16 steps in 10 s is 1.6 steps a second: walking
        let walk = classify(&alternating(1.2, 0.8, 40), 10.0, 16);
        assert_eq!(walk, Activity { kind: Kind::Walking, confidence: Confidence::High });
        // 30 steps in 10 s is 3 steps a second: running
        let run = classify(&alternating(1.2, 0.8, 40), 10.0, 30);
        assert_eq!(run.kind, Kind::Running);
    }

    #[test]
    fn vibration_without_steps_reads_as_a_vehicle_with_a_medium_confidence() {
        let ride = classify(&alternating(1.1, 0.9, 60), 6.0, 0);
        assert_eq!(ride, Activity { kind: Kind::Automotive, confidence: Confidence::Medium });
    }

    #[test]
    fn a_large_spread_without_a_step_rate_is_unknown() {
        let shaken = classify(&alternating(1.5, 0.5, 40), 10.0, 0);
        assert_eq!(shaken, Activity { kind: Kind::Unknown, confidence: Confidence::Low });
    }

    #[test]
    fn short_windows_give_low_confidence() {
        let short = classify(&vec![1.0; 20], 2.0, 0);
        assert_eq!(short, Activity { kind: Kind::Stationary, confidence: Confidence::Low });
    }

    #[test]
    fn empty_windows_and_values_that_are_not_finite_are_unknown() {
        let unknown = Activity { kind: Kind::Unknown, confidence: Confidence::Low };
        assert_eq!(classify(&[], 10.0, 0), unknown);
        assert_eq!(classify(&[1.0, f64::NAN], 10.0, 0), unknown);
        assert_eq!(classify(&[1.0; 10], 0.0, 0), unknown);
        assert_eq!(classify(&[1.0; 10], f64::INFINITY, 0), unknown);
    }
}
