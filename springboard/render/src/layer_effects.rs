//! Layer effects for icon art and asset-catalog shapes, as pure kernels on coverage planes (std only): inner shadow
//! (and, with no offset, inner glow), bevel and emboss from a softened height field, and a frosted-glass icon layer
//! that combines a blurred backdrop, a translucent tint, a light-facing specular rim and an inner shadow. Drop
//! shadows and outer glows are `effects::shadow_layer`; edge lensing is `glass_lens`.
//!
//! All inputs are `effects::Plane` coverage maps (0..=1, row-major). Blurs treat the outside of a plane as 0, so a
//! shape needs a margin of about three blur sigmas plus its offset to the plane edge. `icon_glass.frag` in the GPU
//! shader directory is the per-pixel version of `render_glass_layer`: the blurs come from the Kawase passes and the
//! shader does the bevel gradient, mixing and shading.

use crate::effects::{gaussian_blur, Plane};

/// Bilinear sample of a plane at a fractional position (outside is 0).
fn sample(p: &Plane, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (xi, yi) = (x0 as i32, y0 as i32);
    let top = p.get(xi, yi) * (1.0 - fx) + p.get(xi + 1, yi) * fx;
    let bottom = p.get(xi, yi + 1) * (1.0 - fx) + p.get(xi + 1, yi + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// The plane moved by `(dx, dy)` pixels (positive moves content right and down), bilinear.
pub fn translate(p: &Plane, dx: f32, dy: f32) -> Plane {
    let mut out = Plane::new(p.w, p.h);
    for y in 0..p.h {
        for x in 0..p.w {
            out.a[y * p.w + x] = sample(p, x as f32 - dx, y as f32 - dy);
        }
    }
    out
}

/// Inner shadow of a shape: the inverse of the shape, moved by `(dx, dy)` and blurred by `sigma`, clipped to the
/// shape. The shadow falls from the edge the offset moves away from; with a zero offset this is an inner glow.
pub fn inner_shadow(shape: &Plane, dx: f32, dy: f32, sigma: f32) -> Plane {
    let mut inverse = Plane::new(shape.w, shape.h);
    for (o, s) in inverse.a.iter_mut().zip(&shape.a) {
        *o = 1.0 - *s;
    }
    let moved = translate(&inverse, dx, dy);
    let mut blurred = gaussian_blur(&moved, sigma);
    blurred.multiply(shape);
    blurred
}

/// Bevel and emboss: the shape is softened into a height field and its slope against the light gives a
/// `(highlight, shadow)` pair of planes, both clipped to the shape. `light` points from the surface toward the light
/// in image coordinates (y down), `depth` scales the slope (about 2 to 4 gives a visible edge for a softening of 2
/// pixels). Surfaces rising toward the light darken, surfaces falling toward it catch it, as for a raised plate.
pub fn bevel_emboss(shape: &Plane, light: (f32, f32), depth: f32, soften: f32) -> (Plane, Plane) {
    let l = (light.0 * light.0 + light.1 * light.1).sqrt();
    let (lx, ly) = if l < 1e-6 { (0.0, 0.0) } else { (light.0 / l, light.1 / l) };
    let height = gaussian_blur(shape, soften);
    let mut high = Plane::new(shape.w, shape.h);
    let mut low = Plane::new(shape.w, shape.h);
    for y in 0..shape.h as i32 {
        for x in 0..shape.w as i32 {
            let gx = (height.get(x + 1, y) - height.get(x - 1, y)) * 0.5;
            let gy = (height.get(x, y + 1) - height.get(x, y - 1)) * 0.5;
            let s = -(gx * lx + gy * ly) * depth;
            let cover = shape.get(x, y);
            let i = y as usize * shape.w + x as usize;
            high.a[i] = s.clamp(0.0, 1.0) * cover;
            low.a[i] = (-s).clamp(0.0, 1.0) * cover;
        }
    }
    (high, low)
}

/// Parameters of the frosted-glass icon layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassParams {
    /// Tint colour, 0..=1 per channel.
    pub tint: [f32; 3],
    /// How much of the tint replaces the blurred backdrop (0 clear, 1 opaque tint).
    pub tint_alpha: f32,
    /// Blur sigma of the backdrop seen through the glass.
    pub backdrop_sigma: f32,
    /// Direction toward the light, image coordinates (y down).
    pub light: (f32, f32),
    pub bevel_depth: f32,
    pub bevel_soften: f32,
    pub inner_shadow_offset: (f32, f32),
    pub inner_shadow_sigma: f32,
    pub inner_shadow_strength: f32,
    /// Strength of the highlight (screen) and, at half weight, of the rim shade (multiply).
    pub specular_strength: f32,
}

impl Default for GlassParams {
    fn default() -> Self {
        GlassParams {
            tint: [1.0, 1.0, 1.0],
            tint_alpha: 0.25,
            backdrop_sigma: 4.0,
            light: (-0.5, -1.0),
            bevel_depth: 3.0,
            bevel_soften: 1.5,
            inner_shadow_offset: (0.0, 2.0),
            inner_shadow_sigma: 2.0,
            inner_shadow_strength: 0.35,
            specular_strength: 1.0,
        }
    }
}

/// Renders a glass layer: unpremultiplied RGBA (alpha is the shape coverage), row-major. `backdrop` holds the red,
/// green and blue planes of what lies behind the icon and must match the shape's size, otherwise `None`.
pub fn render_glass_layer(shape: &Plane, backdrop: &[Plane; 3], p: &GlassParams) -> Option<Vec<[f32; 4]>> {
    if backdrop.iter().any(|b| b.w != shape.w || b.h != shape.h) {
        return None;
    }
    let blurred = [
        gaussian_blur(&backdrop[0], p.backdrop_sigma),
        gaussian_blur(&backdrop[1], p.backdrop_sigma),
        gaussian_blur(&backdrop[2], p.backdrop_sigma),
    ];
    let inner = inner_shadow(shape, p.inner_shadow_offset.0, p.inner_shadow_offset.1, p.inner_shadow_sigma);
    let (high, low) = bevel_emboss(shape, p.light, p.bevel_depth, p.bevel_soften);
    let ta = p.tint_alpha.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(shape.w * shape.h);
    for i in 0..shape.w * shape.h {
        let hi = (high.a[i] * p.specular_strength).clamp(0.0, 1.0);
        let shade = (low.a[i] * p.specular_strength * 0.5).max(inner.a[i] * p.inner_shadow_strength).clamp(0.0, 1.0);
        let mut rgb = [0.0f32; 3];
        for c in 0..3 {
            let base = blurred[c].a[i] * (1.0 - ta) + p.tint[c] * ta;
            let lit = base + (1.0 - base) * hi;
            rgb[c] = (lit * (1.0 - shade)).clamp(0.0, 1.0);
        }
        out.push([rgb[0], rgb[1], rgb[2], shape.a[i].clamp(0.0, 1.0)]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Plane {
        let mut p = Plane::new(w, h);
        for y in y0..y1 {
            for x in x0..x1 {
                p.a[y * w + x] = 1.0;
            }
        }
        p
    }

    fn disc(w: usize, h: usize, cx: f32, cy: f32, r: f32) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                p.a[y * w + x] = (r + 0.5 - d).clamp(0.0, 1.0);
            }
        }
        p
    }

    #[test]
    fn translate_moves_content() {
        let p = rect(16, 16, 4, 4, 6, 6);
        let t = translate(&p, 3.0, 2.0);
        assert!((t.get(7, 6) - 1.0).abs() < 1e-6);
        assert!(t.get(4, 4).abs() < 1e-6);
    }

    #[test]
    fn inner_shadow_hugs_the_edge_it_is_offset_from() {
        let shape = rect(64, 64, 12, 12, 52, 52);
        let s = inner_shadow(&shape, 0.0, 3.0, 1.5);
        assert!(s.get(32, 14) > 0.5, "{}", s.get(32, 14));
        assert!(s.get(32, 14) > s.get(32, 50));
        assert!(s.get(32, 32) < 0.01);
        assert_eq!(s.get(2, 2), 0.0);
    }

    #[test]
    fn inner_glow_is_even_all_round() {
        let shape = rect(64, 64, 12, 12, 52, 52);
        let g = inner_shadow(&shape, 0.0, 0.0, 2.0);
        assert!((g.get(13, 32) - g.get(50, 32)).abs() < 0.02);
        assert!((g.get(32, 13) - g.get(32, 50)).abs() < 0.02);
    }

    #[test]
    fn bevel_lights_the_side_facing_the_light() {
        let shape = disc(64, 64, 32.0, 32.0, 10.0);
        let (hi, lo) = bevel_emboss(&shape, (-1.0, -1.0), 5.0, 2.0);
        assert!(hi.get(26, 26) > 0.3, "{}", hi.get(26, 26));
        assert!(lo.get(26, 26) < 0.01);
        assert!(lo.get(38, 38) > 0.3, "{}", lo.get(38, 38));
        assert!(hi.get(38, 38) < 0.01);
        assert_eq!(hi.get(2, 2), 0.0);
        // The flat middle has no slope.
        assert!(hi.get(32, 32) < 0.01 && lo.get(32, 32) < 0.01);
    }

    #[test]
    fn glass_layer_tints_the_blurred_backdrop() {
        let shape = disc(64, 64, 32.0, 32.0, 12.0);
        let white = Plane::filled(64, 64, 1.0);
        let backdrop = [white.clone(), white.clone(), white];
        let params = GlassParams { tint: [0.0, 0.0, 1.0], tint_alpha: 0.5, light: (-1.0, -1.0), bevel_depth: 6.0, inner_shadow_strength: 0.0, ..GlassParams::default() };
        let px = render_glass_layer(&shape, &backdrop, &params).unwrap();
        let at = |x: usize, y: usize| px[y * 64 + x];
        let c = at(32, 32);
        assert!((c[0] - 0.5).abs() < 0.02 && (c[1] - 0.5).abs() < 0.02 && (c[2] - 1.0).abs() < 0.02, "{c:?}");
        assert!((c[3] - 1.0).abs() < 1e-6);
        assert_eq!(at(1, 1)[3], 0.0);
        // The rim facing the light is brighter than the rim facing away.
        let lit = at(25, 25);
        let dark = at(39, 39);
        assert!(lit[0] + lit[1] + lit[2] > dark[0] + dark[1] + dark[2]);
    }

    #[test]
    fn mismatched_backdrop_is_refused() {
        let shape = Plane::new(8, 8);
        let ok = Plane::new(8, 8);
        let bad = Plane::new(4, 8);
        assert!(render_glass_layer(&shape, &[ok.clone(), ok.clone(), bad], &GlassParams::default()).is_none());
    }
}
