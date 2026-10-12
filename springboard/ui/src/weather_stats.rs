//! Historical weather statistics: a summary of a series of daily values (`Summary`), percentiles (`percentile`), the
//! least-squares slope of a series (`trend_per_step`) and how a current value sits against a baseline of past values
//! (`compare`, with `Deviation`). These cover the statistics and comparison types of the official weather framework
//! (`DailyWeatherStatistics`, `Percentiles`, `Trend`, `HistoricalComparisons`) for values the caller already holds. No
//! data is fetched here. Non-finite values are ignored, so one bad sample does not poison a summary.

/// The percentile of a sorted, finite series, by linear interpolation between the two nearest ranks (the common
/// definition: rank `(n - 1) * p / 100`). `p` is in the range 0 to 100; anything else, or an empty series, gives `None`.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if !(0.0..=100.0).contains(&p) {
        return None;
    }
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (sorted.len() - 1) as f64 * p / 100.0;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    Some(sorted[lo] + frac * (sorted[hi] - sorted[lo]))
}

/// Count, extremes, mean and median of a series of values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
}

/// Summarizes the finite values of a series. Returns `None` when no finite value is left.
pub fn summarize(values: &[f64]) -> Option<Summary> {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return None;
    }
    let count = finite.len();
    let sum: f64 = finite.iter().sum();
    let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
    let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some(Summary {
        count,
        min,
        max,
        mean: sum / count as f64,
        median: percentile(&finite, 50.0)?,
    })
}

/// The least-squares slope of the finite values against their position in the series: the average change per step.
/// A series of fewer than two finite values has no slope and gives `None`.
pub fn trend_per_step(values: &[f64]) -> Option<f64> {
    let ys: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let n = ys.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let sum_x = (n * (n - 1) / 2) as f64;
    let sum_x2: f64 = (0..n).map(|i| (i * i) as f64).sum();
    let sum_y: f64 = ys.iter().sum();
    let sum_xy: f64 = ys.iter().enumerate().map(|(i, y)| i as f64 * y).sum();
    let denom = nf * sum_x2 - sum_x * sum_x;
    if denom == 0.0 {
        return None;
    }
    Some((nf * sum_xy - sum_x * sum_y) / denom)
}

/// Where a current value sits against its baseline: above the upper quartile, below the lower one, or in between.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Deviation {
    Above,
    Normal,
    Below,
}

/// Compares `current` with the interquartile range of `baseline` (the 25th and 75th percentiles). A value strictly
/// above the 75th percentile is `Above`, strictly below the 25th is `Below`, and anything else is `Normal`. An empty
/// baseline gives `None`.
pub fn compare(current: f64, baseline: &[f64]) -> Option<Deviation> {
    if !current.is_finite() {
        return None;
    }
    let low = percentile(baseline, 25.0)?;
    let high = percentile(baseline, 75.0)?;
    Some(if current > high {
        Deviation::Above
    } else if current < low {
        Deviation::Below
    } else {
        Deviation::Normal
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_interpolate_between_ranks() {
        // Ranks for [1, 2, 3, 4] are (n - 1) * p / 100 = 3p / 100.
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), Some(1.0));
        assert_eq!(percentile(&v, 50.0), Some(2.5));
        assert_eq!(percentile(&v, 25.0), Some(1.75));
        assert_eq!(percentile(&v, 100.0), Some(4.0));
    }

    #[test]
    fn percentile_rejects_bad_input() {
        assert_eq!(percentile(&[1.0, 2.0], 101.0), None);
        assert_eq!(percentile(&[], 50.0), None);
        // A non-finite value is ignored, so the median of [5.0, NaN] is 5.0.
        assert_eq!(percentile(&[5.0, f64::NAN], 50.0), Some(5.0));
    }

    #[test]
    fn summary_gives_count_extremes_mean_and_median() {
        // Sorted: 2 4 4 4 5 5 7 9. The sum is 40, so the mean is 5.0. The median sits at rank 3.5: (4 + 5) / 2.
        let s = summarize(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap();
        assert_eq!(s.count, 8);
        assert_eq!(s.min, 2.0);
        assert_eq!(s.max, 9.0);
        assert_eq!(s.mean, 5.0);
        assert_eq!(s.median, 4.5);
    }

    #[test]
    fn summary_of_nothing_is_none() {
        assert_eq!(summarize(&[]), None);
        assert_eq!(summarize(&[f64::INFINITY]), None);
    }

    #[test]
    fn trend_is_the_least_squares_slope() {
        // Points (0,1), (1,3), (2,5), (3,7) lie on y = 2x + 1, so the slope is 2.0.
        assert_eq!(trend_per_step(&[1.0, 3.0, 5.0, 7.0]), Some(2.0));
        // A flat series has slope 0.0.
        assert_eq!(trend_per_step(&[5.0, 5.0, 5.0]), Some(0.0));
        // One value has no slope.
        assert_eq!(trend_per_step(&[5.0]), None);
    }

    #[test]
    fn comparison_uses_the_interquartile_range_of_the_baseline() {
        // Baseline 1..=9: the 25th percentile is rank 2 (value 3) and the 75th is rank 6 (value 7).
        let baseline: Vec<f64> = (1..=9).map(|i| i as f64).collect();
        assert_eq!(compare(8.0, &baseline), Some(Deviation::Above));
        assert_eq!(compare(2.0, &baseline), Some(Deviation::Below));
        assert_eq!(compare(5.0, &baseline), Some(Deviation::Normal));
        // Exactly on the 75th percentile is still Normal: the comparison is strict.
        assert_eq!(compare(7.0, &baseline), Some(Deviation::Normal));
        assert_eq!(compare(5.0, &[]), None);
    }
}
