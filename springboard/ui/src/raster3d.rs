//! CPU triangle rasteriser for the 3D scene models: flat-shaded triangles drawn into a colour target with a depth
//! buffer. Each triangle is lit once at its centroid with `shading::shade`, so the colour comes from the same lighting
//! model the scene already uses. Triangles are projected with a view-projection matrix; any triangle with a vertex at
//! or behind the eye plane (`w <= 0`) is skipped rather than clipped. This is the CPU stand-in for a GPU rasteriser.

use crate::linalg::{Matrix4, Vector3};
use crate::shading::{shade, Light, Material, Rgb};

/// One world-space triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub a: Vector3,
    pub b: Vector3,
    pub c: Vector3,
}

/// A colour buffer with a matching depth buffer. Depth is normalised device depth, -1 (near) to 1 (far).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    width: usize,
    height: usize,
    color: Vec<Rgb>,
    depth: Vec<f32>,
}

impl Target {
    /// A target filled with `clear` colour and no depth written.
    pub fn new(width: usize, height: usize, clear: Rgb) -> Target {
        let count = width * height;
        Target { width, height, color: vec![clear; count], depth: vec![f32::INFINITY; count] }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The colour at a pixel, or `None` outside the target.
    pub fn color_at(&self, x: usize, y: usize) -> Option<Rgb> {
        self.index(x, y).map(|i| self.color[i])
    }

    /// The depth at a pixel, or `None` outside the target. Pixels nothing was drawn to read as infinity.
    pub fn depth_at(&self, x: usize, y: usize) -> Option<f32> {
        self.index(x, y).map(|i| self.depth[i])
    }

    fn index(&self, x: usize, y: usize) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(y * self.width + x)
        } else {
            None
        }
    }

    /// Writes a pixel when it is nearer than what is stored there.
    fn put(&mut self, x: usize, y: usize, depth: f32, color: Rgb) {
        if let Some(i) = self.index(x, y) {
            if depth < self.depth[i] {
                self.depth[i] = depth;
                self.color[i] = color;
            }
        }
    }
}

/// Draws `triangles` into `target`. `view_projection` maps world space to clip space, `eye` is the world-space
/// viewpoint used for the specular term, and each triangle is shaded once with `light` and `material`.
pub fn draw(
    target: &mut Target,
    triangles: &[Triangle],
    view_projection: Matrix4,
    eye: Vector3,
    light: &Light,
    material: &Material,
) {
    if target.width == 0 || target.height == 0 {
        return;
    }
    let (w, h) = (target.width as f32, target.height as f32);
    for tri in triangles {
        let normal = (tri.b - tri.a).cross(tri.c - tri.a);
        if normal.length() <= f32::EPSILON {
            continue;
        }
        let Some(pa) = project(view_projection, tri.a) else { continue };
        let Some(pb) = project(view_projection, tri.b) else { continue };
        let Some(pc) = project(view_projection, tri.c) else { continue };

        // Screen space: x grows to the right, y grows downward, pixel centres sit at +0.5.
        let to_screen = |p: Vector3| ((p.x * 0.5 + 0.5) * w, (1.0 - (p.y * 0.5 + 0.5)) * h);
        let (sa, sb, sc) = (to_screen(pa), to_screen(pb), to_screen(pc));
        let area = edge(sa, sb, sc);
        if area.abs() <= f32::EPSILON {
            continue;
        }

        let color = shade(light, material, centroid(tri), normal, eye).clamped();
        let min_x = sa.0.min(sb.0).min(sc.0).floor().max(0.0) as usize;
        let min_y = sa.1.min(sb.1).min(sc.1).floor().max(0.0) as usize;
        let max_x = (sa.0.max(sb.0).max(sc.0).ceil().max(0.0) as usize).min(target.width.saturating_sub(1));
        let max_y = (sa.1.max(sb.1).max(sc.1).ceil().max(0.0) as usize).min(target.height.saturating_sub(1));
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                // Barycentric weights; the sign of `area` makes them positive inside for either winding.
                let wa = edge(sb, sc, p) / area;
                let wb = edge(sc, sa, p) / area;
                let wc = edge(sa, sb, p) / area;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let z = wa * pa.z + wb * pb.z + wc * pc.z;
                if !(-1.0..=1.0).contains(&z) {
                    continue;
                }
                target.put(x, y, z, color);
            }
        }
    }
}

/// Projects a world point to normalised device coordinates. `None` when the point is at or behind the eye plane.
fn project(view_projection: Matrix4, p: Vector3) -> Option<Vector3> {
    let v = view_projection.transform(p.to_vector4(1.0));
    if v.w <= 0.0 {
        return None;
    }
    Some(Vector3::new(v.x / v.w, v.y / v.w, v.z / v.w))
}

/// Twice the signed area of the triangle (a, b, p) in screen space: positive when `p` is left of a->b.
fn edge(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

fn centroid(tri: &Triangle) -> Vector3 {
    (tri.a + tri.b + tri.c) * (1.0 / 3.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::LightSource;

    /// A light that contributes only ambient colour, so the drawn colour is the material's ambient term exactly.
    fn ambient_only(colour: Rgb) -> (Light, Material) {
        let light = Light {
            source: LightSource::Directional(Vector3::new(0.0, 0.0, -1.0)),
            ambient: Rgb::WHITE,
            diffuse: Rgb::BLACK,
            specular: Rgb::BLACK,
        };
        let material = Material { ambient: colour, diffuse: Rgb::BLACK, specular: Rgb::BLACK, shininess: 1.0 };
        (light, material)
    }

    /// A triangle that covers the whole -1..1 clip square at depth `z` under the identity matrix.
    fn full_screen(z: f32) -> Triangle {
        Triangle {
            a: Vector3::new(-1.0, -1.0, z),
            b: Vector3::new(3.0, -1.0, z),
            c: Vector3::new(-1.0, 3.0, z),
        }
    }

    #[test]
    fn full_screen_triangle_fills_every_pixel() {
        let (light, material) = ambient_only(Rgb::new(0.2, 0.4, 0.6));
        let mut target = Target::new(8, 4, Rgb::BLACK);
        draw(&mut target, &[full_screen(0.0)], Matrix4::identity(), Vector3::new(0.0, 0.0, 10.0), &light, &material);
        for y in 0..4 {
            for x in 0..8 {
                assert_eq!(target.color_at(x, y), Some(Rgb::new(0.2, 0.4, 0.6)), "pixel {x},{y}");
                assert_eq!(target.depth_at(x, y), Some(0.0));
            }
        }
    }

    #[test]
    fn nearer_triangle_wins_regardless_of_draw_order() {
        let eye = Vector3::new(0.0, 0.0, 10.0);
        let (near_light, near_material) = ambient_only(Rgb::new(1.0, 0.0, 0.0));
        let (far_light, far_material) = ambient_only(Rgb::new(0.0, 0.0, 1.0));
        let near = full_screen(-0.5);
        let far = full_screen(0.5);

        let mut near_first = Target::new(4, 4, Rgb::BLACK);
        draw(&mut near_first, &[near], Matrix4::identity(), eye, &near_light, &near_material);
        draw(&mut near_first, &[far], Matrix4::identity(), eye, &far_light, &far_material);

        let mut far_first = Target::new(4, 4, Rgb::BLACK);
        draw(&mut far_first, &[far], Matrix4::identity(), eye, &far_light, &far_material);
        draw(&mut far_first, &[near], Matrix4::identity(), eye, &near_light, &near_material);

        assert_eq!(near_first.color_at(2, 2), Some(Rgb::new(1.0, 0.0, 0.0)));
        assert_eq!(far_first.color_at(2, 2), Some(Rgb::new(1.0, 0.0, 0.0)));
    }

    #[test]
    fn triangle_outside_the_view_writes_nothing() {
        let (light, material) = ambient_only(Rgb::WHITE);
        let mut target = Target::new(4, 4, Rgb::BLACK);
        let off = Triangle {
            a: Vector3::new(5.0, 0.0, 0.0),
            b: Vector3::new(6.0, 0.0, 0.0),
            c: Vector3::new(5.0, 1.0, 0.0),
        };
        draw(&mut target, &[off], Matrix4::identity(), Vector3::new(0.0, 0.0, 10.0), &light, &material);
        assert_eq!(target.color_at(1, 1), Some(Rgb::BLACK));
        assert_eq!(target.depth_at(1, 1), Some(f32::INFINITY));
    }

    #[test]
    fn degenerate_triangle_is_skipped() {
        let (light, material) = ambient_only(Rgb::WHITE);
        let mut target = Target::new(4, 4, Rgb::BLACK);
        let line = Triangle {
            a: Vector3::new(-1.0, 0.0, 0.0),
            b: Vector3::new(0.0, 0.0, 0.0),
            c: Vector3::new(1.0, 0.0, 0.0),
        };
        draw(&mut target, &[line], Matrix4::identity(), Vector3::new(0.0, 0.0, 10.0), &light, &material);
        assert_eq!(target.color_at(2, 2), Some(Rgb::BLACK));
    }

    #[test]
    fn point_behind_the_eye_plane_skips_the_triangle() {
        // Perspective projection: a point at z = +5 lies behind the camera looking down -Z (w <= 0).
        let (light, material) = ambient_only(Rgb::WHITE);
        let mut target = Target::new(4, 4, Rgb::BLACK);
        let behind = Triangle {
            a: Vector3::new(-1.0, -1.0, 5.0),
            b: Vector3::new(1.0, -1.0, 5.0),
            c: Vector3::new(0.0, 1.0, 5.0),
        };
        let vp = Matrix4::perspective(1.0, 1.0, 0.1, 100.0);
        draw(&mut target, &[behind], vp, Vector3::new(0.0, 0.0, 10.0), &light, &material);
        assert_eq!(target.color_at(2, 2), Some(Rgb::BLACK));
    }

    #[test]
    fn empty_target_is_a_no_op() {
        let (light, material) = ambient_only(Rgb::WHITE);
        let mut target = Target::new(0, 0, Rgb::BLACK);
        draw(&mut target, &[full_screen(0.0)], Matrix4::identity(), Vector3::new(0.0, 0.0, 10.0), &light, &material);
        assert_eq!(target.color_at(0, 0), None);
    }
}
