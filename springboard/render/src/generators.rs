//! Image generators: patterns that need no source image. Checkerboard and stripes take two packed
//! colours; the gradients take a `gradient::Ramp` and evaluate it per pixel. Generated pixels are
//! opaque, except where a pattern colour is given with its own alpha. Pixels are packed like
//! `gfx::Image` (red in the low byte).
use crate::colorfx::Rgb;
use crate::gfx::Image;
use crate::gradient::{linear_t, radial_t, Ramp};

fn opaque(c: Rgb) -> u32 {
    let c = c.clamped();
    let q = |v: f32| (v * 255.0 + 0.5) as u32;
    q(c.r) | q(c.g) << 8 | q(c.b) << 16 | 0xff00_0000
}

/// A checkerboard of `square` by `square` pixel cells, starting with colour `a` in the top left cell.
/// `None` when `square` is zero.
pub fn checkerboard(w: usize, h: usize, square: usize, a: u32, b: u32) -> Option<Image> {
    if square == 0 {
        return None;
    }
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let even = (x / square + y / square) % 2 == 0;
            out.px[y * w + x] = if even { a } else { b };
        }
    }
    Some(out)
}

/// Stripes of colour `a` and `b`, each `width` pixels across, with the stripe direction turned by
/// `angle` radians from the vertical. The edges are hard. `None` when `width` is not a positive
/// finite number or `angle` is not finite.
pub fn stripes(w: usize, h: usize, width: f32, angle: f32, a: u32, b: u32) -> Option<Image> {
    if !width.is_finite() || width <= 0.0 || !angle.is_finite() {
        return None;
    }
    let (s, c) = angle.sin_cos();
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let across = (x as f32 + 0.5) * c + (y as f32 + 0.5) * s;
            let index = (across / width).floor() as i64;
            out.px[y * w + x] = if index.rem_euclid(2) == 0 { a } else { b };
        }
    }
    Some(out)
}

/// A linear gradient from point `from` (offset 0) to point `to` (offset 1), coordinates in pixels.
/// Points beyond either end take the end colour.
pub fn linear_gradient(w: usize, h: usize, from: (f32, f32), to: (f32, f32), ramp: &Ramp) -> Image {
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = linear_t(from, to, x as f32 + 0.5, y as f32 + 0.5);
            out.px[y * w + x] = opaque(ramp.color_at(t));
        }
    }
    out
}

/// A radial gradient: offset 0 at `center`, offset 1 at distance `radius`, coordinates in pixels.
/// Pixels beyond the radius take the last colour of the ramp.
pub fn radial_gradient(w: usize, h: usize, center: (f32, f32), radius: f32, ramp: &Ramp) -> Image {
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = radial_t(center, radius, x as f32 + 0.5, y as f32 + 0.5);
            out.px[y * w + x] = opaque(ramp.color_at(t));
        }
    }
    out
}

/// An integer hash with good bit mixing (a 32-bit finaliser). It is a bijection on `u32`.
fn mix32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// `CIRandomGenerator`: opaque noise with each channel drawn independently per pixel. The same `seed`
/// always gives the same image, and different seeds give different ones.
pub fn random(w: usize, h: usize, seed: u32) -> Image {
    let mut out = Image::new(w, h);
    for (i, px) in out.px.iter_mut().enumerate() {
        let key = seed ^ (i as u32).wrapping_mul(0x9e37_79b9);
        let r = mix32(key) & 0xff;
        let g = mix32(key ^ 0x68bc_21eb) & 0xff;
        let b = mix32(key ^ 0x02e5_be93) & 0xff;
        *px = r | g << 8 | b << 16 | 0xff00_0000;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkerboard_alternates_cells_from_the_top_left() {
        let img = checkerboard(4, 2, 2, 0xff00_0001, 0xff00_0002).unwrap();
        assert_eq!(
            img.px,
            vec![0xff00_0001, 0xff00_0001, 0xff00_0002, 0xff00_0002, 0xff00_0001, 0xff00_0001, 0xff00_0002, 0xff00_0002]
        );
    }

    #[test]
    fn checkerboard_needs_a_positive_cell_size() {
        assert!(checkerboard(4, 4, 0, 1, 2).is_none());
    }

    #[test]
    fn stripes_with_no_angle_run_down_the_columns() {
        let img = stripes(4, 1, 2.0, 0.0, 10, 20).unwrap();
        assert_eq!(img.px, vec![10, 10, 20, 20]);
    }

    #[test]
    fn stripes_reject_a_zero_width_or_an_infinite_angle() {
        assert!(stripes(4, 4, 0.0, 0.0, 1, 2).is_none());
        assert!(stripes(4, 4, 2.0, f32::INFINITY, 1, 2).is_none());
    }

    #[test]
    fn a_turned_stripe_changes_along_rows_instead_of_columns() {
        let img = stripes(1, 4, 2.0, std::f32::consts::FRAC_PI_2, 10, 20).unwrap();
        assert_eq!(img.px, vec![10, 10, 20, 20]);
    }

    fn black_to_white() -> Ramp {
        Ramp::new(&[(0.0, Rgb { r: 0.0, g: 0.0, b: 0.0 }), (1.0, Rgb { r: 1.0, g: 1.0, b: 1.0 })]).unwrap()
    }

    #[test]
    fn linear_gradient_runs_from_the_start_point_to_the_end_point() {
        let img = linear_gradient(4, 1, (0.0, 0.0), (4.0, 0.0), &black_to_white());
        let (first, last) = (img.px[0] & 0xff, img.px[3] & 0xff);
        assert!(first > 0 && first < 0x40, "the first pixel is near black, got {first:#x}");
        assert!(last > 0xc0 && last < 0xff, "the last pixel is near white, got {last:#x}");
        assert_eq!(img.px[0] >> 24, 0xff, "generated pixels are opaque");
    }

    #[test]
    fn radial_gradient_is_black_at_the_centre_and_white_beyond_the_radius() {
        let img = radial_gradient(9, 9, (4.5, 4.5), 4.0, &black_to_white());
        assert_eq!(img.px[4 * 9 + 4], 0xff00_0000);
        assert_eq!(img.px[0], 0xffff_ffff);
    }

    #[test]
    fn random_is_deterministic_per_seed_and_opaque() {
        let a = random(8, 8, 7);
        assert_eq!(a, random(8, 8, 7));
        assert_ne!(a, random(8, 8, 8));
        assert!(a.px.iter().all(|p| p >> 24 == 0xff));
        let distinct = a.px.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(distinct > 32, "noise should not repeat a few values, got {distinct}");
    }
}
