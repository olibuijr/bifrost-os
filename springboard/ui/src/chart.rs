//! Chart geometry for data-driven charts: linear scales, automatic axis values, equal-width bins and
//! pie or donut sector spans. Pure functions over numbers; renderers draw what they return.
use std::f64::consts::TAU;

/// Maps a data domain onto a range, linearly. Values outside the domain are not clamped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearScale {
    pub domain: (f64, f64),
    pub range: (f64, f64),
}

impl LinearScale {
    pub fn new(domain: (f64, f64), range: (f64, f64)) -> Self {
        Self { domain, range }
    }

    /// Position on the range for a data value. A degenerate domain maps everything to the range start.
    pub fn map(&self, value: f64) -> f64 {
        let (d0, d1) = self.domain;
        if d1 == d0 {
            return self.range.0;
        }
        self.range.0 + (value - d0) / (d1 - d0) * (self.range.1 - self.range.0)
    }

    /// Data value at a position on the range: the inverse of `map`.
    pub fn invert(&self, position: f64) -> f64 {
        let (r0, r1) = self.range;
        if r1 == r0 {
            return self.domain.0;
        }
        self.domain.0 + (position - r0) / (r1 - r0) * (self.domain.1 - self.domain.0)
    }
}

/// Evenly spaced axis values covering `lo..=hi` with a step of 1, 2 or 5 times a power of ten, about
/// `count` values in all. Returns an empty list for `count == 0` or a non-finite bound.
pub fn axis_ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    if count == 0 || !lo.is_finite() || !hi.is_finite() {
        return vec![];
    }
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let span = hi - lo;
    if span <= 0.0 {
        return vec![lo];
    }
    let raw = span / count as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    // Work in whole units of 1/scale so the values print exactly ("0.3", not "0.30000000000000004").
    let scale = 10f64.powi((-mag.log10().floor()).max(0.0) as i32);
    let unit = (step * scale).round() as i64;
    let first = (lo * scale / unit as f64).ceil() as i64;
    let last = (hi * scale / unit as f64 + 1e-9).floor() as i64;
    (first..=last).map(|i| (i * unit) as f64 / scale).collect()
}

/// One equal-width bin of a histogram. Every bin includes its lower edge; the last one also includes its upper edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bin {
    pub lower: f64,
    pub upper: f64,
    pub count: usize,
}

/// Splits the finite values' range into `count` equal-width bins and counts the values in each.
/// Non-finite values are ignored. Returns an empty list when nothing is finite or `count` is 0.
pub fn number_bins(values: &[f64], count: usize) -> Vec<Bin> {
    let finite = || values.iter().copied().filter(|v| v.is_finite());
    let lo = finite().fold(f64::INFINITY, f64::min);
    let hi = finite().fold(f64::NEG_INFINITY, f64::max);
    if count == 0 || lo > hi {
        return vec![];
    }
    // A single repeated value still gets a unit-wide bin, so the bins never have zero width.
    let width = if hi > lo { (hi - lo) / count as f64 } else { 1.0 };
    let mut bins: Vec<Bin> = (0..count)
        .map(|i| Bin { lower: lo + width * i as f64, upper: lo + width * (i + 1) as f64, count: 0 })
        .collect();
    for v in finite() {
        let i = (((v - lo) / width) as usize).min(count - 1);
        bins[i].count += 1;
    }
    bins
}

/// The angular span of one sector of a pie or donut, in radians, measured clockwise from straight up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sector {
    pub start: f64,
    pub end: f64,
}

/// Lays out one sector per value, each sized in proportion to its value and starting where the previous
/// one ends, clockwise from `start` (0 points up, as on a clock face). Zero, negative and non-finite values
/// take no angle, and every value keeps its index. With no positive total, every sector has zero width.
pub fn sectors(values: &[f64], start: f64) -> Vec<Sector> {
    let total: f64 = values.iter().filter(|v| v.is_finite() && **v > 0.0).sum();
    let mut angle = start;
    values
        .iter()
        .map(|v| {
            let share = if total > 0.0 && v.is_finite() && *v > 0.0 { v / total * TAU } else { 0.0 };
            let sector = Sector { start: angle, end: angle + share };
            angle += share;
            sector
        })
        .collect()
}

/// The point at `radius` from `center` along `angle` (radians, clockwise from straight up). Screen y grows downward.
pub fn point(center: (f64, f64), radius: f64, angle: f64) -> (f64, f64) {
    (center.0 + radius * angle.sin(), center.1 - radius * angle.cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn linear_scale_maps_both_ways() {
        let s = LinearScale::new((0.0, 10.0), (0.0, 200.0));
        assert!(close(s.map(2.5), 50.0));
        assert!(close(s.map(-5.0), -100.0), "values outside the domain are not clamped");
        assert!(close(s.invert(50.0), 2.5));
        let flipped = LinearScale::new((0.0, 10.0), (300.0, 100.0));
        assert!(close(flipped.map(0.0), 300.0) && close(flipped.map(10.0), 100.0), "an inverted range flips the axis");
        let flat = LinearScale::new((4.0, 4.0), (10.0, 90.0));
        assert_eq!(flat.map(4.0), 10.0, "a degenerate domain maps to the range start");
    }

    #[test]
    fn axis_ticks_pick_round_steps() {
        assert_eq!(axis_ticks(0.0, 10.0, 5), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(axis_ticks(-3.0, 7.0, 5), vec![-2.0, 0.0, 2.0, 4.0, 6.0]);
        assert_eq!(axis_ticks(0.0, 1.0, 4), vec![0.0, 0.5, 1.0]);
        assert_eq!(axis_ticks(0.0, 3.0, 10), vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0], "a raw step of 0.3 rounds up to 0.5");
    }

    #[test]
    fn axis_ticks_refuse_bad_input() {
        assert!(axis_ticks(0.0, 10.0, 0).is_empty());
        assert!(axis_ticks(f64::NAN, 10.0, 5).is_empty());
        assert_eq!(axis_ticks(7.0, 7.0, 5), vec![7.0]);
        assert_eq!(axis_ticks(10.0, 0.0, 5), axis_ticks(0.0, 10.0, 5), "reversed bounds are sorted first");
    }

    #[test]
    fn number_bins_count_each_range() {
        let bins = number_bins(&[0.0, 1.0, 2.0, 3.0, 10.0], 2);
        assert_eq!(bins.len(), 2);
        assert_eq!((bins[0].lower, bins[0].upper, bins[0].count), (0.0, 5.0, 4));
        assert_eq!((bins[1].lower, bins[1].upper, bins[1].count), (5.0, 10.0, 1), "the maximum lands in the last bin");
        let total: usize = bins.iter().map(|b| b.count).sum();
        assert_eq!(total, 5, "every finite value is counted once");
    }

    #[test]
    fn number_bins_skip_non_finite_and_empty_input() {
        let bins = number_bins(&[f64::NAN, 4.0, f64::INFINITY], 3);
        assert_eq!(bins.iter().map(|b| b.count).sum::<usize>(), 1);
        assert!(number_bins(&[f64::NAN], 3).is_empty());
        assert!(number_bins(&[1.0, 2.0], 0).is_empty());
        let same = number_bins(&[4.0, 4.0], 2);
        assert_eq!(same[0].count, 2, "repeated values share a bin instead of dividing by zero");
    }

    #[test]
    fn sectors_share_the_full_turn() {
        let s = sectors(&[1.0, 1.0, 2.0], 0.0);
        assert_eq!(s.len(), 3);
        assert!(close(s[0].end, TAU / 4.0));
        assert!(close(s[1].start, TAU / 4.0) && close(s[1].end, TAU / 2.0));
        assert!(close(s[2].end, TAU), "the last sector closes the circle");
    }

    #[test]
    fn sectors_ignore_values_without_a_size() {
        let s = sectors(&[-1.0, 3.0, f64::NAN], 1.0);
        assert_eq!(s[0].start, s[0].end, "a negative value takes no angle");
        assert!(close(s[1].end - s[1].start, TAU));
        assert_eq!(s[2].start, s[2].end);
        let none = sectors(&[0.0, 0.0], 0.5);
        assert!(none.iter().all(|x| x.start == 0.5 && x.end == 0.5));
    }

    #[test]
    fn point_runs_clockwise_from_up() {
        let (x, y) = point((0.0, 0.0), 10.0, 0.0);
        assert!(close(x, 0.0) && close(y, -10.0), "angle 0 is straight up, y grows downward");
        let (x, y) = point((5.0, 5.0), 10.0, TAU / 4.0);
        assert!(close(x, 15.0) && close(y, 5.0), "a quarter turn points right");
    }
}
