//! Onset detection on captured samples: the exact frame of the first sample whose magnitude reaches a threshold.
//! The benchmark signal is a one-sample full-scale impulse, so no sub-sample interpolation is needed.

/// The index of the first sample with `|x| >= threshold`.
pub fn first_onset(samples: &[f32], threshold: f32) -> Option<usize> {
    samples.iter().position(|s| s.abs() >= threshold)
}

/// Every onset: after one at index `i`, the next search starts at `i + holdoff` (at least 1), so a pulse that
/// spans several samples is one onset.
pub fn onsets(samples: &[f32], threshold: f32, holdoff: usize) -> Vec<usize> {
    let hold = holdoff.max(1);
    let mut out = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        match first_onset(&samples[i..], threshold) {
            Some(rel) => {
                out.push(i + rel);
                i += rel + hold;
            }
            None => break,
        }
    }
    out
}

/// The largest magnitude in the slice (0.0 when empty).
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_exact_frame() {
        let mut x = vec![0.0f32; 16];
        x[11] = 1.0;
        assert_eq!(first_onset(&x, 0.5), Some(11));
        assert_eq!(first_onset(&x[..11], 0.5), None);
        assert_eq!(first_onset(&[], 0.5), None);
    }

    #[test]
    fn threshold_is_inclusive_and_sign_blind() {
        let x = [0.0, 0.25, -0.5, 1.0];
        assert_eq!(first_onset(&x, 0.5), Some(2));
        assert_eq!(first_onset(&x, 0.25), Some(1));
        assert_eq!(first_onset(&x, 1.5), None);
    }

    #[test]
    fn several_impulses() {
        let mut x = vec![0.0f32; 20];
        x[0] = 1.0;
        x[7] = 1.0;
        x[19] = 1.0;
        assert_eq!(onsets(&x, 0.5, 1), vec![0, 7, 19]);
    }

    #[test]
    fn holdoff_merges_a_wide_pulse() {
        let x = [0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
        // holdoff 1: every sample above the threshold is an onset.
        assert_eq!(onsets(&x, 0.5, 1), vec![1, 2, 3, 6]);
        // holdoff 4: the pulse at 1..=3 is one onset (next search at 5).
        assert_eq!(onsets(&x, 0.5, 4), vec![1, 6]);
        // holdoff 0 behaves as 1.
        assert_eq!(onsets(&x, 0.5, 0), vec![1, 2, 3, 6]);
    }

    #[test]
    fn peak_of_samples() {
        assert_eq!(peak(&[0.1, -0.7, 0.3]), 0.7);
        assert_eq!(peak(&[]), 0.0);
    }
}
