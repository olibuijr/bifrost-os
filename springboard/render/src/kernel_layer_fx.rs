//! Layer-effect kernels of the compositor and the icon renderer: the per-pixel jobs a window server and a
//! UI materials library run on a layer after it is drawn. Layer masks (a coverage plane scales a
//! premultiplied layer), extended-range tone mapping of linear colour for a display with headroom, and the
//! specular shading of a bevelled icon plate (height field to surface normals to a Blinn-Phong highlight).
//! The lensing and rim highlight of the glass edge live in `glass_lens`; blur is in `blur_kernels`.
//!
//! Pixels are packed premultiplied RGBA with red in the low byte, like the rest of the render crate. The
//! GLSL twin is `springboard/gpu/shaders/layer_mask_edr.comp`. Written from the public descriptions of
//! alpha masking, Blinn-Phong shading and exponential highlight roll-off.

use crate::hdr_frame::{edr_tone_map, EDR_KNEE};

/// Scales a premultiplied RGBA pixel by `coverage` (0..=255): all four channels, rounded.
pub fn scale_pixel(px: u32, coverage: u8) -> u32 {
    let c = coverage as u32;
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        let ch = (px >> shift) & 0xff;
        let v = (ch * c + 127) / 255;
        out |= v << shift;
    }
    out
}

/// Applies a layer mask in place. `mask` and `px` must have the same length, otherwise nothing changes
/// and `false` is returned.
pub fn apply_layer_mask(px: &mut [u32], mask: &[u8]) -> bool {
    if px.len() != mask.len() {
        return false;
    }
    for (p, m) in px.iter_mut().zip(mask.iter()) {
        *p = scale_pixel(*p, *m);
    }
    true
}

/// A mask from the luminance of a straight (not premultiplied) pixel, Rec. 709 weights.
pub fn luminance_mask(px: &[u32]) -> Vec<u8> {
    px.iter()
        .map(|p| {
            let r = (p & 0xff) as f32;
            let g = ((p >> 8) & 0xff) as f32;
            let b = ((p >> 16) & 0xff) as f32;
            (0.2126 * r + 0.7152 * g + 0.0722 * b).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// The mask that keeps the alpha channel of each pixel.
pub fn alpha_mask(px: &[u32]) -> Vec<u8> {
    px.iter().map(|p| (p >> 24) as u8).collect()
}

/// Tone-maps one linear RGB triple (1.0 is SDR white) for a display with `headroom` times SDR white. When
/// the headroom is at or below the roll-off knee there is no extended range and the colour clips to 1.
pub fn edr_tone_map_rgb(rgb: [f32; 3], headroom: f32) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let v = rgb[i];
        out[i] = match edr_tone_map(v as f64, headroom as f64) {
            Some(m) => m as f32,
            None => {
                if v.is_finite() {
                    v.clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
        };
    }
    out
}

/// Tone-maps a whole linear RGB buffer in place.
pub fn edr_tone_map_buffer(buf: &mut [[f32; 3]], headroom: f32) {
    for p in buf.iter_mut() {
        *p = edr_tone_map_rgb(*p, headroom);
    }
}

/// The knee below which tone mapping leaves a value unchanged (re-exported for callers and tests).
pub fn edr_knee() -> f32 {
    EDR_KNEE as f32
}

/// Surface normals of a height field by central differences. `scale` multiplies the slope (bevel height
/// per pixel). Edges use one-sided differences. Returns `None` for a size mismatch or a zero dimension.
pub fn height_to_normals(height: &[f32], w: usize, h: usize, scale: f32) -> Option<Vec<[f32; 3]>> {
    if w == 0 || h == 0 || height.len() != w * h {
        return None;
    }
    let at = |x: usize, y: usize| height[y * w + x];
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let xl = if x > 0 { x - 1 } else { x };
            let xr = if x + 1 < w { x + 1 } else { x };
            let yu = if y > 0 { y - 1 } else { y };
            let yd = if y + 1 < h { y + 1 } else { y };
            let dx = (at(xr, y) - at(xl, y)) / ((xr - xl).max(1) as f32) * scale;
            let dy = (at(x, yd) - at(x, yu)) / ((yd - yu).max(1) as f32) * scale;
            let len = (dx * dx + dy * dy + 1.0).sqrt();
            out.push([-dx / len, -dy / len, 1.0 / len]);
        }
    }
    Some(out)
}

/// The Blinn-Phong highlight of a unit normal for a light direction and a view direction (both pointing
/// away from the surface, not necessarily unit length). Zero when the light is behind the surface.
pub fn specular_highlight(normal: [f32; 3], light: [f32; 3], view: [f32; 3], shininess: f32) -> f32 {
    fn norm(v: [f32; 3]) -> Option<[f32; 3]> {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if l.is_finite() && l > 0.0 {
            Some([v[0] / l, v[1] / l, v[2] / l])
        } else {
            None
        }
    }
    let (Some(n), Some(l), Some(v)) = (norm(normal), norm(light), norm(view)) else {
        return 0.0;
    };
    let n_dot_l = n[0] * l[0] + n[1] * l[1] + n[2] * l[2];
    if n_dot_l <= 0.0 {
        return 0.0;
    }
    let Some(hv) = norm([l[0] + v[0], l[1] + v[1], l[2] + v[2]]) else {
        return 0.0;
    };
    let n_dot_h = (n[0] * hv[0] + n[1] * hv[1] + n[2] * hv[2]).max(0.0);
    n_dot_h.powf(shininess.max(1.0))
}

/// The specular layer of a bevelled plate: one highlight value per pixel, scaled by `strength`.
pub fn icon_specular(
    height: &[f32],
    w: usize,
    h: usize,
    bevel: f32,
    light: [f32; 3],
    shininess: f32,
    strength: f32,
) -> Option<Vec<f32>> {
    let normals = height_to_normals(height, w, h, bevel)?;
    Some(
        normals
            .iter()
            .map(|n| (specular_highlight(*n, light, [0.0, 0.0, 1.0], shininess) * strength).clamp(0.0, 1.0))
            .collect(),
    )
}

/// Adds a white highlight layer (0..=1 per pixel) onto a premultiplied RGBA layer, without changing its
/// alpha: the highlight is limited to the layer's own coverage so it never leaks outside the shape.
pub fn add_highlight(px: &mut [u32], highlight: &[f32]) -> bool {
    if px.len() != highlight.len() {
        return false;
    }
    for (p, hl) in px.iter_mut().zip(highlight.iter()) {
        let a = (*p >> 24) & 0xff;
        let add = (hl.clamp(0.0, 1.0) * a as f32).round() as u32;
        let mut out = *p & 0xff00_0000;
        for shift in [0u32, 8, 16] {
            let ch = ((*p >> shift) & 0xff) + add;
            out |= ch.min(a) << shift;
        }
        *p = out;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_mask_keeps_the_pixel_and_zero_clears_it() {
        let px = 0x80_40_20_10u32;
        assert_eq!(scale_pixel(px, 255), px);
        assert_eq!(scale_pixel(px, 0), 0);
    }

    #[test]
    fn half_mask_halves_every_channel() {
        let out = scale_pixel(0xff_ff_ff_ffu32, 128);
        assert_eq!(out & 0xff, 128);
        assert_eq!(out >> 24, 128);
    }

    #[test]
    fn mask_length_mismatch_is_refused() {
        let mut px = [0xffff_ffffu32; 2];
        assert!(!apply_layer_mask(&mut px, &[255]));
        assert_eq!(px, [0xffff_ffffu32; 2]);
        assert!(apply_layer_mask(&mut px, &[255, 0]));
        assert_eq!(px[1], 0);
    }

    #[test]
    fn luminance_and_alpha_masks() {
        let white = 0xffff_ffffu32;
        let black = 0xff00_0000u32;
        assert_eq!(luminance_mask(&[white, black]), vec![255, 0]);
        assert_eq!(alpha_mask(&[white, 0x7f00_0000]), vec![255, 0x7f]);
    }

    #[test]
    fn tone_map_passes_below_the_knee_and_stays_under_the_headroom() {
        let low = edr_tone_map_rgb([0.2, 0.5, 0.7], 4.0);
        assert!((low[0] - 0.2).abs() < 1e-6 && (low[2] - 0.7).abs() < 1e-6);
        let high = edr_tone_map_rgb([50.0, 3.0, 1.0], 4.0);
        assert!(high[0] <= 4.0 && high[0] > 3.5);
        assert!(high[0] > high[1] && high[1] > high[2]);
    }

    #[test]
    fn no_headroom_clips_to_sdr_white() {
        let out = edr_tone_map_rgb([2.0, -1.0, 0.5], 0.5);
        assert_eq!(out, [1.0, 0.0, 0.5]);
        assert!((edr_knee() - 0.8).abs() < 1e-6);
    }

    #[test]
    fn flat_height_gives_straight_up_normals() {
        let n = height_to_normals(&[0.0; 9], 3, 3, 1.0).unwrap();
        for v in n {
            assert_eq!(v, [0.0, 0.0, 1.0]);
        }
        assert!(height_to_normals(&[0.0; 8], 3, 3, 1.0).is_none());
    }

    #[test]
    fn a_ramp_tilts_the_normal_against_the_slope() {
        // Height rises to the right, so the normal leans left (negative x).
        let h = [0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0];
        let n = height_to_normals(&h, 3, 3, 1.0).unwrap();
        assert!(n[4][0] < 0.0 && n[4][1].abs() < 1e-6);
    }

    #[test]
    fn highlight_peaks_at_the_half_vector_and_dies_behind_the_surface() {
        let up = [0.0, 0.0, 1.0];
        let peak = specular_highlight(up, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], 32.0);
        assert!((peak - 1.0).abs() < 1e-5);
        let off = specular_highlight(up, [1.0, 0.0, 1.0], [0.0, 0.0, 1.0], 32.0);
        assert!(off < peak);
        assert_eq!(specular_highlight(up, [0.0, 0.0, -1.0], up, 32.0), 0.0);
        assert_eq!(specular_highlight(up, [0.0; 3], up, 32.0), 0.0);
    }

    #[test]
    fn icon_specular_is_brighter_on_flat_ground_than_on_a_slope_for_a_head_on_light() {
        // A 5x5 pyramid: the top is flat-ish, the sides tilt.
        let mut h = vec![0.0f32; 25];
        h[12] = 2.0;
        let s = icon_specular(&h, 5, 5, 1.0, [0.0, 0.0, 1.0], 16.0, 1.0).unwrap();
        assert!(s[0] > s[11]);
    }

    #[test]
    fn highlight_never_leaves_the_layer_coverage() {
        let mut px = [0x40_00_00_00u32, 0xff_00_00_00u32];
        assert!(add_highlight(&mut px, &[1.0, 0.5]));
        // Alpha is untouched and a colour channel never exceeds alpha (premultiplied invariant).
        assert_eq!(px[0] >> 24, 0x40);
        assert!((px[0] & 0xff) <= 0x40);
        assert_eq!(px[1] & 0xff, 128);
    }
}
