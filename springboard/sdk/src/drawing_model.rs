//! Drawing and printing models: `BezierPath`, `Gradient`, `Shadow` and print pagination math.
//!
//! Pure models over [`Path`](crate::graphics_context::Path) and [`PrintInfo`](crate::print::PrintInfo); colours are
//! packed `u32` and interpolated per byte, so any channel order works. Nothing here touches the system.

use crate::graphics_context::{FillRule, GraphicsContext, Path, Seg};
use crate::print::{Orientation, PrintInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCapStyle {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoinStyle {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindingRule {
    NonZero,
    EvenOdd,
}

/// `BezierPath`.
#[derive(Debug, Clone, PartialEq)]
pub struct BezierPath {
    path: Path,
    pub line_width: f32,
    pub line_cap_style: LineCapStyle,
    pub line_join_style: LineJoinStyle,
    pub winding_rule: WindingRule,
    dash: Vec<f32>,
    dash_phase: f32,
}

impl Default for BezierPath {
    fn default() -> Self {
        BezierPath {
            path: Path::new(),
            line_width: 1.0,
            line_cap_style: LineCapStyle::Butt,
            line_join_style: LineJoinStyle::Miter,
            winding_rule: WindingRule::NonZero,
            dash: Vec::new(),
            dash_phase: 0.0,
        }
    }
}

fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

impl BezierPath {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_rect(x: f32, y: f32, w: f32, h: f32) -> Self {
        let mut p = Self::new();
        p.append_rect(x, y, w, h);
        p
    }
    pub fn with_oval(x: f32, y: f32, w: f32, h: f32) -> Self {
        let mut p = Self::new();
        p.append_oval(x, y, w, h);
        p
    }
    pub fn with_rounded_rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Self {
        let mut p = Self::new();
        p.append_rounded_rect(x, y, w, h, rx, ry);
        p
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn is_empty(&self) -> bool {
        self.path.segs.is_empty()
    }
    pub fn element_count(&self) -> usize {
        self.path.segs.len()
    }
    pub fn remove_all_points(&mut self) {
        self.path.segs.clear();
    }
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(x, y);
    }
    pub fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(x, y);
    }
    /// `curve(to:controlPoint1:controlPoint2:)`.
    pub fn curve_to(&mut self, end: (f32, f32), c1: (f32, f32), c2: (f32, f32)) {
        self.path.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
    }
    pub fn close(&mut self) {
        self.path.close();
    }
    /// The last point added (`currentPoint`); `None` for an empty path.
    pub fn current_point(&self) -> Option<(f32, f32)> {
        self.path.segs.iter().rev().find_map(|s| match *s {
            Seg::Move(x, y) | Seg::Line(x, y) | Seg::Quad(_, _, x, y) | Seg::Cubic(_, _, _, _, x, y) => Some((x, y)),
            Seg::Close => None,
        })
    }
    pub fn append_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.path.segs.extend(Path::rect(x, y, w, h).segs);
    }
    pub fn append_oval(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.path.segs.extend(Path::ellipse(x, y, w, h).segs);
    }
    /// `appendRoundedRect(_:xRadius:yRadius:)`; radii clamp to half the side.
    pub fn append_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) {
        const K: f32 = 0.552_284_8;
        let rx = rx.clamp(0.0, w / 2.0);
        let ry = ry.clamp(0.0, h / 2.0);
        let (r, b) = (x + w, y + h);
        self.move_to(x + rx, y);
        self.line_to(r - rx, y);
        self.curve_to((r, y + ry), (r - rx + K * rx, y), (r, y + ry - K * ry));
        self.line_to(r, b - ry);
        self.curve_to((r - rx, b), (r, b - ry + K * ry), (r - rx + K * rx, b));
        self.line_to(x + rx, b);
        self.curve_to((x, b - ry), (x + rx - K * rx, b), (x, b - ry + K * ry));
        self.line_to(x, y + ry);
        self.curve_to((x + rx, y), (x, y + ry - K * ry), (x + rx - K * rx, y));
        self.close();
    }
    /// `appendArc(withCenter:radius:startAngle:endAngle:clockwise:)`, angles in degrees. Joins the current point with a
    /// line, as the reference does.
    pub fn append_arc(&mut self, center: (f32, f32), radius: f32, start_deg: f32, end_deg: f32, clockwise: bool) {
        let (mut a0, mut a1) = (start_deg.to_radians(), end_deg.to_radians());
        let tau = std::f32::consts::TAU;
        if clockwise {
            while a1 > a0 {
                a1 -= tau;
            }
        } else {
            while a1 < a0 {
                a1 += tau;
            }
        }
        let start = (center.0 + radius * a0.cos(), center.1 + radius * a0.sin());
        if self.is_empty() {
            self.move_to(start.0, start.1);
        } else {
            self.line_to(start.0, start.1);
        }
        let steps = (((a1 - a0).abs() / std::f32::consts::FRAC_PI_2).ceil() as usize).max(1);
        let d = (a1 - a0) / steps as f32;
        let k = 4.0 / 3.0 * (d / 4.0).tan();
        for _ in 0..steps {
            let b0 = a0 + d;
            let p0 = (center.0 + radius * a0.cos(), center.1 + radius * a0.sin());
            let p1 = (center.0 + radius * b0.cos(), center.1 + radius * b0.sin());
            let c1 = (p0.0 - k * radius * a0.sin(), p0.1 + k * radius * a0.cos());
            let c2 = (p1.0 + k * radius * b0.sin(), p1.1 - k * radius * b0.cos());
            self.curve_to(p1, c1, c2);
            a0 = b0;
        }
    }
    /// `setLineDash(_:count:phase:)`; an empty pattern is solid.
    pub fn set_line_dash(&mut self, pattern: &[f32], phase: f32) {
        self.dash = pattern.to_vec();
        self.dash_phase = phase;
    }
    pub fn line_dash(&self) -> (&[f32], f32) {
        (&self.dash, self.dash_phase)
    }

    /// Subpaths flattened to polylines, with a closed flag.
    fn flatten(&self) -> Vec<(Vec<(f32, f32)>, bool)> {
        let mut out = Vec::new();
        let mut cur: Vec<(f32, f32)> = Vec::new();
        for s in &self.path.segs {
            match *s {
                Seg::Move(x, y) => {
                    if cur.len() > 1 {
                        out.push((std::mem::take(&mut cur), false));
                    }
                    cur = vec![(x, y)];
                }
                Seg::Line(x, y) => cur.push((x, y)),
                Seg::Quad(cx, cy, x, y) => {
                    let p0 = cur.last().copied().unwrap_or((0.0, 0.0));
                    for i in 1..=16 {
                        let t = i as f32 / 16.0;
                        cur.push(lerp(lerp(p0, (cx, cy), t), lerp((cx, cy), (x, y), t), t));
                    }
                }
                Seg::Cubic(ax, ay, bx, by, x, y) => {
                    let p0 = cur.last().copied().unwrap_or((0.0, 0.0));
                    for i in 1..=24 {
                        let t = i as f32 / 24.0;
                        let q0 = lerp(p0, (ax, ay), t);
                        let q1 = lerp((ax, ay), (bx, by), t);
                        let q2 = lerp((bx, by), (x, y), t);
                        cur.push(lerp(lerp(q0, q1, t), lerp(q1, q2, t), t));
                    }
                }
                Seg::Close => {
                    if let Some(&first) = cur.first() {
                        if cur.len() > 1 {
                            out.push((std::mem::take(&mut cur), true));
                        }
                        cur = vec![first];
                    }
                }
            }
        }
        if cur.len() > 1 {
            out.push((cur, false));
        }
        out
    }

    /// `bounds` as (x, y, w, h): the tight box of the flattened curve. Empty path gives all zeros.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let pts: Vec<(f32, f32)> = self.flatten().into_iter().flat_map(|(p, _)| p).collect();
        if pts.is_empty() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// `contains(_:)` under the path's winding rule. Every subpath is treated as closed, as a fill does.
    pub fn contains(&self, p: (f32, f32)) -> bool {
        let mut wn = 0i32;
        let mut crossings = 0i32;
        for (poly, _) in self.flatten() {
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let side = (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1);
                // a ray from p towards +x: an upward edge crosses it when p is left of the edge (side > 0), a downward one
                // when p is right of it (side < 0); even-odd counts those crossings, nonzero sums their directions
                if a.1 <= p.1 && b.1 > p.1 && side > 0.0 {
                    crossings += 1;
                    wn += 1;
                } else if a.1 > p.1 && b.1 <= p.1 && side < 0.0 {
                    crossings += 1;
                    wn -= 1;
                }
            }
        }
        match self.winding_rule {
            WindingRule::NonZero => wn != 0,
            WindingRule::EvenOdd => crossings % 2 == 1,
        }
    }

    fn rule(&self) -> FillRule {
        match self.winding_rule {
            WindingRule::NonZero => FillRule::NonZero,
            WindingRule::EvenOdd => FillRule::EvenOdd,
        }
    }
    pub fn fill(&self, ctx: &mut GraphicsContext, color: u32) {
        ctx.fill(&self.path, color, self.rule());
    }
    pub fn stroke(&self, ctx: &mut GraphicsContext, color: u32) {
        ctx.stroke(&self.path, color, self.line_width);
    }
    /// `addClip()`.
    pub fn add_clip(&self, ctx: &mut GraphicsContext) {
        ctx.clip(&self.path);
    }
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        out |= ((x + (y - x) * t).round().clamp(0.0, 255.0) as u32) << shift;
    }
    out
}

/// `Gradient(colors:atLocations:)`. Locations are clamped to 0...1 and kept in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    stops: Vec<(f32, u32)>,
}

impl Gradient {
    /// `None` for an empty colour list. Without `locations`, stops are spread evenly.
    pub fn new(colors: &[u32], locations: Option<&[f32]>) -> Option<Gradient> {
        if colors.is_empty() {
            return None;
        }
        let n = colors.len();
        let mut stops: Vec<(f32, u32)> = colors
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let loc = match locations.and_then(|l| l.get(i)) {
                    Some(&l) => l.clamp(0.0, 1.0),
                    None if n == 1 => 0.0,
                    None => i as f32 / (n - 1) as f32,
                };
                (loc, c)
            })
            .collect();
        stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        Some(Gradient { stops })
    }
    pub fn number_of_color_stops(&self) -> usize {
        self.stops.len()
    }
    /// `getColor(_:location:at:)`.
    pub fn stop(&self, index: usize) -> Option<(u32, f32)> {
        self.stops.get(index).map(|&(l, c)| (c, l))
    }
    /// The interpolated colour at `t` (0...1); outside the stops it holds the end colours.
    pub fn interpolated_color(&self, t: f32) -> u32 {
        let first = self.stops[0];
        let last = self.stops[self.stops.len() - 1];
        if t <= first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        for w in self.stops.windows(2) {
            if t <= w[1].0 {
                let span = w[1].0 - w[0].0;
                let f = if span <= f32::EPSILON { 1.0 } else { (t - w[0].0) / span };
                return mix(w[0].1, w[1].1, f);
            }
        }
        last.1
    }
    /// The position (0...1) of `point` along a linear gradient across the rect (x, y, w, h) at `angle_deg`
    /// (`draw(in:angle:)`: 0 runs left to right, 90 bottom to top, y up).
    pub fn linear_t(&self, rect: (f32, f32, f32, f32), angle_deg: f32, point: (f32, f32)) -> f32 {
        let (x, y, w, h) = rect;
        let (dx, dy) = (angle_deg.to_radians().cos(), angle_deg.to_radians().sin());
        let extent = w * dx.abs() + h * dy.abs();
        if extent <= f32::EPSILON {
            return 0.0;
        }
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        (((point.0 - cx) * dx + (point.1 - cy) * dy) / extent + 0.5).clamp(0.0, 1.0)
    }
    /// The position of `point` in a radial gradient (`draw(fromCenter:radius:toCenter:radius:)` with a shared centre).
    pub fn radial_t(&self, center: (f32, f32), start_radius: f32, end_radius: f32, point: (f32, f32)) -> f32 {
        let d = ((point.0 - center.0).powi(2) + (point.1 - center.1).powi(2)).sqrt();
        let span = end_radius - start_radius;
        if span.abs() <= f32::EPSILON {
            return 0.0;
        }
        ((d - start_radius) / span).clamp(0.0, 1.0)
    }
}

/// `Shadow`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowStyle {
    pub offset: (f32, f32),
    pub blur_radius: f32,
    pub color: u32,
}

impl ShadowStyle {
    pub fn new(offset: (f32, f32), blur_radius: f32, color: u32) -> Self {
        ShadowStyle { offset, blur_radius: blur_radius.max(0.0), color }
    }
    /// The rect (x, y, w, h) a shadow of `rect` covers: offset, then grown by the blur radius on every side.
    pub fn extent(&self, rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let r = self.blur_radius;
        (rect.0 + self.offset.0 - r, rect.1 + self.offset.1 - r, rect.2 + 2.0 * r, rect.3 + 2.0 * r)
    }
}

/// Pagination math for `PrintInfo` with a scaling factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageLayout {
    pub scaling: f32,
}

impl PageLayout {
    pub fn new(scaling: f32) -> Self {
        PageLayout { scaling: if scaling > 0.0 { scaling } else { 1.0 } }
    }
    /// The imageable size in document units: the printable rect divided by the scale.
    pub fn content_size(&self, info: &PrintInfo) -> (f32, f32) {
        let (_, _, w, h) = info.imageable();
        (w / self.scaling, h / self.scaling)
    }
    /// Pages needed to print a document of `doc` size (columns, rows) across the content area; at least one.
    pub fn page_grid(&self, info: &PrintInfo, doc: (f32, f32)) -> (u32, u32) {
        let (w, h) = self.content_size(info);
        let n = |len: f32, per: f32| if per <= 0.0 { 1 } else { ((len / per).ceil() as u32).max(1) };
        (n(doc.0, w), n(doc.1, h))
    }
    pub fn page_count(&self, info: &PrintInfo, doc: (f32, f32)) -> u32 {
        let (c, r) = self.page_grid(info, doc);
        c * r
    }
    /// The document rect (x, y, w, h) printed on 0-based page `index`, reading order, rows first.
    pub fn page_rect(&self, info: &PrintInfo, doc: (f32, f32), index: u32) -> Option<(f32, f32, f32, f32)> {
        let (cols, rows) = self.page_grid(info, doc);
        if index >= cols * rows {
            return None;
        }
        let (w, h) = self.content_size(info);
        Some(((index % cols) as f32 * w, (index / cols) as f32 * h, w, h))
    }
}

/// Flips a portrait info to landscape or back, as the page layout panel does.
pub fn with_orientation(mut info: PrintInfo, orientation: Orientation) -> PrintInfo {
    info.orientation = orientation;
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::PaperSize;

    #[test]
    fn rect_bounds_and_contains() {
        let p = BezierPath::with_rect(10.0, 20.0, 30.0, 40.0);
        assert_eq!(p.bounds(), (10.0, 20.0, 30.0, 40.0));
        assert!(p.contains((25.0, 40.0)));
        assert!(!p.contains((5.0, 40.0)));
    }

    #[test]
    fn even_odd_leaves_a_hole() {
        let mut p = BezierPath::with_rect(0.0, 0.0, 100.0, 100.0);
        p.append_rect(25.0, 25.0, 50.0, 50.0);
        assert!(p.contains((50.0, 50.0)), "nonzero: same winding fills the hole");
        p.winding_rule = WindingRule::EvenOdd;
        assert!(!p.contains((50.0, 50.0)));
        assert!(p.contains((10.0, 10.0)));
    }

    #[test]
    fn rounded_rect_clamps_radius_and_cuts_corner() {
        let p = BezierPath::with_rounded_rect(0.0, 0.0, 100.0, 50.0, 500.0, 500.0);
        assert_eq!(p.bounds().2, 100.0);
        assert!(!p.contains((1.0, 1.0)));
        assert!(p.contains((50.0, 25.0)));
    }

    #[test]
    fn arc_reaches_end_point() {
        let mut p = BezierPath::new();
        p.append_arc((0.0, 0.0), 10.0, 0.0, 90.0, false);
        let (x, y) = p.current_point().unwrap();
        assert!(x.abs() < 1e-3 && (y - 10.0).abs() < 1e-3);
    }

    #[test]
    fn current_point_and_clear() {
        let mut p = BezierPath::new();
        assert_eq!(p.current_point(), None);
        p.move_to(1.0, 2.0);
        p.line_to(3.0, 4.0);
        assert_eq!(p.current_point(), Some((3.0, 4.0)));
        p.remove_all_points();
        assert!(p.is_empty());
    }

    #[test]
    fn dash_roundtrip() {
        let mut p = BezierPath::new();
        p.set_line_dash(&[4.0, 2.0], 1.0);
        assert_eq!(p.line_dash(), (&[4.0, 2.0][..], 1.0));
    }

    #[test]
    fn gradient_stops_and_interpolation() {
        let g = Gradient::new(&[0x0000_0000, 0x0000_00ff], None).unwrap();
        assert_eq!(g.interpolated_color(0.0), 0);
        assert_eq!(g.interpolated_color(1.0), 0xff);
        assert_eq!(g.interpolated_color(0.5), 0x80);
        let g = Gradient::new(&[0, 100, 200], Some(&[0.0, 0.25, 1.0])).unwrap();
        assert_eq!(g.interpolated_color(0.25), 100);
        assert_eq!(g.stop(2), Some((200, 1.0)));
        assert!(Gradient::new(&[], None).is_none());
    }

    #[test]
    fn linear_and_radial_positions() {
        let g = Gradient::new(&[0, 255], None).unwrap();
        let r = (0.0, 0.0, 100.0, 50.0);
        assert!((g.linear_t(r, 0.0, (0.0, 25.0))).abs() < 1e-5);
        assert!((g.linear_t(r, 0.0, (100.0, 25.0)) - 1.0).abs() < 1e-5);
        assert!((g.linear_t(r, 90.0, (50.0, 25.0)) - 0.5).abs() < 1e-5);
        assert!((g.radial_t((0.0, 0.0), 0.0, 10.0, (5.0, 0.0)) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn shadow_extent() {
        let s = ShadowStyle::new((2.0, -3.0), 4.0, 0x80);
        assert_eq!(s.extent((10.0, 10.0, 20.0, 20.0)), (8.0, 3.0, 28.0, 28.0));
    }

    #[test]
    fn a4_page_count_at_scales() {
        let info = PrintInfo::default(); // A4, 72pt margins: 451 x 698 imageable
        let full = PageLayout::new(1.0);
        assert_eq!(full.page_count(&info, (451.0, 698.0)), 1);
        assert_eq!(full.page_count(&info, (451.0, 699.0)), 2);
        assert_eq!(PageLayout::new(0.5).page_count(&info, (451.0, 699.0)), 1);
        assert_eq!(PageLayout::new(2.0).page_grid(&info, (451.0, 698.0)), (2, 2));
    }

    #[test]
    fn letter_landscape_rects() {
        let mut info = with_orientation(PrintInfo::default(), Orientation::Landscape);
        info.paper = PaperSize::Letter; // 792 x 612, imageable 648 x 468
        let l = PageLayout::new(1.0);
        assert_eq!(l.content_size(&info), (648.0, 468.0));
        assert_eq!(l.page_rect(&info, (1296.0, 468.0), 1), Some((648.0, 0.0, 648.0, 468.0)));
        assert_eq!(l.page_rect(&info, (1296.0, 468.0), 2), None);
    }

    #[test]
    fn bad_scaling_falls_back_to_one() {
        assert_eq!(PageLayout::new(0.0).scaling, 1.0);
    }
}
