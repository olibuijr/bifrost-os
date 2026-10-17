//! Block-matching stereo disparity between two rectified 8-bit planes, as plain std code. For each
//! pixel of the left plane a square window (odd `block` size) is compared with windows shifted
//! left by 0..=`max_disparity` pixels in the right plane, by sum of absolute differences (SAD). The
//! shift with the smallest SAD wins; ties go to the smaller shift. Pixels whose window does not fit
//! in either plane keep disparity 0.

use crate::filter::Plane;

/// A disparity map, row-major, one value per left pixel. `None` when the planes differ in size or
/// `block` is zero or even.
pub fn disparity(
    left: &Plane,
    right: &Plane,
    block: usize,
    max_disparity: usize,
) -> Option<Vec<u16>> {
    if left.width != right.width || left.height != right.height || block == 0 || block % 2 == 0 {
        return None;
    }
    let (w, h) = (left.width, left.height);
    let half = block / 2;
    let mut out = vec![0u16; w * h];
    if w < block || h < block {
        return Some(out);
    }
    for y in half..h - half {
        for x in half..w - half {
            let mut best: Option<(u32, usize)> = None;
            for d in 0..=max_disparity {
                // The right window starts at x - d - half, which must not underflow.
                if x < d + half {
                    break;
                }
                let mut sad = 0u32;
                for dy in 0..block {
                    let row = (y + dy - half) * w;
                    for dx in 0..block {
                        let lx = x + dx - half;
                        let rx = lx - d;
                        let a = left.pixels[row + lx] as i32;
                        let b = right.pixels[row + rx] as i32;
                        sad += (a - b).unsigned_abs();
                    }
                }
                if best.map_or(true, |(b, _)| sad < b) {
                    best = Some((sad, d));
                }
            }
            if let Some((_, d)) = best {
                out[y * w + x] = d as u16;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(w: usize, h: usize) -> Plane {
        let pixels = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                ((x * 37 + y * 91 + x * y * 7) % 251) as u8
            })
            .collect();
        Plane {
            width: w,
            height: h,
            pixels,
        }
    }

    #[test]
    fn a_shifted_scene_has_the_shift_as_disparity() {
        let (w, h) = (40, 12);
        let left = texture(w, h);
        // The right view sees the scene 3 pixels to the left: right(x) = left(x + 3).
        let mut pixels = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w - 3 {
                pixels[y * w + x] = left.pixels[y * w + x + 3];
            }
        }
        let right = Plane {
            width: w,
            height: h,
            pixels,
        };
        let map = disparity(&left, &right, 5, 6).unwrap();
        for y in 2..h - 2 {
            for x in 6..w - 6 {
                assert_eq!(map[y * w + x], 3, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn bad_arguments_are_refused() {
        let p = texture(8, 8);
        assert!(disparity(&p, &p, 4, 2).is_none());
        assert!(disparity(&p, &p, 0, 2).is_none());
        let q = texture(8, 4);
        assert!(disparity(&p, &q, 3, 2).is_none());
    }
}
