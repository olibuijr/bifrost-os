//! Order statistics of a sample set. Percentiles use the nearest-rank method: the p-th percentile of n sorted values
//! is the value at rank `ceil(p * n / 100)` (1-based), so it is always one of the inputs.

/// Summary of a set of values (milliseconds, frames, whatever the caller measures).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p99: f64,
}

impl Summary {
    /// `p99 - p50`, the spread a user hears as timing jitter.
    pub fn jitter(&self) -> f64 {
        self.p99 - self.p50
    }

    /// The summary of nothing: all zero, `n == 0`.
    pub fn empty() -> Summary {
        Summary { n: 0, min: 0.0, max: 0.0, p50: 0.0, p99: 0.0 }
    }
}

/// Nearest-rank percentile of an ascending slice. `None` when the slice is empty.
pub fn percentile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    // p * n first, then / 100: both steps are exact for the integer percentiles used here.
    let rank = ((p * n as f64) / 100.0).ceil() as usize;
    Some(sorted[rank.clamp(1, n) - 1])
}

/// Sorts a copy and summarizes it. `None` for an empty input.
pub fn summarize(values: &[f64]) -> Option<Summary> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    Some(Summary {
        n: v.len(),
        min: v[0],
        max: v[v.len() - 1],
        p50: percentile_sorted(&v, 50.0)?,
        p99: percentile_sorted(&v, 99.0)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_to_hundred() {
        let v: Vec<f64> = (1..=100).map(|x| x as f64).collect();
        let s = summarize(&v).unwrap();
        assert_eq!(s, Summary { n: 100, min: 1.0, max: 100.0, p50: 50.0, p99: 99.0 });
        assert_eq!(s.jitter(), 49.0);
    }

    #[test]
    fn unsorted_input_and_three_values() {
        // n = 3: p50 rank = ceil(1.5) = 2, p99 rank = ceil(2.97) = 3.
        let s = summarize(&[3.0, 1.0, 2.0]).unwrap();
        assert_eq!((s.min, s.p50, s.p99, s.max), (1.0, 2.0, 3.0, 3.0));
    }

    #[test]
    fn single_value_and_empty() {
        let s = summarize(&[5.0]).unwrap();
        assert_eq!((s.min, s.p50, s.p99, s.max, s.jitter()), (5.0, 5.0, 5.0, 5.0, 0.0));
        assert_eq!(summarize(&[]), None);
        assert_eq!(percentile_sorted(&[], 50.0), None);
        assert_eq!(Summary::empty().n, 0);
    }

    #[test]
    fn two_values_and_ties() {
        // n = 2: p50 rank = ceil(1.0) = 1, p99 rank = ceil(1.98) = 2.
        let s = summarize(&[10.0, 2.0]).unwrap();
        assert_eq!((s.p50, s.p99), (2.0, 10.0));
        let t = summarize(&[4.0, 4.0, 4.0, 4.0]).unwrap();
        assert_eq!((t.p50, t.p99, t.jitter()), (4.0, 4.0, 0.0));
    }

    #[test]
    fn percentile_extremes_clamp() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile_sorted(&v, 0.0), Some(1.0));
        assert_eq!(percentile_sorted(&v, 100.0), Some(4.0));
        assert_eq!(percentile_sorted(&v, 25.0), Some(1.0));
        assert_eq!(percentile_sorted(&v, 75.0), Some(3.0));
    }
}
