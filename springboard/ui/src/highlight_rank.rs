//! Highlight segments for `HighlightAnalysisRequest`: `highlights` ranks fixed-length windows of per-frame scores by
//! their mean and keeps the best ones that do not overlap. Pure data, no I/O, no model.
//!
//! The caller supplies one score per frame (for example motion or sharpness, measured elsewhere). This module does
//! not measure engagement itself.
//!
//! * A window is `window` consecutive frames, `start..start + window`. Its score is the mean of its frame scores.
//! * Windows are taken best first. A window that shares a frame with a chosen window is skipped.
//! * Ties go to the earlier start. Scores are expected to be finite.

/// A chosen window: frames `start..end`, and its mean score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub score: f64,
}

/// Up to `count` non-overlapping windows of `window` frames, best score first.
/// Returns an empty list when `window` or `count` is 0, or when `window` is longer than `scores`.
pub fn highlights(scores: &[f64], window: usize, count: usize) -> Vec<Segment> {
    if window == 0 || count == 0 || window > scores.len() {
        return Vec::new();
    }
    let mut candidates: Vec<(usize, f64)> = (0..=scores.len() - window)
        .map(|s| (s, scores[s..s + window].iter().sum::<f64>() / window as f64))
        .collect();
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let mut out: Vec<Segment> = Vec::new();
    for (start, score) in candidates {
        if out.len() == count {
            break;
        }
        let overlaps = out.iter().any(|c| start < c.end && c.start < start + window);
        if !overlaps {
            out.push(Segment { start, end: start + window, score });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_window_is_the_one_with_the_highest_mean() {
        // windows of 2: [1,1]=1 at 0, [1,5]=3 at 1, [5,5]=5 at 2, [5,1]=3 at 3, [1,1]=1 at 4
        let got = highlights(&[1.0, 1.0, 5.0, 5.0, 1.0, 1.0], 2, 1);
        assert_eq!(got, vec![Segment { start: 2, end: 4, score: 5.0 }]);
    }

    #[test]
    fn second_window_cannot_overlap_the_first() {
        // after 2..4, starts 1 and 3 overlap it. The free starts 0 and 4 both score 1, so the earlier one (0) wins.
        let got = highlights(&[1.0, 1.0, 5.0, 5.0, 1.0, 1.0], 2, 2);
        assert_eq!(
            got,
            vec![
                Segment { start: 2, end: 4, score: 5.0 },
                Segment { start: 0, end: 2, score: 1.0 },
            ]
        );
    }

    #[test]
    fn third_window_fits_after_the_two_chosen_ones() {
        // chosen 2..4 and 0..2. Start 4 is free: 4 < 2 is false, so it does not overlap 0..2.
        let got = highlights(&[1.0, 1.0, 5.0, 5.0, 1.0, 1.0], 2, 3);
        let starts: Vec<usize> = got.iter().map(|s| s.start).collect();
        assert_eq!(starts, vec![2, 0, 4]);
    }

    #[test]
    fn count_stops_at_the_number_of_free_windows() {
        // the whole list is one window of 3 frames, so asking for 5 still gives 1
        let got = highlights(&[2.0, 4.0, 6.0], 3, 5);
        assert_eq!(got, vec![Segment { start: 0, end: 3, score: 4.0 }]);
    }

    #[test]
    fn mean_is_taken_over_the_whole_window() {
        // windows of 3 over [9, 0, 0, 3, 3, 3]: means at starts 0..=3 are 3, 1, 2, 3. Start 0 wins the tie with 3.
        let got = highlights(&[9.0, 0.0, 0.0, 3.0, 3.0, 3.0], 3, 1);
        assert_eq!(got, vec![Segment { start: 0, end: 3, score: 3.0 }]);
    }

    #[test]
    fn empty_results_for_bad_arguments() {
        assert!(highlights(&[1.0, 2.0], 3, 1).is_empty());
        assert!(highlights(&[1.0, 2.0], 0, 1).is_empty());
        assert!(highlights(&[1.0, 2.0], 1, 0).is_empty());
        assert!(highlights(&[], 1, 1).is_empty());
    }
}
