//! Hand-drawn content: a drawing (`NativeDrawing`) holds strokes (`NativeStroke`), a stroke holds touch points
//! (`NativeStrokePoint`) and each stroke names its ink (`NativeInk`). Points keep the touch data the reference SDK
//! records (time offset, force, azimuth, altitude), so a stylus stroke saves as it was made. The drawing has a text
//! form that saves and reloads. Capturing touch input, rendering and the tool palette belong to the shell and apps.
use crate::affine::AffineTransform;
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
    pub fn transform(&mut self, t: AffineTransform) {
        for p in &mut self.points {
            let (x, y) = t.apply(p.x, p.y);
            p.x = x;
            p.y = y;
        }
    }
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
    pub fn transform(&mut self, t: AffineTransform) {
        for s in &mut self.strokes {
            s.transform(t);
        }
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
        d.transform(AffineTransform::scale(2.0, 2.0).then(AffineTransform::translate(1.0, -1.0)));
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
}
