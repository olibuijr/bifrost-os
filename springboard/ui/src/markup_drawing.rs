//! Drawings: freehand strokes as data. A drawing is a list of strokes. A stroke is an ink plus timed points that carry
//! position, force, altitude and azimuth, and a transform of its own. The model saves a drawing to bytes in a versioned
//! little-endian format that reads back exactly, selects strokes with a polygon (a lasso), transforms them, slices a
//! stroke by distance along it, and turns the flattened outline of a path into strokes. Nothing here reads the input
//! device or draws: the shell or an app feeds points in and renders the strokes that come back.

use crate::cg::path::Affine;
use crate::cg::path::{Path, Point};
use crate::Rect;
use std::f32::consts::FRAC_PI_2;
use std::fmt;

/// The four bytes that open a saved drawing.
pub const MAGIC: [u8; 4] = *b"BDRW";
/// The format version this code writes, and the only one it reads.
pub const VERSION: u8 = 1;
/// The width of a stroke at zero force, as a fraction of the ink width. Full force draws the full width.
pub const MIN_FORCE_WIDTH: f32 = 0.25;

/// The tool an ink was made with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum InkKind {
    Pen = 0,
    Marker = 1,
    Pencil = 2,
}

/// How a stroke is drawn: the tool, a packed colour the caller interprets, the width in points and the opacity (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub kind: InkKind,
    pub color: u32,
    pub width: f32,
    pub opacity: f32,
}

/// One sample along a stroke. `time` is seconds since the stroke began. `force` is the touch force divided by the
/// device maximum, from 0 to 1. `altitude` is the angle between the stylus and the surface in radians (pi/2 is upright).
/// `azimuth` is the stylus heading in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub time: f32,
    pub force: f32,
    pub altitude: f32,
    pub azimuth: f32,
}

impl StrokePoint {
    /// A point with no timing, full force and an upright stylus.
    pub fn at(x: f32, y: f32) -> Self {
        StrokePoint { x, y, time: 0.0, force: 1.0, altitude: FRAC_PI_2, azimuth: 0.0 }
    }

    /// A point from raw device readings. The force is divided by the device maximum and clamped to 0 to 1. A device
    /// that reports no maximum, or a non-finite force, gives zero force.
    pub fn from_device(x: f32, y: f32, time: f32, raw_force: f32, max_force: f32, altitude: f32, azimuth: f32) -> Self {
        let force = if raw_force.is_finite() && max_force > 0.0 { (raw_force / max_force).clamp(0.0, 1.0) } else { 0.0 };
        StrokePoint { x, y, time, force, altitude, azimuth }
    }

    fn lerp(a: StrokePoint, b: StrokePoint, t: f32) -> StrokePoint {
        let mix = |p: f32, q: f32| p + (q - p) * t;
        StrokePoint {
            x: mix(a.x, b.x),
            y: mix(a.y, b.y),
            time: mix(a.time, b.time),
            force: mix(a.force, b.force),
            altitude: mix(a.altitude, b.altitude),
            azimuth: mix(a.azimuth, b.azimuth),
        }
    }
}

fn seg(a: &StrokePoint, b: &StrokePoint) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    (dx * dx + dy * dy).sqrt()
}

/// One stroke: its ink, a transform from stroke space to drawing space, and its points in stroke space.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub ink: Ink,
    pub transform: Affine,
    pub points: Vec<StrokePoint>,
}

impl Stroke {
    pub fn new(ink: Ink, points: Vec<StrokePoint>) -> Self {
        Stroke { ink, transform: Affine::IDENTITY, points }
    }

    /// The drawn width at a point: the ink width, scaled from 25 percent at zero force to the full width at full force.
    pub fn width_at(&self, point: &StrokePoint) -> f32 {
        let force = if point.force.is_finite() { point.force.clamp(0.0, 1.0) } else { 0.0 };
        self.ink.width * (MIN_FORCE_WIDTH + (1.0 - MIN_FORCE_WIDTH) * force)
    }

    /// The length of the path through the points, in stroke space.
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| seg(&w[0], &w[1])).sum()
    }

    /// The point a distance along the stroke from its first point, interpolated between the samples. A distance of zero
    /// or less (or not a number) gives the first point, and a distance past the end gives the last. `None` when the
    /// stroke has no points.
    pub fn point_at(&self, distance: f32) -> Option<StrokePoint> {
        let first = *self.points.first()?;
        if !(distance > 0.0) || self.points.len() == 1 {
            return Some(first);
        }
        let mut walked = 0.0;
        for pair in self.points.windows(2) {
            let step = seg(&pair[0], &pair[1]);
            if distance <= walked + step {
                let t = if step > 0.0 { (distance - walked) / step } else { 0.0 };
                return Some(StrokePoint::lerp(pair[0], pair[1], t));
            }
            walked += step;
        }
        self.points.last().copied()
    }

    /// The part of the stroke between two distances along it, with interpolated ends and the samples in between. The
    /// range is clamped to the stroke. `None` when the range is empty or the stroke has no length.
    pub fn slice(&self, from: f32, to: f32) -> Option<Stroke> {
        let from = from.max(0.0);
        let to = to.min(self.length());
        if !(to > from) {
            return None;
        }
        let mut points = vec![self.point_at(from)?];
        let mut walked = 0.0;
        for (i, pair) in self.points.windows(2).enumerate() {
            walked += seg(&pair[0], &pair[1]);
            if walked > from && walked < to {
                points.push(self.points[i + 1]);
            }
        }
        points.push(self.point_at(to)?);
        Some(Stroke { ink: self.ink, transform: self.transform, points })
    }

    /// The bounds of the points in drawing space, after the stroke's transform. The drawn width is not included.
    pub fn bounds(&self) -> Option<Rect> {
        let mut it = self.points.iter().map(|p| self.transform.apply(p.x, p.y));
        let (x0, y0) = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (x0, y0, x0, y0);
        for (x, y) in it {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }
}

/// Even-odd point in polygon test. The polygon closes itself.
fn inside(polygon: &[Point], x: f32, y: f32) -> bool {
    let mut hit = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let (xi, yi) = polygon[i];
        let (xj, yj) = polygon[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            hit = !hit;
        }
        j = i;
    }
    hit
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// A set of strokes. Later strokes draw on top of earlier ones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    strokes: Vec<Stroke>,
}

impl Drawing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_strokes(strokes: Vec<Stroke>) -> Self {
        Drawing { strokes }
    }

    /// Strokes from a path. Each flattened subpath, at `tolerance`, becomes one stroke in the given ink with zero time,
    /// full force and an upright stylus. Subpaths with fewer than two points are dropped.
    pub fn from_path(ink: Ink, path: &Path, tolerance: f32) -> Self {
        let strokes: Vec<Stroke> = path
            .flatten(tolerance)
            .into_iter()
            .map(|line| Stroke::new(ink, line.into_iter().map(|(x, y)| StrokePoint::at(x, y)).collect()))
            .collect();
        Drawing { strokes }
    }

    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    /// Adds the strokes of `other` on top.
    pub fn append(&mut self, other: Drawing) {
        self.strokes.extend(other.strokes);
    }

    pub fn appending(mut self, other: Drawing) -> Drawing {
        self.append(other);
        self
    }

    /// Applies a transform to every stroke, after each stroke's own transform.
    pub fn transform(&mut self, t: Affine) {
        for s in &mut self.strokes {
            s.transform = s.transform.then(t);
        }
    }

    /// Applies a transform to the strokes at `indices` only. Each index counts once, and indices past the end are
    /// ignored. Returns how many strokes were transformed.
    pub fn transform_strokes(&mut self, indices: &[usize], t: Affine) -> usize {
        let mut hit = vec![false; self.strokes.len()];
        for &i in indices {
            if i < hit.len() {
                hit[i] = true;
            }
        }
        let mut changed = 0;
        for (s, on) in self.strokes.iter_mut().zip(hit) {
            if on {
                s.transform = s.transform.then(t);
                changed += 1;
            }
        }
        changed
    }

    /// The bounds of all strokes in drawing space, or `None` when no stroke has a point.
    pub fn bounds(&self) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        for r in self.strokes.iter().filter_map(Stroke::bounds) {
            out = Some(match out {
                None => r,
                Some(u) => union(u, r),
            });
        }
        out
    }

    /// The indices of the strokes with at least one point inside `polygon`, in drawing space after the transforms. The
    /// polygon closes itself. With fewer than three corners nothing is selected.
    pub fn lasso(&self, polygon: &[Point]) -> Vec<usize> {
        if polygon.len() < 3 {
            return Vec::new();
        }
        self.strokes
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.points.iter().any(|p| {
                    let (x, y) = s.transform.apply(p.x, p.y);
                    inside(polygon, x, y)
                })
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The drawing as bytes: the magic, the version, the stroke count, then each stroke's tool, colour, width, opacity,
    /// transform and points. Every number is little endian and every point is six 32-bit floats.
    pub fn data_representation(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        put_u32(&mut out, self.strokes.len() as u32);
        for s in &self.strokes {
            out.push(s.ink.kind as u8);
            put_u32(&mut out, s.ink.color);
            put_f32(&mut out, s.ink.width);
            put_f32(&mut out, s.ink.opacity);
            let t = s.transform;
            for v in [t.a, t.b, t.c, t.d, t.tx, t.ty] {
                put_f32(&mut out, v);
            }
            put_u32(&mut out, s.points.len() as u32);
            for p in &s.points {
                for v in [p.x, p.y, p.time, p.force, p.altitude, p.azimuth] {
                    put_f32(&mut out, v);
                }
            }
        }
        out
    }

    /// Reads a drawing written by `data_representation`. Rejects a bad magic, another version, an unknown tool, a
    /// truncated or over-long input, and any non-finite number.
    pub fn from_data(bytes: &[u8]) -> Result<Drawing, DrawingError> {
        let mut r = Reader { bytes, pos: 0 };
        if r.take(4)? != &MAGIC[..] {
            return Err(DrawingError::BadMagic);
        }
        let version = r.u8()?;
        if version != VERSION {
            return Err(DrawingError::UnsupportedVersion(version));
        }
        // A stroke is at least 41 bytes before its points (tool, colour, width, opacity, six transform numbers, count).
        let count = r.count(41)?;
        let mut strokes = Vec::with_capacity(count);
        for _ in 0..count {
            let v = r.u8()?;
            let kind = match v {
                0 => InkKind::Pen,
                1 => InkKind::Marker,
                2 => InkKind::Pencil,
                _ => return Err(DrawingError::BadInkKind(v)),
            };
            let color = r.u32()?;
            let width = r.f32()?;
            let opacity = r.f32()?;
            let transform = Affine {
                a: r.f32()?,
                b: r.f32()?,
                c: r.f32()?,
                d: r.f32()?,
                tx: r.f32()?,
                ty: r.f32()?,
            };
            let n = r.count(24)?;
            let mut points = Vec::with_capacity(n);
            for _ in 0..n {
                points.push(StrokePoint {
                    x: r.f32()?,
                    y: r.f32()?,
                    time: r.f32()?,
                    force: r.f32()?,
                    altitude: r.f32()?,
                    azimuth: r.f32()?,
                });
            }
            strokes.push(Stroke { ink: Ink { kind, color, width, opacity }, transform, points });
        }
        if r.pos != bytes.len() {
            return Err(DrawingError::TrailingBytes);
        }
        Ok(Drawing { strokes })
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    put_u32(out, v.to_bits());
}

/// Why saved bytes could not be read as a drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingError {
    BadMagic,
    UnsupportedVersion(u8),
    BadInkKind(u8),
    Truncated,
    TrailingBytes,
    NonFinite,
}

impl fmt::Display for DrawingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawingError::BadMagic => write!(f, "not a drawing"),
            DrawingError::UnsupportedVersion(v) => write!(f, "drawing version {v} is not supported"),
            DrawingError::BadInkKind(v) => write!(f, "unknown ink kind {v}"),
            DrawingError::Truncated => write!(f, "drawing data ends early"),
            DrawingError::TrailingBytes => write!(f, "drawing data has bytes after the last stroke"),
            DrawingError::NonFinite => write!(f, "drawing data has a number that is not finite"),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DrawingError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or(DrawingError::Truncated)?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, DrawingError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, DrawingError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self) -> Result<f32, DrawingError> {
        let v = f32::from_bits(self.u32()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(DrawingError::NonFinite)
        }
    }

    /// Reads an element count and checks that the rest of the input can hold that many elements of `min_each` bytes,
    /// so a bad count cannot ask for a huge allocation.
    fn count(&mut self, min_each: usize) -> Result<usize, DrawingError> {
        let n = self.u32()? as usize;
        let need = n.checked_mul(min_each).ok_or(DrawingError::Truncated)?;
        if need > self.bytes.len() - self.pos {
            return Err(DrawingError::Truncated);
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink() -> Ink {
        Ink { kind: InkKind::Pen, color: 0xff00_00ff, width: 4.0, opacity: 1.0 }
    }

    fn stroke(pts: &[(f32, f32)]) -> Stroke {
        Stroke::new(ink(), pts.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect())
    }

    fn timed(pts: &[(f32, f32, f32)]) -> Stroke {
        Stroke::new(ink(), pts.iter().map(|&(x, y, t)| StrokePoint { time: t, ..StrokePoint::at(x, y) }).collect())
    }

    fn sample() -> Drawing {
        let mut s2 = stroke(&[(0.0, 0.0), (3.0, 4.0), (3.0, 10.0)]);
        s2.transform = Affine::translate(5.0, 0.0);
        Drawing::from_strokes(vec![stroke(&[(0.0, 0.0), (1.0, 1.0)]), s2])
    }

    #[test]
    fn saved_data_reads_back_exactly_and_has_the_documented_size() {
        let d = sample();
        let bytes = d.data_representation();
        // header 4 + 1 + 4, then 41 + 2 * 24 for the first stroke and 41 + 3 * 24 for the second
        assert_eq!(bytes.len(), 9 + 89 + 113);
        assert_eq!(&bytes[..4], b"BDRW");
        assert_eq!(bytes[4], 1);
        assert_eq!(Drawing::from_data(&bytes), Ok(d));
    }

    #[test]
    fn bad_saved_data_is_rejected_with_the_matching_error() {
        let bytes = sample().data_representation();
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(Drawing::from_data(&magic), Err(DrawingError::BadMagic));
        let mut version = bytes.clone();
        version[4] = 2;
        assert_eq!(Drawing::from_data(&version), Err(DrawingError::UnsupportedVersion(2)));
        let mut kind = bytes.clone();
        kind[9] = 7;
        assert_eq!(Drawing::from_data(&kind), Err(DrawingError::BadInkKind(7)));
        assert_eq!(Drawing::from_data(&bytes[..bytes.len() - 1]), Err(DrawingError::Truncated));
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(Drawing::from_data(&long), Err(DrawingError::TrailingBytes));
        let mut nan = sample();
        nan.strokes[0].points[0].x = f32::NAN;
        assert_eq!(Drawing::from_data(&nan.data_representation()), Err(DrawingError::NonFinite));
    }

    #[test]
    fn point_at_interpolates_position_and_time_along_the_path() {
        let s = timed(&[(0.0, 0.0, 0.0), (3.0, 4.0, 1.0), (3.0, 10.0, 2.0)]);
        assert_eq!(s.length(), 11.0);
        let mid = s.point_at(2.5).unwrap();
        assert_eq!((mid.x, mid.y, mid.time), (1.5, 2.0, 0.5));
        let second = s.point_at(8.0).unwrap();
        assert_eq!((second.x, second.y, second.time), (3.0, 7.0, 1.5));
        assert_eq!(s.point_at(99.0).unwrap().y, 10.0);
        assert_eq!(s.point_at(-1.0).unwrap().x, 0.0);
    }

    #[test]
    fn slice_keeps_the_samples_inside_the_range_and_interpolates_the_ends() {
        let s = stroke(&[(0.0, 0.0), (3.0, 4.0), (3.0, 10.0)]);
        let part = s.slice(2.5, 8.0).unwrap();
        let xy: Vec<(f32, f32)> = part.points.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(xy, vec![(1.5, 2.0), (3.0, 4.0), (3.0, 7.0)]);
        assert_eq!(s.slice(4.0, 4.0), None);
        assert_eq!(stroke(&[(1.0, 1.0)]).slice(0.0, 5.0), None);
    }

    #[test]
    fn stroke_width_follows_force_from_a_quarter_to_full() {
        let s = stroke(&[(0.0, 0.0)]);
        let at = |raw: f32, max: f32| StrokePoint::from_device(0.0, 0.0, 0.0, raw, max, FRAC_PI_2, 0.0);
        assert_eq!(s.width_at(&at(0.0, 1000.0)), 1.0);
        assert_eq!(s.width_at(&at(1000.0, 1000.0)), 4.0);
        assert_eq!(s.width_at(&at(500.0, 1000.0)), 2.5);
        // a device without a maximum gives no force, and readings past the maximum clamp to full
        assert_eq!(at(500.0, 0.0).force, 0.0);
        assert_eq!(at(2000.0, 1000.0).force, 1.0);
    }

    #[test]
    fn lasso_selects_strokes_with_a_point_inside_the_polygon_after_transforms() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut d = Drawing::from_strokes(vec![stroke(&[(5.0, 5.0), (20.0, 20.0)]), stroke(&[(20.0, 20.0), (30.0, 30.0)])]);
        assert_eq!(d.lasso(&square), vec![0]);
        assert_eq!(d.transform_strokes(&[1, 9], Affine::translate(-18.0, -18.0)), 1);
        assert_eq!(d.lasso(&square), vec![0, 1]);
        assert_eq!(d.lasso(&square[..2]), Vec::<usize>::new());
    }

    #[test]
    fn bounds_are_taken_in_drawing_space() {
        let mut s = stroke(&[(0.0, 0.0), (10.0, 5.0)]);
        s.transform = Affine::translate(100.0, 0.0);
        assert_eq!(Drawing::from_strokes(vec![s]).bounds(), Some(Rect { x: 100.0, y: 0.0, w: 10.0, h: 5.0 }));
        assert_eq!(Drawing::new().bounds(), None);
    }

    #[test]
    fn a_path_becomes_one_stroke_per_subpath() {
        let mut p = Path::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        let d = Drawing::from_path(ink(), &p, 0.5);
        assert_eq!(d.strokes().len(), 1);
        let pts = &d.strokes()[0].points;
        assert_eq!(pts.len(), 3);
        assert_eq!((pts[2].x, pts[2].y), (10.0, 10.0));
        assert_eq!(pts[0].force, 1.0);
    }
}
