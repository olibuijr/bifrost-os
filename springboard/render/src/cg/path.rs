//! STUB (cg-context branch): the signatures the context codes against. The cg-raster agent owns the real module;
//! drop this file on merge and adjust any call whose argument shape differs (all path building in the context is
//! in `context.rs` helpers `rect_path`, `ellipse_path` and the `Path` calls in `DrawContext::add_*`).
//! `Affine` is complete (it is used by value everywhere).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub fn translate(tx: f32, ty: f32) -> Self {
        Affine { tx, ty, ..Self::IDENTITY }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Affine { a: sx, d: sy, ..Self::IDENTITY }
    }

    pub fn rotate(rad: f32) -> Self {
        let (s, c) = rad.sin_cos();
        Affine { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 }
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

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    pub fn det(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    pub fn invert(&self) -> Option<Affine> {
        let det = self.det();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Affine { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) })
    }
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Clone, Debug, Default)]
pub struct Path {
    _stub: Vec<f32>,
}

impl Path {
    pub fn new() -> Self {
        Path::default()
    }
    pub fn is_empty(&self) -> bool {
        todo!()
    }
    pub fn move_to(&mut self, _x: f32, _y: f32) {
        todo!()
    }
    pub fn line_to(&mut self, _x: f32, _y: f32) {
        todo!()
    }
    pub fn quad_to(&mut self, _cx: f32, _cy: f32, _x: f32, _y: f32) {
        todo!()
    }
    pub fn cubic_to(&mut self, _c1x: f32, _c1y: f32, _c2x: f32, _c2y: f32, _x: f32, _y: f32) {
        todo!()
    }
    /// Arc of radius `r` about (cx, cy) from angle `a0` to `a1` (radians); a line joins it to the current point.
    pub fn arc(&mut self, _cx: f32, _cy: f32, _r: f32, _a0: f32, _a1: f32, _clockwise: bool) {
        todo!()
    }
    pub fn arc_to(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _r: f32) {
        todo!()
    }
    pub fn add_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {
        todo!()
    }
    pub fn add_ellipse(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {
        todo!()
    }
    pub fn close(&mut self) {
        todo!()
    }
    /// Appends all subpaths of `other`.
    pub fn append(&mut self, _other: &Path) {
        todo!()
    }
    pub fn transform(&self, _t: &Affine) -> Path {
        todo!()
    }
    /// (x, y, w, h) of the control-point box, None when empty.
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        todo!()
    }
    pub fn current_point(&self) -> Option<(f32, f32)> {
        todo!()
    }
    pub fn contains(&self, _x: f32, _y: f32, _rule: super::raster::FillRule) -> bool {
        todo!()
    }
}
