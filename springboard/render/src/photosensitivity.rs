//! General-flash analysis for photosensitive seizure safety, after WCAG 2.3.1 ("three flashes or
//! below threshold"). The input is a time-ordered list of `(seconds, relative luminance)` frames,
//! luminance in 0..=1. A flash is a pair of opposing luminance changes, each at least `FLASH_STEP`,
//! where the darker side is below `DARK_LIMIT`. Red-flash analysis is not implemented here.

/// Minimum luminance change of one step of a flash (WCAG: 10%).
pub const FLASH_STEP: f32 = 0.10;
/// The darker side of a flash must be below this relative luminance (WCAG: 0.8).
pub const DARK_LIMIT: f32 = 0.80;
/// More flashes than this inside any one-second window fail the rule.
pub const MAX_FLASHES_PER_SECOND: usize = 3;

/// The times at which flashes complete, in order. Samples must be sorted by time.
pub fn flash_times(samples: &[(f32, f32)]) -> Vec<f32> {
    let mut flashes = Vec::new();
    // Direction (+1 rise, -1 fall) and darkest level of the last significant change.
    let mut last: Option<(f32, f32)> = None;
    for w in samples.windows(2) {
        let (a, b) = (w[0].1, w[1].1);
        let t = w[1].0;
        let delta = b - a;
        if !delta.is_finite() || delta.abs() < FLASH_STEP {
            continue;
        }
        let sign = delta.signum();
        let low = a.min(b);
        if let Some((prev_sign, prev_low)) = last {
            if prev_sign != sign && prev_low.min(low) < DARK_LIMIT {
                flashes.push(t);
            }
        }
        last = Some((sign, low));
    }
    flashes
}

/// True when no one-second window that starts at a flash holds more than
/// `MAX_FLASHES_PER_SECOND` flashes.
pub fn is_safe(samples: &[(f32, f32)]) -> bool {
    let flashes = flash_times(samples);
    flashes.iter().enumerate().all(|(i, &start)| {
        flashes[i..]
            .iter()
            .take_while(|&&t| t < start + 1.0)
            .count()
            <= MAX_FLASHES_PER_SECOND
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames every `dt` seconds with luminance alternating 0 and 1, `count` frames in all.
    fn alternating(count: usize, dt: f32) -> Vec<(f32, f32)> {
        (0..count)
            .map(|k| (k as f32 * dt, if k % 2 == 0 { 0.0 } else { 1.0 }))
            .collect()
    }

    #[test]
    fn three_flashes_in_a_second_are_safe() {
        let samples = alternating(5, 0.1); // transitions 1..4 make flashes at frames 2, 3 and 4
        assert_eq!(flash_times(&samples).len(), 3);
        assert!(is_safe(&samples));
    }

    #[test]
    fn four_flashes_in_a_second_fail() {
        let samples = alternating(6, 0.1);
        assert_eq!(flash_times(&samples).len(), 4);
        assert!(!is_safe(&samples));
    }

    #[test]
    fn a_bright_plateau_is_not_a_flash() {
        // Both sides are above the 0.8 limit, so the opposing changes do not count.
        let samples = [(0.0, 0.85), (0.1, 0.99), (0.2, 0.85)];
        assert!(flash_times(&samples).is_empty());
    }

    #[test]
    fn small_changes_are_not_flashes() {
        let samples = [(0.0, 0.2), (0.1, 0.25), (0.2, 0.2), (0.3, 0.25)];
        assert!(flash_times(&samples).is_empty());
        assert!(is_safe(&samples));
    }
}
