//! Element-wise arithmetic and transposition of 8-bit planes. Each pixel is
//! read as a fraction of 255, the operation is applied to the fractions, and
//! the result is rounded to the nearest 8-bit value and clamped to 0..=255.
//! Two planes must have the same size. Plain std code over `filter::Plane`.
//!
//! Pixel conversion covers two more operations: `invert` and `convert_transfer`, which maps codes between linear light
//! and the sRGB transfer curve (IEC 61966-2-1). Colour matrices and planar-to-interleaved packing are not modelled.

use crate::filter::Plane;

/// The transfer function that relates an 8-bit code to light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    /// The code is proportional to light.
    Linear,
    /// The code is sRGB encoded.
    Srgb,
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Converts every pixel of `src` from one transfer function to another, rounding each result to an 8-bit code.
/// A round trip through the two curves can move a code by one step because each conversion rounds once.
pub fn convert_transfer(src: &Plane, from: Transfer, to: Transfer) -> Plane {
    let pixels = src
        .pixels
        .iter()
        .map(|&code| {
            let v = code as f32 / 255.0;
            let light = match from {
                Transfer::Linear => v,
                Transfer::Srgb => srgb_to_linear(v),
            };
            let encoded = match to {
                Transfer::Linear => light,
                Transfer::Srgb => linear_to_srgb(light),
            };
            (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect();
    Plane { width: src.width, height: src.height, pixels }
}

/// The photographic negative of `src`: each code `v` becomes `255 - v`.
pub fn invert(src: &Plane) -> Plane {
    let pixels = src.pixels.iter().map(|&v| 255 - v).collect();
    Plane { width: src.width, height: src.height, pixels }
}

/// The operation applied to each pair of pixels `(a, b)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `a + b`, clamped at 255.
    Add,
    /// `a - b`, clamped at 0.
    Subtract,
    /// `a * b`.
    Multiply,
    /// `a / b`. A zero divisor gives 1 (255), whatever the dividend.
    Divide,
}

/// Applies `op` to one pair of 8-bit values.
fn apply(op: Op, a: u8, b: u8) -> u8 {
    let (x, y) = (a as u32, b as u32);
    let v = match op {
        Op::Add => x + y,
        Op::Subtract => x.saturating_sub(y),
        // round(x * y / 255), computed as floor((2xy + 255) / 510).
        Op::Multiply => (2 * x * y + 255) / 510,
        // round(255 * x / y), computed as floor((2 * 255x + y) / 2y).
        Op::Divide => {
            if y == 0 {
                255
            } else {
                (2 * 255 * x + y) / (2 * y)
            }
        }
    };
    v.min(255) as u8
}

/// Combines two planes pixel by pixel. Returns `None` when the sizes differ.
pub fn combine(a: &Plane, b: &Plane, op: Op) -> Option<Plane> {
    if a.width != b.width || a.height != b.height {
        return None;
    }
    let pixels = a
        .pixels
        .iter()
        .zip(&b.pixels)
        .map(|(&x, &y)| apply(op, x, y))
        .collect();
    Some(Plane { width: a.width, height: a.height, pixels })
}

/// The transpose: pixel (x, y) of `src` moves to (y, x). The result is
/// `height` wide and `width` tall.
pub fn transpose(src: &Plane) -> Plane {
    let (w, h) = (src.width, src.height);
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            pixels[x * h + y] = src.pixels[y * w + x];
        }
    }
    Plane { width: h, height: w, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_saturates_at_the_top_of_the_range() {
        let a = Plane::new(3, 1, vec![100, 200, 255]).unwrap();
        let b = Plane::new(3, 1, vec![100, 100, 1]).unwrap();
        assert_eq!(combine(&a, &b, Op::Add).unwrap().pixels, vec![200, 255, 255]);
    }

    #[test]
    fn subtract_stops_at_zero() {
        let a = Plane::new(2, 1, vec![10, 200]).unwrap();
        let b = Plane::new(2, 1, vec![20, 50]).unwrap();
        assert_eq!(combine(&a, &b, Op::Subtract).unwrap().pixels, vec![0, 150]);
    }

    #[test]
    fn multiply_works_on_fractions_of_255() {
        let a = Plane::new(3, 1, vec![255, 128, 0]).unwrap();
        let b = Plane::new(3, 1, vec![255, 128, 200]).unwrap();
        // 1 * 1 = 1, 0.5 * 0.5 = 0.25 (63.75, rounds to 64), 0 * x = 0.
        assert_eq!(combine(&a, &b, Op::Multiply).unwrap().pixels, vec![255, 64, 0]);
    }

    #[test]
    fn divide_gives_the_quotient_and_saturates_on_zero() {
        let a = Plane::new(3, 1, vec![100, 0, 255]).unwrap();
        let b = Plane::new(3, 1, vec![200, 0, 255]).unwrap();
        // 100 / 200 = 0.5 -> 127.5 -> 128. 0 / 0 is a zero divisor -> 255. 1 / 1 -> 255.
        assert_eq!(combine(&a, &b, Op::Divide).unwrap().pixels, vec![128, 255, 255]);
        let big = Plane::new(1, 1, vec![255]).unwrap();
        let one = Plane::new(1, 1, vec![1]).unwrap();
        assert_eq!(combine(&big, &one, Op::Divide).unwrap().pixels, vec![255]);
    }

    #[test]
    fn combine_rejects_planes_of_different_sizes() {
        let a = Plane::filled(2, 2, 1);
        let b = Plane::filled(2, 3, 1);
        assert!(combine(&a, &b, Op::Add).is_none());
    }

    #[test]
    fn transpose_swaps_rows_and_columns() {
        // [1 2 3]      [1 4]
        // [4 5 6]  ->  [2 5]
        //              [3 6]
        let src = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap();
        let t = transpose(&src);
        assert_eq!((t.width, t.height), (2, 3));
        assert_eq!(t.pixels, vec![1, 4, 2, 5, 3, 6]);
    }

    #[test]
    fn transpose_twice_is_the_identity() {
        let src = Plane::new(4, 2, vec![9, 8, 7, 6, 5, 4, 3, 2]).unwrap();
        assert_eq!(transpose(&transpose(&src)), src);
    }
}
