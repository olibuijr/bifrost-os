//! Shape tiles with the parameters of the reference filters `CITriangleTile`, `CIParallelogramTile`
//! and `CIPerspectiveTile`. Each tile cuts the plane into cells and shows the image folded into each
//! cell. Pixels are premultiplied RGBA packed in a `u32` (red in the low byte), like `gfx::Image`,
//! and the output has the size of the input. Cell geometry and the mirroring are this model's own;
//! the reference cell shapes are not reproduced exactly.
use crate::distortion_filters::warp;
use crate::gfx::Image;

fn fin(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// Sample position inside the image for cell coordinates `fu`, `fv` in 0..=1, kept off the edge
/// so a tile never blends with the transparent outside.
fn to_pixel(img: &Image, fu: f32, fv: f32) -> (f32, f32) {
    let (iw, ih) = (img.w as f32, img.h as f32);
    ((fu * iw).clamp(0.5, iw - 0.5), (fv * ih).clamp(0.5, ih - 0.5))
}

/// `CITriangleTile`: the plane is cut into right triangles whose legs are `width` pixels, turned by
/// `angle` radians about `center`. Each cell is a square of `width` pixels split along its diagonal;
/// the image is shown in both triangles, the upper one reflected across the diagonal, so the two
/// halves of a cell mirror each other. A width that is not positive and finite returns a copy.
pub fn triangle_tile(img: &Image, center: (f32, f32), angle: f32, width: f32) -> Image {
    let w = fin(width, 0.0);
    if img.w == 0 || img.h == 0 || w <= 0.0 {
        return img.clone();
    }
    let (sn, cs) = fin(angle, 0.0).sin_cos();
    let (cx, cy) = (fin(center.0, 0.0), fin(center.1, 0.0));
    warp(img, |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let u = (dx * cs + dy * sn) / w;
        let v = (-dx * sn + dy * cs) / w;
        let (fu, fv) = (u - u.floor(), v - v.floor());
        let (fu, fv) = if fu + fv > 1.0 { (1.0 - fv, 1.0 - fu) } else { (fu, fv) };
        to_pixel(img, fu, fv)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, p: u32) -> Image {
        let mut i = Image::new(w, h);
        i.px.iter_mut().for_each(|x| *x = p);
        i
    }

    fn gradient(w: usize, h: usize) -> Image {
        let mut i = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                i.px[y * w + x] = (x as u32 * 17 & 0xff) | ((y as u32 * 31 & 0xff) << 8) | 0xff00_0000;
            }
        }
        i
    }

    fn near(a: u32, b: u32) -> bool {
        (0..4).all(|k| ((a >> (8 * k)) & 0xff).abs_diff((b >> (8 * k)) & 0xff) <= 2)
    }

    #[test]
    fn a_flat_image_stays_flat_under_the_triangle_tile() {
        let i = flat(9, 9, 0xff33_6699);
        let o = triangle_tile(&i, (4.0, 4.0), 0.3, 5.0);
        assert!(o.px.iter().all(|&p| p == 0xff33_6699));
    }

    #[test]
    fn a_zero_or_infinite_width_returns_the_input() {
        let i = gradient(6, 6);
        assert_eq!(triangle_tile(&i, (0.0, 0.0), 0.0, 0.0), i);
        assert_eq!(triangle_tile(&i, (0.0, 0.0), 0.0, f32::INFINITY), i);
    }

    #[test]
    fn the_pattern_repeats_every_width_along_the_axis() {
        // With no turn, shifting by one cell width (4 pixels) maps to the same cell coordinates.
        let i = gradient(12, 12);
        let o = triangle_tile(&i, (0.0, 0.0), 0.0, 4.0);
        for y in 0..12 {
            for x in 0..8 {
                assert!(near(o.px[y * 12 + x], o.px[y * 12 + x + 4]), "x {x} y {y}");
            }
        }
    }

    #[test]
    fn the_output_has_the_size_of_the_input() {
        let i = gradient(7, 5);
        let o = triangle_tile(&i, (3.0, 2.0), 1.1, 3.0);
        assert_eq!((o.w, o.h, o.px.len()), (7, 5, 35));
    }
}
