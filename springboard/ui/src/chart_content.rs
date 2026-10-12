//! A chart as a list of marks. An app adds the marks it wants (bars, lines, areas, points, rectangles, rules and
//! pie or donut sectors), the chart works out its automatic scale domain from the data, and `render` returns the
//! screen geometry of every mark inside a frame. Pure geometry: the caller draws what comes back. Built from the
//! mark, scale and batch helpers in `chart_marks`, `chart_vector` and `chart`.
use crate::chart::sectors;
use crate::chart_marks::{
    area_outline, horizontal_rule, line_path, point_marks, rectangle_mark, sector_outline, vertical_rule,
};
use crate::chart_marks::{Frame, Interpolation, Plot, Segment};
use crate::chart_vector::bar_plot;

/// Segments per pie or donut arc when a sector is drawn.
const ARC_SEGMENTS: usize = 32;

/// One piece of chart content, in data values.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// Bars of `width` points rising from `baseline` to each (x, y) pair.
    Bar { data: Vec<(f64, f64)>, width: f64, baseline: f64 },
    /// A connected line through the (x, y) pairs.
    Line { data: Vec<(f64, f64)>, method: Interpolation },
    /// A closed area between the line and `baseline`.
    Area { data: Vec<(f64, f64)>, method: Interpolation, baseline: f64 },
    /// A dot per (x, y) pair.
    Point { data: Vec<(f64, f64)> },
    /// A filled rectangle spanning `x` and `y` (each a pair of data values, in either order).
    Rectangle { x: (f64, f64), y: (f64, f64) },
    /// A vertical line at data value `x`, spanning the data values `y`.
    RuleX { x: f64, y: (f64, f64) },
    /// A horizontal line at data value `y`, spanning the data values `x`.
    RuleY { y: f64, x: (f64, f64) },
    /// A pie (`inner_ratio` 0) or donut chart of one sector per value, starting straight up and running clockwise.
    /// `inner_ratio` is the hole's radius as a share of the outer radius, clamped to 0..=1.
    Sector { values: Vec<f64>, inner_ratio: f64 },
}

/// Screen geometry for one mark. Points and paths are in logical points, with y growing downward.
#[derive(Clone, Debug, PartialEq)]
pub enum Drawn {
    Points(Vec<(f64, f64)>),
    Path(Vec<(f64, f64)>),
    Polygon(Vec<(f64, f64)>),
    Rects(Vec<Frame>),
    Segment(Segment),
}

/// The marks of one chart, in drawing order. Build it with `mark`, which returns the chart so calls can chain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chart {
    marks: Vec<Mark>,
}

impl Chart {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one mark after the existing ones.
    pub fn mark(mut self, mark: Mark) -> Self {
        self.marks.push(mark);
        self
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// The automatic scale domains as ((x min, x max), (y min, y max)), covering every finite value the marks plot.
    /// Bars and areas include their baseline on the y axis. A single value gets a unit-wide domain centred on it, so
    /// a one-point chart still has room to draw. None when no mark plots a finite position.
    pub fn domains(&self) -> Option<((f64, f64), (f64, f64))> {
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for mark in &self.marks {
            match mark {
                Mark::Bar { data, baseline, .. } | Mark::Area { data, baseline, .. } => {
                    add_pairs(&mut xs, &mut ys, data);
                    ys.push(*baseline);
                }
                Mark::Line { data, .. } | Mark::Point { data } => add_pairs(&mut xs, &mut ys, data),
                Mark::Rectangle { x, y } => {
                    xs.extend([x.0, x.1]);
                    ys.extend([y.0, y.1]);
                }
                Mark::RuleX { x, y } => {
                    xs.push(*x);
                    ys.extend([y.0, y.1]);
                }
                Mark::RuleY { y, x } => {
                    ys.push(*y);
                    xs.extend([x.0, x.1]);
                }
                Mark::Sector { .. } => {}
            }
        }
        Some((extent(&xs)?, extent(&ys)?))
    }

    /// Screen geometry of every mark inside `frame`, with the given data domains. Marks keep their order. A sector
    /// mark ignores the domains and fills the frame's shorter side.
    pub fn render(&self, frame: Frame, x_domain: (f64, f64), y_domain: (f64, f64)) -> Vec<Drawn> {
        let plot = Plot::new(frame, x_domain, y_domain);
        let mut out = Vec::new();
        for mark in &self.marks {
            match mark {
                Mark::Bar { data, width, baseline } => out.push(Drawn::Rects(bar_plot(&plot, data, *width, *baseline))),
                Mark::Line { data, method } => out.push(Drawn::Path(line_path(&plot, data, *method))),
                Mark::Area { data, method, baseline } => {
                    out.push(Drawn::Polygon(area_outline(&plot, data, *method, *baseline)))
                }
                Mark::Point { data } => out.push(Drawn::Points(point_marks(&plot, data))),
                Mark::Rectangle { x, y } => {
                    let rect = rectangle_mark(&plot, x.0, x.1, y.0, y.1);
                    out.push(Drawn::Rects(rect.into_iter().collect()));
                }
                Mark::RuleX { x, y } => {
                    if let Some(seg) = vertical_rule(&plot, *x, y.0, y.1) {
                        out.push(Drawn::Segment(seg));
                    }
                }
                Mark::RuleY { y, x } => {
                    if let Some(seg) = horizontal_rule(&plot, *y, x.0, x.1) {
                        out.push(Drawn::Segment(seg));
                    }
                }
                Mark::Sector { values, inner_ratio } => {
                    let center = frame.center();
                    let outer = frame.w.min(frame.h) / 2.0;
                    let inner = outer * inner_ratio.clamp(0.0, 1.0);
                    if !(outer > 0.0) {
                        continue;
                    }
                    for sector in sectors(values, 0.0) {
                        let outline = sector_outline(center, inner, outer, sector, 0.0, ARC_SEGMENTS);
                        if !outline.is_empty() {
                            out.push(Drawn::Polygon(outline));
                        }
                    }
                }
            }
        }
        out
    }

    /// `render` with the domains from `domains`, or 0..1 on both axes when the chart has no positioned data.
    pub fn render_auto(&self, frame: Frame) -> Vec<Drawn> {
        let ((x0, x1), (y0, y1)) = self.domains().unwrap_or(((0.0, 1.0), (0.0, 1.0)));
        self.render(frame, (x0, x1), (y0, y1))
    }
}

fn add_pairs(xs: &mut Vec<f64>, ys: &mut Vec<f64>, data: &[(f64, f64)]) {
    for &(x, y) in data {
        xs.push(x);
        ys.push(y);
    }
}

/// The smallest and largest finite value, widened to a unit-wide span when they are equal. None when there is none.
fn extent(values: &[f64]) -> Option<(f64, f64)> {
    let finite = || values.iter().copied().filter(|v| v.is_finite());
    let lo = finite().fold(f64::INFINITY, f64::min);
    let hi = finite().fold(f64::NEG_INFINITY, f64::max);
    if lo > hi {
        None
    } else if lo == hi {
        Some((lo - 0.5, hi + 0.5))
    } else {
        Some((lo, hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn frame() -> Frame {
        Frame::new(0.0, 0.0, 100.0, 50.0)
    }

    #[test]
    fn domains_cover_every_plotted_value() {
        let chart = Chart::new()
            .mark(Mark::Bar { data: vec![(1.0, 2.0), (3.0, 4.0)], width: 5.0, baseline: 0.0 })
            .mark(Mark::Point { data: vec![(0.0, -1.0)] });
        assert_eq!(chart.domains(), Some(((0.0, 3.0), (-1.0, 4.0))), "bars include the baseline, points widen both axes");
    }

    #[test]
    fn a_single_value_gets_a_unit_wide_domain() {
        let chart = Chart::new().mark(Mark::Point { data: vec![(5.0, 5.0)] });
        assert_eq!(chart.domains(), Some(((4.5, 5.5), (4.5, 5.5))));
    }

    #[test]
    fn empty_and_sector_only_charts_have_no_domain() {
        assert_eq!(Chart::new().domains(), None);
        let pie = Chart::new().mark(Mark::Sector { values: vec![1.0, 2.0], inner_ratio: 0.0 });
        assert_eq!(pie.domains(), None, "sectors do not use the data scales");
        assert!(Chart::new().render_auto(frame()).is_empty());
    }

    #[test]
    fn bars_render_as_batched_rectangles() {
        // Plot x 0..10 over 100 points and y 0..5 over 50 points: x 5 maps to 50, y 3 maps to 20, baseline 0 to 50.
        let chart = Chart::new().mark(Mark::Bar { data: vec![(5.0, 3.0)], width: 8.0, baseline: 0.0 });
        let out = chart.render(frame(), (0.0, 10.0), (0.0, 5.0));
        assert_eq!(out, vec![Drawn::Rects(vec![Frame::new(46.0, 20.0, 8.0, 30.0)])]);
    }

    #[test]
    fn step_lines_hold_the_value_until_the_next_point() {
        // Points (0, 0) and (10, 5) map to (0, 50) and (100, 0). StepEnd changes at the start point, so it adds (0, 0).
        let chart = Chart::new().mark(Mark::Line { data: vec![(0.0, 0.0), (10.0, 5.0)], method: Interpolation::StepEnd });
        let out = chart.render(frame(), (0.0, 10.0), (0.0, 5.0));
        assert_eq!(out, vec![Drawn::Path(vec![(0.0, 50.0), (0.0, 0.0), (100.0, 0.0)])]);
    }

    #[test]
    fn rectangle_and_rule_marks_map_their_bounds() {
        let chart = Chart::new()
            .mark(Mark::Rectangle { x: (2.0, 4.0), y: (3.0, 1.0) })
            .mark(Mark::RuleX { x: 5.0, y: (0.0, 5.0) })
            .mark(Mark::RuleY { y: 1.0, x: (0.0, 10.0) });
        let out = chart.render(frame(), (0.0, 10.0), (0.0, 5.0));
        // x 2..4 maps to 20..40; y 3 and 1 map to 20 and 40, so the rectangle spans 20..40 both ways.
        assert_eq!(out[0], Drawn::Rects(vec![Frame::new(20.0, 20.0, 20.0, 20.0)]), "reversed y bounds are normalised");
        assert_eq!(out[1], Drawn::Segment(Segment { from: (50.0, 50.0), to: (50.0, 0.0) }));
        assert_eq!(out[2], Drawn::Segment(Segment { from: (0.0, 40.0), to: (100.0, 40.0) }));
    }

    #[test]
    fn pie_sectors_fill_the_shorter_side() {
        // A 100 by 60 frame has radius 30 around (50, 30). Two equal sectors, each with ARC_SEGMENTS + 1 points on
        // the outer arc and as many on the inner arc.
        let chart = Chart::new().mark(Mark::Sector { values: vec![1.0, 1.0], inner_ratio: 0.0 });
        let out = chart.render(Frame::new(0.0, 0.0, 100.0, 60.0), (0.0, 1.0), (0.0, 1.0));
        assert_eq!(out.len(), 2);
        let Drawn::Polygon(first) = &out[0] else { panic!("sectors draw as polygons") };
        assert_eq!(first.len(), 2 * (ARC_SEGMENTS + 1));
        assert!(close(first[0].0, 50.0) && close(first[0].1, 0.0), "the first sector starts straight up at the rim");
    }

    #[test]
    fn zero_sector_values_draw_nothing() {
        let chart = Chart::new().mark(Mark::Sector { values: vec![0.0, 0.0], inner_ratio: 0.5 });
        let out = chart.render(frame(), (0.0, 1.0), (0.0, 1.0));
        assert!(out.is_empty(), "zero-width sectors have no outline");
    }
}
