//! Portrait relighting from a depth map, as a CPU model. The surface normal at each pixel is taken
//! from the slope of the depth map, and the picture is brightened where the surface faces a
//! light placed to one side and darkened where it turns away. This is a Lambert model of this
//! crate's own, not the reference portrait lighting kernels, whose curves are not reproduced.
//! Pixels are premultiplied RGBA packed in a `u32` (red in the low byte), like `gfx::Image`.
use crate::colorfx::{self, Rgb};
use crate::gfx::Image;

/// How strongly a depth step tilts the normal: a larger value makes the same depth change look steeper.
const RELIEF: f32 = 8.0;

/// The height of the light above the picture plane. The light's xy direction is set by the caller.
const LIGHT_Z: f32 = 0.7;

/// Relights `img` from `depth`, one value per pixel (row-major, the same size as `img`; larger is
/// nearer). `light` is the direction of the light in the picture plane (x right, y down): a light at
/// (-1, 0) comes from the left. `strength` 0 leaves the picture unchanged and 1 applies the full
/// Lambert gain. Returns `None` when the depth map does not match the image size or `light` or
/// `strength` is not finite.
pub fn relight(img: &Image, depth: &[f32], light: (f32, f32), strength: f32) -> Option<Image> {
    let n = img.w.checked_mul(img.h)?;
    if n == 0 || depth.len() != n || img.px.len() != n {
        return None;
    }
    if !light.0.is_finite() || !light.1.is_finite() || !strength.is_finite() {
        return None;
    }
    let len = (light.0 * light.0 + light.1 * light.1 + LIGHT_Z * LIGHT_Z).sqrt();
    let l = (light.0 / len, light.1 / len, LIGHT_Z / len);
    let s = strength.clamp(0.0, 1.0);
    let (w, h) = (img.w, img.h);
    let d = |x: usize, y: usize| depth[y.min(h - 1) * w + x.min(w - 1)];
    let mut out = img.clone();
    for y in 0..h {
        for x in 0..w {
            // Central differences with the edge samples repeated at the border.
            let gx = (d((x + 1).min(w - 1), y) - d(x.saturating_sub(1), y)) * RELIEF;
            let gy = (d(x, (y + 1).min(h - 1)) - d(x, y.saturating_sub(1))) * RELIEF;
            let norm = (gx * gx + gy * gy + 1.0).sqrt();
            let (nx, ny, nz) = (-gx / norm, -gy / norm, 1.0 / norm);
            let dot = nx * l.0 + ny * l.1 + nz * l.2;
            // A flat face has dot = LIGHT_Z (normalised), so it gets gain 1.
            let gain = (1.0 + s * (dot / (LIGHT_Z / len) - 1.0)).clamp(0.0, 2.0);
            let i = y * w + x;
            out.px[i] = colorfx::map_pixel(img.px[i], |c| Rgb::new(c.r * gain, c.g * gain, c.b * gain));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(w: usize, h: usize) -> Image {
        let mut i = Image::new(w, h);
        i.px.iter_mut().for_each(|p| *p = 0xff80_8080);
        i
    }

    fn ramp(w: usize, h: usize) -> Vec<f32> {
        (0..w * h).map(|i| (i % w) as f32 / w as f32).collect()
    }

    #[test]
    fn a_flat_depth_map_leaves_the_picture_unchanged() {
        let img = grey(6, 4);
        let depth = vec![0.5; 24];
        assert_eq!(relight(&img, &depth, (-1.0, 0.0), 1.0).unwrap(), img);
    }

    #[test]
    fn zero_strength_leaves_the_picture_unchanged() {
        let img = grey(8, 4);
        assert_eq!(relight(&img, &ramp(8, 4), (-1.0, 0.0), 0.0).unwrap(), img);
    }

    #[test]
    fn a_slope_facing_the_light_is_brighter_than_one_turned_away() {
        // Depth rises to the right, so the surface faces left: a light from the left brightens it.
        let img = grey(16, 4);
        let depth = ramp(16, 4);
        let lit = relight(&img, &depth, (-1.0, 0.0), 1.0).unwrap();
        let shaded = relight(&img, &depth, (1.0, 0.0), 1.0).unwrap();
        let i = 2 * 16 + 8;
        assert!(lit.px[i] & 0xff > 0x80, "lit side should be brighter");
        assert!(shaded.px[i] & 0xff < 0x80, "turned-away side should be darker");
    }

    #[test]
    fn mismatched_depth_or_non_finite_inputs_are_refused() {
        let img = grey(4, 4);
        assert!(relight(&img, &[0.0; 3], (-1.0, 0.0), 1.0).is_none());
        assert!(relight(&img, &[0.0; 16], (f32::NAN, 0.0), 1.0).is_none());
        assert!(relight(&img, &[0.0; 16], (-1.0, 0.0), f32::INFINITY).is_none());
    }
}
