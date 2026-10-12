//! Statistical summaries of one column of a table (the `NumericSummary` and `CategoricalSummary` model): counts of
//! present and missing elements, and for numbers the mean, sample standard deviation, extremes, median and
//! quartiles; for categories the number of distinct values and the most common values.
//!
//! A missing element is `None`. For numbers a NaN counts as missing too, so one bad reading does not poison the
//! statistics. Pure model: the caller supplies the column and draws the result.
//!
//! The official page for the numeric summary does not state the divisor of the standard deviation or the
//! interpolation of the quartiles. This model uses the sample deviation (divisor n - 1) and nearest-rank quartiles
//! (the value at rank ceil(q * n)), and the median is the mean of the two middle values for an even count.
//!
//! Gaps: no type-erased summary, no data frame or typed column to summarise, no per-column statistic streaming, and
//! no formatting of the summary (its debug description).

/// Statistics of a numeric column. Every statistic other than the counts is `None` when it has no value to use:
/// the mean, extremes, median and quartiles need one present element, the standard deviation needs two.
#[derive(Clone, Debug, PartialEq)]
pub struct NumericSummary {
    /// Present (non-missing) elements.
    pub some_count: usize,
    /// Missing elements, including NaN.
    pub none_count: usize,
    pub mean: Option<f64>,
    pub standard_deviation: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub median: Option<f64>,
    pub first_quartile: Option<f64>,
    pub third_quartile: Option<f64>,
}

impl NumericSummary {
    /// Summarise a column. `None` and NaN are missing.
    pub fn new(column: &[Option<f64>]) -> Self {
        let mut present: Vec<f64> = column.iter().flatten().copied().filter(|v| !v.is_nan()).collect();
        let none_count = column.len() - present.len();
        present.sort_by(|a, b| a.total_cmp(b));
        let n = present.len();
        if n == 0 {
            return NumericSummary {
                some_count: 0,
                none_count,
                mean: None,
                standard_deviation: None,
                min: None,
                max: None,
                median: None,
                first_quartile: None,
                third_quartile: None,
            };
        }
        let mean = present.iter().sum::<f64>() / n as f64;
        let standard_deviation = if n < 2 {
            None
        } else {
            let squares: f64 = present.iter().map(|v| (v - mean) * (v - mean)).sum();
            Some((squares / (n - 1) as f64).sqrt())
        };
        let median = if n % 2 == 1 {
            present[n / 2]
        } else {
            (present[n / 2 - 1] + present[n / 2]) / 2.0
        };
        NumericSummary {
            some_count: n,
            none_count,
            mean: Some(mean),
            standard_deviation,
            min: Some(present[0]),
            max: Some(present[n - 1]),
            median: Some(median),
            first_quartile: Some(nearest_rank(&present, 0.25)),
            third_quartile: Some(nearest_rank(&present, 0.75)),
        }
    }

    /// Present and missing elements together.
    pub fn total_count(&self) -> usize {
        self.some_count + self.none_count
    }
}

/// The value at rank ceil(q * n) of the sorted, non-empty list (ranks start at 1).
fn nearest_rank(sorted: &[f64], q: f64) -> f64 {
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.max(1) - 1]
}

/// Statistics of a categorical column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoricalSummary<T> {
    /// Present (non-missing) elements.
    pub some_count: usize,
    /// Missing elements.
    pub none_count: usize,
    /// Distinct present values.
    pub unique_count: usize,
    /// The most common present values, in ascending order. Ties all appear. Empty when no element is present.
    pub mode: Vec<T>,
}

impl<T: Ord + Clone> CategoricalSummary<T> {
    /// Summarise a column. `None` is missing.
    pub fn new(column: &[Option<T>]) -> Self {
        let mut counts: std::collections::BTreeMap<&T, usize> = std::collections::BTreeMap::new();
        for value in column.iter().flatten() {
            *counts.entry(value).or_insert(0) += 1;
        }
        let top = counts.values().copied().max().unwrap_or(0);
        let mode = counts.iter().filter(|(_, c)| **c == top).map(|(v, _)| (*v).clone()).collect();
        let some_count: usize = counts.values().sum();
        CategoricalSummary {
            some_count,
            none_count: column.len() - some_count,
            unique_count: counts.len(),
            mode,
        }
    }

    /// Present and missing elements together.
    pub fn total_count(&self) -> usize {
        self.some_count + self.none_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|v| (v - b).abs() < 1e-9)
    }

    #[test]
    fn even_count_with_a_missing_element_uses_the_middle_pair_and_nearest_ranks() {
        // Present: 1 2 3 4. Missing: one.
        let s = NumericSummary::new(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0), None]);
        assert_eq!((s.some_count, s.none_count, s.total_count()), (4, 1, 5));
        assert!(close(s.mean, 2.5), "10 / 4");
        assert!(close(s.median, 2.5), "(2 + 3) / 2");
        assert!(close(s.first_quartile, 1.0), "rank ceil(0.25 * 4) = 1");
        assert!(close(s.third_quartile, 3.0), "rank ceil(0.75 * 4) = 3");
        assert!(close(s.min, 1.0) && close(s.max, 4.0));
        // Deviations -1.5, -0.5, 0.5, 1.5 square to 5; divided by n - 1 = 3, then square-rooted.
        assert!(close(s.standard_deviation, (5.0f64 / 3.0).sqrt()));
    }

    #[test]
    fn odd_count_takes_the_middle_element_and_unsorted_input_is_fine() {
        let s = NumericSummary::new(&[Some(5.0), Some(1.0), Some(3.0)]);
        assert!(close(s.median, 3.0));
        assert!(close(s.first_quartile, 1.0), "rank ceil(0.75) = 1");
        assert!(close(s.third_quartile, 5.0), "rank ceil(2.25) = 3");
        assert!(close(s.mean, 3.0));
        assert!(close(s.standard_deviation, 2.0), "deviations -2, 0, 2 square to 8; 8 / (3 - 1) = 4, root 2");
    }

    #[test]
    fn nan_counts_as_missing_and_an_empty_column_has_no_statistics() {
        let s = NumericSummary::new(&[Some(f64::NAN), None]);
        assert_eq!((s.some_count, s.none_count), (0, 2));
        assert_eq!((s.mean, s.min, s.median), (None, None, None));

        let empty = NumericSummary::new(&[]);
        assert_eq!(empty.total_count(), 0);
        assert_eq!(empty.standard_deviation, None);
    }

    #[test]
    fn a_single_value_has_a_mean_but_no_standard_deviation() {
        let s = NumericSummary::new(&[None, Some(7.0)]);
        assert!(close(s.mean, 7.0) && close(s.median, 7.0));
        assert_eq!(s.standard_deviation, None, "one value has no spread to measure");
        assert_eq!(s.none_count, 1);
    }

    #[test]
    fn categorical_counts_distinct_values_and_keeps_every_tie_in_the_mode() {
        let s = CategoricalSummary::new(&[Some(1), Some(2), Some(2), Some(1), Some(3)]);
        assert_eq!((s.some_count, s.none_count, s.unique_count), (5, 0, 3));
        assert_eq!(s.mode, vec![1, 2], "1 and 2 both appear twice, in ascending order");
    }

    #[test]
    fn categorical_mode_is_the_single_most_common_value_and_missing_is_not_a_category() {
        let column = [Some("a"), Some("b"), Some("a"), None, Some("c"), Some("a"), Some("b")];
        let s = CategoricalSummary::new(&column);
        assert_eq!((s.some_count, s.none_count, s.unique_count), (6, 1, 3));
        assert_eq!(s.mode, vec!["a"]);
        assert_eq!(s.total_count(), 7);
    }

    #[test]
    fn an_all_missing_or_empty_categorical_column_has_no_mode() {
        let all_missing: CategoricalSummary<u8> = CategoricalSummary::new(&[None, None]);
        assert_eq!((all_missing.some_count, all_missing.none_count, all_missing.unique_count), (0, 2, 0));
        assert!(all_missing.mode.is_empty());
        let empty: CategoricalSummary<u8> = CategoricalSummary::new(&[]);
        assert_eq!(empty.total_count(), 0);
    }
}
