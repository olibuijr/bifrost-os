//! Perspective correction (`CIPerspectiveCorrection`, `CIPerspectiveTransform`).
//!
//! A quadrilateral in the source image (for example a photographed sheet of paper) is mapped onto an
//! upright rectangle. The mapping is a projective transform (a homography) solved from the four corner
//! pairs with 8 x 8 Gaussian elimination. The output is computed by inverse mapping: each output pixel
//! centre is sent back into the source and bilinearly sampled, and points outside the source are
//! transparent. Pixels are premultiplied RGBA packed with R in the low byte, as `gfx::Image` stores them.
//!
//! CPU only. Host unit tests check identity, scale and degenerate quads by hand.
use crate::gfx::Image;

/// A point in pixel coordinates. Pixel `i` covers `[i, i + 1)`, so its centre is `i + 0.5`.
pub type Point = (f32, f32);

/// A 3 x 3 projective transform in row-major order, with the last entry fixed at 1 by convention.
pub type Homography = [f64; 9];

/// Solves for the homography that maps each `src[i]` to `dst[i]`. `None` when the points are
/// degenerate (for example three of them collinear, or two equal), so no unique transform exists.
pub fn homography(src: [Point; 4], dst: [Point; 4]) -> Option<Homography> {
    // Rows of the 8 x 9 augmented system for h0..h7 (h8 = 1).
    let mut m = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (src[i].0 as f64, src[i].1 as f64);
        let (u, v) = (dst[i].0 as f64, dst[i].1 as f64);
        m[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        m[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    // Gaussian elimination with partial pivoting.
    for col in 0..8 {
        let pivot = (col..8).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[pivot][col].abs() < 1e-9 {
            return None;
        }
        m.swap(col, pivot);
        for row in 0..8 {
            if row != col {
                let f = m[row][col] / m[col][col];
                if f != 0.0 {
                    for k in col..9 {
                        m[row][k] -= f * m[col][k];
                    }
                }
            }
        }
    }
    let mut h = [0.0f64; 9];
    for i in 0..8 {
        h[i] = m[i][8] / m[i][i];
    }
    h[8] = 1.0;
    Some(h)
}

/// Applies a homography to a point. `None` when the point maps to infinity.
pub fn map_point(h: &Homography, p: Point) -> Option<Point> {
    let (x, y) = (p.0 as f64, p.1 as f64);
    let w = h[6] * x + h[7] * y + h[8];
    if w.abs() < 1e-12 {
        return None;
    }
    Some((((h[0] * x + h[1] * y + h[2]) / w) as f32, ((h[3] * x + h[4] * y + h[5]) / w) as f32))
}

/// Maps the quadrilateral `quad` (top-left, top-right, bottom-right, bottom-left in the source) onto an
/// upright `out_w` by `out_h` image. Returns `None` for an empty output, a degenerate quad, or a
/// source whose pixel count does not match its size.
pub fn perspective_correct(img: &Image, quad: [Point; 4], out_w: usize, out_h: usize) -> Option<Image> {
    if img.w.checked_mul(img.h)? != img.px.len() || out_w == 0 || out_h == 0 {
        return None;
    }
    let corners = [(0.0, 0.0), (out_w as f32, 0.0), (out_w as f32, out_h as f32), (0.0, out_h as f32)];
    // Inverse mapping: output pixel -> source point.
    let back = homography(corners, quad)?;
    let mut px = vec![0u32; out_w * out_h];
    for y in 0..out_h {
        for x in 0..out_w {
            let centre = (x as f32 + 0.5, y as f32 + 0.5);
            if let Some((sx, sy)) = map_point(&back, centre) {
                px[y * out_w + x] = bilinear(img, sx, sy);
            }
        }
    }
    Some(Image { w: out_w, h: out_h, px })
}

/// Bilinear sample at continuous source coordinates. Neighbours outside the image count as transparent,
/// so the edge fades out rather than repeating.
fn bilinear(img: &Image, sx: f32, sy: f32) -> u32 {
    let gx = sx - 0.5;
    let gy = sy - 0.5;
    let x0 = gx.floor();
    let y0 = gy.floor();
    let tx = gx - x0;
    let ty = gy - y0;
    let (x0, y0) = (x0 as i64, y0 as i64);
    let mut acc = [0.0f32; 4];
    for (dy, wy) in [(0i64, 1.0 - ty), (1, ty)] {
        for (dx, wx) in [(0i64, 1.0 - tx), (1, tx)] {
            let w = wx * wy;
            if w == 0.0 {
                continue;
            }
            let (x, y) = (x0 + dx, y0 + dy);
            if x < 0 || y < 0 || x >= img.w as i64 || y >= img.h as i64 {
                continue;
            }
            let p = img.px[y as usize * img.w + x as usize];
            for (c, a) in acc.iter_mut().enumerate() {
                *a += w * ((p >> (8 * c)) & 0xff) as f32;
            }
        }
    }
    let q = |v: f32| (v + 0.5).clamp(0.0, 255.0) as u32;
    q(acc[0]) | (q(acc[1]) << 8) | (q(acc[2]) << 16) | (q(acc[3]) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: usize, h: usize) -> Image {
        // Opaque pixels whose red channel is the x index and green is the y index.
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                px.push(x as u32 | ((y as u32) << 8) | (255 << 24));
            }
        }
        Image { w, h, px }
    }

    const UNIT: [Point; 4] = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];

    #[test]
    fn homography_of_identity_is_identity() {
        let h = homography(UNIT, UNIT).unwrap();
        let expect = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for (a, b) in h.iter().zip(expect.iter()) {
            assert!((a - b).abs() < 1e-9, "{h:?}");
        }
    }

    #[test]
    fn homography_maps_corners_and_scale() {
        // Unit square onto a square twice as big: (1,1) -> (2,2).
        let dst = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let h = homography([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], dst).unwrap();
        let p = map_point(&h, (1.0, 1.0)).unwrap();
        assert!((p.0 - 2.0).abs() < 1e-5 && (p.1 - 2.0).abs() < 1e-5);
    }

    #[test]
    fn degenerate_quad_is_rejected() {
        // Three collinear points on one edge have no unique projective map.
        let collinear = [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (0.0, 2.0)];
        assert!(homography(collinear, UNIT).is_none());
        assert!(homography([(1.0, 1.0); 4], UNIT).is_none());
    }

    #[test]
    fn identity_quad_reproduces_the_image() {
        let img = ramp(4, 4);
        let out = perspective_correct(&img, UNIT, 4, 4).unwrap();
        assert_eq!(out, img);
    }

    #[test]
    fn sub_quad_crops_the_top_left_corner() {
        // The top-left 2x2 of a 4x4 ramp, sampled 1:1 into a 2x2 output.
        let img = ramp(4, 4);
        let quad = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let out = perspective_correct(&img, quad, 2, 2).unwrap();
        assert_eq!(out.px, vec![img.px[0], img.px[1], img.px[4], img.px[5]]);
    }

    #[test]
    fn outside_the_source_is_transparent() {
        let img = ramp(4, 4);
        // A quad that reaches past the right edge: the far output column samples outside.
        let quad = [(0.0, 0.0), (8.0, 0.0), (8.0, 4.0), (0.0, 4.0)];
        let out = perspective_correct(&img, quad, 2, 1).unwrap();
        assert_eq!(out.px[1] >> 24, 0);
        assert_eq!(out.px[0] >> 24, 255);
    }

    #[test]
    fn rejects_bad_sizes() {
        let img = ramp(4, 4);
        assert!(perspective_correct(&img, UNIT, 0, 4).is_none());
        let bad = Image { w: 2, h: 2, px: vec![0; 3] };
        assert!(perspective_correct(&bad, UNIT, 2, 2).is_none());
    }
}
