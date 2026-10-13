//! Edge detection and highlight and shadow adjustment, with the parameters of the reference
//! filters `CIEdges` and `CIHighlightShadowAdjust`. Pixels are packed `u32` values, red in the low
//! byte, as in `gfx::Image`.
//!
//! Edges: the Sobel gradient magnitude of the luma, scaled and written as grey. Highlight and
//! shadow: a per-colour gain that darkens the highlights and lifts or darkens the shadows. The gain
//! curves are approximations of the reference ones; the defaults (highlight 1, shadow 0) are the
//! identity, as in the reference.
use crate::colorfx::{self, Rgb};
use crate::gfx::Image;

/// Rec. 601 luma (0..=255) of a packed pixel, in integer arithmetic.
fn luma_u8(p: u32) -> f32 {
    let r = p & 0xff;
    let g = (p >> 8) & 0xff;
    let b = (p >> 16) & 0xff;
    ((r * 299 + g * 587 + b * 114) / 1000) as f32
}

/// Sobel gradient magnitude of the luma, scaled by `scale` and written as grey with the input's
/// alpha. Pixels on the image border use the nearest border sample. A flat area gives 0; a step from
/// black to white gives 255 at `scale` 1. A `scale` that is not finite or not positive returns the
/// image unchanged.
pub fn edges(img: &Image, scale: f32) -> Image {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() != w * h || !scale.is_finite() || scale <= 0.0 {
        return img.clone();
    }
    let at = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        luma_u8(img.px[y * w + x])
    };
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            let gx = (at(xi + 1, yi - 1) + 2.0 * at(xi + 1, yi) + at(xi + 1, yi + 1))
                - (at(xi - 1, yi - 1) + 2.0 * at(xi - 1, yi) + at(xi - 1, yi + 1));
            let gy = (at(xi - 1, yi + 1) + 2.0 * at(xi, yi + 1) + at(xi + 1, yi + 1))
                - (at(xi - 1, yi - 1) + 2.0 * at(xi, yi - 1) + at(xi + 1, yi - 1));
            let mag = ((gx * gx + gy * gy).sqrt() / 4.0 * scale + 0.5).clamp(0.0, 255.0) as u32;
            let a = img.px[y * w + x] >> 24;
            let g = mag.min(a);
            out.px[y * w + x] = g | (g << 8) | (g << 16) | (a << 24);
        }
    }
    out
}

/// The highlight and shadow adjustment of one colour. `highlights` runs from 0 to 1 (default 1, no
/// change; lower values darken the bright parts, most at white). `shadows` runs from -1 to 1 (default
/// 0, no change; positive values lift the dark parts, most at black). Out-of-range values are clamped.
pub fn highlight_shadow_colour(c: Rgb, highlights: f32, shadows: f32) -> Rgb {
    let h = highlights.clamp(0.0, 1.0);
    let s = shadows.clamp(-1.0, 1.0);
    let l = c.luma().clamp(0.0, 1.0);
    let highlight_gain = 1.0 - (1.0 - h) * l * l;
    let shadow_gain = 1.0 + s * (1.0 - l) * (1.0 - l);
    let g = highlight_gain * shadow_gain;
    Rgb::new(c.r * g, c.g * g, c.b * g).clamped()
}

/// Applies `highlight_shadow_colour` to every pixel of `img`, keeping alpha.
pub fn highlight_shadow(img: &Image, highlights: f32, shadows: f32) -> Image {
    let mut out = img.clone();
    for p in &mut out.px {
        *p = colorfx::map_pixel(*p, |c| highlight_shadow_colour(c, highlights, shadows));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn flat_image_has_no_edges() {
        let img = Image { w: 3, h: 3, px: vec![0xff80_8080; 9] };
        let out = edges(&img, 1.0);
        assert!(out.px.iter().all(|&p| p & 0xff == 0));
    }

    #[test]
    fn step_edge_gives_full_response_on_the_inner_column() {
        // Columns 0 and 1 are black, columns 2 and 3 are white.
        let mut img = Image::new(4, 4);
        for y in 0..4 {
            for x in 2..4 {
                img.px[y * 4 + x] = 0xffff_ffff;
            }
        }
        let out = edges(&img, 1.0);
        assert_eq!(out.px[1 * 4 + 1] & 0xff, 255);
        assert_eq!(out.px[1 * 4 + 0] & 0xff, 0);
        assert_eq!(out.px[1 * 4 + 3] & 0xff, 0);
    }

    #[test]
    fn edges_keep_alpha_and_stay_premultiplied() {
        // Half-transparent white on the right half, transparent black on the left.
        let row = [0x0000_0000, 0x0000_0000, 0x80ff_ffff, 0x80ff_ffff];
        let img = Image { w: 4, h: 4, px: row.repeat(4) };
        let out = edges(&img, 4.0);
        for p in out.px {
            let a = p >> 24;
            assert!((p & 0xff) <= a, "premultiplied channel above alpha");
        }
        assert_eq!(out.px[1 * 4 + 1] >> 24, 0x00);
        assert_eq!(out.px[1 * 4 + 2] >> 24, 0x80);
    }

    #[test]
    fn defaults_are_the_identity() {
        let c = Rgb::new(0.2, 0.6, 0.9);
        let out = highlight_shadow_colour(c, 1.0, 0.0);
        assert!(close(out.r, c.r) && close(out.g, c.g) && close(out.b, c.b));
        let img = Image { w: 2, h: 1, px: vec![0xff40_80c0, 0xffff_ffff] };
        assert_eq!(highlight_shadow(&img, 1.0, 0.0).px, img.px);
    }

    #[test]
    fn zero_highlight_darkens_white_and_leaves_black() {
        let white = highlight_shadow_colour(Rgb::new(1.0, 1.0, 1.0), 0.0, 0.0);
        assert!(close(white.r, 0.0));
        let black = highlight_shadow_colour(Rgb::new(0.0, 0.0, 0.0), 0.0, 0.0);
        assert!(close(black.r, 0.0));
    }

    #[test]
    fn positive_shadow_lifts_black_and_not_white() {
        let black = highlight_shadow_colour(Rgb::new(0.0, 0.0, 0.0), 1.0, 1.0);
        assert!(close(black.r, 0.0));
        let dark = highlight_shadow_colour(Rgb::new(0.2, 0.2, 0.2), 1.0, 1.0);
        assert!(dark.r > 0.2);
        let white = highlight_shadow_colour(Rgb::new(1.0, 1.0, 1.0), 1.0, 1.0);
        assert!(close(white.r, 1.0));
    }
}
