//! Paths and affine transforms: the one path type of the drawing stack. A path is data (move, line, quadratic and
//! cubic curves, close); `raster` turns it into pixels. Coordinates are `f32`, y grows downwards on screen.
use super::raster::{self, FillRule, StrokeStyle};

pub type Point = (f32, f32);

const KAPPA: f32 = 0.552_284_8;
const TAU: f32 = std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, p: Point) -> bool {
        p.0 >= self.x && p.1 >= self.y && p.0 < self.right() && p.1 < self.bottom()
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let (x0, y0) = (self.x.min(o.x), self.y.min(o.y));
        let (x1, y1) = (self.right().max(o.right()), self.bottom().max(o.bottom()));
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x <= o.right() && o.x <= self.right() && self.y <= o.bottom() && o.y <= self.bottom()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

/// A 2x3 matrix `a b c d tx ty`: `(x, y)` maps to `(a*x + c*y + tx, b*x + d*y + ty)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

/// Scale, shear, rotation and translation of a matrix (see [`Affine::decompose`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decomposition {
    pub scale: (f32, f32),
    /// Tangent of the horizontal shear angle.
    pub shear: f32,
    pub rotation: f32,
    pub translation: (f32, f32),
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub const fn new(a: f32, b: f32, c: f32, d: f32, tx: f32, ty: f32) -> Affine {
        Affine { a, b, c, d, tx, ty }
    }
    pub fn translation(tx: f32, ty: f32) -> Affine {
        Affine { tx, ty, ..Self::IDENTITY }
    }
    pub fn scaling(sx: f32, sy: f32) -> Affine {
        Affine { a: sx, d: sy, ..Self::IDENTITY }
    }
    /// Rotation about the origin; a positive angle turns the x axis towards the y axis.
    pub fn rotation(radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        Affine { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 }
    }
    // Short names kept for callers that came from the old transform type.
    pub fn translate(tx: f32, ty: f32) -> Affine {
        Self::translation(tx, ty)
    }
    pub fn scale(sx: f32, sy: f32) -> Affine {
        Self::scaling(sx, sy)
    }
    pub fn rotate(radians: f32) -> Affine {
        Self::rotation(radians)
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Applies `self` first and then `next`.
    pub fn then(self, next: Affine) -> Affine {
        let (p, n) = (self, next);
        let (tx, ty) = n.apply(p.tx, p.ty);
        Affine {
            a: n.a * p.a + n.c * p.b,
            b: n.b * p.a + n.d * p.b,
            c: n.a * p.c + n.c * p.d,
            d: n.b * p.c + n.d * p.d,
            tx,
            ty,
        }
    }
    /// Same as [`Affine::then`].
    pub fn concat(self, next: Affine) -> Affine {
        self.then(next)
    }
    /// `self` preceded by a translation (the translation is applied to points first).
    pub fn translated(self, tx: f32, ty: f32) -> Affine {
        Affine::translation(tx, ty).then(self)
    }
    pub fn scaled(self, sx: f32, sy: f32) -> Affine {
        Affine::scaling(sx, sy).then(self)
    }
    pub fn rotated(self, radians: f32) -> Affine {
        Affine::rotation(radians).then(self)
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
    pub fn apply_point(&self, p: Point) -> Point {
        self.apply(p.0, p.1)
    }
    /// Maps a size (a vector: no translation).
    pub fn apply_size(&self, s: Size) -> Size {
        Size { w: self.a * s.w + self.c * s.h, h: self.b * s.w + self.d * s.h }
    }
    /// The bounding box of the four transformed corners.
    pub fn apply_rect(&self, r: Rect) -> Rect {
        let c = [self.apply(r.x, r.y), self.apply(r.right(), r.y), self.apply(r.x, r.bottom()), self.apply(r.right(), r.bottom())];
        let min_x = c.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let min_y = c.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let max_x = c.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
        let max_y = c.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y }
    }
    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }
    /// The inverse, or `None` when the matrix collapses an axis.
    pub fn invert(&self) -> Option<Affine> {
        let det = self.determinant();
        if !(det.abs() > 1e-12) {
            return None;
        }
        Some(Affine {
            a: self.d / det,
            b: -self.b / det,
            c: -self.c / det,
            d: self.a / det,
            tx: (self.c * self.ty - self.d * self.tx) / det,
            ty: (self.b * self.tx - self.a * self.ty) / det,
        })
    }
    /// The inverse, or `self` when there is none.
    pub fn inverted_or_self(&self) -> Affine {
        self.invert().unwrap_or(*self)
    }
    /// Splits into scale, shear, rotation and translation; [`Affine::from_decomposition`] puts them back together
    /// (shear, then scale, then rotation, then translation).
    pub fn decompose(&self) -> Decomposition {
        let sx = self.a.hypot(self.b);
        let (r0x, r0y) = if sx > 0.0 { (self.a / sx, self.b / sx) } else { (1.0, 0.0) };
        let shear_raw = r0x * self.c + r0y * self.d;
        let (px, py) = (self.c - shear_raw * r0x, self.d - shear_raw * r0y);
        let mut sy = px.hypot(py);
        // The second axis must be the first turned a quarter; a flip shows up as a negative y scale.
        if r0x * py - r0y * px < 0.0 {
            sy = -sy;
        }
        Decomposition {
            scale: (sx, sy),
            shear: if sy != 0.0 { shear_raw / sy } else { 0.0 },
            rotation: self.b.atan2(self.a),
            translation: (self.tx, self.ty),
        }
    }
    /// The matrix [`Affine::decompose`] split.
    pub fn from_decomposition(d: &Decomposition) -> Affine {
        let (sx, sy) = d.scale;
        let shear = Affine::new(sx, 0.0, d.shear * sy, sy, 0.0, 0.0);
        shear.then(Affine::rotation(d.rotation)).then(Affine::translation(d.translation.0, d.translation.1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Element {
    Move(Point),
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
    Close,
}

/// One flattened subpath. A closed one has an implicit edge from its last point to its first.
#[derive(Clone, Debug, PartialEq)]
pub struct Subpath {
    pub points: Vec<Point>,
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Path {
    elements: Vec<Element>,
    cur: Option<Point>,
    start: Point,
}

impl PartialEq for Path {
    fn eq(&self, o: &Path) -> bool {
        self.elements == o.elements
    }
}

fn dist(a: Point, b: Point) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}
fn lerp(a: Point, b: Point, t: f32) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}
fn quad_at(p0: Point, p1: Point, p2: Point, t: f32) -> Point {
    lerp(lerp(p0, p1, t), lerp(p1, p2, t), t)
}
fn cubic_at(p0: Point, p1: Point, p2: Point, p3: Point, t: f32) -> Point {
    let (a, b, c) = (lerp(p0, p1, t), lerp(p1, p2, t), lerp(p2, p3, t));
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}

/// Parameters in (0, 1) where one coordinate of a cubic has a turning point.
fn cubic_extrema(a0: f32, a1: f32, a2: f32, a3: f32) -> Vec<f32> {
    let (qa, qb, qc) = (-a0 + 3.0 * a1 - 3.0 * a2 + a3, 2.0 * (a0 - 2.0 * a1 + a2), a1 - a0);
    let mut out = Vec::new();
    if qa.abs() < 1e-9 {
        if qb.abs() > 1e-9 {
            out.push(-qc / qb);
        }
    } else {
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            let s = disc.sqrt();
            out.push((-qb + s) / (2.0 * qa));
            out.push((-qb - s) / (2.0 * qa));
        }
    }
    out.retain(|t| *t > 0.0 && *t < 1.0);
    out
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }
    pub fn from_rect(r: Rect) -> Path {
        let mut p = Path::new();
        p.add_rect(r);
        p
    }
    pub fn from_rounded_rect(r: Rect, rx: f32, ry: f32) -> Path {
        let mut p = Path::new();
        p.add_rounded_rect(r, rx, ry);
        p
    }
    pub fn from_ellipse(r: Rect) -> Path {
        let mut p = Path::new();
        p.add_ellipse(r);
        p
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }
    pub fn iter(&self) -> std::slice::Iter<'_, Element> {
        self.elements.iter()
    }
    /// Calls `f` for every element in order.
    pub fn apply(&self, mut f: impl FnMut(&Element)) {
        self.elements.iter().for_each(&mut f);
    }
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
    pub fn current_point(&self) -> Option<Point> {
        self.cur
    }

    pub fn move_to(&mut self, p: Point) {
        // A move right after a move replaces it.
        if let Some(Element::Move(_)) = self.elements.last() {
            self.elements.pop();
        }
        self.elements.push(Element::Move(p));
        self.cur = Some(p);
        self.start = p;
    }
    /// Makes sure a subpath is open (a line or curve with no current point starts one at `p`).
    fn open(&mut self, p: Point) -> bool {
        match (self.cur, self.elements.last()) {
            (None, _) => {
                self.move_to(p);
                false
            }
            (Some(_), Some(Element::Close)) => {
                let s = self.start;
                self.elements.push(Element::Move(s));
                true
            }
            _ => true,
        }
    }
    pub fn line_to(&mut self, p: Point) {
        if self.open(p) {
            self.elements.push(Element::Line(p));
            self.cur = Some(p);
        }
    }
    pub fn quad_to(&mut self, c: Point, p: Point) {
        if self.open(c) {
            self.elements.push(Element::Quad(c, p));
            self.cur = Some(p);
        }
    }
    pub fn cubic_to(&mut self, c1: Point, c2: Point, p: Point) {
        if self.open(c1) {
            self.elements.push(Element::Cubic(c1, c2, p));
            self.cur = Some(p);
        }
    }
    pub fn close(&mut self) {
        if self.cur.is_some() && !matches!(self.elements.last(), Some(Element::Close)) {
            self.elements.push(Element::Close);
            self.cur = Some(self.start);
        }
    }
    pub fn add_lines(&mut self, pts: &[Point]) {
        if let Some((&first, rest)) = pts.split_first() {
            self.move_to(first);
            for &p in rest {
                self.line_to(p);
            }
        }
    }

    /// A closed rectangle subpath from the top left corner.
    pub fn add_rect(&mut self, r: Rect) {
        self.move_to((r.x, r.y));
        self.line_to((r.right(), r.y));
        self.line_to((r.right(), r.bottom()));
        self.line_to((r.x, r.bottom()));
        self.close();
    }
    pub fn add_rects(&mut self, rs: &[Rect]) {
        rs.iter().for_each(|r| self.add_rect(*r));
    }
    /// A closed rounded rectangle; radii are clamped to half the box.
    pub fn add_rounded_rect(&mut self, r: Rect, rx: f32, ry: f32) {
        let rx = rx.clamp(0.0, r.w.max(0.0) / 2.0);
        let ry = ry.clamp(0.0, r.h.max(0.0) / 2.0);
        if rx <= 0.0 || ry <= 0.0 {
            return self.add_rect(r);
        }
        let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        self.move_to((x0 + rx, y0));
        self.line_to((x1 - rx, y0));
        self.cubic_to((x1 - rx + kx, y0), (x1, y0 + ry - ky), (x1, y0 + ry));
        self.line_to((x1, y1 - ry));
        self.cubic_to((x1, y1 - ry + ky), (x1 - rx + kx, y1), (x1 - rx, y1));
        self.line_to((x0 + rx, y1));
        self.cubic_to((x0 + rx - kx, y1), (x0, y1 - ry + ky), (x0, y1 - ry));
        self.line_to((x0, y0 + ry));
        self.cubic_to((x0, y0 + ry - ky), (x0 + rx - kx, y0), (x0 + rx, y0));
        self.close();
    }
    /// A closed ellipse inscribed in `r`, four cubic quarter arcs from the right-most point.
    pub fn add_ellipse(&mut self, r: Rect) {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (rx, ry) = (r.w / 2.0, r.h / 2.0);
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        self.move_to((cx + rx, cy));
        self.cubic_to((cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry));
        self.cubic_to((cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy));
        self.cubic_to((cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry));
        self.cubic_to((cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy));
        self.close();
    }

    /// Cubic pieces for an elliptical arc from angle `start` sweeping `delta`, continuing from the current point
    /// (which the caller has put at the arc start).
    fn arc_pieces(&mut self, c: Point, rx: f32, ry: f32, start: f32, delta: f32) {
        let n = ((delta.abs() / std::f32::consts::FRAC_PI_2).ceil() as usize).max(1);
        let step = delta / n as f32;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        for i in 0..n {
            let a0 = start + step * i as f32;
            let a1 = a0 + step;
            let (s0, c0) = a0.sin_cos();
            let (s1, c1) = a1.sin_cos();
            self.cubic_to(
                (c.0 + rx * (c0 - k * s0), c.1 + ry * (s0 + k * c0)),
                (c.0 + rx * (c1 + k * s1), c.1 + ry * (s1 - k * c1)),
                (c.0 + rx * c1, c.1 + ry * s1),
            );
        }
    }
    /// An arc about `center` from angle `start` to `end` (radians, angle 0 along +x). With `clockwise` false the
    /// angle increases (towards +y), with true it decreases; this is the sense of the angle, whatever way the y
    /// axis points on screen. A line joins the current point to the arc start.
    pub fn add_arc(&mut self, center: Point, radius: f32, start: f32, end: f32, clockwise: bool) {
        let raw = end - start;
        let delta = if !clockwise {
            if raw >= TAU {
                TAU
            } else if raw < 0.0 {
                raw.rem_euclid(TAU)
            } else {
                raw
            }
        } else if raw <= -TAU {
            -TAU
        } else if raw > 0.0 {
            -((-raw).rem_euclid(TAU))
        } else {
            raw
        };
        self.add_relative_arc(center, radius, start, delta);
    }
    /// An arc from `start` sweeping `delta` radians (positive increases the angle).
    pub fn add_relative_arc(&mut self, center: Point, radius: f32, start: f32, delta: f32) {
        let delta = delta.clamp(-TAU, TAU);
        let from = (center.0 + radius * start.cos(), center.1 + radius * start.sin());
        if self.cur.is_some() {
            self.line_to(from);
        } else {
            self.move_to(from);
        }
        if delta != 0.0 {
            self.arc_pieces(center, radius, radius, start, delta);
        }
    }
    /// A tangent arc: from the current point towards `p1`, rounding the corner at `p1` with `radius`, ending on the
    /// line to `p2`. Collinear points or a zero radius give a straight line to `p1`.
    pub fn add_arc_to(&mut self, p1: Point, p2: Point, radius: f32) {
        let Some(p0) = self.cur else {
            return self.move_to(p1);
        };
        let unit = |a: Point, b: Point| {
            let l = dist(a, b);
            if l > 0.0 { ((b.0 - a.0) / l, (b.1 - a.1) / l) } else { (0.0, 0.0) }
        };
        let (d0, d1) = (unit(p1, p0), unit(p1, p2));
        let cosang = (d0.0 * d1.0 + d0.1 * d1.1).clamp(-1.0, 1.0);
        let ang = cosang.acos();
        if radius <= 0.0 || ang < 1e-4 || (std::f32::consts::PI - ang) < 1e-4 || d0 == (0.0, 0.0) || d1 == (0.0, 0.0) {
            return self.line_to(p1);
        }
        let td = radius / (ang / 2.0).tan();
        let t0 = (p1.0 + d0.0 * td, p1.1 + d0.1 * td);
        let (bx, by) = (d0.0 + d1.0, d0.1 + d1.1);
        let bl = bx.hypot(by);
        let cd = radius / (ang / 2.0).sin();
        let c = (p1.0 + bx / bl * cd, p1.1 + by / bl * cd);
        let t1 = (p1.0 + d1.0 * td, p1.1 + d1.1 * td);
        let a0 = (t0.1 - c.1).atan2(t0.0 - c.0);
        let a1 = (t1.1 - c.1).atan2(t1.0 - c.0);
        let mut delta = (a1 - a0) % TAU;
        if delta > std::f32::consts::PI {
            delta -= TAU;
        } else if delta < -std::f32::consts::PI {
            delta += TAU;
        }
        self.line_to(t0);
        self.arc_pieces(c, radius, radius, a0, delta);
    }

    /// Appends `other`.
    pub fn add_path(&mut self, other: &Path) {
        self.add_path_transformed(other, &Affine::IDENTITY);
    }
    pub fn add_path_transformed(&mut self, other: &Path, t: &Affine) {
        for e in &other.elements {
            match *e {
                Element::Move(p) => self.move_to(t.apply_point(p)),
                Element::Line(p) => self.line_to(t.apply_point(p)),
                Element::Quad(c, p) => self.quad_to(t.apply_point(c), t.apply_point(p)),
                Element::Cubic(a, b, p) => self.cubic_to(t.apply_point(a), t.apply_point(b), t.apply_point(p)),
                Element::Close => self.close(),
            }
        }
    }

    /// Maps every point through `t`, in place.
    pub fn transform(&mut self, t: &Affine) {
        *self = self.transformed(t);
    }
    pub fn transformed(&self, t: &Affine) -> Path {
        let mut out = Path::new();
        out.add_path_transformed(self, t);
        out
    }

    /// The subpaths with curves flattened to within about `tolerance`. A subpath may have a single point.
    pub fn flatten_subpaths(&self, tolerance: f32) -> Vec<Subpath> {
        let tol = tolerance.max(1e-4);
        let mut out: Vec<Subpath> = Vec::new();
        let mut cur: Vec<Point> = Vec::new();
        let mut start = (0.0, 0.0);
        let mut last = (0.0, 0.0);
        let flush = |cur: &mut Vec<Point>, out: &mut Vec<Subpath>, closed: bool| {
            if !cur.is_empty() {
                out.push(Subpath { points: std::mem::take(cur), closed });
            }
        };
        for e in &self.elements {
            match *e {
                Element::Move(p) => {
                    flush(&mut cur, &mut out, false);
                    cur.push(p);
                    start = p;
                    last = p;
                }
                Element::Line(p) => {
                    cur.push(p);
                    last = p;
                }
                Element::Quad(c, p) => {
                    let dd = (last.0 - 2.0 * c.0 + p.0).hypot(last.1 - 2.0 * c.1 + p.1);
                    let n = ((dd / (4.0 * tol)).sqrt().ceil() as usize).clamp(1, 256);
                    for i in 1..=n {
                        cur.push(quad_at(last, c, p, i as f32 / n as f32));
                    }
                    last = p;
                }
                Element::Cubic(c1, c2, p) => {
                    let d1 = (last.0 - 2.0 * c1.0 + c2.0).hypot(last.1 - 2.0 * c1.1 + c2.1);
                    let d2 = (c1.0 - 2.0 * c2.0 + p.0).hypot(c1.1 - 2.0 * c2.1 + p.1);
                    let n = (((0.75 * d1.max(d2)) / tol).sqrt().ceil() as usize).clamp(1, 256);
                    for i in 1..=n {
                        cur.push(cubic_at(last, c1, c2, p, i as f32 / n as f32));
                    }
                    last = p;
                }
                Element::Close => {
                    if cur.len() > 1 && cur.last() == cur.first() {
                        cur.pop();
                    }
                    flush(&mut cur, &mut out, true);
                    last = start;
                }
            }
        }
        flush(&mut cur, &mut out, false);
        out
    }
    /// Flattened polylines; a closed subpath repeats its first point at the end. Single-point subpaths are dropped.
    pub fn flatten(&self, tolerance: f32) -> Vec<Vec<Point>> {
        self.flatten_subpaths(tolerance)
            .into_iter()
            .map(|mut s| {
                if s.closed {
                    let f = s.points[0];
                    s.points.push(f);
                }
                s.points
            })
            .filter(|p| p.len() > 1)
            .collect()
    }
    /// A copy with every curve replaced by line segments.
    pub fn flattened(&self, tolerance: f32) -> Path {
        let mut out = Path::new();
        for s in self.flatten_subpaths(tolerance) {
            out.add_lines(&s.points);
            if s.closed {
                out.close();
            }
        }
        out
    }

    /// The smallest box around the control points (curves may stay inside it), or `None` when empty.
    pub fn control_bounds(&self) -> Option<Rect> {
        let mut b = Bounds::default();
        for e in &self.elements {
            match *e {
                Element::Move(p) | Element::Line(p) => b.add(p),
                Element::Quad(c, p) => {
                    b.add(c);
                    b.add(p)
                }
                Element::Cubic(c1, c2, p) => {
                    b.add(c1);
                    b.add(c2);
                    b.add(p)
                }
                Element::Close => {}
            }
        }
        b.rect()
    }
    /// The exact box of the drawn path (curve extrema included), or `None` when empty.
    pub fn bounds(&self) -> Option<Rect> {
        let mut b = Bounds::default();
        let mut last = (0.0, 0.0);
        for e in &self.elements {
            match *e {
                Element::Move(p) | Element::Line(p) => {
                    b.add(p);
                    last = p;
                }
                Element::Quad(c, p) => {
                    b.add(p);
                    for axis in 0..2 {
                        let g = |q: Point| if axis == 0 { q.0 } else { q.1 };
                        let den = g(last) - 2.0 * g(c) + g(p);
                        if den.abs() > 1e-9 {
                            let t = (g(last) - g(c)) / den;
                            if t > 0.0 && t < 1.0 {
                                b.add(quad_at(last, c, p, t));
                            }
                        }
                    }
                    last = p;
                }
                Element::Cubic(c1, c2, p) => {
                    b.add(p);
                    for axis in 0..2 {
                        let g = |q: Point| if axis == 0 { q.0 } else { q.1 };
                        for t in cubic_extrema(g(last), g(c1), g(c2), g(p)) {
                            b.add(cubic_at(last, c1, c2, p, t));
                        }
                    }
                    last = p;
                }
                Element::Close => {}
            }
        }
        b.rect()
    }

    /// Whether `p` is inside the filled path under `rule`.
    pub fn contains(&self, p: Point, rule: FillRule) -> bool {
        let mut wind = 0i32;
        for s in self.flatten_subpaths(0.05) {
            let n = s.points.len();
            for i in 0..n {
                let (a, b) = (s.points[i], s.points[(i + 1) % n]);
                if a.1 <= p.1 {
                    if b.1 > p.1 && cross(a, b, p) > 0.0 {
                        wind += 1;
                    }
                } else if b.1 <= p.1 && cross(a, b, p) < 0.0 {
                    wind -= 1;
                }
            }
        }
        match rule {
            FillRule::NonZero => wind != 0,
            FillRule::EvenOdd => wind & 1 != 0,
        }
    }

    /// The rectangle when the path is exactly one axis-aligned rectangle.
    pub fn is_rect(&self) -> Option<Rect> {
        let mut pts: Vec<Point> = Vec::new();
        let mut closed = false;
        for (i, e) in self.elements.iter().enumerate() {
            match *e {
                Element::Move(p) if i == 0 => pts.push(p),
                Element::Line(p) if !closed && i > 0 => pts.push(p),
                Element::Close if !closed => closed = true,
                _ => return None,
            }
        }
        if pts.len() == 5 && pts[4] == pts[0] {
            pts.pop();
        }
        if pts.len() != 4 {
            return None;
        }
        let horiz_first = pts[0].1 == pts[1].1 && pts[1].0 == pts[2].0 && pts[2].1 == pts[3].1 && pts[3].0 == pts[0].0;
        let vert_first = pts[0].0 == pts[1].0 && pts[1].1 == pts[2].1 && pts[2].0 == pts[3].0 && pts[3].1 == pts[0].1;
        if !(horiz_first || vert_first) {
            return None;
        }
        let x0 = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let x1 = pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
        let y0 = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let y1 = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }

    /// Total length of the flattened path, closing edges included.
    pub fn length(&self, tolerance: f32) -> f32 {
        self.flatten_subpaths(tolerance).iter().flat_map(edges).map(|(p, q)| dist(p, q)).sum()
    }
    /// The part between `from` and `to` (fractions of the length, clamped to 0..=1) as open polylines.
    pub fn trimmed(&self, from: f32, to: f32, tolerance: f32) -> Path {
        let subs = self.flatten_subpaths(tolerance);
        let total: f32 = subs.iter().flat_map(edges).map(|(p, q)| dist(p, q)).sum();
        let mut out = Path::new();
        let (a, b) = (from.clamp(0.0, 1.0) * total, to.clamp(0.0, 1.0) * total);
        if total <= 0.0 || b <= a {
            return out;
        }
        let mut at = 0.0f32;
        let mut last: Option<Point> = None;
        for (p, q) in subs.iter().flat_map(edges) {
            let len = dist(p, q);
            if len <= 0.0 {
                continue;
            }
            let (s0, s1) = (at, at + len);
            at = s1;
            let (lo, hi) = (s0.max(a), s1.min(b));
            if hi <= lo {
                continue;
            }
            let start = lerp(p, q, (lo - s0) / len);
            let end = lerp(p, q, (hi - s0) / len);
            if !last.map_or(false, |l| dist(l, start) < 1e-4) {
                out.move_to(start);
            }
            out.line_to(end);
            last = Some(end);
        }
        out
    }

    /// A copy cut into dashes (`pattern` alternates on and off lengths; see [`raster::dash_polyline`]).
    pub fn copy_by_dashing(&self, pattern: &[f32], phase: f32, tolerance: f32) -> Path {
        let mut out = Path::new();
        for s in self.flatten_subpaths(tolerance) {
            let mut pts = s.points;
            if s.closed && pts.len() > 1 {
                pts.push(pts[0]);
            }
            for run in raster::dash_polyline(&pts, pattern, phase) {
                out.add_lines(&run);
            }
        }
        out
    }
    /// The outline of this path stroked with `style`; fill it with the non-zero rule.
    pub fn copy_by_stroking(&self, style: &StrokeStyle) -> Path {
        raster::stroke_path(self, style)
    }

    /// Whether the filled areas of the two paths overlap.
    pub fn intersects(&self, other: &Path, rule: FillRule) -> bool {
        let (a, b) = (self.flatten_subpaths(0.05), other.flatten_subpaths(0.05));
        let (Some(ba), Some(bb)) = (self.bounds(), other.bounds()) else { return false };
        if !ba.intersects(&bb) {
            return false;
        }
        for (p, q) in a.iter().flat_map(edges) {
            for (r, s) in b.iter().flat_map(edges) {
                if segments_cross(p, q, r, s) {
                    return true;
                }
            }
        }
        let first = |subs: &[Subpath]| subs.iter().find_map(|s| s.points.first().copied());
        first(&a).map_or(false, |p| other.contains(p, rule)) || first(&b).map_or(false, |p| self.contains(p, rule))
    }

    /// Splits into one path per outer outline, each with the holes inside it.
    pub fn separate_components(&self) -> Vec<Path> {
        let subs = self.flatten_subpaths(0.05);
        let mut owners: Vec<Option<usize>> = Vec::new();
        for (i, s) in subs.iter().enumerate() {
            let probe = s.points[0];
            let mut depth = 0;
            let mut inner: Option<(usize, f32)> = None;
            for (j, o) in subs.iter().enumerate() {
                if i != j && o.points.len() > 2 && Path::from_subpath(o).contains(probe, FillRule::EvenOdd) {
                    depth += 1;
                    let area = poly_area(&o.points).abs();
                    if inner.map_or(true, |(_, a)| area < a) {
                        inner = Some((j, area));
                    }
                }
            }
            owners.push(if depth % 2 == 1 { inner.map(|x| x.0) } else { None });
        }
        let mut out = Vec::new();
        for (i, s) in subs.iter().enumerate() {
            if owners[i].is_some() {
                continue;
            }
            let mut p = Path::from_subpath(s);
            for (j, o) in subs.iter().enumerate() {
                if owners[j] == Some(i) {
                    p.add_path(&Path::from_subpath(o));
                }
            }
            out.push(p);
        }
        out
    }
    fn from_subpath(s: &Subpath) -> Path {
        let mut p = Path::new();
        p.add_lines(&s.points);
        if s.closed {
            p.close();
        }
        p
    }
}

fn poly_area(p: &[Point]) -> f32 {
    let n = p.len();
    (0..n).map(|i| p[i].0 * p[(i + 1) % n].1 - p[(i + 1) % n].0 * p[i].1).sum::<f32>() / 2.0
}

fn cross(a: Point, b: Point, p: Point) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1)
}

fn segments_cross(p: Point, q: Point, r: Point, s: Point) -> bool {
    let (d1, d2) = (cross(p, q, r), cross(p, q, s));
    let (d3, d4) = (cross(r, s, p), cross(r, s, q));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) && d1 != 0.0 && d2 != 0.0 && d3 != 0.0 && d4 != 0.0
}

fn edges(s: &Subpath) -> impl Iterator<Item = (Point, Point)> + '_ {
    let n = s.points.len();
    let open = s.points.windows(2).map(|w| (w[0], w[1]));
    let closing = (s.closed && n > 1).then(|| (s.points[n - 1], s.points[0]));
    open.chain(closing)
}

struct Bounds {
    min: Point,
    max: Point,
}
impl Default for Bounds {
    fn default() -> Self {
        Bounds { min: (f32::INFINITY, f32::INFINITY), max: (f32::NEG_INFINITY, f32::NEG_INFINITY) }
    }
}
impl Bounds {
    fn add(&mut self, p: Point) {
        self.min = (self.min.0.min(p.0), self.min.1.min(p.1));
        self.max = (self.max.0.max(p.0), self.max.1.max(p.1));
    }
    fn rect(&self) -> Option<Rect> {
        (self.min.0 <= self.max.0).then(|| Rect { x: self.min.0, y: self.min.1, w: self.max.0 - self.min.0, h: self.max.1 - self.min.1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.5
    }
    fn pnear(a: Point, b: Point) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn affine_basics() {
        assert!(Affine::IDENTITY.is_identity());
        assert!(pnear(Affine::translation(10.0, 5.0).apply(1.0, 1.0), (11.0, 6.0)));
        let m = Affine::scaling(2.0, 2.0).then(Affine::translation(1.0, 0.0));
        assert!(pnear(m.apply(1.0, 1.0), (3.0, 2.0)));
        assert!(pnear(Affine::rotation(std::f32::consts::FRAC_PI_2).apply(1.0, 0.0), (0.0, 1.0)));
        let s = Affine::scaling(2.0, 3.0).apply_size(Size { w: 1.0, h: 1.0 });
        assert_eq!((s.w, s.h), (2.0, 3.0));
        let t = Affine::IDENTITY.translated(5.0, 0.0).scaled(2.0, 2.0);
        assert!(pnear(t.apply(1.0, 0.0), (7.0, 0.0)), "scale first, then translate: {:?}", t.apply(1.0, 0.0));
    }

    #[test]
    fn affine_invert_and_rect() {
        let t = Affine::rotation(0.7).then(Affine::scaling(2.0, 3.0)).then(Affine::translation(5.0, -8.0));
        let inv = t.invert().unwrap();
        let (x, y) = t.apply(1.5, -2.5);
        assert!(pnear(inv.apply(x, y), (1.5, -2.5)));
        assert!(Affine::scaling(0.0, 1.0).invert().is_none());
        let b = Affine::rotation(std::f32::consts::FRAC_PI_2).apply_rect(Rect::new(0.0, 0.0, 10.0, 4.0));
        assert!(near(b.w, 4.0) && near(b.h, 10.0));
    }

    #[test]
    fn decompose_round_trips() {
        let t = Affine::scaling(2.0, 3.0).then(Affine::rotation(0.4)).then(Affine::translation(7.0, 9.0));
        let d = t.decompose();
        assert!((d.scale.0 - 2.0).abs() < 1e-3 && (d.scale.1 - 3.0).abs() < 1e-3 && (d.rotation - 0.4).abs() < 1e-3);
        let back = Affine::from_decomposition(&d);
        assert!(pnear(back.apply(3.0, 4.0), t.apply(3.0, 4.0)));
        let skew = Affine::new(1.0, 0.0, 0.5, 1.0, 0.0, 0.0);
        assert!(pnear(Affine::from_decomposition(&skew.decompose()).apply(2.0, 2.0), skew.apply(2.0, 2.0)));
    }

    #[test]
    fn rect_path_flattens_closed_and_is_rect() {
        let p = Path::from_rect(R);
        let lines = p.flatten(1.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 5);
        assert_eq!(p.bounds(), Some(R));
        assert_eq!(p.is_rect(), Some(R));
        assert_eq!(Path::from_ellipse(R).is_rect(), None);
    }

    #[test]
    fn ellipse_and_rounded_rect_bounds() {
        let b = Path::from_ellipse(R).bounds().unwrap();
        assert!(near(b.x, R.x) && near(b.w, R.w) && near(b.h, R.h));
        let b = Path::from_rounded_rect(R, 10.0, 10.0).bounds().unwrap();
        assert!(near(b.w, R.w) && near(b.h, R.h));
        assert!(Path::from_rounded_rect(R, 1000.0, 1000.0).bounds().is_some());
    }

    #[test]
    fn curve_bounds_use_extrema_control_bounds_use_points() {
        let mut p = Path::new();
        p.move_to((0.0, 0.0));
        p.quad_to((5.0, 10.0), (10.0, 0.0));
        let b = p.bounds().unwrap();
        assert!(near(b.h, 5.0), "{b:?}");
        assert!(near(p.control_bounds().unwrap().h, 10.0));
    }

    #[test]
    fn arcs() {
        let mut p = Path::new();
        p.add_arc((0.0, 0.0), 10.0, 0.0, std::f32::consts::PI, false);
        let b = p.bounds().unwrap();
        assert!(near(b.y, 0.0) && near(b.h, 10.0) && near(b.w, 20.0), "{b:?}");
        assert!(pnear(p.current_point().unwrap(), (-10.0, 0.0)));
        let mut q = Path::new();
        q.add_arc((0.0, 0.0), 10.0, 0.0, std::f32::consts::FRAC_PI_2, true);
        // decreasing from 0 to pi/2 goes the long way round (three quarters)
        assert!(near(q.bounds().unwrap().h, 20.0));
        let mut r = Path::new();
        r.add_relative_arc((0.0, 0.0), 5.0, 0.0, TAU);
        assert!(near(r.bounds().unwrap().w, 10.0));
    }

    #[test]
    fn arc_to_rounds_a_corner() {
        let mut p = Path::new();
        p.move_to((0.0, 0.0));
        p.add_arc_to((10.0, 0.0), (10.0, 10.0), 2.0);
        p.line_to((10.0, 10.0));
        let b = p.bounds().unwrap();
        assert!(near(b.w, 10.0) && near(b.h, 10.0));
        // the sharp corner itself is cut off
        assert!(p.flatten(0.05)[0].iter().all(|q| dist(*q, (10.0, 0.0)) > 0.3));
    }

    #[test]
    fn transform_and_equality() {
        let p = Path::from_rect(R);
        let moved = p.transformed(&Affine::translation(5.0, -5.0));
        assert_eq!(moved.bounds(), Some(Rect { x: 15.0, y: 15.0, ..R }));
        assert_eq!(p, p.clone());
        assert_ne!(p, moved);
        let mut q = p.clone();
        q.transform(&Affine::IDENTITY);
        assert_eq!(p, q);
    }

    #[test]
    fn contains_follows_the_rule() {
        let mut p = Path::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        p.add_rect(Rect::new(2.0, 2.0, 6.0, 6.0));
        assert!(p.contains((5.0, 5.0), FillRule::NonZero));
        assert!(!p.contains((5.0, 5.0), FillRule::EvenOdd));
        assert!(p.contains((1.0, 1.0), FillRule::EvenOdd));
        assert!(!p.contains((11.0, 1.0), FillRule::NonZero));
    }

    #[test]
    fn empty_and_lone_move() {
        let e = Path::new();
        assert!(e.is_empty() && e.bounds().is_none() && e.flatten(1.0).is_empty());
        let mut l = Path::new();
        l.move_to((4.0, 4.0));
        assert!(l.flatten(1.0).is_empty());
        assert_eq!(l.current_point(), Some((4.0, 4.0)));
    }

    #[test]
    fn line_after_close_starts_at_the_subpath_start() {
        let mut p = Path::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.close();
        p.line_to((10.0, 10.0));
        let l = p.flatten(1.0);
        assert_eq!(l.len(), 2);
        assert_eq!(l[1], vec![(0.0, 0.0), (10.0, 10.0)]);
    }

    #[test]
    fn length_trim_and_dash() {
        let p = Path::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(near(p.length(0.1), 40.0));
        let half = p.trimmed(0.0, 0.5, 0.1);
        assert!(near(half.length(0.1), 20.0));
        let mut line = Path::new();
        line.add_lines(&[(0.0, 0.0), (10.0, 0.0)]);
        let d = line.copy_by_dashing(&[4.0, 2.0], 0.0, 0.1);
        assert_eq!(d.flatten(0.1), vec![vec![(0.0, 0.0), (4.0, 0.0)], vec![(6.0, 0.0), (10.0, 0.0)]]);
    }

    #[test]
    fn intersection_and_components() {
        let a = Path::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = Path::from_rect(Rect::new(5.0, 5.0, 10.0, 10.0));
        let far = Path::from_rect(Rect::new(50.0, 50.0, 1.0, 1.0));
        let inner = Path::from_rect(Rect::new(2.0, 2.0, 2.0, 2.0));
        assert!(a.intersects(&b, FillRule::NonZero));
        assert!(!a.intersects(&far, FillRule::NonZero));
        assert!(a.intersects(&inner, FillRule::NonZero));
        let mut two = a.clone();
        two.add_rect(Rect::new(3.0, 3.0, 2.0, 2.0));
        two.add_path(&far);
        assert_eq!(two.separate_components().len(), 2);
    }

    #[test]
    fn add_path_and_flattened_copy() {
        let mut p = Path::new();
        p.add_path(&Path::from_ellipse(R));
        let f = p.flattened(0.2);
        assert!(f.iter().all(|e| matches!(e, Element::Move(_) | Element::Line(_) | Element::Close)));
        assert!(near(f.bounds().unwrap().w, R.w));
    }
}
