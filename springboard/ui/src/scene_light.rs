//! Light sources for 3D scenes (`NativeLight`, `NativeLightType`, `NativePhysicallyPlausibleLight`). A light has a type,
//! a colour, a position and direction in its scene, a total intensity in lumens, and for the point and spot types a
//! distance range over which it fades out. `irradiance_at` gives the light arriving at a point. Only the ambient,
//! directional, point and spot types are computed; the other types return `None`. The fade is linear between the two
//! attenuation distances and is this model's own rule, not a copy of any file format's falloff. Lighting a surface is
//! done by `shading.rs`, which this module does not depend on.

use crate::linalg::{radians_to_degrees, Vector3};

/// The shape and style of illumination (`NativeLightType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetLightType {
    Unknown,
    Ambient,
    Directional,
    Spot,
    Point,
    Linear,
    DiscArea,
    RectangularArea,
    SuperElliptical,
    Photometric,
    Probe,
    Environment,
}

/// One light source in a scene (`NativeLight` with the physically plausible fields).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssetLight {
    pub kind: AssetLightType,
    /// Position in the light's scene space. Used by the point and spot types.
    pub position: Vector3,
    /// Direction the light points along. Used by the spot and directional types. Need not be unit length.
    pub direction: Vector3,
    /// Linear colour, each channel 0..=1.
    pub color: [f32; 3],
    /// Total intensity in lumens.
    pub lumens: f32,
    /// Spot cone: full strength within `inner_cone_deg` of the direction, none beyond `outer_cone_deg`.
    pub inner_cone_deg: f32,
    pub outer_cone_deg: f32,
    /// Distances from the light at which the falloff starts and at which it reaches zero.
    pub attenuation_start: f32,
    pub attenuation_end: f32,
}

impl AssetLight {
    /// A white light of the given type, 1 lumen, pointing down the negative z axis, with no falloff.
    pub fn new(kind: AssetLightType) -> Self {
        AssetLight {
            kind,
            position: Vector3::ZERO,
            direction: Vector3::new(0.0, 0.0, -1.0),
            color: [1.0, 1.0, 1.0],
            lumens: 1.0,
            inner_cone_deg: 0.0,
            outer_cone_deg: 0.0,
            attenuation_start: 0.0,
            attenuation_end: f32::INFINITY,
        }
    }

    /// The light arriving at `point`, in lumens scaled by the falloff and the cone. `None` for a type this model does not
    /// compute. Ambient and directional light arrive the same everywhere.
    pub fn irradiance_at(&self, point: Vector3) -> Option<f32> {
        match self.kind {
            AssetLightType::Ambient | AssetLightType::Directional => Some(self.lumens),
            AssetLightType::Point => Some(self.lumens * self.falloff(point)),
            AssetLightType::Spot => Some(self.lumens * self.falloff(point) * self.cone_factor(point)),
            _ => None,
        }
    }

    /// 1 up to `attenuation_start`, 0 from `attenuation_end` on, linear in between.
    fn falloff(&self, point: Vector3) -> f32 {
        let d = (point - self.position).length();
        if d <= self.attenuation_start {
            1.0
        } else if d >= self.attenuation_end {
            0.0
        } else {
            1.0 - (d - self.attenuation_start) / (self.attenuation_end - self.attenuation_start)
        }
    }

    /// 1 inside `inner_cone_deg`, 0 beyond `outer_cone_deg`, linear in the angle between them. A zero direction gives 0.
    fn cone_factor(&self, point: Vector3) -> f32 {
        let axis = self.direction.normalized();
        if axis == Vector3::ZERO {
            return 0.0;
        }
        let to_point = point - self.position;
        if to_point.length() == 0.0 {
            return 1.0;
        }
        let cos = axis.dot(to_point.normalized()).clamp(-1.0, 1.0);
        let angle = radians_to_degrees(cos.acos());
        if angle <= self.inner_cone_deg {
            1.0
        } else if angle >= self.outer_cone_deg {
            0.0
        } else {
            1.0 - (angle - self.inner_cone_deg) / (self.outer_cone_deg - self.inner_cone_deg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn at(l: &AssetLight, p: Vector3) -> f32 {
        l.irradiance_at(p).expect("type is computed")
    }

    #[test]
    fn ambient_light_is_the_same_everywhere() {
        let mut l = AssetLight::new(AssetLightType::Ambient);
        l.lumens = 40.0;
        assert!(near(at(&l, Vector3::ZERO), 40.0));
        assert!(near(at(&l, Vector3::new(1000.0, -3.0, 7.0)), 40.0));
    }

    #[test]
    fn directional_light_ignores_position() {
        let mut l = AssetLight::new(AssetLightType::Directional);
        l.lumens = 25.0;
        l.position = Vector3::new(9.0, 9.0, 9.0);
        assert!(near(at(&l, Vector3::new(-4.0, 0.0, 2.0)), 25.0));
    }

    #[test]
    fn point_light_fades_linearly_between_its_attenuation_distances() {
        let mut l = AssetLight::new(AssetLightType::Point);
        l.lumens = 100.0;
        l.attenuation_start = 2.0;
        l.attenuation_end = 10.0;
        // Inside the start distance: full strength.
        assert!(near(at(&l, Vector3::new(1.0, 0.0, 0.0)), 100.0));
        // Distance 6 is 4 of the 8 units from start to end: half strength.
        assert!(near(at(&l, Vector3::new(6.0, 0.0, 0.0)), 50.0));
        // At and beyond the end distance: nothing.
        assert!(near(at(&l, Vector3::new(10.0, 0.0, 0.0)), 0.0));
        assert!(near(at(&l, Vector3::new(0.0, 12.0, 0.0)), 0.0));
    }

    #[test]
    fn point_light_position_is_the_origin_of_the_distance() {
        let mut l = AssetLight::new(AssetLightType::Point);
        l.position = Vector3::new(5.0, 0.0, 0.0);
        l.attenuation_start = 0.0;
        l.attenuation_end = 4.0;
        // The point is 2 units from the light: 1 - 2/4 = 0.5 of full strength.
        assert!(near(at(&l, Vector3::new(7.0, 0.0, 0.0)), 0.5));
    }

    #[test]
    fn spot_light_full_on_axis_half_way_across_its_cone_edge() {
        let mut l = AssetLight::new(AssetLightType::Spot);
        l.lumens = 80.0;
        l.direction = Vector3::new(0.0, 0.0, -1.0);
        l.inner_cone_deg = 30.0;
        l.outer_cone_deg = 60.0;
        // Attenuation distances far beyond these points, so only the cone changes the result.
        l.attenuation_start = 100.0;
        l.attenuation_end = 200.0;
        // Straight down the axis: angle 0, inside the inner cone.
        assert!(near(at(&l, Vector3::new(0.0, 0.0, -5.0)), 80.0));
        // (5, 0, -5) is 45 degrees off the axis: cone 1 - (45 - 30) / 30 = 0.5, so 40 lumens.
        assert!(near(at(&l, Vector3::new(5.0, 0.0, -5.0)), 40.0));
        // Straight to the side is 90 degrees: beyond the outer cone.
        assert!(near(at(&l, Vector3::new(5.0, 0.0, 0.0)), 0.0));
    }

    #[test]
    fn spot_light_with_zero_direction_gives_nothing() {
        let mut l = AssetLight::new(AssetLightType::Spot);
        l.direction = Vector3::ZERO;
        l.inner_cone_deg = 30.0;
        l.outer_cone_deg = 60.0;
        l.attenuation_start = 100.0;
        l.attenuation_end = 200.0;
        assert!(near(at(&l, Vector3::new(0.0, 0.0, -5.0)), 0.0));
    }

    #[test]
    fn types_not_modelled_return_none() {
        let l = AssetLight::new(AssetLightType::Linear);
        assert_eq!(l.irradiance_at(Vector3::ZERO), None);
        let p = AssetLight::new(AssetLightType::Probe);
        assert_eq!(p.irradiance_at(Vector3::new(1.0, 2.0, 3.0)), None);
    }
}
