//! Shader-free effects as a pure model: the base effect (`NativeBaseEffect`) that transforms a vertex, lights it with up to
//! three lights and fogs it, the reflection vector used by a reflection-map effect, and the cube-map face lookup used by a
//! reflection-map or skybox effect. A renderer supplies the GPU work. This module supplies no shader source and creates no
//! GPU object: a caller that wants a custom shader still has no app-facing path for one.
//!
//! Conventions follow the GL clip and cube-map rules: eye space looks down the negative z axis, the eye sits at the origin
//! of eye space, and matrices are column-major (see `linalg::Matrix4`).

use crate::linalg::{Matrix4, Vector3, Vector4};
use crate::shading::{shade, Fog, Light, Material, Rgb};

/// The number of lights a base effect holds (`NativeBaseEffect` has three light slots).
pub const MAX_LIGHTS: usize = 3;

/// Why an effect setting is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectError {
    /// The light slot is not 0, 1 or 2. Carries the index given.
    LightIndex(usize),
}

/// The transforms, lights, material and fog of a base effect. With no light set, a vertex takes the material's diffuse
/// colour unlit, as a fixed-function pipeline does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseEffect {
    pub projection: Matrix4,
    pub model_view: Matrix4,
    pub lights: [Option<Light>; MAX_LIGHTS],
    pub material: Material,
    /// The fog model and the colour fog mixes toward. `None` disables fog.
    pub fog: Option<(Fog, Rgb)>,
}

impl Default for BaseEffect {
    fn default() -> Self {
        BaseEffect {
            projection: Matrix4::identity(),
            model_view: Matrix4::identity(),
            lights: [None; MAX_LIGHTS],
            material: Material {
                ambient: Rgb::WHITE,
                diffuse: Rgb::WHITE,
                specular: Rgb::BLACK,
                shininess: 1.0,
            },
            fog: None,
        }
    }
}

impl BaseEffect {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets or clears one light slot. Slots past `MAX_LIGHTS` are refused.
    pub fn set_light(&mut self, slot: usize, light: Option<Light>) -> Result<(), EffectError> {
        let entry = self.lights.get_mut(slot).ok_or(EffectError::LightIndex(slot))?;
        *entry = light;
        Ok(())
    }

    /// True when at least one light slot is set.
    pub fn lighting_enabled(&self) -> bool {
        self.lights.iter().any(Option::is_some)
    }

    /// The position of a model-space point in eye space.
    pub fn eye_position(&self, point: Vector3) -> Vector3 {
        self.model_view.transform(point.to_vector4(1.0)).xyz()
    }

    /// The normal of a model-space normal in eye space: the inverse transpose of the model-view matrix applied to a
    /// direction. A singular model-view matrix leaves the normal unchanged.
    pub fn eye_normal(&self, normal: Vector3) -> Vector3 {
        match self.model_view.inverse() {
            Some(inverse) => inverse.transposed().transform(normal.to_vector4(0.0)).xyz(),
            None => normal,
        }
    }

    /// The clip-space position of a model-space point: projection times model-view times the point.
    pub fn clip_position(&self, point: Vector3) -> Vector4 {
        (self.projection * self.model_view).transform(point.to_vector4(1.0))
    }

    /// The colour of a vertex: the sum of every set light (clamped once at the end), or the material diffuse colour when
    /// no light is set, then fogged by distance from the eye.
    pub fn shade_vertex(&self, point: Vector3, normal: Vector3) -> Rgb {
        let eye_point = self.eye_position(point);
        let color = if self.lighting_enabled() {
            let eye_normal = self.eye_normal(normal);
            let eye = Vector3::ZERO;
            self.lights
                .iter()
                .flatten()
                .fold(Rgb::BLACK, |sum, light| sum.added(shade(light, &self.material, eye_point, eye_normal, eye)))
                .clamped()
        } else {
            self.material.diffuse
        };
        match self.fog {
            Some((fog, fog_color)) => fog.apply(color, fog_color, eye_point.length()),
            None => color,
        }
    }
}

/// The reflection of an incident direction about a surface normal (the GL `reflect` rule). The normal is normalized first.
pub fn reflect(incident: Vector3, normal: Vector3) -> Vector3 {
    let n = normal.normalized();
    let d = 2.0 * incident.dot(n);
    Vector3::new(incident.x - d * n.x, incident.y - d * n.y, incident.z - d * n.z)
}

/// A face of a cube map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubeFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

/// A cube-map lookup: the face and the texture coordinates on it, each in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeCoord {
    pub face: CubeFace,
    pub s: f32,
    pub t: f32,
}

/// The cube-map face and coordinates a direction hits, by the largest component of the direction (the GL cube-map table).
/// A zero direction maps to the positive x face at the centre.
pub fn cube_lookup(direction: Vector3) -> CubeCoord {
    let (x, y, z) = (direction.x, direction.y, direction.z);
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, sc, tc, major) = if ax >= ay && ax >= az {
        if x >= 0.0 {
            (CubeFace::PositiveX, -z, -y, ax)
        } else {
            (CubeFace::NegativeX, z, -y, ax)
        }
    } else if ay >= az {
        if y >= 0.0 {
            (CubeFace::PositiveY, x, z, ay)
        } else {
            (CubeFace::NegativeY, x, -z, ay)
        }
    } else if z >= 0.0 {
        (CubeFace::PositiveZ, x, -y, az)
    } else {
        (CubeFace::NegativeZ, -x, -y, az)
    };
    if major == 0.0 {
        return CubeCoord { face, s: 0.5, t: 0.5 };
    }
    CubeCoord { face, s: 0.5 * (sc / major + 1.0), t: 0.5 * (tc / major + 1.0) }
}

/// The world-space direction seen through a normalized device coordinate (`ndc_x`, `ndc_y` in -1..=1) for a view with the
/// given projection and view matrices. This is the ray a skybox samples. `None` when either matrix is singular or the
/// unprojected point is at infinity.
pub fn skybox_direction(projection: Matrix4, view: Matrix4, ndc_x: f32, ndc_y: f32) -> Option<Vector3> {
    let inverse_projection = projection.inverse()?;
    let inverse_view = view.inverse()?;
    // A point on the far plane (NDC z = 1) in eye space; dividing by w gives the eye-space point.
    let far = inverse_projection.transform(Vector4::new(ndc_x, ndc_y, 1.0, 1.0));
    if far.w == 0.0 || !far.w.is_finite() {
        return None;
    }
    let eye_point = far.xyz() * (1.0 / far.w);
    let world = inverse_view.transform(eye_point.to_vector4(0.0)).xyz();
    let len = world.length();
    if len == 0.0 || !len.is_finite() {
        return None;
    }
    Some(world.normalized())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::LightSource;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_vec(a: Vector3, b: Vector3) {
        assert!(close(a.x, b.x) && close(a.y, b.y) && close(a.z, b.z), "{a:?} != {b:?}");
    }

    fn assert_rgb(a: Rgb, b: Rgb) {
        assert!(close(a.r, b.r) && close(a.g, b.g) && close(a.b, b.b), "{a:?} != {b:?}");
    }

    fn head_on_light() -> Light {
        Light {
            source: LightSource::Directional(Vector3::new(0.0, 0.0, -1.0)),
            ambient: Rgb::BLACK,
            diffuse: Rgb::WHITE,
            specular: Rgb::BLACK,
        }
    }

    #[test]
    fn unlit_vertex_takes_the_material_diffuse_colour() {
        let mut e = BaseEffect::new();
        e.material.diffuse = Rgb::new(0.2, 0.4, 0.6);
        assert_rgb(e.shade_vertex(Vector3::ZERO, Vector3::new(0.0, 0.0, 1.0)), Rgb::new(0.2, 0.4, 0.6));
    }

    #[test]
    fn a_light_facing_the_surface_gives_full_diffuse() {
        let mut e = BaseEffect::new();
        e.set_light(0, Some(head_on_light())).unwrap();
        // The surface faces +z, the light travels along -z, so the light hits it head on.
        assert_rgb(e.shade_vertex(Vector3::ZERO, Vector3::new(0.0, 0.0, 1.0)), Rgb::WHITE);
        // A surface facing away from the light gets nothing from it.
        assert_rgb(e.shade_vertex(Vector3::ZERO, Vector3::new(0.0, 0.0, -1.0)), Rgb::BLACK);
    }

    #[test]
    fn lights_add_and_the_sum_is_clamped() {
        let mut e = BaseEffect::new();
        e.set_light(0, Some(head_on_light())).unwrap();
        e.set_light(1, Some(head_on_light())).unwrap();
        assert_rgb(e.shade_vertex(Vector3::ZERO, Vector3::new(0.0, 0.0, 1.0)), Rgb::WHITE);
        assert!(e.lighting_enabled());
        e.set_light(0, None).unwrap();
        e.set_light(1, None).unwrap();
        assert!(!e.lighting_enabled());
    }

    #[test]
    fn light_slots_past_the_maximum_are_refused() {
        let mut e = BaseEffect::new();
        assert_eq!(e.set_light(MAX_LIGHTS, Some(head_on_light())), Err(EffectError::LightIndex(MAX_LIGHTS)));
        assert_eq!(e.set_light(2, Some(head_on_light())), Ok(()));
    }

    #[test]
    fn fog_mixes_the_colour_toward_the_fog_colour_by_eye_distance() {
        let mut e = BaseEffect::new();
        e.material.diffuse = Rgb::WHITE;
        e.fog = Some((Fog::Linear { start: 0.0, end: 10.0 }, Rgb::BLACK));
        // A point 5 units in front of the eye is half fogged.
        let mid = e.shade_vertex(Vector3::new(0.0, 0.0, -5.0), Vector3::new(0.0, 0.0, 1.0));
        assert_rgb(mid, Rgb::new(0.5, 0.5, 0.5));
    }

    #[test]
    fn model_view_moves_the_point_into_eye_space() {
        let mut e = BaseEffect::new();
        e.model_view = Matrix4::translation(0.0, 0.0, -4.0);
        assert_vec(e.eye_position(Vector3::ZERO), Vector3::new(0.0, 0.0, -4.0));
    }

    #[test]
    fn normals_under_a_uniform_scale_stay_unit_length_direction() {
        let mut e = BaseEffect::new();
        e.model_view = Matrix4::scaling(2.0, 2.0, 2.0);
        let n = e.eye_normal(Vector3::new(0.0, 1.0, 0.0));
        assert_vec(n.normalized(), Vector3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn clip_position_applies_projection_after_model_view() {
        let mut e = BaseEffect::new();
        e.model_view = Matrix4::translation(0.0, 0.0, -1.0);
        let c = e.clip_position(Vector3::ZERO);
        assert!(close(c.x, 0.0) && close(c.y, 0.0) && close(c.z, -1.0) && close(c.w, 1.0));
    }

    #[test]
    fn reflect_bounces_a_direction_off_a_plane() {
        // Coming down at 45 degrees onto a floor that faces up.
        let r = reflect(Vector3::new(1.0, -1.0, 0.0).normalized(), Vector3::new(0.0, 1.0, 0.0));
        assert_vec(r, Vector3::new(1.0, 1.0, 0.0).normalized());
        // A direction along the normal reverses.
        assert_vec(reflect(Vector3::new(0.0, -1.0, 0.0), Vector3::new(0.0, 1.0, 0.0)), Vector3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn the_axis_directions_pick_their_faces_and_centre() {
        assert_eq!(cube_lookup(Vector3::new(1.0, 0.0, 0.0)), CubeCoord { face: CubeFace::PositiveX, s: 0.5, t: 0.5 });
        assert_eq!(cube_lookup(Vector3::new(-2.0, 0.0, 0.0)).face, CubeFace::NegativeX);
        assert_eq!(cube_lookup(Vector3::new(0.0, 3.0, 0.0)).face, CubeFace::PositiveY);
        assert_eq!(cube_lookup(Vector3::new(0.0, -3.0, 0.0)).face, CubeFace::NegativeY);
        assert_eq!(cube_lookup(Vector3::new(0.0, 0.0, 1.0)).face, CubeFace::PositiveZ);
        assert_eq!(cube_lookup(Vector3::new(0.0, 0.0, -1.0)).face, CubeFace::NegativeZ);
    }

    #[test]
    fn a_point_off_centre_on_a_face_gets_its_coordinates() {
        // On the +X face, the major axis is x = 1; y = 0.5 gives t = 0.5 * (-0.5 + 1) = 0.25.
        let c = cube_lookup(Vector3::new(1.0, 0.5, 0.0));
        assert_eq!(c.face, CubeFace::PositiveX);
        assert!(close(c.s, 0.5) && close(c.t, 0.25));
    }

    #[test]
    fn skybox_centre_with_identity_matrices_looks_down_positive_z() {
        let d = skybox_direction(Matrix4::identity(), Matrix4::identity(), 0.0, 0.0).unwrap();
        assert_vec(d, Vector3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn skybox_direction_follows_the_inverse_view_rotation() {
        // A view translated along z does not change which way the ray points.
        let d = skybox_direction(Matrix4::identity(), Matrix4::translation(0.0, 0.0, -5.0), 0.0, 0.0).unwrap();
        assert_vec(d, Vector3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn a_singular_view_has_no_skybox_direction() {
        assert_eq!(skybox_direction(Matrix4::identity(), Matrix4 { m: [0.0; 16] }, 0.0, 0.0), None);
    }
}
