//! `CGGradient` and `CGAffineTransform` for `GraphicsContext`.

/// `CGAffineTransform`: `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`.
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

    pub fn translation(tx: f32, ty: f32) -> Affine {
        Affine { tx, ty, ..Affine::IDENTITY }
    }
    pub fn scale(sx: f32, sy: f32) -> Affine {
        Affine { a: sx, d: sy, ..Affine::IDENTITY }
    }
    pub fn rotation(radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        Affine { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 }
    }
    pub fn from_array(t: [f32; 6]) -> Affine {
        Affine { a: t[0], b: t[1], c: t[2], d: t[3], tx: t[4], ty: t[5] }
    }
    pub fn to_array(self) -> [f32; 6] {
        [self.a, self.b, self.c, self.d, self.tx, self.ty]
    }
    pub fn is_identity(&self) -> bool {
        *self == Affine::IDENTITY
    }
    /// `CGAffineTransformConcat(self, other)`: apply `self` first, then `other`.
    pub fn concatenating(&self, o: &Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            tx: self.tx * o.a + self.ty * o.c + o.tx,
            ty: self.tx * o.b + self.ty * o.d + o.ty,
        }
    }
    pub fn translated_by(&self, tx: f32, ty: f32) -> Affine {
        Affine::translation(tx, ty).concatenating(self)
    }
    pub fn scaled_by(&self, sx: f32, sy: f32) -> Affine {
        Affine::scale(sx, sy).concatenating(self)
    }
    pub fn rotated_by(&self, radians: f32) -> Affine {
        Affine::rotation(radians).concatenating(self)
    }
    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }
    /// `CGAffineTransformInvert`; `None` when singular.
    pub fn inverted(&self) -> Option<Affine> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Affine { a, b, c, d, tx: -(self.tx * a + self.ty * c), ty: -(self.tx * b + self.ty * d) })
    }
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
}

/// `CGGradientDrawingOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GradientOptions {
    pub draws_before_start: bool,
    pub draws_after_end: bool,
}

/// `CGGradient`: colour stops at locations in 0..=1. Colours are unpremultiplied RGBA (red low byte).
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    stops: Vec<(f32, u32)>,
}

impl Gradient {
    /// `None` when `stops` is empty. Locations are clamped to 0..=1 and sorted.
    pub fn new(stops: &[(f32, u32)]) -> Option<Gradient> {
        if stops.is_empty() {
            return None;
        }
        let mut v: Vec<(f32, u32)> = stops.iter().map(|&(l, c)| (l.clamp(0.0, 1.0), c)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Gradient { stops: v })
    }

    /// Evenly spaced colours, like `CGGradient` created without locations.
    pub fn evenly(colors: &[u32]) -> Option<Gradient> {
        let n = colors.len();
        let stops: Vec<(f32, u32)> =
            colors.iter().enumerate().map(|(i, &c)| (if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 }, c)).collect();
        Gradient::new(&stops)
    }

    pub fn color_at(&self, t: f32) -> u32 {
        let t = t.clamp(0.0, 1.0);
        let first = self.stops[0];
        if t <= first.0 {
            return first.1;
        }
        for w in self.stops.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t <= b.0 {
                let u = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
                let ch = |s: u32| {
                    let (x, y) = (((a.1 >> s) & 0xff) as f32, ((b.1 >> s) & 0xff) as f32);
                    ((x + (y - x) * u) + 0.5) as u32
                };
                return ch(0) | (ch(8) << 8) | (ch(16) << 16) | (ch(24) << 24);
            }
        }
        self.stops[self.stops.len() - 1].1
    }
}

/// Parameter `t` of a linear gradient from `s` to `e` at `p`, or `None` outside the allowed range.
pub fn linear_param(s: (f32, f32), e: (f32, f32), p: (f32, f32), o: GradientOptions) -> Option<f32> {
    let (dx, dy) = (e.0 - s.0, e.1 - s.1);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return None;
    }
    extend(((p.0 - s.0) * dx + (p.1 - s.1) * dy) / len2, o)
}

/// Parameter `t` of a two-circle radial gradient at `p`: the largest `t` whose circle (centre and radius
/// interpolated between the start and end circle, radius not negative) passes through `p`.
pub fn radial_param(c0: (f32, f32), r0: f32, c1: (f32, f32), r1: f32, p: (f32, f32), o: GradientOptions) -> Option<f32> {
    let (cdx, cdy) = (c1.0 - c0.0, c1.1 - c0.1);
    let (pdx, pdy) = (p.0 - c0.0, p.1 - c0.1);
    let dr = r1 - r0;
    let qa = cdx * cdx + cdy * cdy - dr * dr;
    let qb = pdx * cdx + pdy * cdy + r0 * dr;
    let qc = pdx * pdx + pdy * pdy - r0 * r0;
    let mut roots = [f32::NAN; 2];
    if qa.abs() < 1e-9 {
        if qb.abs() > 1e-9 {
            roots[0] = qc / (2.0 * qb);
        }
    } else {
        let disc = qb * qb - qa * qc;
        if disc >= 0.0 {
            let sq = disc.sqrt();
            roots = [(qb + sq) / qa, (qb - sq) / qa];
        }
    }
    if roots[0] < roots[1] {
        roots.swap(0, 1);
    }
    for t in roots {
        if t.is_nan() || r0 + t * dr < 0.0 {
            continue;
        }
        if let Some(t) = extend(t, o) {
            return Some(t);
        }
    }
    None
}

fn extend(t: f32, o: GradientOptions) -> Option<f32> {
    if t < 0.0 {
        o.draws_before_start.then_some(0.0)
    } else if t > 1.0 {
        o.draws_after_end.then_some(1.0)
    } else {
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: u32 = 0xff00_0000;
    const WHITE: u32 = 0xffff_ffff;

    #[test]
    fn transform_concat_applies_self_first() {
        let t = Affine::translation(10.0, 0.0).concatenating(&Affine::scale(2.0, 2.0));
        assert_eq!(t.apply(1.0, 1.0), (22.0, 2.0));
    }

    #[test]
    fn transform_inverse_round_trips() {
        let t = Affine::rotation(0.7).concatenating(&Affine::translation(3.0, -4.0)).scaled_by(2.0, 3.0);
        let inv = t.inverted().unwrap();
        let p = inv.apply(t.apply(5.0, 6.0).0, t.apply(5.0, 6.0).1);
        assert!((p.0 - 5.0).abs() < 1e-4 && (p.1 - 6.0).abs() < 1e-4);
        assert!(Affine::scale(0.0, 1.0).inverted().is_none());
    }

    #[test]
    fn gradient_interpolates_between_stops() {
        let g = Gradient::new(&[(0.0, BLACK), (1.0, WHITE)]).unwrap();
        assert_eq!(g.color_at(0.0), BLACK);
        assert_eq!(g.color_at(1.0), WHITE);
        assert_eq!(g.color_at(0.5) & 0xff, 128);
        assert!(Gradient::new(&[]).is_none());
    }

    #[test]
    fn linear_param_respects_extend_options() {
        let none = GradientOptions::default();
        assert_eq!(linear_param((0.0, 0.0), (10.0, 0.0), (5.0, 3.0), none), Some(0.5));
        assert_eq!(linear_param((0.0, 0.0), (10.0, 0.0), (-1.0, 0.0), none), None);
        let before = GradientOptions { draws_before_start: true, draws_after_end: false };
        assert_eq!(linear_param((0.0, 0.0), (10.0, 0.0), (-1.0, 0.0), before), Some(0.0));
    }

    #[test]
    fn radial_param_grows_from_centre() {
        let none = GradientOptions::default();
        let t = radial_param((5.0, 5.0), 0.0, (5.0, 5.0), 4.0, (7.0, 5.0), none).unwrap();
        assert!((t - 0.5).abs() < 1e-4);
        assert_eq!(radial_param((5.0, 5.0), 0.0, (5.0, 5.0), 4.0, (12.0, 5.0), none), None);
    }
}
