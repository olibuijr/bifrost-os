//! Lighting and fog evaluated per point on the CPU (`NativeBaseEffect` lighting, `NativeEffectPropertyLight`,
//! `NativeEffectPropertyMaterial`, `NativeEffectPropertyFog`). The model is ambient plus Lambert diffuse plus a
//! Blinn-Phong highlight, clamped to the displayable range. A renderer may run it per vertex or per pixel.

use crate::linalg::Vector3;

/// A linear colour with components in 0..=1 (values outside that range are allowed until `clamped`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0.0, g: 0.0, b: 0.0 };
    pub const WHITE: Rgb = Rgb { r: 1.0, g: 1.0, b: 1.0 };

    pub fn new(r: f32, g: f32, b: f32) -> Self {
        Rgb { r, g, b }
    }

    /// Component-wise product: light colour times surface colour.
    pub fn modulate(self, o: Rgb) -> Rgb {
        Rgb::new(self.r * o.r, self.g * o.g, self.b * o.b)
    }

    pub fn scaled(self, s: f32) -> Rgb {
        Rgb::new(self.r * s, self.g * s, self.b * s)
    }

    pub fn added(self, o: Rgb) -> Rgb {
        Rgb::new(self.r + o.r, self.g + o.g, self.b + o.b)
    }

    /// Each component limited to 0..=1.
    pub fn clamped(self) -> Rgb {
        Rgb::new(self.r.clamp(0.0, 1.0), self.g.clamp(0.0, 1.0), self.b.clamp(0.0, 1.0))
    }
}

/// Where a light comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightSource {
    /// A light at a position in the same space as the surface.
    Point(Vector3),
    /// Parallel light travelling along this direction (the opposite of the direction towards the light).
    Directional(Vector3),
}

/// One light and its three colour terms (`NativeEffectPropertyLight`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub source: LightSource,
    pub ambient: Rgb,
    pub diffuse: Rgb,
    pub specular: Rgb,
}

/// How a surface reflects each term (`NativeEffectPropertyMaterial`). `shininess` is the highlight exponent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub ambient: Rgb,
    pub diffuse: Rgb,
    pub specular: Rgb,
    pub shininess: f32,
}

/// Colour of a surface point lit by `light`, seen from `eye`. The normal need not be unit length.
pub fn shade(light: &Light, material: &Material, point: Vector3, normal: Vector3, eye: Vector3) -> Rgb {
    let n = normal.normalized();
    let to_light = match light.source {
        LightSource::Point(p) => (p - point).normalized(),
        LightSource::Directional(d) => (-d).normalized(),
    };
    let to_eye = (eye - point).normalized();
    let lambert = n.dot(to_light).max(0.0);
    let mut out = light
        .ambient
        .modulate(material.ambient)
        .added(light.diffuse.modulate(material.diffuse).scaled(lambert));
    if lambert > 0.0 {
        let half = (to_light + to_eye).normalized();
        let highlight = n.dot(half).max(0.0).powf(material.shininess);
        out = out.added(light.specular.modulate(material.specular).scaled(highlight));
    }
    out.clamped()
}

/// GGX (Trowbridge-Reitz) normal distribution at `n_dot_h`, the cosine between the surface normal and the half vector.
/// `roughness` is the perceptual roughness in 0..=1; the microfacet alpha is its square.
pub fn ggx_distribution(n_dot_h: f32, roughness: f32) -> f32 {
    let alpha = roughness.clamp(0.0, 1.0).max(0.001).powi(2);
    let a2 = alpha * alpha;
    let d = n_dot_h.clamp(0.0, 1.0);
    let denom = d * d * (a2 - 1.0) + 1.0;
    a2 / (std::f32::consts::PI * denom * denom)
}

/// Smith geometry term for one direction (Schlick-GGX form) at cosine `n_dot_x`.
fn smith_g1(n_dot_x: f32, roughness: f32) -> f32 {
    let r = roughness.clamp(0.0, 1.0) + 1.0;
    let k = r * r / 8.0;
    n_dot_x / (n_dot_x * (1.0 - k) + k)
}

/// Physically based colour of a surface point lit by `light` (metallic-roughness Cook-Torrance). `albedo` is the base
/// colour, `metallic` is 0 for dielectrics and 1 for metals, `roughness` is perceptual in 0..=1. The light's diffuse
/// colour is its radiance and its ambient colour adds albedo-tinted ambient light. The light's specular colour is not
/// used: the specular lobe is GGX coloured by the Schlick Fresnel reflectance. The normal need not be unit length.
pub fn shade_pbr(light: &Light, albedo: Rgb, metallic: f32, roughness: f32, point: Vector3, normal: Vector3, eye: Vector3) -> Rgb {
    let n = normal.normalized();
    let to_light = match light.source {
        LightSource::Point(p) => (p - point).normalized(),
        LightSource::Directional(d) => (-d).normalized(),
    };
    let v = (eye - point).normalized();
    let ambient = light.ambient.modulate(albedo);
    let n_dot_l = n.dot(to_light).max(0.0);
    if n_dot_l <= 0.0 {
        return ambient.clamped();
    }
    let h = (to_light + v).normalized();
    let n_dot_v = n.dot(v).max(1e-4);
    let n_dot_h = n.dot(h).max(0.0);
    let v_dot_h = v.dot(h).max(0.0);
    let metallic = metallic.clamp(0.0, 1.0);
    let d = ggx_distribution(n_dot_h, roughness);
    let g = smith_g1(n_dot_v, roughness) * smith_g1(n_dot_l, roughness);
    let spec_scale = d * g / (4.0 * n_dot_v * n_dot_l);
    let falloff = (1.0 - v_dot_h).clamp(0.0, 1.0).powi(5);
    let channel = |base: f32, radiance: f32| {
        let f0 = 0.04 + (base - 0.04) * metallic;
        let f = f0 + (1.0 - f0) * falloff;
        let diffuse = base * (1.0 - metallic) * (1.0 - f) / std::f32::consts::PI;
        radiance * (diffuse + f * spec_scale) * n_dot_l
    };
    let lit = Rgb::new(
        channel(albedo.r, light.diffuse.r),
        channel(albedo.g, light.diffuse.g),
        channel(albedo.b, light.diffuse.b),
    );
    lit.added(ambient).clamped()
}

/// Distance fog (`NativeEffectPropertyFog`): the fraction of fog colour mixed into a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fog {
    /// Clear at `start`, fully fogged at `end`, linear in between.
    Linear { start: f32, end: f32 },
    /// Fogged amount `1 - e^(-density * distance)`.
    Exponential { density: f32 },
}

impl Fog {
    /// 0 for a clear point, 1 for a fully fogged one, at `distance` from the eye.
    pub fn amount(self, distance: f32) -> f32 {
        match self {
            Fog::Linear { start, end } => {
                if end <= start {
                    return if distance >= end { 1.0 } else { 0.0 };
                }
                ((distance - start) / (end - start)).clamp(0.0, 1.0)
            }
            Fog::Exponential { density } => (1.0 - (-density * distance.max(0.0)).exp()).clamp(0.0, 1.0),
        }
    }

    /// The colour of a point after fog: `color` mixed toward `fog_color` by `amount`.
    pub fn apply(self, color: Rgb, fog_color: Rgb, distance: f32) -> Rgb {
        let a = self.amount(distance);
        color.scaled(1.0 - a).added(fog_color.scaled(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_rgb(actual: Rgb, expected: Rgb) {
        assert!(
            close(actual.r, expected.r) && close(actual.g, expected.g) && close(actual.b, expected.b),
            "expected {expected:?}, got {actual:?}"
        );
    }

    fn matte(ambient: Rgb, diffuse: Rgb) -> Material {
        Material { ambient, diffuse, specular: Rgb::BLACK, shininess: 1.0 }
    }

    #[test]
    fn surface_facing_the_light_gets_full_diffuse() {
        let light = Light {
            source: LightSource::Point(Vector3::new(0.0, 0.0, 10.0)),
            ambient: Rgb::BLACK,
            diffuse: Rgb::new(0.5, 0.5, 0.5),
            specular: Rgb::BLACK,
        };
        let eye = Vector3::new(0.0, 0.0, 10.0);
        let c = shade(&light, &matte(Rgb::WHITE, Rgb::WHITE), Vector3::ZERO, Vector3::new(0.0, 0.0, 1.0), eye);
        assert_rgb(c, Rgb::new(0.5, 0.5, 0.5));
    }

    #[test]
    fn surface_facing_away_gets_only_the_ambient_term() {
        let light = Light {
            source: LightSource::Point(Vector3::new(0.0, 0.0, 10.0)),
            ambient: Rgb::new(0.2, 0.2, 0.2),
            diffuse: Rgb::WHITE,
            specular: Rgb::WHITE,
        };
        let eye = Vector3::new(0.0, 0.0, 10.0);
        let c = shade(&light, &matte(Rgb::WHITE, Rgb::WHITE), Vector3::ZERO, Vector3::new(0.0, 0.0, -1.0), eye);
        assert_rgb(c, Rgb::new(0.2, 0.2, 0.2));
    }

    #[test]
    fn highlight_is_strongest_when_the_eye_sits_on_the_reflection() {
        // A directional light travelling along -z lights a +z-facing surface from +z.
        let light = Light {
            source: LightSource::Directional(Vector3::new(0.0, 0.0, -1.0)),
            ambient: Rgb::BLACK,
            diffuse: Rgb::BLACK,
            specular: Rgb::WHITE,
        };
        let shiny = Material { ambient: Rgb::BLACK, diffuse: Rgb::BLACK, specular: Rgb::WHITE, shininess: 10.0 };
        let n = Vector3::new(0.0, 0.0, 1.0);
        let on_axis = shade(&light, &shiny, Vector3::ZERO, n, Vector3::new(0.0, 0.0, 1.0));
        assert_rgb(on_axis, Rgb::WHITE);
        let off_axis = shade(&light, &shiny, Vector3::ZERO, n, Vector3::new(3.0, 0.0, 1.0));
        assert!(off_axis.r < 0.5, "the highlight should fall away off axis, got {off_axis:?}");
    }

    #[test]
    fn output_is_clamped_to_the_displayable_range() {
        let light = Light {
            source: LightSource::Point(Vector3::new(0.0, 0.0, 1.0)),
            ambient: Rgb::new(2.0, 2.0, 2.0),
            diffuse: Rgb::new(3.0, 0.0, 0.0),
            specular: Rgb::BLACK,
        };
        let c = shade(&light, &matte(Rgb::WHITE, Rgb::WHITE), Vector3::ZERO, Vector3::new(0.0, 0.0, 1.0), Vector3::new(0.0, 0.0, 1.0));
        assert_rgb(c, Rgb::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn linear_fog_ramps_between_start_and_end() {
        let fog = Fog::Linear { start: 10.0, end: 30.0 };
        assert!(close(fog.amount(5.0), 0.0));
        assert!(close(fog.amount(10.0), 0.0));
        assert!(close(fog.amount(20.0), 0.5));
        assert!(close(fog.amount(30.0), 1.0));
        assert!(close(fog.amount(100.0), 1.0));
    }

    #[test]
    fn exponential_fog_is_clear_at_the_eye_and_mixes_colours() {
        let fog = Fog::Exponential { density: 0.1 };
        assert!(close(fog.amount(0.0), 0.0));
        assert!(close(fog.amount(-4.0), 0.0), "negative distances are treated as the eye");
        assert!(fog.amount(100.0) > 0.99);
        let half = Fog::Linear { start: 0.0, end: 2.0 }.apply(Rgb::BLACK, Rgb::WHITE, 1.0);
        assert_rgb(half, Rgb::new(0.5, 0.5, 0.5));
    }
}

#[cfg(test)]
mod pbr_tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn sun_from_above() -> Light {
        Light {
            source: LightSource::Directional(Vector3::new(0.0, 0.0, -1.0)),
            ambient: Rgb::BLACK,
            diffuse: Rgb::WHITE,
            specular: Rgb::BLACK,
        }
    }

    #[test]
    fn ggx_peaks_at_the_normal_and_is_narrower_when_smoother() {
        assert!(ggx_distribution(1.0, 0.5) > ggx_distribution(0.5, 0.5));
        assert!(ggx_distribution(1.0, 0.2) > ggx_distribution(1.0, 0.8));
    }

    #[test]
    fn head_on_rough_white_dielectric_matches_hand_value() {
        // Roughness 1, metallic 0, albedo 1, light and eye on the normal: diffuse (1 - 0.04) / pi plus the specular
        // term 0.04 * (1 / pi) / 4, about 0.3086 per channel.
        let c = shade_pbr(
            &sun_from_above(),
            Rgb::WHITE,
            0.0,
            1.0,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.0, 0.0, 5.0),
        );
        assert!((c.r - 0.3086).abs() < 2e-3, "{}", c.r);
        assert!((c.g - c.r).abs() < 1e-6 && (c.b - c.r).abs() < 1e-6);
    }

    #[test]
    fn light_behind_the_surface_leaves_only_ambient() {
        let mut light = sun_from_above();
        light.source = LightSource::Directional(Vector3::new(0.0, 0.0, 1.0));
        light.ambient = Rgb::new(0.2, 0.2, 0.2);
        let c = shade_pbr(&light, Rgb::new(0.5, 0.5, 0.5), 1.0, 0.3, Vector3::ZERO, Vector3::new(0.0, 0.0, 1.0), Vector3::new(0.0, 0.0, 5.0));
        assert!(near(c.r, 0.1));
    }

    #[test]
    fn metal_has_no_diffuse_so_its_colour_comes_from_the_highlight() {
        // Metallic 1: the diffuse term is zero, and a viewer 80 degrees off the normal sees only a faint highlight.
        let c = shade_pbr(
            &sun_from_above(),
            Rgb::new(0.5, 0.5, 0.5),
            1.0,
            0.2,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.985, 0.0, 0.174),
        );
        assert!(c.r < 0.01, "{}", c.r);
    }
}
