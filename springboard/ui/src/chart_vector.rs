//! Batched plots for large data sets. One pass over a series produces the screen geometry for all of it, so a
//! series with thousands of points costs one sort and one walk, and the output stays small enough to draw. Pure
//! functions over f64 values in logical points, built on the plot from `chart_marks`.
use crate::chart_marks::{Frame, Plot};

/// Reduces a series to at most four screen points per plot column: the first, last, lowest and highest point that
/// falls in that column. A line through the result keeps every peak and trough of the series, which a plain stride
/// through the data would lose. The output is in x order. Non-finite pairs are dropped. `columns` is the number of
/// columns across the frame; zero returns nothing.
pub fn decimate(plot: &Plot, data: &[(f64, f64)], columns: usize) -> Vec<(f64, f64)> {
    if columns == 0 {
        return vec![];
    }
    let mut sorted: Vec<(f64, f64)> = data.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let screen: Vec<(f64, f64)> = sorted.iter().map(|&(x, y)| plot.position(x, y)).collect();
    let cols: Vec<usize> = screen.iter().map(|&(px, _)| column_of(plot.frame, px, columns)).collect();

    let mut out = Vec::new();
    let mut start = 0;
    while start < screen.len() {
        let col = cols[start];
        let mut end = start + 1;
        while end < screen.len() && cols[end] == col {
            end += 1;
        }
        // Indices into `screen` that this column keeps, all inside start..end.
        let (mut low, mut high) = (start, start);
        for i in start..end {
            if screen[i].1 < screen[low].1 {
                low = i;
            }
            if screen[i].1 > screen[high].1 {
                high = i;
            }
        }
        let mut keep = vec![start, end - 1, low, high];
        keep.sort_unstable();
        keep.dedup();
        out.extend(keep.into_iter().map(|i| screen[i]));
        start = end;
    }
    out
}

/// The column a screen x position falls in, clamped to `0..columns`. A frame with no width puts everything in column 0.
fn column_of(frame: Frame, px: f64, columns: usize) -> usize {
    if !(frame.w > 0.0) || !px.is_finite() {
        return 0;
    }
    let c = ((px - frame.x) / frame.w * columns as f64).floor();
    (c.max(0.0) as usize).min(columns - 1)
}

/// Batched bars, one per valid data pair: centred on the pair's x value, `width` points wide, rising from the
/// `baseline` data value to the pair's y value. Bars drop below the baseline when y is smaller. Returns nothing
/// for a width that is not a finite positive number, or a baseline that is not finite.
pub fn bar_plot(plot: &Plot, data: &[(f64, f64)], width: f64, baseline: f64) -> Vec<Frame> {
    if !(width.is_finite() && width > 0.0) || !baseline.is_finite() {
        return vec![];
    }
    let base = plot.y.map(baseline);
    data.iter()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|&(x, y)| {
            let (cx, top) = plot.position(x, y);
            Frame::new(cx - width / 2.0, top.min(base), width, (top - base).abs())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot() -> Plot {
        Plot::new(Frame::new(0.0, 0.0, 100.0, 50.0), (0.0, 100.0), (0.0, 50.0))
    }

    #[test]
    fn decimate_keeps_the_extremes_of_each_column() {
        // x 0..100 maps one to one onto a 100 point wide frame, and 10 columns are 10 points wide.
        // Column 0 holds x 0, 0.5, 0.7 and 1.2. Column 1 holds x 10.
        let data = [(0.0, 0.0), (0.5, 10.0), (0.7, 20.0), (1.2, 5.0), (10.0, 30.0)];
        let out = decimate(&plot(), &data, 10);
        // Screen y is 50 - value. In column 0 the lowest screen y is value 20 at x 0.7, the highest is value 0 at x 0.
        assert_eq!(out, vec![(0.0, 50.0), (0.7, 30.0), (1.2, 45.0), (10.0, 20.0)]);
    }

    #[test]
    fn decimate_bounds_a_large_series() {
        let data: Vec<(f64, f64)> = (0..10_000).map(|i| (i as f64 / 100.0, (i % 7) as f64)).collect();
        let out = decimate(&plot(), &data, 50);
        assert!(out.len() <= 200, "at most four points per column, got {}", out.len());
        assert!(out.len() >= 50, "every column with data keeps a point");
        assert!(out.windows(2).all(|w| w[0].0 <= w[1].0), "output stays in x order");
    }

    #[test]
    fn decimate_refuses_bad_input() {
        assert!(decimate(&plot(), &[(1.0, 1.0)], 0).is_empty());
        assert!(decimate(&plot(), &[], 10).is_empty());
        let out = decimate(&plot(), &[(f64::NAN, 1.0), (4.0, f64::INFINITY), (5.0, 5.0)], 10);
        assert_eq!(out, vec![(5.0, 45.0)], "non-finite pairs are dropped");
    }

    #[test]
    fn bar_plot_rises_from_the_baseline() {
        let p = Plot::new(Frame::new(0.0, 0.0, 100.0, 50.0), (0.0, 10.0), (0.0, 5.0));
        // x 5 maps to 50, y 3 maps to 50 - 30 = 20, the baseline 0 maps to 50.
        let bars = bar_plot(&p, &[(5.0, 3.0)], 8.0, 0.0);
        assert_eq!(bars, vec![Frame::new(46.0, 20.0, 8.0, 30.0)]);
    }

    #[test]
    fn bar_plot_hangs_negative_values_below_the_baseline() {
        let p = Plot::new(Frame::new(0.0, 0.0, 100.0, 50.0), (0.0, 10.0), (0.0, 5.0));
        // Value -1 maps to 60, below the baseline at 50, so the bar spans 50..60.
        let bars = bar_plot(&p, &[(2.0, -1.0)], 4.0, 0.0);
        assert_eq!(bars, vec![Frame::new(18.0, 50.0, 4.0, 10.0)]);
    }

    #[test]
    fn bar_plot_skips_bad_pairs_and_widths() {
        let p = Plot::new(Frame::new(0.0, 0.0, 100.0, 50.0), (0.0, 10.0), (0.0, 5.0));
        assert_eq!(bar_plot(&p, &[(f64::NAN, 1.0), (5.0, 3.0)], 8.0, 0.0).len(), 1);
        assert!(bar_plot(&p, &[(5.0, 3.0)], 0.0, 0.0).is_empty());
        assert!(bar_plot(&p, &[(5.0, 3.0)], 8.0, f64::NAN).is_empty());
    }
}
