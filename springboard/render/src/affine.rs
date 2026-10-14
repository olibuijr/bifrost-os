//! Two-dimensional affine transforms for app drawing (`CGAffineTransform`) and the current transform stack of a drawing
//! context (`CGContextSaveGState`, `CGContextConcatCTM`). Points map as x' = a*x + c*y + tx and y' = b*x + d*y + ty.
//! Pure arithmetic: no device and no display.

/// A 2x3 affine matrix.
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
    /// The transform that leaves every point where it is.
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    /// A move by (`tx`, `ty`).
    pub const fn translate(tx: f32, ty: f32) -> Affine {
        Affine { tx, ty, ..Affine::IDENTITY }
    }

    /// A scale by (`sx`, `sy`) about the origin.
    pub const fn scale(sx: f32, sy: f32) -> Affine {
        Affine { a: sx, d: sy, ..Affine::IDENTITY }
    }

    /// A rotation by `radians` about the origin, counter-clockwise on a y-up surface.
    pub fn rotate(radians: f32) -> Affine {
        let (sin, cos) = radians.sin_cos();
        Affine { a: cos, b: sin, c: -sin, d: cos, tx: 0.0, ty: 0.0 }
    }

    /// The transform that applies `self` first and `next` second.
    pub fn then(self, next: Affine) -> Affine {
        Affine {
            a: self.a * next.a + self.b * next.c,
            b: self.a * next.b + self.b * next.d,
            c: self.c * next.a + self.d * next.c,
            d: self.c * next.b + self.d * next.d,
            tx: self.tx * next.a + self.ty * next.c + next.tx,
            ty: self.tx * next.b + self.ty * next.d + next.ty,
        }
    }

    /// Maps a point through the transform.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    /// The transform that undoes `self`. `None` when `self` collapses the plane onto a line or a point.
    pub fn invert(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det == 0.0 {
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
}

/// The current transform of a drawing context with its saved states. The base state is the identity.
#[derive(Clone, Debug, PartialEq)]
pub struct TransformStack {
    current: Affine,
    saved: Vec<Affine>,
}

impl Default for TransformStack {
    fn default() -> Self {
        TransformStack { current: Affine::IDENTITY, saved: Vec::new() }
    }
}

impl TransformStack {
    /// A stack at the identity transform with no saved states.
    pub fn new() -> Self {
        Self::default()
    }

    /// The transform in force now.
    pub fn current(&self) -> Affine {
        self.current
    }

    /// Concatenates `transform` onto the current one: points are mapped by `transform` before the current transform.
    pub fn concat(&mut self, transform: Affine) {
        self.current = transform.then(self.current);
    }

    /// Saves the current transform.
    pub fn save(&mut self) {
        self.saved.push(self.current);
    }

    /// Restores the most recently saved transform. Returns `false` when nothing is saved.
    pub fn restore(&mut self) -> bool {
        match self.saved.pop() {
            Some(transform) => {
                self.current = transform;
                true
            }
            None => false,
        }
    }

    /// The number of saved states.
    pub fn depth(&self) -> usize {
        self.saved.len()
    }
}
