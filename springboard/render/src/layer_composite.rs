//! Window-server layer compositing in floating point (std only): the extended-range path that SDR 8-bit blending
//! cannot do. Pixels are premultiplied linear RGBA `f32` where 1.0 is reference white and values above it are headroom
//! for an extended-range (EDR) display. Provides layer blend modes with per-layer opacity and an alpha mask, a
//! colour-matrix layer filter (saturation, tint) that keeps headroom, and tone-mapping operators (clip, extended
//! Reinhard, an ACES-style fit) applied per channel or on luminance.
//!
//! `tonemap.frag` (GPU shaders) is the per-pixel version of the tone-mapping operators. The packed 8-bit layer mask
//! and the soft-knee EDR roll-off are `kernel_layer_fx`; 8-bit blending lives in `blend`; transfer functions and
//! frame metadata in `hdr_transfer` and `hdr_frame`.

/// A premultiplied RGBA pixel.
pub type Px = [f32; 4];

/// How a layer combines with what is under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerBlend {
    /// Source over destination.
    Normal,
    /// Colours multiply (darkens).
    Multiply,
    /// Inverse of the product of inverses (lightens, stays below 1 for inputs below 1).
    Screen,
    /// Colours add; values may exceed 1 (light accumulation, glow).
    Add,
}

/// Blends `src` onto `dst` (both premultiplied) with a layer opacity in 0..=1.
pub fn blend_pixel(dst: Px, src: Px, blend: LayerBlend, opacity: f32) -> Px {
    let o = opacity.clamp(0.0, 1.0);
    let s = [src[0] * o, src[1] * o, src[2] * o, src[3] * o];
    let (sa, da) = (s[3], dst[3]);
    let mut out = [0.0f32; 4];
    for c in 0..3 {
        out[c] = match blend {
            LayerBlend::Normal => s[c] + dst[c] * (1.0 - sa),
            LayerBlend::Multiply => s[c] * (1.0 - da) + dst[c] * (1.0 - sa) + s[c] * dst[c],
            LayerBlend::Screen => s[c] + dst[c] - s[c] * dst[c],
            LayerBlend::Add => s[c] + dst[c],
        };
    }
    out[3] = match blend {
        LayerBlend::Add => (sa + da).min(1.0),
        _ => sa + da - sa * da,
    };
    out
}

/// Composites `src` onto `dst` in place. `mask` (0..=1 per pixel) scales the layer's contribution. Returns false and
/// changes nothing when the slice lengths differ.
pub fn composite_layer(dst: &mut [Px], src: &[Px], mask: Option<&[f32]>, opacity: f32, blend: LayerBlend) -> bool {
    if dst.len() != src.len() || mask.is_some_and(|m| m.len() != dst.len()) {
        return false;
    }
    for (i, (d, s)) in dst.iter_mut().zip(src).enumerate() {
        let m = mask.map_or(1.0, |m| m[i].clamp(0.0, 1.0));
        *d = blend_pixel(*d, *s, blend, opacity * m);
    }
    true
}

/// A 3x3 colour matrix, row-major: output channel `i` is the dot product of row `i` with the input colour.
pub type Matrix3 = [[f32; 3]; 3];

/// The matrix that scales colour saturation by `s` (0 grey, 1 unchanged, above 1 boosts), with Rec. 709 luma weights.
pub fn saturation_matrix(s: f32) -> Matrix3 {
    let (lr, lg, lb) = (0.2126, 0.7152, 0.0722);
    [
        [lr * (1.0 - s) + s, lg * (1.0 - s), lb * (1.0 - s)],
        [lr * (1.0 - s), lg * (1.0 - s) + s, lb * (1.0 - s)],
        [lr * (1.0 - s), lg * (1.0 - s), lb * (1.0 - s) + s],
    ]
}

/// Applies a colour matrix and a per-channel offset to a premultiplied pixel: the colour is unpremultiplied, mapped,
/// clamped at 0 below (not above, so headroom survives) and premultiplied again.
pub fn color_matrix(p: Px, m: &Matrix3, offset: [f32; 3]) -> Px {
    if p[3] <= 0.0 {
        return p;
    }
    let u = [p[0] / p[3], p[1] / p[3], p[2] / p[3]];
    let mut o = [0.0f32; 4];
    for i in 0..3 {
        o[i] = (m[i][0] * u[0] + m[i][1] * u[1] + m[i][2] * u[2] + offset[i]).max(0.0) * p[3];
    }
    o[3] = p[3];
    o
}

/// A tone-mapping operator for one scene-linear channel (1.0 is reference white).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToneMap {
    /// Clamp to 0..=1.
    Clip,
    /// Reinhard with a white point: `x (1 + x / w^2) / (1 + x)`; `w` maps to 1.
    ReinhardExtended { white: f32 },
    /// A rational fit of an ACES-style filmic curve (Narkowicz), tuned for input scaled to about 0..16.
    Aces,
}

/// Maps one channel. Negative input maps to 0.
pub fn tone_map(op: ToneMap, x: f32) -> f32 {
    let x = x.max(0.0);
    match op {
        ToneMap::Clip => x.min(1.0),
        ToneMap::ReinhardExtended { white } => {
            let w2 = (white * white).max(1e-6);
            x * (1.0 + x / w2) / (1.0 + x)
        }
        ToneMap::Aces => {
            let a = x * (2.51 * x + 0.03);
            let b = x * (2.43 * x + 0.59) + 0.14;
            (a / b).clamp(0.0, 1.0)
        }
    }
}

/// Tone maps a colour channel by channel after multiplying by `exposure`.
pub fn tone_map_rgb(op: ToneMap, rgb: [f32; 3], exposure: f32) -> [f32; 3] {
    [tone_map(op, rgb[0] * exposure), tone_map(op, rgb[1] * exposure), tone_map(op, rgb[2] * exposure)]
}

/// Tone maps the luminance and scales the colour with it, which keeps hue and saturation ratios (channel values may
/// exceed the operator's range for very saturated colours).
pub fn tone_map_luma(op: ToneMap, rgb: [f32; 3], exposure: f32) -> [f32; 3] {
    let c = [rgb[0] * exposure, rgb[1] * exposure, rgb[2] * exposure];
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    if l <= 1e-9 {
        return [0.0; 3];
    }
    let k = tone_map(op, l) / l;
    [c[0] * k, c[1] * k, c[2] * k]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Px, b: Px) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn normal_blend_is_source_over() {
        let dst = [0.0, 0.0, 0.5, 1.0];
        let src = [0.5, 0.0, 0.0, 0.5];
        assert!(near(blend_pixel(dst, src, LayerBlend::Normal, 1.0), [0.5, 0.0, 0.25, 1.0]));
        // Half opacity halves the layer.
        assert!(near(blend_pixel(dst, src, LayerBlend::Normal, 0.5), [0.25, 0.0, 0.375, 1.0]));
        // Zero opacity changes nothing.
        assert_eq!(blend_pixel(dst, src, LayerBlend::Normal, 0.0), dst);
    }

    #[test]
    fn multiply_and_screen_follow_their_formulas() {
        let a = [0.5, 0.5, 0.5, 1.0];
        let b = [0.5, 0.25, 1.0, 1.0];
        assert!(near(blend_pixel(a, b, LayerBlend::Multiply, 1.0), [0.25, 0.125, 0.5, 1.0]));
        assert!(near(blend_pixel(a, b, LayerBlend::Screen, 1.0), [0.75, 0.625, 1.0, 1.0]));
        // Multiplying by a transparent source leaves the backdrop.
        assert!(near(blend_pixel(a, [0.0; 4], LayerBlend::Multiply, 1.0), a));
        // Over a transparent backdrop multiply gives the source.
        assert!(near(blend_pixel([0.0; 4], b, LayerBlend::Multiply, 1.0), b));
    }

    #[test]
    fn add_keeps_headroom() {
        let out = blend_pixel([1.0, 1.0, 1.0, 1.0], [0.5, 0.5, 0.5, 0.5], LayerBlend::Add, 1.0);
        assert!(out[0] > 1.4 && out[3] == 1.0, "{out:?}");
    }

    #[test]
    fn layers_composite_with_masks() {
        let mut dst = vec![[0.0, 0.0, 0.0, 1.0]; 3];
        let src = vec![[1.0, 1.0, 1.0, 1.0]; 3];
        assert!(composite_layer(&mut dst, &src, Some(&[0.0, 0.5, 1.0]), 1.0, LayerBlend::Normal));
        assert!(near(dst[0], [0.0, 0.0, 0.0, 1.0]));
        assert!(near(dst[1], [0.5, 0.5, 0.5, 1.0]));
        assert!(near(dst[2], [1.0, 1.0, 1.0, 1.0]));
        assert!(!composite_layer(&mut dst, &src[..2], None, 1.0, LayerBlend::Normal));
        assert!(!composite_layer(&mut dst, &src, Some(&[1.0]), 1.0, LayerBlend::Normal));
    }

    #[test]
    fn colour_matrix_filters() {
        let p = [0.4, 0.2, 0.1, 0.5];
        let id = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert!(near(color_matrix(p, &id, [0.0; 3]), p));
        let grey = color_matrix(p, &saturation_matrix(0.0), [0.0; 3]);
        assert!((grey[0] - grey[1]).abs() < 1e-6 && (grey[1] - grey[2]).abs() < 1e-6);
        assert_eq!(grey[3], 0.5);
        assert!(near(color_matrix(p, &saturation_matrix(1.0), [0.0; 3]), p));
        // Transparent pixels pass through; above-one results survive.
        assert_eq!(color_matrix([0.0; 4], &id, [1.0; 3]), [0.0; 4]);
        let bright = color_matrix([1.0, 1.0, 1.0, 1.0], &id, [0.5; 3]);
        assert!((bright[0] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn tone_map_operators() {
        assert_eq!(tone_map(ToneMap::Clip, 3.0), 1.0);
        assert_eq!(tone_map(ToneMap::Clip, -1.0), 0.0);
        let r = ToneMap::ReinhardExtended { white: 4.0 };
        assert!((tone_map(r, 4.0) - 1.0).abs() < 1e-5);
        assert!(tone_map(r, 0.5) < 0.4 && tone_map(r, 0.5) > 0.3);
        assert_eq!(tone_map(r, 0.0), 0.0);
        assert!(tone_map(ToneMap::Aces, 0.0) == 0.0);
        assert!(tone_map(ToneMap::Aces, 100.0) <= 1.0);
        assert!(tone_map(ToneMap::Aces, 0.5) > tone_map(ToneMap::Aces, 0.25));
    }

    #[test]
    fn luma_tone_mapping_keeps_hue_ratios() {
        let c = [2.0, 1.0, 0.5];
        let t = tone_map_luma(ToneMap::ReinhardExtended { white: 4.0 }, c, 1.0);
        assert!((t[0] / t[1] - 2.0).abs() < 1e-4 && (t[1] / t[2] - 2.0).abs() < 1e-4);
        assert_eq!(tone_map_luma(ToneMap::Aces, [0.0; 3], 1.0), [0.0; 3]);
        // Exposure scales before mapping.
        let a = tone_map_rgb(ToneMap::Clip, [0.25, 0.5, 0.75], 2.0);
        assert_eq!(a, [0.5, 1.0, 1.0]);
    }
}
