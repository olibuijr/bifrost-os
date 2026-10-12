//! Reader text size: the article's body text steps through these multiples of the body size. View > Larger Text and
//! View > Smaller Text move one step; the body is wrapped and drawn at the chosen size. The title keeps its size.

/// The body size multiples, smallest first. The default is the body size (step 1).
pub const STEPS: [f32; 5] = [0.85, 1.0, 1.15, 1.3, 1.5];
/// The step a new reader starts at (the body size).
pub const DEFAULT_STEP: usize = 1;

/// The multiple of the body size for a step (a step past the end is the largest).
pub fn factor(step: usize) -> f32 {
    STEPS[step.min(STEPS.len() - 1)]
}

/// The step after `step` for Larger Text, stopping at the largest.
pub fn larger(step: usize) -> usize {
    (step + 1).min(STEPS.len() - 1)
}

/// The step before `step` for Smaller Text, stopping at the smallest.
pub fn smaller(step: usize) -> usize {
    step.saturating_sub(1)
}

/// Whether Larger Text can go further.
pub fn can_grow(step: usize) -> bool {
    step + 1 < STEPS.len()
}

/// Whether Smaller Text can go further.
pub fn can_shrink(step: usize) -> bool {
    step > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_step_is_the_body_size() {
        assert_eq!(factor(DEFAULT_STEP), 1.0);
    }

    #[test]
    fn steps_stop_at_both_ends() {
        assert_eq!(larger(1), 2);
        assert_eq!(larger(4), 4, "the largest stays the largest");
        assert_eq!(smaller(1), 0);
        assert_eq!(smaller(0), 0, "the smallest stays the smallest");
        assert!(!can_grow(4));
        assert!(can_grow(3));
        assert!(!can_shrink(0));
        assert!(can_shrink(1));
    }

    #[test]
    fn a_step_past_the_end_uses_the_largest_size() {
        assert_eq!(factor(0), 0.85);
        assert_eq!(factor(99), 1.5);
    }
}
