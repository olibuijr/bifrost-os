//! Mark geometry for data charts: point, rectangle and rule marks, line and area paths, sector outlines,
//! annotation placement, and the conversions between data values and screen points. Pure functions over f64
//! values in logical points. Screen y grows downward, so the y scale runs from the bottom of the plot up to its top.
use crate::chart::{point, LinearScale, Sector};

/// A rectangle in logical points: left edge `x`, top edge `y`, width `w` and height `h`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The plot area of a chart and the scales that place data values inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plot {
    pub frame: Frame,
    pub x: LinearScale,
    pub y: LinearScale,
}

impl Plot {
    /// Maps `x_domain` across the frame's width, left to right, and `y_domain` up its height, so larger values sit higher.
    pub fn new(frame: Frame, x_domain: (f64, f64), y_domain: (f64, f64)) -> Self {
        Self {
            frame,
            x: LinearScale::new(x_domain, (frame.x, frame.right())),
            y: LinearScale::new(y_domain, (frame.bottom(), frame.y)),
        }
    }

    /// Screen position of a data value pair. The plot's counterpart of a chart proxy's `position(for:)`.
    pub fn position(&self, x: f64, y: f64) -> (f64, f64) {
        (self.x.map(x), self.y.map(y))
    }

    /// Data values at a screen position, the inverse of `position`. Like a chart proxy's `value(at:)`.
    pub fn value(&self, px: f64, py: f64) -> (f64, f64) {
        (self.x.invert(px), self.y.invert(py))
    }
}

/// Screen positions of point marks. A pair with a non-finite value is skipped, as chart content skips invalid data.
pub fn point_marks(plot: &Plot, data: &[(f64, f64)]) -> Vec<(f64, f64)> {
    data.iter()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|&(x, y)| plot.position(x, y))
        .collect()
}

/// Screen frame of a rectangle mark spanning the data values `x0..x1` and `y0..y1`. Reversed bounds are normalised.
/// None when a bound is not finite.
pub fn rectangle_mark(plot: &Plot, x0: f64, x1: f64, y0: f64, y1: f64) -> Option<Frame> {
    if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (ax, bx) = (plot.x.map(x0), plot.x.map(x1));
    let (ay, by) = (plot.y.map(y0), plot.y.map(y1));
    Some(Frame::new(ax.min(bx), ay.min(by), (bx - ax).abs(), (by - ay).abs()))
}

/// A straight segment on screen, from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from: (f64, f64),
    pub to: (f64, f64),
}

/// A vertical rule mark at data value `x`, spanning `y0..y1`. None when a value is not finite.
pub fn vertical_rule(plot: &Plot, x: f64, y0: f64, y1: f64) -> Option<Segment> {
    if ![x, y0, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(Segment { from: plot.position(x, y0), to: plot.position(x, y1) })
}

/// A horizontal rule mark at data value `y`, spanning `x0..x1`. None when a value is not finite.
pub fn horizontal_rule(plot: &Plot, y: f64, x0: f64, x1: f64) -> Option<Segment> {
    if ![y, x0, x1].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(Segment { from: plot.position(x0, y), to: plot.position(x1, y) })
}

/// How a line or area joins its points. The step methods hold each value across an interval and change it at
/// the start, the end or the middle of that interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    StepStart,
    StepEnd,
    StepCenter,
}

/// The screen path through the data pairs, in x order. Invalid pairs are dropped, and the input need not be sorted.
pub fn line_path(plot: &Plot, data: &[(f64, f64)], method: Interpolation) -> Vec<(f64, f64)> {
    let mut valid: Vec<(f64, f64)> = data.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
    valid.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pts: Vec<(f64, f64)> = valid.iter().map(|&(x, y)| plot.position(x, y)).collect();
    let mut out = Vec::with_capacity(pts.len() * 2);
    for (i, &a) in pts.iter().enumerate() {
        out.push(a);
        if let Some(&b) = pts.get(i + 1) {
            match method {
                Interpolation::Linear => {}
                // The step changes at the next point, so the current value holds until then.
                Interpolation::StepStart => out.push((b.0, a.1)),
                // The step changes at the current point, so the next value holds from here on.
                Interpolation::StepEnd => out.push((a.0, b.1)),
                Interpolation::StepCenter => {
                    let mid = (a.0 + b.0) / 2.0;
                    out.push((mid, a.1));
                    out.push((mid, b.1));
                }
            }
        }
    }
    out
}

/// The closed outline of an area mark: the line path, then straight down to `baseline` (a data value) and back
/// to the first point. Empty when there is no valid data.
pub fn area_outline(plot: &Plot, data: &[(f64, f64)], method: Interpolation, baseline: f64) -> Vec<(f64, f64)> {
    let mut outline = line_path(plot, data, method);
    if outline.is_empty() {
        return outline;
    }
    let first = outline[0];
    let last = outline[outline.len() - 1];
    let base = plot.y.map(baseline);
    outline.push((last.0, base));
    outline.push((first.0, base));
    outline
}

/// The outline of one pie or donut sector around `center`, with radii `inner..outer` in points. `inset` (radians) is
/// shaved from both ends of the sector, leaving a gap between neighbours. `segments` sets how finely each arc is
/// sampled. The outer arc runs clockwise, then the inner arc back. Empty for bad radii or when no angle is left.
pub fn sector_outline(
    center: (f64, f64),
    inner: f64,
    outer: f64,
    sector: Sector,
    inset: f64,
    segments: usize,
) -> Vec<(f64, f64)> {
    if !inner.is_finite() || !outer.is_finite() || inner < 0.0 || outer <= inner {
        return vec![];
    }
    let (start, end) = (sector.start + inset, sector.end - inset);
    if !(end > start) {
        return vec![];
    }
    let n = segments.max(1);
    let angle = |i: usize| start + (end - start) * i as f64 / n as f64;
    let mut outline: Vec<(f64, f64)> = (0..=n).map(|i| point(center, outer, angle(i))).collect();
    outline.extend((0..=n).rev().map(|i| point(center, inner, angle(i))));
    outline
}

/// Where an annotation sits relative to the item it describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    /// Above the item when it fits inside the plot, otherwise below.
    Automatic,
    Top,
    Bottom,
    Leading,
    Trailing,
    TopLeading,
    TopTrailing,
    BottomLeading,
    BottomTrailing,
    /// Centred on the item.
    Overlay,
}

/// What happens when an annotation would stick out of the plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    /// Slide the annotation back inside the plot. An annotation larger than the plot is pinned to its start edge.
    Fit,
    /// Leave the annotation where its position puts it.
    Disabled,
}

/// The frame of an annotation of `size` (width, height), placed `gap` points away from `item` on the given side.
pub fn place_annotation(item: Frame, size: (f64, f64), position: Position, gap: f64, plot: Frame, overflow: Overflow) -> Frame {
    let (w, h) = size;
    let (cx, cy) = item.center();
    let above = item.y - gap - h;
    let below = item.bottom() + gap;
    let (x, y) = match position {
        Position::Automatic => {
            let resolved = if above >= plot.y { Position::Top } else { Position::Bottom };
            return place_annotation(item, size, resolved, gap, plot, overflow);
        }
        Position::Top => (cx - w / 2.0, above),
        Position::Bottom => (cx - w / 2.0, below),
        Position::Leading => (item.x - gap - w, cy - h / 2.0),
        Position::Trailing => (item.right() + gap, cy - h / 2.0),
        Position::TopLeading => (item.x, above),
        Position::TopTrailing => (item.right() - w, above),
        Position::BottomLeading => (item.x, below),
        Position::BottomTrailing => (item.right() - w, below),
        Position::Overlay => (cx - w / 2.0, cy - h / 2.0),
    };
    match overflow {
        Overflow::Disabled => Frame::new(x, y, w, h),
        Overflow::Fit => Frame::new(fit(x, w, plot.x, plot.w), fit(y, h, plot.y, plot.h), w, h),
    }
}

/// Moves `start` so that a span of `len` lies inside `lo..lo + span`. A span longer than the space is pinned to `lo`.
fn fit(start: f64, len: f64, lo: f64, span: f64) -> f64 {
    if len >= span {
        lo
    } else {
        start.max(lo).min(lo + span - len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::point;
    use std::f64::consts::PI;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    fn plot() -> Plot {
        Plot::new(Frame::new(0.0, 0.0, 100.0, 50.0), (0.0, 10.0), (0.0, 5.0))
    }

    #[test]
    fn plot_maps_larger_values_higher_on_screen() {
        let p = plot();
        assert!(close(p.position(0.0, 0.0), (0.0, 50.0)), "the origin sits at the bottom left");
        assert!(close(p.position(10.0, 5.0), (100.0, 0.0)), "the far corner sits at the top right");
        let (x, y) = p.value(50.0, 25.0);
        assert!((x - 5.0).abs() < 1e-9 && (y - 2.5).abs() < 1e-9, "value is the inverse of position");
    }

    #[test]
    fn point_marks_skip_invalid_pairs() {
        let pts = point_marks(&plot(), &[(0.0, 0.0), (f64::NAN, 1.0), (10.0, 5.0)]);
        assert_eq!(pts.len(), 2);
        assert!(close(pts[1], (100.0, 0.0)));
    }

    #[test]
    fn rectangle_mark_normalises_reversed_bounds() {
        let r = rectangle_mark(&plot(), 8.0, 2.0, 4.0, 1.0).unwrap();
        assert!((r.x - 20.0).abs() < 1e-9 && (r.w - 60.0).abs() < 1e-9, "the left edge is the smaller x");
        assert!((r.y - 10.0).abs() < 1e-9 && (r.h - 30.0).abs() < 1e-9, "the top edge is the larger y");
        assert!(rectangle_mark(&plot(), f64::INFINITY, 2.0, 1.0, 2.0).is_none());
    }

    #[test]
    fn rules_span_the_given_values() {
        let v = vertical_rule(&plot(), 5.0, 0.0, 5.0).unwrap();
        assert!(close(v.from, (50.0, 50.0)) && close(v.to, (50.0, 0.0)));
        let h = horizontal_rule(&plot(), 2.5, 0.0, 10.0).unwrap();
        assert!(close(h.from, (0.0, 25.0)) && close(h.to, (100.0, 25.0)));
        assert!(vertical_rule(&plot(), 1.0, f64::NAN, 2.0).is_none());
    }

    #[test]
    fn linear_line_joins_points_in_x_order() {
        let path = line_path(&plot(), &[(10.0, 0.0), (0.0, 5.0), (f64::NAN, 1.0)], Interpolation::Linear);
        assert_eq!(path.len(), 2, "invalid points are dropped");
        assert!(close(path[0], (0.0, 0.0)) && close(path[1], (100.0, 50.0)), "sorted by x before joining");
    }

    #[test]
    fn step_lines_change_where_the_method_says() {
        let data = [(0.0, 0.0), (5.0, 5.0)];
        let start = line_path(&plot(), &data, Interpolation::StepStart);
        assert!(close(start[1], (50.0, 50.0)), "start: the step changes at the next point");
        let end = line_path(&plot(), &data, Interpolation::StepEnd);
        assert!(close(end[1], (0.0, 0.0)), "end: the step changes at the first point");
        let center = line_path(&plot(), &data, Interpolation::StepCenter);
        assert_eq!(center.len(), 4);
        assert!(close(center[1], (25.0, 50.0)) && close(center[2], (25.0, 0.0)), "center: the step changes midway");
    }

    #[test]
    fn area_outline_closes_on_the_baseline() {
        let outline = area_outline(&plot(), &[(0.0, 5.0), (10.0, 5.0)], Interpolation::Linear, 0.0);
        assert_eq!(outline.len(), 4);
        assert!(close(outline[2], (100.0, 50.0)) && close(outline[3], (0.0, 50.0)), "back along the baseline to the start");
        assert!(area_outline(&plot(), &[], Interpolation::Linear, 0.0).is_empty());
    }

    #[test]
    fn sector_outline_follows_both_radii() {
        let s = Sector { start: 0.0, end: PI / 2.0 };
        let ring = sector_outline((0.0, 0.0), 5.0, 10.0, s, 0.0, 4);
        assert_eq!(ring.len(), 10);
        assert!(close(ring[0], (0.0, -10.0)), "the outer arc starts straight up");
        assert!(close(ring[4], (10.0, 0.0)), "the outer arc ends at the right");
        assert!(close(ring[5], (5.0, 0.0)), "the inner arc comes back at the inner radius");
        assert!(close(ring[9], (0.0, -5.0)));
        let inset = sector_outline((0.0, 0.0), 0.0, 10.0, s, 0.1, 1);
        assert_eq!(inset.len(), 4);
        assert!(close(inset[0], point((0.0, 0.0), 10.0, 0.1)), "the inset shortens both ends");
        assert!(sector_outline((0.0, 0.0), 10.0, 5.0, s, 0.0, 4).is_empty(), "an outer radius inside the inner one is refused");
        assert!(sector_outline((0.0, 0.0), 0.0, 10.0, s, 1.0, 4).is_empty(), "an inset past the sector leaves no angle");
    }

    #[test]
    fn annotations_sit_on_the_requested_side() {
        let item = Frame::new(40.0, 20.0, 20.0, 10.0);
        let bounds = Frame::new(0.0, 0.0, 100.0, 50.0);
        let top = place_annotation(item, (30.0, 10.0), Position::Top, 2.0, bounds, Overflow::Fit);
        assert!((top.x - 35.0).abs() < 1e-9 && (top.y - 8.0).abs() < 1e-9, "centred above the item with a 2 pt gap");
        let trailing = place_annotation(item, (30.0, 10.0), Position::Trailing, 2.0, bounds, Overflow::Disabled);
        assert!((trailing.x - 62.0).abs() < 1e-9 && (trailing.y - 20.0).abs() < 1e-9, "level with the item's centre");
        let overlay = place_annotation(item, (30.0, 10.0), Position::Overlay, 0.0, bounds, Overflow::Fit);
        assert!((overlay.x - 35.0).abs() < 1e-9 && (overlay.y - 20.0).abs() < 1e-9, "centred on the item");
    }

    #[test]
    fn annotations_fit_inside_the_plot_or_stay_put() {
        let bounds = Frame::new(0.0, 0.0, 100.0, 50.0);
        let edge = Frame::new(90.0, 0.0, 5.0, 5.0);
        let fitted = place_annotation(edge, (30.0, 10.0), Position::Trailing, 2.0, bounds, Overflow::Fit);
        assert!((fitted.x - 70.0).abs() < 1e-9 && fitted.y.abs() < 1e-9, "slid back inside the right and top edges");
        let raw = place_annotation(edge, (30.0, 10.0), Position::Trailing, 2.0, bounds, Overflow::Disabled);
        assert!((raw.x - 97.0).abs() < 1e-9 && (raw.y + 2.5).abs() < 1e-9, "no overflow handling leaves it outside");
    }

    #[test]
    fn automatic_position_prefers_above_and_falls_back_below() {
        let bounds = Frame::new(0.0, 0.0, 100.0, 50.0);
        let high = Frame::new(40.0, 20.0, 20.0, 10.0);
        let up = place_annotation(high, (30.0, 10.0), Position::Automatic, 2.0, bounds, Overflow::Fit);
        assert!((up.y - 8.0).abs() < 1e-9, "room above, so it goes on top");
        let at_top = Frame::new(40.0, 0.0, 20.0, 10.0);
        let down = place_annotation(at_top, (30.0, 10.0), Position::Automatic, 2.0, bounds, Overflow::Fit);
        assert!((down.y - 12.0).abs() < 1e-9, "no room above, so it goes below");
    }
}
