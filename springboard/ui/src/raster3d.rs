//! CPU triangle rasteriser for the 3D scene models: triangles drawn into a colour target with a depth buffer. Two
//! paths share one scan converter. `draw` is the flat path: each triangle is lit once at its centroid with
//! `shading::shade`. `draw_shaded` is the per-pixel path: vertices carry a normal and a texture coordinate, which are
//! interpolated perspective-correctly, and every pixel is lit with either the Blinn-Phong model (`Surface::Phong`) or
//! the metallic-roughness GGX model (`Surface::Pbr`, `shading::shade_pbr`), with the base colour optionally read from
//! a `Texture` (bilinear, repeat or clamp). Both clip triangles against the near plane before projecting, so a
//! triangle that crosses the eye plane is cut instead of dropped. This is the CPU stand-in for a GPU rasteriser; the
//! GPU port is not built yet.

use crate::linalg::{Matrix4, Vector3, Vector4};
use crate::shading::{shade, shade_pbr, Light, Material, Rgb};

/// One world-space triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub a: Vector3,
    pub b: Vector3,
    pub c: Vector3,
}

/// A mesh vertex for the per-pixel path: world position, world-space normal (need not be unit length; a zero normal
/// falls back to the triangle's face normal) and a texture coordinate in 0..1 across the texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: Vector3,
    pub normal: Vector3,
    pub uv: (f32, f32),
}

/// One world-space triangle with per-vertex attributes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshTriangle {
    pub a: Vertex,
    pub b: Vertex,
    pub c: Vertex,
}

/// What happens to a texture coordinate outside 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    /// The image tiles.
    Repeat,
    /// The edge texels extend.
    Clamp,
}

/// A colour image sampled with bilinear filtering. Texel `(0, 0)` is at `uv = (0, 0)`; `v` grows down the rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    width: usize,
    height: usize,
    texels: Vec<Rgb>,
    wrap: Wrap,
}

impl Texture {
    /// A texture of `width` by `height` texels in row-major order. `None` when either size is zero or the number of
    /// texels does not match.
    pub fn new(width: usize, height: usize, texels: Vec<Rgb>, wrap: Wrap) -> Option<Texture> {
        (width > 0 && height > 0 && texels.len() == width * height).then_some(Texture { width, height, texels, wrap })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The texel at integer coordinates, wrapped by the texture's mode.
    fn texel(&self, x: i64, y: i64) -> Rgb {
        let (w, h) = (self.width as i64, self.height as i64);
        let (x, y) = match self.wrap {
            Wrap::Repeat => (x.rem_euclid(w), y.rem_euclid(h)),
            Wrap::Clamp => (x.clamp(0, w - 1), y.clamp(0, h - 1)),
        };
        self.texels[(y * w + x) as usize]
    }

    /// The bilinearly filtered colour at `(u, v)`. Texel centres sit at half-integer positions, so sampling exactly
    /// at a centre returns that texel.
    pub fn sample(&self, u: f32, v: f32) -> Rgb {
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.texel(x0, y0).scaled(1.0 - tx).added(self.texel(x0 + 1, y0).scaled(tx));
        let bottom = self.texel(x0, y0 + 1).scaled(1.0 - tx).added(self.texel(x0 + 1, y0 + 1).scaled(tx));
        top.scaled(1.0 - ty).added(bottom.scaled(ty))
    }
}

/// How the per-pixel path lights a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    /// Ambient plus Lambert plus Blinn-Phong (`shading::shade`). A texture multiplies the ambient and diffuse colours.
    Phong(Material),
    /// Metallic-roughness GGX (`shading::shade_pbr`). A texture multiplies the albedo.
    Pbr { albedo: Rgb, metallic: f32, roughness: f32 },
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

/// A vertex in clip space with the attributes the per-pixel path interpolates.
#[derive(Clone, Copy, Debug)]
struct ClipVertex {
    clip: Vector4,
    world: Vector3,
    normal: Vector3,
    uv: (f32, f32),
}

impl ClipVertex {
    fn new(view_projection: Matrix4, v: Vertex) -> ClipVertex {
        ClipVertex { clip: view_projection.transform(v.position.to_vector4(1.0)), world: v.position, normal: v.normal, uv: v.uv }
    }

    fn lerp(self, o: ClipVertex, t: f32) -> ClipVertex {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        ClipVertex {
            clip: Vector4::new(mix(self.clip.x, o.clip.x), mix(self.clip.y, o.clip.y), mix(self.clip.z, o.clip.z), mix(self.clip.w, o.clip.w)),
            world: self.world + (o.world - self.world) * t,
            normal: self.normal + (o.normal - self.normal) * t,
            uv: (mix(self.uv.0, o.uv.0), mix(self.uv.1, o.uv.1)),
        }
    }
}

/// The smallest clip-space `w` kept after clipping, so the perspective divide never divides by zero.
const MIN_W: f32 = 1e-5;

/// Clips a convex polygon against the near plane (`z >= -w`) and the eye plane (`w >= MIN_W`) (Sutherland-Hodgman).
/// A polygon wholly in front comes back unchanged, one crossing a plane comes back with the part in front (a triangle
/// with one vertex behind becomes a quad), and one wholly behind comes back empty.
fn clip_near(poly: &[ClipVertex]) -> Vec<ClipVertex> {
    let planes: [fn(&ClipVertex) -> f32; 2] = [|v| v.clip.z + v.clip.w, |v| v.clip.w - MIN_W];
    let mut current = poly.to_vec();
    for distance in planes {
        let mut next = Vec::with_capacity(current.len() + 1);
        for i in 0..current.len() {
            let (a, b) = (current[i], current[(i + 1) % current.len()]);
            let (da, db) = (distance(&a), distance(&b));
            if da >= 0.0 {
                next.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                next.push(a.lerp(b, da / (da - db)));
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current
}

/// Perspective-correct weights from screen-space barycentric weights `bary` and the clip-space `w` of each vertex:
/// each weight is divided by its `w` and the three are renormalised to sum to one. Equal `w` leaves `bary` unchanged.
fn perspective_weights(bary: [f32; 3], w: [f32; 3]) -> [f32; 3] {
    let scaled = [bary[0] / w[0], bary[1] / w[1], bary[2] / w[2]];
    let sum = scaled[0] + scaled[1] + scaled[2];
    if sum.abs() <= f32::EPSILON {
        return bary;
    }
    [scaled[0] / sum, scaled[1] / sum, scaled[2] / sum]
}

/// Scan-converts the (already clipped) triangle `v` into `target`. For every covered pixel that passes the depth range
/// `shade_pixel` is called with the perspective-correct weights of the three vertices and returns the pixel colour.
fn rasterise(target: &mut Target, v: [ClipVertex; 3], mut shade_pixel: impl FnMut([f32; 3]) -> Rgb) {
    let (w, h) = (target.width as f32, target.height as f32);
    let ndc = |c: &ClipVertex| Vector3::new(c.clip.x / c.clip.w, c.clip.y / c.clip.w, c.clip.z / c.clip.w);
    let (pa, pb, pc) = (ndc(&v[0]), ndc(&v[1]), ndc(&v[2]));
    // Screen space: x grows to the right, y grows downward, pixel centres sit at +0.5.
    let to_screen = |p: Vector3| ((p.x * 0.5 + 0.5) * w, (1.0 - (p.y * 0.5 + 0.5)) * h);
    let (sa, sb, sc) = (to_screen(pa), to_screen(pb), to_screen(pc));
    let area = edge(sa, sb, sc);
    if area.abs() <= f32::EPSILON {
        return;
    }
    let ws = [v[0].clip.w, v[1].clip.w, v[2].clip.w];
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
            let color = shade_pixel(perspective_weights([wa, wb, wc], ws));
            target.put(x, y, z, color);
        }
    }
}

/// Draws `triangles` into `target`. `view_projection` maps world space to clip space, `eye` is the world-space
/// viewpoint used for the specular term, and each triangle is shaded once with `light` and `material`. A triangle that
/// crosses the near plane is clipped; one wholly behind the eye is dropped.
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
    for tri in triangles {
        let normal = (tri.b - tri.a).cross(tri.c - tri.a);
        if normal.length() <= f32::EPSILON {
            continue;
        }
        let color = shade(light, material, centroid(tri), normal, eye).clamped();
        let corner = |p: Vector3| ClipVertex::new(view_projection, Vertex { position: p, normal, uv: (0.0, 0.0) });
        for piece in fan(&clip_near(&[corner(tri.a), corner(tri.b), corner(tri.c)])) {
            rasterise(target, piece, |_| color);
        }
    }
}

/// Draws `triangles` with per-pixel lighting. Each pixel interpolates the vertex normal and texture coordinate
/// perspective-correctly, lights the point with `surface` under `light` seen from `eye`, and takes its base colour from
/// `texture` when there is one. Surfaces are two-sided: a normal that faces away from the eye is flipped, so the back
/// of a triangle is lit like its front. Near-plane clipping is as in [`draw`].
pub fn draw_shaded(
    target: &mut Target,
    triangles: &[MeshTriangle],
    view_projection: Matrix4,
    eye: Vector3,
    light: &Light,
    surface: &Surface,
    texture: Option<&Texture>,
) {
    if target.width == 0 || target.height == 0 {
        return;
    }
    for tri in triangles {
        let face = (tri.b.position - tri.a.position).cross(tri.c.position - tri.a.position);
        if face.length() <= f32::EPSILON {
            continue;
        }
        let corner = |v: Vertex| {
            let normal = if v.normal.length() > f32::EPSILON { v.normal } else { face };
            ClipVertex::new(view_projection, Vertex { normal, ..v })
        };
        for piece in fan(&clip_near(&[corner(tri.a), corner(tri.b), corner(tri.c)])) {
            rasterise(target, piece, |wt| {
                let at = |f: fn(&ClipVertex) -> Vector3| f(&piece[0]) * wt[0] + f(&piece[1]) * wt[1] + f(&piece[2]) * wt[2];
                let point = at(|c| c.world);
                let mut normal = at(|c| c.normal).normalized();
                if normal.dot(eye - point) < 0.0 {
                    normal = -normal;
                }
                let u = piece[0].uv.0 * wt[0] + piece[1].uv.0 * wt[1] + piece[2].uv.0 * wt[2];
                let v = piece[0].uv.1 * wt[0] + piece[1].uv.1 * wt[1] + piece[2].uv.1 * wt[2];
                let tint = texture.map_or(Rgb::WHITE, |t| t.sample(u, v));
                match *surface {
                    Surface::Phong(m) => {
                        let m = Material { ambient: m.ambient.modulate(tint), diffuse: m.diffuse.modulate(tint), ..m };
                        shade(light, &m, point, normal, eye).clamped()
                    }
                    Surface::Pbr { albedo, metallic, roughness } => {
                        shade_pbr(light, albedo.modulate(tint), metallic, roughness, point, normal, eye).clamped()
                    }
                }
            });
        }
    }
}

/// The triangles of a convex polygon, fanned from its first vertex.
fn fan(poly: &[ClipVertex]) -> impl Iterator<Item = [ClipVertex; 3]> + '_ {
    (1..poly.len().saturating_sub(1)).map(move |i| [poly[0], poly[i], poly[i + 1]])
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

    // ---- near-plane clipping, textures and per-pixel shading ----

    /// Whether a pixel read back is the colour `want`, to within float rounding.
    fn same(got: Option<Rgb>, want: Rgb) -> bool {
        got.is_some_and(|g| (g.r - want.r).abs() < 1e-4 && (g.g - want.g).abs() < 1e-4 && (g.b - want.b).abs() < 1e-4)
    }

    fn cv(x: f32, y: f32, z: f32, w: f32) -> ClipVertex {
        ClipVertex { clip: Vector4::new(x, y, z, w), world: Vector3::new(x, y, z), normal: Vector3::new(0.0, 0.0, 1.0), uv: (x, y) }
    }

    fn sun_along_minus_z() -> Light {
        Light {
            source: LightSource::Directional(Vector3::new(0.0, 0.0, -1.0)),
            ambient: Rgb::BLACK,
            diffuse: Rgb::WHITE,
            specular: Rgb::BLACK,
        }
    }

    fn diffuse_white() -> Material {
        Material { ambient: Rgb::BLACK, diffuse: Rgb::WHITE, specular: Rgb::BLACK, shininess: 1.0 }
    }

    #[test]
    fn a_polygon_wholly_in_front_is_not_clipped() {
        let tri = [cv(0.0, 0.0, 0.0, 1.0), cv(1.0, 0.0, 0.0, 1.0), cv(0.0, 1.0, 0.0, 1.0)];
        assert_eq!(clip_near(&tri).len(), 3);
    }

    #[test]
    fn one_vertex_behind_the_near_plane_makes_a_quad_and_two_make_a_triangle() {
        // z + w >= 0 is in front; the third vertex has z + w = -1.
        let one = [cv(0.0, 0.0, 0.0, 1.0), cv(1.0, 0.0, 0.0, 1.0), cv(0.0, 1.0, -2.0, 1.0)];
        let quad = clip_near(&one);
        assert_eq!(quad.len(), 4);
        assert!(quad.iter().all(|v| v.clip.z + v.clip.w >= -1e-5), "every kept vertex is in front");
        let two = [cv(0.0, 0.0, 0.0, 1.0), cv(1.0, 0.0, -2.0, 1.0), cv(0.0, 1.0, -2.0, 1.0)];
        assert_eq!(clip_near(&two).len(), 3);
    }

    #[test]
    fn a_polygon_wholly_behind_is_removed_and_the_cut_point_interpolates_attributes() {
        let behind = [cv(0.0, 0.0, -2.0, 1.0), cv(1.0, 0.0, -2.0, 1.0), cv(0.0, 1.0, -2.0, 1.0)];
        assert!(clip_near(&behind).is_empty());
        // The edge from z = 0 to z = -2 crosses z + w = 0 at z = -1, half way along.
        let edge_in = [cv(0.0, 0.0, 0.0, 1.0), cv(0.0, 0.0, -2.0, 1.0), cv(2.0, 0.0, 0.0, 1.0)];
        let cut = clip_near(&edge_in);
        let mid = cut.iter().find(|v| (v.clip.z + 1.0).abs() < 1e-5).expect("cut vertex on the plane");
        assert!((mid.world.z + 1.0).abs() < 1e-5, "the world position is interpolated with the clip position");
    }

    #[test]
    fn a_triangle_crossing_the_eye_plane_is_drawn_not_dropped() {
        // Camera at the origin looking down -Z; the triangle spans z = -2 .. +5, so one vertex is behind the eye.
        let (light, material) = ambient_only(Rgb::WHITE);
        let vp = Matrix4::perspective(1.2, 1.0, 0.1, 100.0);
        let tri = Triangle {
            a: Vector3::new(-1.0, -1.0, -2.0),
            b: Vector3::new(1.0, -1.0, -2.0),
            c: Vector3::new(0.0, 1.0, 5.0),
        };
        let mut target = Target::new(16, 16, Rgb::BLACK);
        draw(&mut target, &[tri], vp, Vector3::ZERO, &light, &material);
        let covered = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .filter(|&(x, y)| target.depth_at(x, y).unwrap().is_finite())
            .count();
        assert!(covered > 0, "the visible part must reach the target");
    }

    #[test]
    fn texture_sampling_returns_the_texel_at_its_centre_and_blends_between_centres() {
        let red = Rgb::new(1.0, 0.0, 0.0);
        let blue = Rgb::new(0.0, 0.0, 1.0);
        let t = Texture::new(2, 1, vec![red, blue], Wrap::Clamp).unwrap();
        assert_eq!(t.sample(0.25, 0.5), red);
        assert_eq!(t.sample(0.75, 0.5), blue);
        let mid = t.sample(0.5, 0.5);
        assert!((mid.r - 0.5).abs() < 1e-5 && (mid.b - 0.5).abs() < 1e-5);
        assert_eq!(t.sample(-3.0, 0.5), red, "clamp extends the edge texel");
        assert_eq!(t.sample(9.0, 0.5), blue);
    }

    #[test]
    fn repeat_wraps_and_a_bad_texture_is_refused() {
        let red = Rgb::new(1.0, 0.0, 0.0);
        let blue = Rgb::new(0.0, 0.0, 1.0);
        let t = Texture::new(2, 1, vec![red, blue], Wrap::Repeat).unwrap();
        assert_eq!(t.sample(1.25, 0.5), red);
        assert_eq!(t.sample(-0.25, 0.5), blue);
        assert!(Texture::new(0, 1, vec![], Wrap::Clamp).is_none());
        assert!(Texture::new(2, 2, vec![red], Wrap::Clamp).is_none());
    }

    #[test]
    fn perspective_weights_favour_the_nearer_vertex_and_sum_to_one() {
        let even = perspective_weights([0.2, 0.3, 0.5], [2.0, 2.0, 2.0]);
        assert!(even.iter().zip([0.2, 0.3, 0.5]).all(|(a, b)| (a - b).abs() < 1e-6), "equal w changes nothing");
        let skewed = perspective_weights([0.5, 0.5, 0.0], [1.0, 4.0, 1.0]);
        assert!(skewed[0] > 0.5 && skewed[1] < 0.5, "the vertex with the smaller w gets more weight");
        assert!((skewed.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    fn mesh_full_screen(normals: [Vector3; 3], uvs: [(f32, f32); 3]) -> MeshTriangle {
        let v = |x: f32, y: f32, i: usize| Vertex { position: Vector3::new(x, y, 0.0), normal: normals[i], uv: uvs[i] };
        MeshTriangle { a: v(-1.0, -1.0, 0), b: v(3.0, -1.0, 1), c: v(-1.0, 3.0, 2) }
    }

    #[test]
    fn a_texture_colours_the_pixels_by_their_interpolated_uv() {
        let red = Rgb::new(1.0, 0.0, 0.0);
        let blue = Rgb::new(0.0, 0.0, 1.0);
        let texture = Texture::new(2, 1, vec![red, blue], Wrap::Clamp).unwrap();
        let (light, material) = ambient_only(Rgb::WHITE);
        let up = Vector3::new(0.0, 0.0, 1.0);
        // u runs 0 at the left edge of the screen to 1 at the right edge.
        let tri = mesh_full_screen([up; 3], [(0.0, 0.0), (2.0, 0.0), (0.0, 2.0)]);
        let mut target = Target::new(8, 4, Rgb::BLACK);
        let eye = Vector3::new(0.0, 0.0, 10.0);
        draw_shaded(&mut target, &[tri], Matrix4::identity(), eye, &light, &Surface::Phong(material), Some(&texture));
        assert!(same(target.color_at(0, 1), red), "left edge samples red");
        assert!(same(target.color_at(7, 1), blue), "right edge samples blue");
    }

    #[test]
    fn interpolated_normals_vary_the_lighting_across_one_triangle() {
        // Facing the light at the lower left corner, tilted well away at the others.
        let tri = mesh_full_screen(
            [Vector3::new(0.0, 0.0, 1.0), Vector3::new(1.0, 0.0, 0.2), Vector3::new(0.0, 1.0, 0.2)],
            [(0.0, 0.0); 3],
        );
        let mut target = Target::new(8, 8, Rgb::BLACK);
        let eye = Vector3::new(0.0, 0.0, 10.0);
        let surface = Surface::Phong(diffuse_white());
        draw_shaded(&mut target, &[tri], Matrix4::identity(), eye, &sun_along_minus_z(), &surface, None);
        let near_a = target.color_at(0, 7).unwrap().r;
        let far = target.color_at(7, 0).unwrap().r;
        assert!(near_a > 0.9, "the corner with the light-facing normal is bright, got {near_a}");
        assert!(far < near_a - 0.1, "tilted normals are darker: {far} vs {near_a}");
    }

    #[test]
    fn the_pbr_surface_matches_the_shading_model_at_a_pixel() {
        let light = sun_along_minus_z();
        let albedo = Rgb::new(0.8, 0.4, 0.2);
        let eye = Vector3::new(0.0, 0.0, 1000.0);
        let up = Vector3::new(0.0, 0.0, 1.0);
        let tri = mesh_full_screen([up; 3], [(0.0, 0.0); 3]);
        let mut target = Target::new(8, 8, Rgb::BLACK);
        let surface = Surface::Pbr { albedo, metallic: 0.0, roughness: 0.6 };
        draw_shaded(&mut target, &[tri], Matrix4::identity(), eye, &light, &surface, None);
        // Pixel (4, 4) sits at about (0.125, -0.125, 0) in world space.
        let want = shade_pbr(&light, albedo, 0.0, 0.6, Vector3::new(0.125, -0.125, 0.0), up, eye).clamped();
        let got = target.color_at(4, 4).unwrap();
        let close = (got.r - want.r).abs() < 1e-2 && (got.g - want.g).abs() < 1e-2 && (got.b - want.b).abs() < 1e-2;
        assert!(close, "{got:?} vs {want:?}");
        assert!(got.r > 0.0, "lit by the light");
    }

    #[test]
    fn the_back_of_a_triangle_faces_the_eye_before_it_is_lit() {
        // The eye is behind the surface, so the stored normal faces away from it and is flipped towards the eye.
        let tri = mesh_full_screen([Vector3::new(0.0, 0.0, 1.0); 3], [(0.0, 0.0); 3]);
        let surface = Surface::Phong(diffuse_white());
        let mut back = Target::new(4, 4, Rgb::BLACK);
        draw_shaded(&mut back, &[tri], Matrix4::identity(), Vector3::new(0.0, 0.0, -10.0), &sun_along_minus_z(), &surface, None);
        // Flipped to -Z, the normal points with the light's travel direction, so there is no diffuse.
        assert_eq!(back.color_at(2, 2), Some(Rgb::BLACK));
        let mut front = Target::new(4, 4, Rgb::BLACK);
        draw_shaded(&mut front, &[tri], Matrix4::identity(), Vector3::new(0.0, 0.0, 10.0), &sun_along_minus_z(), &surface, None);
        assert!(same(front.color_at(2, 2), Rgb::WHITE));
    }

    #[test]
    fn a_zero_vertex_normal_falls_back_to_the_face_normal() {
        let tri = mesh_full_screen([Vector3::ZERO; 3], [(0.0, 0.0); 3]);
        let mut target = Target::new(4, 4, Rgb::BLACK);
        let eye = Vector3::new(0.0, 0.0, 10.0);
        draw_shaded(&mut target, &[tri], Matrix4::identity(), eye, &sun_along_minus_z(), &Surface::Phong(diffuse_white()), None);
        assert!(same(target.color_at(2, 2), Rgb::WHITE), "the counter-clockwise face normal is +Z, towards the light");
    }

    #[test]
    fn a_triangle_crossing_the_eye_plane_keeps_its_texture_coordinates_sensible() {
        // The clipped part of a textured triangle must still sample inside the texture range along u.
        let red = Rgb::new(1.0, 0.0, 0.0);
        let texture = Texture::new(1, 1, vec![red], Wrap::Clamp).unwrap();
        let (light, material) = ambient_only(Rgb::WHITE);
        let vp = Matrix4::perspective(1.2, 1.0, 0.1, 100.0);
        let n = Vector3::new(0.0, 0.0, 1.0);
        let v = |x: f32, y: f32, z: f32| Vertex { position: Vector3::new(x, y, z), normal: n, uv: (0.5, 0.5) };
        let tri = MeshTriangle { a: v(-1.0, -1.0, -2.0), b: v(1.0, -1.0, -2.0), c: v(0.0, 1.0, 5.0) };
        let mut target = Target::new(16, 16, Rgb::BLACK);
        draw_shaded(&mut target, &[tri], vp, Vector3::ZERO, &light, &Surface::Phong(material), Some(&texture));
        let drawn = (0..16).flat_map(|y| (0..16).map(move |x| (x, y))).filter(|&(x, y)| target.depth_at(x, y).unwrap().is_finite());
        let mut count = 0;
        for (x, y) in drawn {
            count += 1;
            assert!(same(target.color_at(x, y), red));
        }
        assert!(count > 0);
    }
}
