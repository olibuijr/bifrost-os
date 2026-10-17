//! Physically based surface shading for one light and one point: a Cook-Torrance specular term
//! (GGX normal distribution, Smith visibility with Schlick-GGX geometry, Schlick Fresnel) over a
//! Lambert diffuse term, with metallic/roughness parameters. Standard public formulas (Walter et al.
//! 2007, Karis 2013). Pure f32 math; the 3D rasteriser or an app calls it per visible point.

use crate::colorfx::Rgb;
use std::f32::consts::PI;

/// Material parameters of one surface point. `metallic` and `roughness` are clamped to 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub base: Rgb,
    pub metallic: f32,
    pub roughness: f32,
}

type V3 = [f32; 3];

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: V3) -> Option<V3> {
    let len = dot(v, v).sqrt();
    if len.is_finite() && len > 0.0 {
        Some([v[0] / len, v[1] / len, v[2] / len])
    } else {
        None
    }
}

/// The outgoing radiance towards `view` at a point with `normal`, lit by one light arriving from
/// `light_dir` (the direction towards the light) with colour `light`. Each channel is clamped to
/// 0..=1. A light behind the surface, a viewer behind it, or a zero vector gives black.
pub fn shade(normal: V3, view: V3, light_dir: V3, light: Rgb, surface: &Surface) -> Rgb {
    let black = Rgb::new(0.0, 0.0, 0.0);
    let (n, v, l) = match (normalize(normal), normalize(view), normalize(light_dir)) {
        (Some(n), Some(v), Some(l)) => (n, v, l),
        _ => return black,
    };
    let nl = dot(n, l);
    let nv = dot(n, v);
    if nl <= 0.0 || nv <= 0.0 {
        return black;
    }
    let h = normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]).unwrap_or(n);
    let nh = dot(n, h).max(0.0);
    let vh = dot(v, h).max(0.0);

    let metallic = surface.metallic.clamp(0.0, 1.0);
    // Roughness is squared into the GGX alpha; the floor keeps the highlight finite.
    let rough = surface.roughness.clamp(0.045, 1.0);
    let alpha = rough * rough;
    let a2 = alpha * alpha;
    let d_denom = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * d_denom * d_denom);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let g1 = |x: f32| x / (x * (1.0 - k) + k);
    let g = g1(nv) * g1(nl);
    let spec_scale = d * g / (4.0 * nv * nl);
    let fresnel_falloff = (1.0 - vh).powi(5);

    let channel = |base: f32, light_c: f32| {
        // Dielectrics reflect 4% at normal incidence; metals tint the reflection with their base colour.
        let f0 = 0.04 + (base - 0.04) * metallic;
        let f = f0 + (1.0 - f0) * fresnel_falloff;
        let diffuse = (1.0 - f) * (1.0 - metallic) * base / PI;
        ((diffuse + f * spec_scale) * light_c * nl).clamp(0.0, 1.0)
    };
    let b = surface.base;
    Rgb::new(
        channel(b.r, light.r),
        channel(b.g, light.g),
        channel(b.b, light.b),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: V3 = [0.0, 0.0, 1.0];

    fn white() -> Rgb {
        Rgb::new(1.0, 1.0, 1.0)
    }

    fn surface(base: f32, metallic: f32, roughness: f32) -> Surface {
        Surface {
            base: Rgb::new(base, base, base),
            metallic,
            roughness,
        }
    }

    #[test]
    fn light_behind_the_surface_is_black() {
        let c = shade(UP, UP, [0.0, 0.0, -1.0], white(), &surface(1.0, 0.0, 0.5));
        assert_eq!((c.r, c.g, c.b), (0.0, 0.0, 0.0));
    }

    #[test]
    fn zero_vectors_are_black() {
        let c = shade([0.0, 0.0, 0.0], UP, UP, white(), &surface(1.0, 0.0, 0.5));
        assert_eq!(c.g, 0.0);
    }

    #[test]
    fn lit_dielectric_stays_in_range() {
        let c = shade(UP, UP, UP, white(), &surface(1.0, 0.0, 1.0));
        assert!(c.r > 0.0 && c.r <= 1.0);
    }

    #[test]
    fn metal_has_no_diffuse_term() {
        // Head-on, roughness 1: a dielectric is mostly diffuse, a metal of the same base has none.
        let dielectric = shade(UP, UP, UP, white(), &surface(0.5, 0.0, 1.0));
        let metal = shade(UP, UP, UP, white(), &surface(0.5, 1.0, 1.0));
        assert!(metal.r < dielectric.r);
    }

    #[test]
    fn smoother_surface_has_a_stronger_highlight() {
        // Black base, so only the specular lobe is visible; view and light at the normal.
        let smooth = shade(UP, UP, UP, white(), &surface(0.0, 0.0, 0.1));
        let rough = shade(UP, UP, UP, white(), &surface(0.0, 0.0, 0.9));
        assert!(smooth.r > rough.r);
    }
}
