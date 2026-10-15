//! Scattering functions: how a surface spreads light that reaches it. Two models are here: Lambertian diffuse, which
//! sends light equally in every direction, and normalized Blinn-Phong, which adds a highlight around the half vector
//! between the light and the viewer. `evaluate` gives the BRDF value for one pair of directions; `reflect` multiplies
//! it by the cosine of the light angle, which is the outgoing radiance per unit of light intensity. All directions
//! point away from the surface point. Pure maths on `Vector3`.
//!
//! The physically based model (microfacet distribution, Fresnel term, energy conservation) is not modelled here.

use crate::linalg::Vector3;
use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScatteringFunction {
    /// Diffuse only. `albedo` is the fraction of light the surface reflects, from 0 to 1.
    Lambert { albedo: f32 },
    /// Diffuse plus a highlight. `specular` scales the highlight and `shininess` is its exponent; larger values give a
    /// smaller, sharper highlight.
    BlinnPhong { albedo: f32, specular: f32, shininess: f32 },
}

impl ScatteringFunction {
    /// The BRDF value for light arriving from `to_light` and leaving toward `to_viewer`. Zero when either direction is
    /// degenerate or the light is below the surface.
    pub fn evaluate(&self, normal: Vector3, to_light: Vector3, to_viewer: Vector3) -> f32 {
        let n = normal.normalized();
        let l = to_light.normalized();
        let v = to_viewer.normalized();
        if n.dot(l) <= 0.0 || v.dot(n) <= 0.0 {
            return 0.0;
        }
        match *self {
            ScatteringFunction::Lambert { albedo } => albedo / PI,
            ScatteringFunction::BlinnPhong { albedo, specular, shininess } => {
                let h = (l + v).normalized();
                let n_dot_h = n.dot(h).max(0.0);
                let normalization = (shininess + 8.0) / (8.0 * PI);
                albedo / PI + specular * normalization * n_dot_h.powf(shininess)
            }
        }
    }

    /// The outgoing radiance for a light of unit intensity: the BRDF times the cosine of the light angle.
    pub fn reflect(&self, normal: Vector3, to_light: Vector3, to_viewer: Vector3) -> f32 {
        let cosine = normal.normalized().dot(to_light.normalized());
        if cosine <= 0.0 {
            return 0.0;
        }
        self.evaluate(normal, to_light, to_viewer) * cosine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn up() -> Vector3 {
        Vector3::new(0.0, 1.0, 0.0)
    }

    #[test]
    fn lambert_is_albedo_over_pi_from_every_direction() {
        let f = ScatteringFunction::Lambert { albedo: 0.8 };
        let seen = f.evaluate(up(), up(), up());
        let oblique = f.evaluate(up(), Vector3::new(1.0, 1.0, 0.0), up());
        assert!(close(seen, 0.8 / PI));
        assert!(close(oblique, 0.8 / PI));
    }

    #[test]
    fn reflect_scales_by_the_light_cosine() {
        let f = ScatteringFunction::Lambert { albedo: 1.0 };
        // 60 degrees from the normal gives a cosine of one half.
        let light = Vector3::new(3f32.sqrt() / 2.0, 0.5, 0.0);
        let got = f.reflect(up(), light, up());
        assert!(close(got, 0.5 / PI));
    }

    #[test]
    fn light_below_the_surface_or_a_viewer_below_it_gives_nothing() {
        let f = ScatteringFunction::BlinnPhong { albedo: 1.0, specular: 1.0, shininess: 20.0 };
        assert_eq!(f.evaluate(up(), -up(), up()), 0.0);
        assert_eq!(f.reflect(up(), -up(), up()), 0.0);
        assert_eq!(f.evaluate(up(), up(), -up()), 0.0);
    }

    #[test]
    fn degenerate_directions_give_nothing() {
        let f = ScatteringFunction::Lambert { albedo: 1.0 };
        assert_eq!(f.evaluate(up(), Vector3::ZERO, up()), 0.0);
        assert_eq!(f.reflect(Vector3::ZERO, up(), up()), 0.0);
    }

    #[test]
    fn blinn_phong_peaks_when_the_viewer_is_on_the_mirror_direction() {
        let f = ScatteringFunction::BlinnPhong { albedo: 0.0, specular: 1.0, shininess: 32.0 };
        let peak = f.evaluate(up(), up(), up());
        // The normalization makes the peak (shininess + 8) / (8 pi); with no diffuse part nothing else contributes.
        assert!(close(peak, 40.0 / (8.0 * PI)));
        let off = f.evaluate(up(), up(), Vector3::new(1.0, 1.0, 0.0));
        assert!(off < peak);
    }

    #[test]
    fn higher_shininess_narrows_the_highlight() {
        let light = Vector3::new(1.0, 1.0, 0.0);
        let view = Vector3::new(0.0, 1.0, 0.0);
        let soft = ScatteringFunction::BlinnPhong { albedo: 0.0, specular: 1.0, shininess: 4.0 };
        let sharp = ScatteringFunction::BlinnPhong { albedo: 0.0, specular: 1.0, shininess: 256.0 };
        let off_peak = Vector3::new(0.3, 1.0, 0.0);
        assert!(sharp.evaluate(up(), light, off_peak) < soft.evaluate(up(), light, off_peak));
        assert!(sharp.evaluate(up(), light, view) > 0.0);
    }

    #[test]
    fn blinn_phong_with_no_specular_is_lambert() {
        let albedo = 0.5;
        let phong = ScatteringFunction::BlinnPhong { albedo, specular: 0.0, shininess: 10.0 };
        let lambert = ScatteringFunction::Lambert { albedo };
        let light = Vector3::new(0.2, 1.0, 0.1);
        assert!(close(phong.evaluate(up(), light, up()), lambert.evaluate(up(), light, up())));
    }
}
