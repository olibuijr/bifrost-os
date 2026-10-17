//! Playback speed for the video and audio players: the steps the speed picker offers, the label each shows and the
//! stepping between them. Pure: the player applies the chosen rate through `timestretch::Stretcher::set_rate`, which
//! clamps it to the same range (`clamp_rate`).

use super::timestretch::{clamp_rate, RATE_MAX, RATE_MIN};

/// The speeds the picker offers, slowest first. Every step lies in `RATE_MIN..=RATE_MAX`.
pub const STEPS: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0];

/// Tolerance for comparing rates that came from float arithmetic.
const EPS: f32 = 1e-4;

/// The label a speed picker row shows: "0.5x", "1x", "1.25x", "2x". The rate is clamped first.
pub fn label(rate: f32) -> String {
    let text = format!("{:.2}", clamp_rate(rate));
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}x")
}

/// The next faster step after `rate`, or the fastest step when `rate` is already there.
pub fn next(rate: f32) -> f32 {
    let r = clamp_rate(rate);
    STEPS.iter().copied().find(|s| *s > r + EPS).unwrap_or(STEPS[STEPS.len() - 1])
}

/// The next slower step before `rate`, or the slowest step when `rate` is already there.
pub fn prev(rate: f32) -> f32 {
    let r = clamp_rate(rate);
    STEPS.iter().rev().copied().find(|s| *s < r - EPS).unwrap_or(STEPS[0])
}

/// The step nearest to `rate` (after clamping); on a tie the slower step wins.
pub fn nearest(rate: f32) -> f32 {
    let r = clamp_rate(rate);
    STEPS.iter().copied().min_by(|a, b| (a - r).abs().total_cmp(&(b - r).abs())).unwrap_or(1.0)
}

/// Whether `rate` is exactly one of the picker's steps.
pub fn is_step(rate: f32) -> bool {
    STEPS.iter().any(|s| (s - rate).abs() < EPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_ascending_and_inside_the_stretcher_range() {
        assert!(STEPS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(STEPS[0], RATE_MIN);
        assert_eq!(STEPS[STEPS.len() - 1], RATE_MAX);
    }

    #[test]
    fn labels_drop_trailing_zeros() {
        assert_eq!(label(1.0), "1x");
        assert_eq!(label(0.5), "0.5x");
        assert_eq!(label(0.75), "0.75x");
        assert_eq!(label(1.25), "1.25x");
        assert_eq!(label(2.0), "2x");
        assert_eq!(label(9.0), "3x");
        assert_eq!(label(f32::NAN), "1x");
    }

    #[test]
    fn next_and_prev_walk_the_steps_and_stop_at_the_ends() {
        assert_eq!(next(1.0), 1.25);
        assert_eq!(prev(1.0), 0.75);
        assert_eq!(next(1.25), 1.5);
        assert_eq!(prev(0.5), 0.5);
        assert_eq!(next(3.0), 3.0);
        assert_eq!(next(9.0), 3.0);
        assert_eq!(prev(0.1), 0.5);
    }

    #[test]
    fn nearest_picks_the_closest_step_and_the_slower_one_on_a_tie() {
        assert_eq!(nearest(1.1), 1.0);
        assert_eq!(nearest(0.6), 0.5);
        assert_eq!(nearest(1.375), 1.25);
        assert_eq!(nearest(f32::NAN), 1.0);
        assert_eq!(nearest(0.0), 0.5);
    }

    #[test]
    fn is_step_matches_only_the_offered_speeds() {
        assert!(is_step(1.25));
        assert!(!is_step(1.1));
    }
}
