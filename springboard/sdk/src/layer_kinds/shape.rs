//! Shape content: a path with a fill, a stroke, a fill rule, caps, joins, a miter limit, a dash pattern and a stroke
//! range. `ShapeContent::render` rasterises it; a `Layer` or a `ShapeView` holds it.
use super::raster::{self, Coverage, FillRule, Pt, Run};
use bifrost_render::bezier::BezierPath;
use bifrost_render::gfx::Image;
use bifrost_ui::shape::{Path, Point};
use bifrost_ui::stroke_style::StrokeStyle;

/// Flattening tolerance for curves, in points.
const TOLERANCE: f32 = 0.1;

#[derive(Clone, Debug, PartialEq)]
pub struct ShapeContent {
    /// The outline, in the content's own point space (origin at the top left).
    pub path: Path,
    /// Straight RGBA fill; `None` leaves the inside empty.
    pub fill: Option<[u8; 4]>,
    pub fill_rule: FillRule,
    /// Straight RGBA stroke colour; `None` draws no stroke.
    pub stroke: Option<[u8; 4]>,
    /// Width, caps, joins, miter limit and dash pattern of the stroke.
    pub style: StrokeStyle,
    /// The part of the outline that is stroked, as fractions of its length (0 to 1).
    pub stroke_start: f32,
    pub stroke_end: f32,
}

impl Default for ShapeContent {
    fn default() -> Self {
        ShapeContent {
            path: Path::new(),
            fill: Some([0, 0, 0, 255]),
            fill_rule: FillRule::NonZero,
            stroke: None,
            style: StrokeStyle::default(),
            stroke_start: 0.0,
            stroke_end: 1.0,
        }
    }
}

impl ShapeContent {
    pub fn new(path: Path) -> Self {
        ShapeContent { path, ..Default::default() }
    }

    /// Builds the outline from curves (cubic segments are flattened).
    pub fn from_curves(path: &bifrost_ui::path::Path) -> Self {
        let mut out = Path::new();
        for line in path.polylines(TOLERANCE) {
            add_polyline(&mut out, &line);
        }
        ShapeContent::new(out)
    }

    /// Builds the outline from a bezier path.
    pub fn from_bezier(path: &BezierPath) -> Self {
        let mut out = Path::new();
        for line in path.flatten(TOLERANCE) {
            add_polyline(&mut out, &line);
        }
        ShapeContent::new(out)
    }

    /// The rect the outline covers, widened by half the stroke width when stroked.
    pub fn bounds(&self) -> Option<bifrost_ui::Rect> {
        let b = self.path.bounds()?;
        let grow = if self.stroke.is_some() { self.style.width.max(0.0) / 2.0 } else { 0.0 };
        Some(bifrost_ui::Rect { x: b.x - grow, y: b.y - grow, w: b.w + 2.0 * grow, h: b.h + 2.0 * grow })
    }

    /// The stroked pieces after the stroke range and the dash pattern, in points.
    pub fn stroke_runs(&self) -> Vec<(Vec<Pt>, bool)> {
        let whole = self.stroke_start <= 0.0 && self.stroke_end >= 1.0;
        let ranged = if whole { self.path.clone() } else { self.path.trimmed(self.stroke_start, self.stroke_end) };
        let dashed = if self.style.dash.is_empty() { ranged.clone() } else { ranged.dashed(&self.style.dash, self.style.dash_phase) };
        let keep_closed = whole && self.style.dash.is_empty();
        dashed
            .subpaths()
            .iter()
            .map(|s| (s.points.iter().map(|p| (p.x, p.y)).collect(), keep_closed && s.closed))
            .collect()
    }

    /// True when `(x, y)` (content points) is inside the filled outline under the fill rule.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let mut wind = 0i32;
        for s in self.path.subpaths() {
            let n = s.points.len();
            if n < 2 {
                continue;
            }
            for i in 0..n {
                let (a, b) = (s.points[i], s.points[(i + 1) % n]);
                if (a.y <= y) != (b.y <= y) {
                    let t = (y - a.y) / (b.y - a.y);
                    if a.x + t * (b.x - a.x) > x {
                        wind += if b.y > a.y { 1 } else { -1 };
                    }
                }
            }
        }
        match self.fill_rule {
            FillRule::NonZero => wind != 0,
            FillRule::EvenOdd => wind & 1 != 0,
        }
    }

    /// Rasterises the shape into a `w` x `h` image. `scale` is device pixels per point.
    pub fn render(&self, w: usize, h: usize, scale: f32) -> Image {
        let mut img = Image::new(w, h);
        if w == 0 || h == 0 {
            return img;
        }
        if let Some(fill) = self.fill {
            let polys: Vec<Vec<Pt>> = self
                .path
                .subpaths()
                .iter()
                .filter(|s| s.points.len() > 2)
                .map(|s| s.points.iter().map(|p| (p.x * scale, p.y * scale)).collect())
                .collect();
            let cov = raster::fill_polygons(&polys, self.fill_rule, w, h);
            raster::paint(&mut img, &cov, fill);
        }
        if let Some(color) = self.stroke {
            let cov = self.stroke_coverage(w, h, scale);
            raster::paint(&mut img, &cov, color);
        }
        img
    }

    fn stroke_coverage(&self, w: usize, h: usize, scale: f32) -> Coverage {
        let runs: Vec<Run> = self
            .stroke_runs()
            .into_iter()
            .map(|(pts, closed)| Run { points: pts.into_iter().map(|p| (p.0 * scale, p.1 * scale)).collect(), closed })
            .collect();
        let mut style = self.style.clone();
        style.width *= scale;
        raster::fill_polygons(&raster::stroke_polygons(&runs, &style), FillRule::NonZero, w, h)
    }
}

fn add_polyline(out: &mut Path, line: &[(f32, f32)]) {
    let mut pts = line.to_vec();
    let closed = pts.len() > 2 && pts.first() == pts.last();
    if closed {
        pts.pop();
    }
    for (i, p) in pts.iter().enumerate() {
        let p = Point { x: p.0, y: p.1 };
        if i == 0 {
            out.move_to(p);
        } else {
            out.line_to(p);
        }
    }
    if closed {
        out.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::gfx::rgba;
    use bifrost_ui::stroke_style::LineCap;
    use bifrost_ui::Rect;

    fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Path {
        let mut p = Path::new();
        p.add_rect(Rect { x, y, w, h });
        p
    }

    #[test]
    fn a_filled_rect_renders_its_fill_colour_inside_only() {
        let mut s = ShapeContent::new(rect_path(2.0, 2.0, 4.0, 4.0));
        s.fill = Some([255, 0, 0, 255]);
        let img = s.render(8, 8, 1.0);
        assert_eq!(img.px[3 * 8 + 3], rgba(255, 0, 0, 255));
        assert_eq!(img.px[0], 0);
        assert_eq!(img.px[7 * 8 + 7], 0);
    }

    #[test]
    fn scale_converts_points_to_pixels() {
        let s = ShapeContent::new(rect_path(0.0, 0.0, 2.0, 2.0));
        let img = s.render(8, 8, 2.0);
        assert_eq!(img.px[3 * 8 + 3], rgba(0, 0, 0, 255));
        assert_eq!(img.px[4 * 8 + 4], 0);
    }

    #[test]
    fn even_odd_cuts_a_hole_that_non_zero_fills() {
        let mut p = rect_path(0.0, 0.0, 8.0, 8.0);
        p.add_rect(Rect { x: 2.0, y: 2.0, w: 4.0, h: 4.0 });
        let mut s = ShapeContent::new(p);
        assert_eq!(s.render(8, 8, 1.0).px[4 * 8 + 4], rgba(0, 0, 0, 255));
        s.fill_rule = FillRule::EvenOdd;
        assert_eq!(s.render(8, 8, 1.0).px[4 * 8 + 4], 0);
        assert!(!s.contains(4.0, 4.0));
        assert!(s.contains(1.0, 1.0));
    }

    #[test]
    fn a_stroke_only_shape_leaves_the_inside_empty() {
        let mut s = ShapeContent::new(rect_path(2.0, 2.0, 8.0, 8.0));
        s.fill = None;
        s.stroke = Some([0, 0, 255, 255]);
        s.style.width = 2.0;
        let img = s.render(12, 12, 1.0);
        assert_eq!(img.px[6 * 12 + 6], 0);
        assert_eq!(img.px[6 * 12 + 2], rgba(0, 0, 255, 255));
        assert_eq!(img.px[2 * 12 + 2], rgba(0, 0, 255, 255), "closed outline has a mitred corner");
    }

    #[test]
    fn stroke_end_trims_the_stroked_part() {
        let mut p = Path::new();
        p.move_to(Point { x: 0.0, y: 5.0 });
        p.line_to(Point { x: 10.0, y: 5.0 });
        let mut s = ShapeContent::new(p);
        s.fill = None;
        s.stroke = Some([255, 255, 255, 255]);
        s.style.width = 2.0;
        s.stroke_end = 0.5;
        let img = s.render(10, 10, 1.0);
        assert_ne!(img.px[5 * 10 + 2], 0);
        assert_eq!(img.px[5 * 10 + 7], 0);
        s.stroke_start = 0.5;
        s.stroke_end = 1.0;
        let img = s.render(10, 10, 1.0);
        assert_eq!(img.px[5 * 10 + 2], 0);
        assert_ne!(img.px[5 * 10 + 7], 0);
    }

    #[test]
    fn a_dash_pattern_leaves_gaps() {
        let mut p = Path::new();
        p.move_to(Point { x: 0.0, y: 2.0 });
        p.line_to(Point { x: 12.0, y: 2.0 });
        let mut s = ShapeContent::new(p);
        s.fill = None;
        s.stroke = Some([255, 255, 255, 255]);
        s.style.width = 2.0;
        s.style.dash = vec![4.0, 4.0];
        let img = s.render(12, 4, 1.0);
        assert_ne!(img.px[2 * 12 + 1], 0);
        assert_eq!(img.px[2 * 12 + 5], 0);
        assert_ne!(img.px[2 * 12 + 9], 0);
        s.style.dash_phase = 4.0;
        let img = s.render(12, 4, 1.0);
        assert_eq!(img.px[2 * 12 + 1], 0, "the phase shifts the pattern");
        assert_ne!(img.px[2 * 12 + 5], 0);
    }

    #[test]
    fn round_caps_extend_an_open_stroke() {
        let mut p = Path::new();
        p.move_to(Point { x: 5.0, y: 5.0 });
        p.line_to(Point { x: 10.0, y: 5.0 });
        let mut s = ShapeContent::new(p);
        s.fill = None;
        s.stroke = Some([255, 255, 255, 255]);
        s.style.width = 6.0;
        assert_eq!(s.render(16, 10, 1.0).px[5 * 16 + 3], 0);
        s.style.cap = LineCap::Round;
        assert_ne!(s.render(16, 10, 1.0).px[5 * 16 + 3], 0);
    }

    #[test]
    fn curves_flatten_into_a_closed_ellipse() {
        let mut c = bifrost_ui::path::Path::new();
        c.add_ellipse_in(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 });
        let s = ShapeContent::from_curves(&c);
        assert!(s.contains(5.0, 5.0));
        assert!(!s.contains(0.5, 0.5));
        let img = s.render(10, 10, 1.0);
        assert_eq!(img.px[5 * 10 + 5], rgba(0, 0, 0, 255));
        assert_eq!(img.px[0], 0);
    }

    #[test]
    fn bounds_grow_by_half_the_stroke_width() {
        let mut s = ShapeContent::new(rect_path(2.0, 2.0, 4.0, 4.0));
        assert_eq!(s.bounds(), Some(Rect { x: 2.0, y: 2.0, w: 4.0, h: 4.0 }));
        s.stroke = Some([0, 0, 0, 255]);
        s.style.width = 2.0;
        assert_eq!(s.bounds(), Some(Rect { x: 1.0, y: 1.0, w: 6.0, h: 6.0 }));
    }
}
