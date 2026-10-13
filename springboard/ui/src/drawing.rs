//! Hand-drawn content: a drawing (`NativeDrawing`) holds strokes (`NativeStroke`), a stroke holds touch points
//! (`NativeStrokePoint`) and each stroke names its ink (`NativeInk`). Points keep the touch data the reference SDK
//! records (time offset, force, azimuth, altitude), so a stylus stroke saves as it was made. The drawing has a text
//! form that saves and reloads. Capturing touch input, rendering and the tool palette belong to the shell and apps.
use crate::cg::path::Affine;
use crate::Rect;
use std::fmt;

/// First line of a saved drawing.
const FORMAT: &str = "bifrost-drawing 1";

/// The kinds of ink a stroke can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkKind {
    Pen,
    Marker,
    Pencil,
}

impl InkKind {
    fn name(self) -> &'static str {
        match self {
            InkKind::Pen => "pen",
            InkKind::Marker => "marker",
            InkKind::Pencil => "pencil",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "pen" => Some(InkKind::Pen),
            "marker" => Some(InkKind::Marker),
            "pencil" => Some(InkKind::Pencil),
            _ => None,
        }
    }
}

/// The ink of a stroke (`NativeInk`). The colour is packed 0xAARRGGBB, the width is in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub kind: InkKind,
    pub color: u32,
    pub width: f32,
}

/// One sampled point of a stroke (`NativeStrokePoint`). `time_offset` is seconds since the stroke began. `force`,
/// `azimuth` and `altitude` are the stylus readings for the point, in the units the input device reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub time_offset: f32,
    pub size: (f32, f32),
    pub opacity: f32,
    pub force: f32,
    pub azimuth: f32,
    pub altitude: f32,
}

impl StrokePoint {
    /// A point at `(x, y)` with no stylus data: zero time, unit opacity, no force, azimuth or altitude.
    pub fn at(x: f32, y: f32) -> Self {
        StrokePoint { x, y, time_offset: 0.0, size: (0.0, 0.0), opacity: 1.0, force: 0.0, azimuth: 0.0, altitude: 0.0 }
    }
}

/// A stroke (`NativeStroke`): an ink and the points it passes through, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub ink: Ink,
    pub points: Vec<StrokePoint>,
}

impl Stroke {
    /// The box around the stroke's point locations, or `None` for a stroke with no points.
    pub fn bounds(&self) -> Option<Rect> {
        bounds_of(self.points.iter().map(|p| (p.x, p.y)))
    }

    /// Maps every point through `t`. Times, sizes and stylus readings are kept.
    pub fn transform(&mut self, t: Affine) {
        for p in &mut self.points {
            let (x, y) = t.apply(p.x, p.y);
            p.x = x;
            p.y = y;
        }
    }
}

/// Blends two points. Positions, times, sizes, opacity and stylus readings are interpolated linearly; an angle
/// (azimuth) is blended as a plain number, which is exact for angles that do not wrap past pi.
fn lerp_point(a: StrokePoint, b: StrokePoint, t: f32) -> StrokePoint {
    let l = |x: f32, y: f32| x + (y - x) * t;
    StrokePoint {
        x: l(a.x, b.x),
        y: l(a.y, b.y),
        time_offset: l(a.time_offset, b.time_offset),
        size: (l(a.size.0, b.size.0), l(a.size.1, b.size.1)),
        opacity: l(a.opacity, b.opacity),
        force: l(a.force, b.force),
        azimuth: l(a.azimuth, b.azimuth),
        altitude: l(a.altitude, b.altitude),
    }
}

/// Blends two packed 0xAARRGGBB colours channel by channel, rounding to the nearest value.
fn lerp_color(a: u32, b: u32, t: f32) -> u32 {
    (0..4).fold(0u32, |out, i| {
        let shift = i * 8;
        let (ca, cb) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        let v = (ca + (cb - ca) * t).round().clamp(0.0, 255.0) as u32;
        out | (v << shift)
    })
}

impl Stroke {
    /// The length of the stroke's polyline through its points.
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| seg_len(w[0], w[1])).sum()
    }

    /// The part of the stroke between `from` and `to`, given as fractions of its length (clamped to 0..=1). The
    /// ends are interpolated and every point between them is kept. An empty range gives a stroke with no points.
    pub fn slice(&self, from: f32, to: f32) -> Stroke {
        let total = self.length();
        let mut out = Stroke { ink: self.ink, points: Vec::new() };
        let (a, b) = (from.clamp(0.0, 1.0) * total, to.clamp(0.0, 1.0) * total);
        if total <= 0.0 || b <= a {
            return out;
        }
        let mut at = 0.0f32;
        for w in self.points.windows(2) {
            let (p, q) = (w[0], w[1]);
            let len = seg_len(p, q);
            let (s0, s1) = (at, at + len);
            at = s1;
            if len <= 0.0 || s1 < a || s0 > b {
                continue;
            }
            let (lo, hi) = (s0.max(a), s1.min(b));
            if out.points.is_empty() {
                out.points.push(lerp_point(p, q, (lo - s0) / len));
            }
            out.points.push(lerp_point(p, q, (hi - s0) / len));
        }
        out
    }

    /// `n` points spaced evenly by length along the stroke, the first at its start and the last at its end. A stroke
    /// of zero length repeats its first point. A stroke with no points gives no points.
    pub fn resampled(&self, n: usize) -> Stroke {
        let mut out = Stroke { ink: self.ink, points: Vec::new() };
        if n == 0 || self.points.is_empty() {
            return out;
        }
        let total = self.length();
        for i in 0..n {
            let d = if n == 1 { 0.0 } else { total * i as f32 / (n - 1) as f32 };
            out.points.push(self.point_at(d));
        }
        out
    }

    /// The point `d` units along the stroke. Distances past either end clamp to the end points.
    fn point_at(&self, d: f32) -> StrokePoint {
        let mut at = 0.0f32;
        for w in self.points.windows(2) {
            let len = seg_len(w[0], w[1]);
            if len > 0.0 && d <= at + len {
                return lerp_point(w[0], w[1], ((d - at) / len).clamp(0.0, 1.0));
            }
            at += len;
        }
        self.points.last().copied().unwrap_or(StrokePoint::at(0.0, 0.0))
    }

    /// The stroke at `t` between two strokes: each is resampled to the longer point count and the points are blended
    /// pairwise. The ink colour and width blend too, and the kind switches at the halfway point. `t` is clamped to 0..=1.
    pub fn blend(a: &Stroke, b: &Stroke, t: f32) -> Stroke {
        let t = t.clamp(0.0, 1.0);
        let n = a.points.len().max(b.points.len());
        if n == 0 {
            return Stroke { ink: Ink { kind: if t < 0.5 { a.ink.kind } else { b.ink.kind }, ..a.ink }, points: Vec::new() };
        }
        let (ra, rb) = (a.resampled(n), b.resampled(n));
        let points = ra.points.iter().zip(&rb.points).map(|(&p, &q)| lerp_point(p, q, t)).collect();
        let ink = Ink {
            kind: if t < 0.5 { a.ink.kind } else { b.ink.kind },
            color: lerp_color(a.ink.color, b.ink.color, t),
            width: a.ink.width + (b.ink.width - a.ink.width) * t,
        };
        Stroke { ink, points }
    }
}

/// The distance between two points.
fn seg_len(p: StrokePoint, q: StrokePoint) -> f32 {
    ((q.x - p.x).powi(2) + (q.y - p.y).powi(2)).sqrt()
}

/// A drawing (`NativeDrawing`): the strokes in the order they were drawn. The last stroke is on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    strokes: Vec<Stroke>,
}

impl Drawing {
    pub fn new() -> Self {
        Self::default()
    }

    /// A drawing holding `strokes` in the given order.
    pub fn from_strokes(strokes: Vec<Stroke>) -> Self {
        Drawing { strokes }
    }

    /// A drawing with one stroke per subpath of `path`, all in `ink`. Curves are flattened to points within
    /// `tolerance` (the path import of the reference SDK). Subpaths that draw nothing add no stroke.
    pub fn from_path(path: &crate::cg::path::Path, tolerance: f32, ink: Ink) -> Drawing {
        let strokes = path
            .flatten(tolerance)
            .into_iter()
            .map(|line| Stroke { ink, points: line.into_iter().map(|(x, y)| StrokePoint::at(x, y)).collect() })
            .collect();
        Drawing { strokes }
    }

    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    pub fn stroke_mut(&mut self, index: usize) -> Option<&mut Stroke> {
        self.strokes.get_mut(index)
    }

    pub fn push(&mut self, stroke: Stroke) {
        self.strokes.push(stroke);
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    /// The box that holds every stroke's points, or `None` when there are no points at all.
    pub fn bounds(&self) -> Option<Rect> {
        self.strokes.iter().filter_map(Stroke::bounds).fold(None, |acc, r| match acc {
            None => Some(r),
            Some(a) => Some(union(a, r)),
        })
    }

    /// Maps every stroke through `t`.
    pub fn transform(&mut self, t: Affine) {
        for s in &mut self.strokes {
            s.transform(t);
        }
    }

    /// Renders the drawing over `rect` (drawing coordinates) at `scale` pixels per point, the way
    /// the reference SDK image call does. The image is `ceil(w * scale)` by `ceil(h * scale)` pixels, starting at the
    /// rect's top left. Strokes are painted in drawing order with source-over blending. A stroke is its width around its
    /// segments, with a half-pixel edge ramp. The colour alpha scales the coverage. Per-point opacity, pressure and
    /// the blend modes of marker ink are not rendered yet. An empty, non-finite or too large rect or scale gives an
    /// empty raster.
    pub fn image(&self, rect: Rect, scale: f32) -> Raster {
        let finite = [rect.x, rect.y, rect.w, rect.h, scale].iter().all(|v| v.is_finite());
        if !finite || rect.w <= 0.0 || rect.h <= 0.0 || scale <= 0.0 {
            return Raster::default();
        }
        let w = (rect.w * scale).ceil() as usize;
        let h = (rect.h * scale).ceil() as usize;
        if w == 0 || h == 0 || w.checked_mul(h).map_or(true, |n| n > MAX_RASTER_PIXELS) {
            return Raster::default();
        }
        let mut pixels = vec![0u32; w * h];
        for s in &self.strokes {
            paint_stroke(s, rect, scale, w, h, &mut pixels);
        }
        Raster { width: w, height: h, pixels }
    }

    /// Erases the strokes that pass within `radius` of `(x, y)`, as the reference vector eraser does (vector mode):
    /// a stroke goes as a whole when any of its segments comes that close. A negative or non-finite radius, or a
    /// non-finite point, erases nothing. Returns how many strokes were removed.
    pub fn erase_strokes_at(&mut self, x: f32, y: f32, radius: f32) -> usize {
        if !(x.is_finite() && y.is_finite() && radius.is_finite() && radius >= 0.0) {
            return 0;
        }
        let before = self.strokes.len();
        self.strokes.retain(|s| !stroke_within(s, x, y, radius));
        before - self.strokes.len()
    }

    /// The text form: a header line, then for each stroke a `stroke` line and one `point` line per point.
    /// Floats are written with their shortest exact form, so `from_text` gives back the same values.
    pub fn to_text(&self) -> String {
        let mut out = String::from(FORMAT);
        out.push('\n');
        for s in &self.strokes {
            out.push_str(&format!("stroke {} {:08x} {}\n", s.ink.kind.name(), s.ink.color, s.ink.width));
            for p in &s.points {
                out.push_str(&format!(
                    "point {} {} {} {} {} {} {} {} {}\n",
                    p.x, p.y, p.time_offset, p.size.0, p.size.1, p.opacity, p.force, p.azimuth, p.altitude
                ));
            }
        }
        out
    }

    /// Reads the text form. Blank lines are skipped. Any malformed line, a non-finite number, or a point before
    /// the first stroke is an error that names the line.
    pub fn from_text(text: &str) -> Result<Drawing, DrawingError> {
        let mut lines = text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty());
        match lines.next() {
            Some((_, l)) if l.trim() == FORMAT => {}
            _ => return Err(DrawingError::BadHeader),
        }
        let mut drawing = Drawing::new();
        for (idx, line) in lines {
            let line_no = idx + 1;
            let fail = |reason: &'static str| DrawingError::Line { line: line_no, reason };
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.first().copied() {
                Some("stroke") => {
                    if f.len() != 4 {
                        return Err(fail("stroke needs a kind, a colour and a width"));
                    }
                    let kind = InkKind::from_name(f[1]).ok_or_else(|| fail("unknown ink kind"))?;
                    let color = u32::from_str_radix(f[2], 16).map_err(|_| fail("bad colour"))?;
                    let width = finite(f[3]).ok_or_else(|| fail("bad width"))?;
                    drawing.push(Stroke { ink: Ink { kind, color, width }, points: Vec::new() });
                }
                Some("point") => {
                    if f.len() != 10 {
                        return Err(fail("point needs nine numbers"));
                    }
                    let mut n = [0.0f32; 9];
                    for (slot, text) in n.iter_mut().zip(&f[1..]) {
                        *slot = finite(text).ok_or_else(|| fail("bad number in point"))?;
                    }
                    let point = StrokePoint {
                        x: n[0],
                        y: n[1],
                        time_offset: n[2],
                        size: (n[3], n[4]),
                        opacity: n[5],
                        force: n[6],
                        azimuth: n[7],
                        altitude: n[8],
                    };
                    let stroke = drawing.strokes.last_mut().ok_or_else(|| fail("point before any stroke"))?;
                    stroke.points.push(point);
                }
                _ => return Err(fail("unknown line")),
            }
        }
        Ok(drawing)
    }
}

/// Why a drawing text could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingError {
    BadHeader,
    Line { line: usize, reason: &'static str },
}

impl fmt::Display for DrawingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawingError::BadHeader => write!(f, "not a drawing document"),
            DrawingError::Line { line, reason } => write!(f, "line {line}: {reason}"),
        }
    }
}

impl std::error::Error for DrawingError {}

/// The largest raster `Drawing::image` will build, in pixels (about 256 MB as 0xAARRGGBB).
const MAX_RASTER_PIXELS: usize = 1 << 26;

/// A rendered drawing (the result of `image`): `width * height` pixels as 0xAARRGGBB with straight alpha, row by
/// row from the top. Pixels with no ink are 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

/// The pixel span `[a, b)` that drawing units `lo..hi` cover, when pixel 0 starts at `origin` and there are `scale`
/// pixels per unit. The span is clipped to `0..max`.
fn pixel_span(lo: f32, hi: f32, origin: f32, scale: f32, max: usize) -> (usize, usize) {
    let a = ((lo - origin) * scale).floor().clamp(0.0, max as f32) as usize;
    let b = ((hi - origin) * scale).ceil().clamp(0.0, max as f32) as usize;
    (a, b.max(a))
}

/// Source-over blend of colour `rgb` with coverage `alpha` (0..=1) onto a straight-alpha pixel `dst`.
fn over(rgb: u32, alpha: f32, dst: u32) -> u32 {
    let da = (dst >> 24) as f32 / 255.0;
    let out_a = alpha + da * (1.0 - alpha);
    if out_a <= 0.0 {
        return dst;
    }
    let mut out = ((out_a * 255.0).round().min(255.0) as u32) << 24;
    for shift in [0u32, 8, 16] {
        let s = ((rgb >> shift) & 0xff) as f32;
        let d = ((dst >> shift) & 0xff) as f32;
        let v = (s * alpha + d * da * (1.0 - alpha)) / out_a;
        out |= (v.round().clamp(0.0, 255.0) as u32) << shift;
    }
    out
}

/// Paints one stroke into `pixels`, a `w` by `h` image over `rect` at `scale`. The coverage of each pixel is the
/// largest over the stroke's segments, so joints do not darken, and the stroke is blended once per pixel.
fn paint_stroke(s: &Stroke, rect: Rect, scale: f32, w: usize, h: usize, pixels: &mut [u32]) {
    let bounds = match s.bounds() {
        Some(b) => b,
        None => return,
    };
    let radius = (s.ink.width * 0.5).max(0.0);
    let reach = radius + 1.0 / scale;
    let (x0, x1) = pixel_span(bounds.x - reach, bounds.x + bounds.w + reach, rect.x, scale, w);
    let (y0, y1) = pixel_span(bounds.y - reach, bounds.y + bounds.h + reach, rect.y, scale, h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let cw = x1 - x0;
    let mut cov = vec![0.0f32; cw * (y1 - y0)];
    for (a, b) in segments(s) {
        let (sx0, sx1) = pixel_span(a.0.min(b.0) - reach, a.0.max(b.0) + reach, rect.x, scale, w);
        let (sy0, sy1) = pixel_span(a.1.min(b.1) - reach, a.1.max(b.1) + reach, rect.y, scale, h);
        for j in sy0.max(y0)..sy1.min(y1) {
            let cy = rect.y + (j as f32 + 0.5) / scale;
            for i in sx0.max(x0)..sx1.min(x1) {
                let cx = rect.x + (i as f32 + 0.5) / scale;
                let c = ((radius - seg_dist(cx, cy, a, b)) * scale + 0.5).clamp(0.0, 1.0);
                let cell = &mut cov[(j - y0) * cw + (i - x0)];
                *cell = cell.max(c);
            }
        }
    }
    let alpha = (s.ink.color >> 24) as f32 / 255.0;
    let rgb = s.ink.color & 0x00ff_ffff;
    for j in y0..y1 {
        for i in x0..x1 {
            let c = cov[(j - y0) * cw + (i - x0)];
            if c > 0.0 {
                let p = &mut pixels[j * w + i];
                *p = over(rgb, alpha * c, *p);
            }
        }
    }
}

/// The segments of a stroke as `((x0, y0), (x1, y1))`. A stroke of one point gives a zero-length segment at that point,
/// so a dot can still be hit or painted.
fn segments(s: &Stroke) -> Vec<((f32, f32), (f32, f32))> {
    match s.points.as_slice() {
        [] => Vec::new(),
        [p] => vec![((p.x, p.y), (p.x, p.y))],
        pts => pts.windows(2).map(|w| ((w[0].x, w[0].y), (w[1].x, w[1].y))).collect(),
    }
}

/// The distance from `(px, py)` to the segment from `a` to `b`.
fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Whether any segment of `s` lies within `radius` of `(x, y)`.
fn stroke_within(s: &Stroke, x: f32, y: f32, radius: f32) -> bool {
    segments(s).into_iter().any(|(a, b)| seg_dist(x, y, a, b) <= radius)
}

/// Parses one number and rejects NaN and the infinities.
fn finite(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// The smallest box around a set of points, or `None` when the set is empty.
fn bounds_of(points: impl Iterator<Item = (f32, f32)>) -> Option<Rect> {
    let mut it = points;
    let (x0, y0) = it.next()?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (x0, y0, x0, y0);
    for (x, y) in it {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    Some(Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y })
}

/// The smallest box that holds both `a` and `b`.
pub fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink() -> Ink {
        Ink { kind: InkKind::Pen, color: 0xff00_00ff, width: 3.0 }
    }

    fn stroke(points: &[(f32, f32)]) -> Stroke {
        Stroke { ink: ink(), points: points.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect() }
    }

    #[test]
    fn stroke_bounds_are_the_point_box() {
        let s = stroke(&[(10.0, 20.0), (4.0, 30.0), (7.0, 12.0)]);
        assert_eq!(s.bounds(), Some(Rect { x: 4.0, y: 12.0, w: 6.0, h: 18.0 }));
        assert_eq!(stroke(&[]).bounds(), None);
    }

    #[test]
    fn drawing_bounds_join_every_stroke() {
        let d = Drawing::from_strokes(vec![stroke(&[(0.0, 0.0), (5.0, 5.0)]), stroke(&[(20.0, -3.0), (22.0, 1.0)])]);
        // x from 0 to 22, y from -3 to 5
        assert_eq!(d.bounds(), Some(Rect { x: 0.0, y: -3.0, w: 22.0, h: 8.0 }));
        assert_eq!(Drawing::new().bounds(), None);
        assert!(Drawing::new().is_empty());
    }

    #[test]
    fn transform_moves_points_and_keeps_stylus_data() {
        let mut d = Drawing::new();
        let mut s = stroke(&[(3.0, 4.0)]);
        s.points[0].force = 0.5;
        s.points[0].time_offset = 0.25;
        d.push(s);
        d.transform(Affine::scale(2.0, 2.0).then(Affine::translate(1.0, -1.0)));
        let p = d.strokes()[0].points[0];
        assert_eq!((p.x, p.y), (7.0, 7.0));
        assert_eq!(p.force, 0.5);
        assert_eq!(p.time_offset, 0.25);
    }

    #[test]
    fn text_form_round_trips_exactly() {
        let mut s = stroke(&[(1.5, 2.25), (3.0, 0.1)]);
        s.ink = Ink { kind: InkKind::Marker, color: 0x80ff_8800, width: 14.0 };
        s.points[1].force = 0.7;
        s.points[1].altitude = 1.2;
        let d = Drawing::from_strokes(vec![s, stroke(&[(9.0, 9.0)])]);
        let text = d.to_text();
        assert!(text.starts_with("bifrost-drawing 1\nstroke marker 80ff8800 14\n"));
        assert_eq!(Drawing::from_text(&text), Ok(d));
        let empty = Drawing::new();
        assert_eq!(Drawing::from_text(&empty.to_text()), Ok(empty));
    }

    #[test]
    fn reading_rejects_a_bad_header_and_a_point_before_any_stroke() {
        assert_eq!(Drawing::from_text("markup 1\n"), Err(DrawingError::BadHeader));
        assert_eq!(Drawing::from_text(""), Err(DrawingError::BadHeader));
        let text = "bifrost-drawing 1\npoint 1 2 0 0 0 1 0 0 0\n";
        assert_eq!(Drawing::from_text(text), Err(DrawingError::Line { line: 2, reason: "point before any stroke" }));
    }

    #[test]
    fn reading_rejects_non_finite_and_short_lines() {
        let nan = "bifrost-drawing 1\nstroke pen 000000ff 2\npoint NaN 2 0 0 0 1 0 0 0\n";
        assert_eq!(Drawing::from_text(nan), Err(DrawingError::Line { line: 3, reason: "bad number in point" }));
        let short = "bifrost-drawing 1\nstroke pen 000000ff 2\npoint 1 2 0\n";
        assert_eq!(Drawing::from_text(short), Err(DrawingError::Line { line: 3, reason: "point needs nine numbers" }));
        let kind = "bifrost-drawing 1\nstroke crayon 000000ff 2\n";
        assert_eq!(Drawing::from_text(kind), Err(DrawingError::Line { line: 2, reason: "unknown ink kind" }));
    }

    #[test]
    fn union_takes_the_outer_edges() {
        let a = Rect { x: 0.0, y: 0.0, w: 4.0, h: 4.0 };
        let b = Rect { x: 2.0, y: -2.0, w: 10.0, h: 3.0 };
        assert_eq!(union(a, b), Rect { x: 0.0, y: -2.0, w: 12.0, h: 6.0 });
    }

    fn xy(s: &Stroke) -> Vec<(f32, f32)> {
        s.points.iter().map(|p| (p.x, p.y)).collect()
    }

    /// Right-angle L: 10 units along x, then 10 units along y. Total length 20.
    fn corner() -> Stroke {
        stroke(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])
    }

    #[test]
    fn slice_interpolates_the_ends_and_keeps_the_points_between() {
        let s = corner();
        assert_eq!(s.length(), 20.0);
        // Fractions 0.25 to 0.75 are lengths 5 to 15: from (5,0) through the corner to (10,5).
        assert_eq!(xy(&s.slice(0.25, 0.75)), vec![(5.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
        assert_eq!(xy(&s.slice(0.0, 1.0)), xy(&s), "the whole stroke is unchanged");
        assert!(s.slice(0.5, 0.5).points.is_empty(), "an empty range keeps no points");
    }

    #[test]
    fn resample_spaces_points_evenly_by_length() {
        let r = corner().resampled(5);
        assert_eq!(xy(&r), vec![(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (10.0, 5.0), (10.0, 10.0)]);
        assert_eq!(corner().resampled(0).points.len(), 0);
        let still = stroke(&[(3.0, 3.0), (3.0, 3.0)]).resampled(3);
        assert_eq!(xy(&still), vec![(3.0, 3.0); 3], "zero length repeats the start");
    }

    #[test]
    fn blend_moves_points_and_colours_halfway() {
        let mut a = stroke(&[(0.0, 0.0), (10.0, 0.0)]);
        a.ink = Ink { kind: InkKind::Pen, color: 0x0000_0000, width: 2.0 };
        let mut b = stroke(&[(0.0, 10.0), (10.0, 10.0)]);
        b.ink = Ink { kind: InkKind::Marker, color: 0xffff_ffff, width: 4.0 };
        let mid = Stroke::blend(&a, &b, 0.5);
        assert_eq!(xy(&mid), vec![(0.0, 5.0), (10.0, 5.0)]);
        assert_eq!(mid.ink.color, 0x8080_8080, "127.5 rounds to 128 in each channel");
        assert_eq!(mid.ink.width, 3.0);
        assert_eq!(mid.ink.kind, InkKind::Marker, "the kind switches at the halfway point");
        assert_eq!(xy(&Stroke::blend(&a, &b, 0.0)), xy(&a), "t of 0 gives the first stroke");
    }

    #[test]
    fn blend_pads_the_shorter_stroke_by_resampling() {
        // A 2-point line is resampled to 3 points, so its middle point lands at length 10.
        let line = stroke(&[(0.0, 0.0), (20.0, 0.0)]);
        let out = Stroke::blend(&line, &corner(), 0.0);
        assert_eq!(xy(&out), vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)]);
    }

    fn ink_of(color: u32, width: f32) -> Ink {
        Ink { kind: InkKind::Pen, color, width }
    }

    fn line_at_y5(color: u32, width: f32) -> Stroke {
        Stroke { ink: ink_of(color, width), points: vec![StrokePoint::at(0.0, 5.0), StrokePoint::at(10.0, 5.0)] }
    }

    fn px(r: &Raster, x: usize, y: usize) -> u32 {
        r.pixels[y * r.width + x]
    }

    #[test]
    fn image_size_is_the_rect_times_the_scale() {
        let d = Drawing::from_strokes(vec![stroke(&[(1.0, 1.0)])]);
        let r = d.image(Rect { x: 0.0, y: 0.0, w: 10.0, h: 4.0 }, 2.0);
        assert_eq!((r.width, r.height, r.pixels.len()), (20, 8, 160));
        assert_eq!(d.image(Rect { x: 0.0, y: 0.0, w: 0.0, h: 4.0 }, 2.0), Raster::default());
        assert_eq!(d.image(Rect { x: 0.0, y: 0.0, w: 10.0, h: 4.0 }, 0.0), Raster::default());
        assert_eq!(d.image(Rect { x: f32::NAN, y: 0.0, w: 10.0, h: 4.0 }, 1.0), Raster::default());
    }

    #[test]
    fn a_horizontal_line_paints_its_width_and_nothing_beyond_it() {
        let d = Drawing::from_strokes(vec![line_at_y5(0xff00_00ff, 2.0)]);
        let r = d.image(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 1.0);
        // A width of 2 covers the rows whose centres lie within 1 point of y = 5: rows 4 and 5.
        assert_eq!(px(&r, 5, 4), 0xff00_00ff);
        assert_eq!(px(&r, 5, 5), 0xff00_00ff);
        assert_eq!(px(&r, 5, 3), 0);
        assert_eq!(px(&r, 5, 6), 0);
    }

    #[test]
    fn colour_alpha_scales_the_ink() {
        let d = Drawing::from_strokes(vec![line_at_y5(0x8000_00ff, 2.0)]);
        let r = d.image(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 1.0);
        assert_eq!(px(&r, 5, 5) >> 24, 0x80);
        assert_eq!(px(&r, 5, 5) & 0x00ff_ffff, 0x0000_ff, "the colour itself is unchanged");
    }

    #[test]
    fn later_strokes_are_painted_over_earlier_ones() {
        let d = Drawing::from_strokes(vec![line_at_y5(0xff00_00ff, 2.0), line_at_y5(0xffff_0000, 2.0)]);
        let r = d.image(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 1.0);
        assert_eq!(px(&r, 5, 5), 0xffff_0000);
    }

    #[test]
    fn a_dot_paints_a_disc_around_its_point() {
        let dot = Stroke { ink: ink_of(0xff00_0000, 4.0), points: vec![StrokePoint::at(5.0, 5.0)] };
        let r = Drawing::from_strokes(vec![dot]).image(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 1.0);
        assert_eq!(px(&r, 5, 5), 0xff00_0000);
        assert_eq!(px(&r, 0, 0), 0);
    }

    #[test]
    fn strokes_outside_the_rect_are_clipped_without_panicking() {
        let d = Drawing::from_strokes(vec![line_at_y5(0xff00_00ff, 2.0)]);
        let r = d.image(Rect { x: 5.0, y: 0.0, w: 5.0, h: 10.0 }, 1.0);
        assert_eq!((r.width, r.height), (5, 10));
        assert_eq!(px(&r, 0, 5), 0xff00_00ff, "world x 5.5 is the first column");
        let away = d.image(Rect { x: 100.0, y: 100.0, w: 5.0, h: 5.0 }, 1.0);
        assert!(away.pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn vector_eraser_removes_whole_strokes_within_the_radius() {
        let mut d = Drawing::from_strokes(vec![
            stroke(&[(0.0, 0.0), (10.0, 0.0)]),
            stroke(&[(0.0, 10.0), (10.0, 10.0)]),
            stroke(&[(20.0, 0.0), (20.0, 10.0)]),
        ]);
        assert_eq!(d.erase_strokes_at(5.0, 0.5, 1.0), 1, "only the first line is within reach");
        assert_eq!(d.strokes().len(), 2);
        assert_eq!(d.erase_strokes_at(5.0, 4.0, 1.0), 0, "a point between the lines hits nothing");
        assert_eq!(d.erase_strokes_at(20.0, 5.0, 0.0), 1, "a zero radius hits a segment it lies on");
        assert_eq!(d.strokes().len(), 1);
        assert_eq!(d.erase_strokes_at(0.0, 0.0, f32::NAN), 0, "a NaN radius erases nothing");
        assert_eq!(d.erase_strokes_at(0.0, 0.0, -1.0), 0, "a negative radius erases nothing");
        assert_eq!(d.strokes().len(), 1);
    }

    #[test]
    fn from_path_makes_one_stroke_per_subpath() {
        let mut p = crate::cg::path::Path::new();
        p.move_to((0.0, 0.0));
        p.cubic_to((10.0, 0.0), (20.0, 0.0), (30.0, 0.0));
        p.move_to((5.0, 5.0));
        let d = Drawing::from_path(&p, 1.0, ink());
        assert_eq!(d.strokes().len(), 1, "a lone move draws nothing");
        let pts = &d.strokes()[0].points;
        assert!(pts.len() > 2, "the curve is split into several points");
        assert_eq!(pts.last().map(|q| (q.x, q.y)), Some((30.0, 0.0)));
        assert!(pts.iter().all(|q| q.y.abs() < 1e-4));
        assert_eq!(d.strokes()[0].ink, ink());
    }
}
