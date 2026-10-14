//! Affine geometry for images: the affine transform filter, the crop filter, and the resampling that
//! moves pixels through a transform. A transform maps a point `(x, y)` to
//! `(a * x + c * y + e, b * x + d * y + f)`, with the same six entries as the reference affine matrix.
//! Coordinates are in pixels with the origin at the top left, as elsewhere in
//! the render crate, and the image is resampled bilinearly. Positions outside the source read
//! transparent black.
use crate::distortion_filters::warp;
use crate::gfx::Image;

/// An affine transform with the matrix `[a c e; b d f]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    /// The transform that leaves every point where it is.
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    /// A move by `(tx, ty)` pixels.
    pub fn translation(tx: f32, ty: f32) -> Affine {
        Affine { e: tx, f: ty, ..Affine::IDENTITY }
    }

    /// A scale about the origin by `(sx, sy)`.
    pub fn scale(sx: f32, sy: f32) -> Affine {
        Affine { a: sx, d: sy, ..Affine::IDENTITY }
    }

    /// A rotation about the origin by `radians`. A positive angle turns the y axis towards the x
    /// axis, which is clockwise on screen where y points down.
    pub fn rotation(radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// The transform that applies `self` first and then `next`.
    pub fn then(self, next: Affine) -> Affine {
        Affine {
            a: self.a * next.a + self.b * next.c,
            b: self.a * next.b + self.b * next.d,
            c: self.c * next.a + self.d * next.c,
            d: self.c * next.b + self.d * next.d,
            e: self.e * next.a + self.f * next.c + next.e,
            f: self.e * next.b + self.f * next.d + next.f,
        }
    }

    /// Maps a point through the transform.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// The transform that undoes this one. `None` when the matrix is singular or not finite.
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < f32::EPSILON {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        let inv = Affine { a, b, c, d, e: 0.0, f: 0.0 };
        let (e, f) = inv.apply(-self.e, -self.f);
        Some(Affine { e, f, ..inv })
    }
}

/// Resamples `img` through `t`: each output pixel takes the source pixel that `t` maps it from.
/// The output has the size of the input, so a transform that moves content off the image cuts it
/// off. `None` when `t` has no inverse.
pub fn transform(img: &Image, t: Affine) -> Option<Image> {
    let inv = t.inverse()?;
    Some(warp(img, |x, y| inv.apply(x, y)))
}

/// The part of `img` inside the rectangle at `(x, y)` with size `w` by `h`, as a new image. Pixels
/// of the rectangle that lie outside the source are transparent. A rectangle with no area gives an
/// empty image.
pub fn crop(img: &Image, x: i64, y: i64, w: usize, h: usize) -> Image {
    let mut out = Image::new(w, h);
    for row in 0..h {
        let sy = y + row as i64;
        if sy < 0 || sy >= img.h as i64 {
            continue;
        }
        for col in 0..w {
            let sx = x + col as i64;
            if sx >= 0 && sx < img.w as i64 {
                out.px[row * w + col] = img.px[sy as usize * img.w + sx as usize];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: usize, h: usize) -> Image {
        let mut i = Image::new(w, h);
        for (k, p) in i.px.iter_mut().enumerate() {
            *p = 0xff00_0000 | (k as u32 & 0xff);
        }
        i
    }

    #[test]
    fn identity_transform_keeps_every_pixel() {
        let i = ramp(5, 4);
        assert_eq!(transform(&i, Affine::IDENTITY).unwrap().px, i.px);
    }

    #[test]
    fn then_applies_the_first_transform_first() {
        let t = Affine::translation(10.0, 0.0).then(Affine::scale(2.0, 2.0));
        assert_eq!(t.apply(1.0, 1.0), (22.0, 2.0));
    }

    #[test]
    fn a_quarter_turn_maps_the_x_axis_onto_the_y_axis() {
        let (x, y) = Affine::rotation(std::f32::consts::FRAC_PI_2).apply(1.0, 0.0);
        assert!(x.abs() < 1e-6);
        assert!((y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn inverse_undoes_the_transform() {
        let t = Affine::rotation(0.3).then(Affine::translation(4.0, -2.0)).then(Affine::scale(1.5, 0.5));
        let (x, y) = t.apply(7.0, 3.0);
        let (bx, by) = t.inverse().unwrap().apply(x, y);
        assert!((bx - 7.0).abs() < 1e-4);
        assert!((by - 3.0).abs() < 1e-4);
    }

    #[test]
    fn a_singular_transform_has_no_inverse() {
        assert_eq!(Affine::scale(0.0, 1.0).inverse(), None);
        assert!(transform(&ramp(2, 2), Affine::scale(0.0, 1.0)).is_none());
    }

    #[test]
    fn translation_moves_content_and_leaves_an_empty_edge() {
        let i = ramp(4, 1);
        let o = transform(&i, Affine::translation(1.0, 0.0)).unwrap();
        assert_eq!(o.px[0], 0);
        assert_eq!(o.px[1], i.px[0]);
        assert_eq!(o.px[3], i.px[2]);
    }

    #[test]
    fn crop_copies_the_rectangle_and_pads_outside_pixels_with_transparency() {
        let i = ramp(3, 3);
        let o = crop(&i, 2, 2, 2, 2);
        assert_eq!(o.px, vec![i.px[8], 0, 0, 0]);
    }

    #[test]
    fn crop_with_no_area_is_empty() {
        let o = crop(&ramp(3, 3), 0, 0, 0, 5);
        assert_eq!((o.w, o.h, o.px.len()), (0, 5, 0));
    }
}
