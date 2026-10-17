//! Physically based shading of one point: a Cook-Torrance specular lobe (GGX distribution, Smith
//! geometry with the Schlick-GGX form, Schlick Fresnel) over a Lambert diffuse term, for one
//! directional light of unit radiance. Pure per-point maths; a renderer or the 3D scene evaluates it
//! per pixel. Roughness is the artist value in 0..=1, with GGX alpha = roughness squared. Colours are
//! linear RGB. The formulas follow the Walter et al. (GGX) and Karis (Epic) course notes, written here
//! from those public descriptions.

use std::f32::consts::PI;

/// A linear RGB triple.
pub type Vec3 = [f32; 3];

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: Vec3) -> Option<Vec3> {
    let len = dot(v, v).sqrt();
    if !(len.is_finite() && len > 0.0) {
        return None;
    }
    Some([v[0] / len, v[1] / len, v[2] / len])
}

fn clamp_roughness(roughness: f32) -> f32 {
    if roughness.is_nan() {
        1.0
    } else {
        roughness.clamp(0.0, 1.0)
    }
}

/// The GGX (Trowbridge-Reitz) normal distribution at `n_dot_h`, for an artist roughness. At alpha 1
/// (roughness 1) it is the constant 1/pi.
pub fn ggx_distribution(n_dot_h: f32, roughness: f32) -> f32 {
    let alpha = clamp_roughness(roughness).powi(2);
    let a2 = alpha * alpha;
    let c = n_dot_h.clamp(0.0, 1.0);
    let denom = c * c * (a2 - 1.0) + 1.0;
    a2 / (PI * denom * denom)
}

/// The Smith masking term for one direction (Schlick-GGX form) at `n_dot_x`. Zero roughness gives 1.
pub fn smith_geometry(n_dot_x: f32, roughness: f32) -> f32 {
    let r = clamp_roughness(roughness) + 1.0;
    let k = r * r / 8.0;
    let c = n_dot_x.clamp(0.0, 1.0);
    c / (c * (1.0 - k) + k)
}

/// Schlick's Fresnel approximation: reflectance `f0` at normal incidence, rising to 1 at grazing angles.
pub fn fresnel_schlick(cos_theta: f32, f0: Vec3) -> Vec3 {
    let c = cos_theta.clamp(0.0, 1.0);
    let k = (1.0 - c).powi(5);
    [f0[0] + (1.0 - f0[0]) * k, f0[1] + (1.0 - f0[1]) * k, f0[2] + (1.0 - f0[2]) * k]
}

/// The outgoing linear RGB radiance from a surface lit by one directional light of unit radiance.
/// `normal`, `view` (towards the eye) and `light` (towards the light) need not be unit length. A
/// degenerate vector, a light behind the surface, or non-finite input gives black. `metallic` in
/// 0..=1 blends the reflectance from the dielectric 0.04 to the albedo and removes the diffuse term.
pub fn shade(normal: Vec3, view: Vec3, light: Vec3, albedo: Vec3, roughness: f32, metallic: f32) -> Vec3 {
    let (Some(n), Some(v), Some(l)) = (normalize(normal), normalize(view), normalize(light)) else {
        return [0.0; 3];
    };
    let n_dot_l = dot(n, l);
    let n_dot_v = dot(n, v).clamp(0.0, 1.0);
    if n_dot_l <= 0.0 || n_dot_v <= 0.0 {
        return [0.0; 3];
    }
    let h = normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]).unwrap_or(n);
    let n_dot_h = dot(n, h).clamp(0.0, 1.0);
    let v_dot_h = dot(v, h).clamp(0.0, 1.0);
    let rough = clamp_roughness(roughness);
    let m = if metallic.is_nan() { 0.0 } else { metallic.clamp(0.0, 1.0) };
    let f0 = [
        0.04 + (albedo[0] - 0.04) * m,
        0.04 + (albedo[1] - 0.04) * m,
        0.04 + (albedo[2] - 0.04) * m,
    ];
    let fresnel = fresnel_schlick(v_dot_h, f0);
    let d = ggx_distribution(n_dot_h, rough);
    let g = smith_geometry(n_dot_l, rough) * smith_geometry(n_dot_v, rough);
    let spec_scale = d * g / (4.0 * n_dot_l * n_dot_v).max(1e-6);
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let diffuse_weight = (1.0 - fresnel[i]) * (1.0 - m);
        let diffuse = diffuse_weight * albedo[i] / PI;
        out[i] = (diffuse + fresnel[i] * spec_scale) * n_dot_l;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn ggx_at_roughness_one_is_one_over_pi() {
        for c in [0.0f32, 0.3, 0.7, 1.0] {
            assert!(close(ggx_distribution(c, 1.0), 1.0 / PI), "n.h {c}");
        }
    }

    #[test]
    fn ggx_peaks_when_the_halfway_vector_is_the_normal() {
        let peak = ggx_distribution(1.0, 0.4);
        assert!(peak > ggx_distribution(0.8, 0.4));
        assert!(peak > ggx_distribution(0.2, 0.4));
    }

    #[test]
    fn smith_term_is_one_facing_the_viewer_and_zero_at_grazing() {
        assert!(close(smith_geometry(1.0, 0.5), 1.0));
        assert!(close(smith_geometry(1.0, 0.0), 1.0));
        assert!(close(smith_geometry(0.0, 0.5), 0.0));
    }

    #[test]
    fn fresnel_is_f0_head_on_and_full_at_grazing() {
        let f0 = [0.04, 0.5, 0.9];
        let head = fresnel_schlick(1.0, f0);
        assert!(close(head[0], 0.04) && close(head[1], 0.5) && close(head[2], 0.9));
        let grazing = fresnel_schlick(0.0, f0);
        assert!(grazing.iter().all(|&c| close(c, 1.0)));
    }

    #[test]
    fn a_light_behind_the_surface_gives_black() {
        let out = shade([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [1.0; 3], 0.5, 0.0);
        assert_eq!(out, [0.0; 3]);
    }

    #[test]
    fn degenerate_or_non_finite_input_gives_black() {
        let zero = [0.0f32; 3];
        assert_eq!(shade(zero, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [1.0; 3], 0.5, 0.0), [0.0; 3]);
        assert_eq!(shade([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [f32::NAN, 0.0, 1.0], [1.0; 3], 0.5, 0.0), [0.0; 3]);
    }

    #[test]
    fn a_lit_dielectric_is_finite_non_negative_and_brighter_when_lit_head_on() {
        let n = [0.0, 0.0, 1.0];
        let head = shade(n, n, n, [0.8, 0.8, 0.8], 0.6, 0.0);
        let oblique = shade(n, n, [1.0, 0.0, 1.0], [0.8, 0.8, 0.8], 0.6, 0.0);
        for c in head.iter().chain(oblique.iter()) {
            assert!(c.is_finite() && *c >= 0.0);
        }
        assert!(head[0] > oblique[0]);
    }

    #[test]
    fn a_metal_has_no_diffuse_term() {
        // A fully metallic surface has a tinted specular only: the same albedo at metallic 1 scales
        // with Fresnel, so its colour ratio follows the albedo ratio at normal incidence.
        let n = [0.0, 0.0, 1.0];
        let out = shade(n, n, n, [1.0, 0.5, 0.25], 1.0, 1.0);
        assert!(out[0] > out[1] && out[1] > out[2]);
    }
}
