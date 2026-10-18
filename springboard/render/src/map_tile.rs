//! Geometry kernels for drawing vector map tiles (roads, outlines, buildings, labels), as pure functions (std only):
//! polyline stroking to triangles with miter, bevel and round joins and butt, square and round caps; road drawing as a
//! wide casing under a narrower fill; ear-clipping triangulation of simple polygons; Douglas-Peucker generalisation of
//! polylines for coarse zoom levels; a point and heading at a distance along a polyline for road labels; and the
//! web-mercator tile coordinate of a longitude and latitude.
//!
//! Output is a triangle mesh (`Mesh2d`) in the caller's coordinates (tile pixels, y up or down: the winding of stroke
//! triangles follows the coordinate system, so draw with culling off). Strokes are built from one quad per segment plus
//! join and cap pieces, so triangles overlap at joins: draw them opaque or through a stencil, not with additive alpha.
//! The GPU version runs the same layout in a vertex shader fed by the segment list.

use std::f32::consts::PI;

/// A 2D point.
pub type P2 = [f32; 2];

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: P2, b: P2) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: P2, b: P2) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}
fn len(a: P2) -> f32 {
    dot(a, a).sqrt()
}
fn unit(a: P2) -> P2 {
    let l = len(a);
    [a[0] / l, a[1] / l]
}

/// An indexed triangle mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh2d {
    pub vertices: Vec<P2>,
    pub indices: Vec<u32>,
}

impl Mesh2d {
    fn vertex(&mut self, p: P2) -> u32 {
        self.vertices.push(p);
        (self.vertices.len() - 1) as u32
    }
    fn triangle(&mut self, a: P2, b: P2, c: P2) {
        let (ia, ib, ic) = (self.vertex(a), self.vertex(b), self.vertex(c));
        self.indices.extend_from_slice(&[ia, ib, ic]);
    }
    /// Sum of the triangle areas (overlaps count twice).
    pub fn area(&self) -> f32 {
        self.indices
            .chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (self.vertices[t[0] as usize], self.vertices[t[1] as usize], self.vertices[t[2] as usize]);
                0.5 * cross(sub(b, a), sub(c, a)).abs()
            })
            .sum()
    }
}

/// How two stroked segments meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    /// Sharp corner, cut to a bevel when the miter is longer than `limit` times half the width.
    Miter { limit: f32 },
    Bevel,
    Round,
}

/// How a stroked line ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cap {
    Butt,
    Square,
    Round,
}

/// Adds a triangle fan from `from_angle` sweeping `sweep` radians around `center`.
fn fan(mesh: &mut Mesh2d, center: P2, radius: f32, from_angle: f32, sweep: f32) {
    let steps = ((sweep.abs() / (PI / 12.0)).ceil() as usize).max(1);
    let point = |k: usize| {
        let a = from_angle + sweep * k as f32 / steps as f32;
        [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
    };
    for k in 0..steps {
        mesh.triangle(center, point(k), point(k + 1));
    }
}

/// Wraps an angle difference into (-pi, pi].
fn wrap(a: f32) -> f32 {
    let mut a = a;
    while a > PI {
        a -= 2.0 * PI;
    }
    while a <= -PI {
        a += 2.0 * PI;
    }
    a
}

/// Strokes a polyline `width` wide. Repeated points are dropped. `None` for a non-positive or non-finite width, a
/// non-finite point, or fewer than two distinct points.
pub fn stroke_polyline(points: &[P2], width: f32, join: Join, cap: Cap) -> Option<Mesh2d> {
    if !(width > 0.0) || !width.is_finite() || points.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
        return None;
    }
    let mut pts: Vec<P2> = Vec::with_capacity(points.len());
    for p in points {
        if pts.last().is_none_or(|l| len(sub(*p, *l)) > 1e-6) {
            pts.push(*p);
        }
    }
    if pts.len() < 2 {
        return None;
    }
    let h = width * 0.5;
    let n = pts.len();
    if cap == Cap::Square {
        let d0 = unit(sub(pts[1], pts[0]));
        let d1 = unit(sub(pts[n - 1], pts[n - 2]));
        pts[0] = [pts[0][0] - d0[0] * h, pts[0][1] - d0[1] * h];
        pts[n - 1] = [pts[n - 1][0] + d1[0] * h, pts[n - 1][1] + d1[1] * h];
    }
    let mut mesh = Mesh2d::default();
    let dirs: Vec<P2> = (0..n - 1).map(|i| unit(sub(pts[i + 1], pts[i]))).collect();
    let normal = |d: P2| -> P2 { [-d[1], d[0]] };

    for i in 0..n - 1 {
        let nn = normal(dirs[i]);
        let (a, b) = (pts[i], pts[i + 1]);
        let a1 = [a[0] + nn[0] * h, a[1] + nn[1] * h];
        let a2 = [a[0] - nn[0] * h, a[1] - nn[1] * h];
        let b1 = [b[0] + nn[0] * h, b[1] + nn[1] * h];
        let b2 = [b[0] - nn[0] * h, b[1] - nn[1] * h];
        mesh.triangle(a1, a2, b1);
        mesh.triangle(a2, b2, b1);
    }

    for i in 1..n - 1 {
        let (d0, d1) = (dirs[i - 1], dirs[i]);
        let c = cross(d0, d1);
        let d = dot(d0, d1);
        if c.abs() < 1e-6 && d > 0.0 {
            continue; // straight on
        }
        let (n0, n1) = (normal(d0), normal(d1));
        // The outside of the turn is the side the path turns away from.
        let s = if c > 0.0 { -1.0 } else { 1.0 };
        let p = pts[i];
        let ca = [p[0] + s * h * n0[0], p[1] + s * h * n0[1]];
        let cb = [p[0] + s * h * n1[0], p[1] + s * h * n1[1]];
        match join {
            Join::Bevel => mesh.triangle(p, ca, cb),
            Join::Miter { limit } => {
                let denom = 1.0 + d;
                if denom > 1e-6 && (2.0 / denom).sqrt() <= limit {
                    let k = s * h / denom;
                    let m = [p[0] + (n0[0] + n1[0]) * k, p[1] + (n0[1] + n1[1]) * k];
                    mesh.triangle(p, ca, m);
                    mesh.triangle(p, m, cb);
                } else {
                    mesh.triangle(p, ca, cb);
                }
            }
            Join::Round => {
                let a0 = (s * n0[1]).atan2(s * n0[0]);
                let a1 = (s * n1[1]).atan2(s * n1[0]);
                fan(&mut mesh, p, h, a0, wrap(a1 - a0));
            }
        }
    }

    if cap == Cap::Round {
        let n0 = normal(dirs[0]);
        fan(&mut mesh, pts[0], h, n0[1].atan2(n0[0]), PI);
        let n1 = normal(dirs[n - 2]);
        fan(&mut mesh, pts[n - 1], h, (-n1[1]).atan2(-n1[0]), PI);
    }
    Some(mesh)
}

/// A road as two strokes: the wider casing (the outline colour) drawn first and the fill on top. `casing` is the
/// extra width on each side. `None` under the same conditions as `stroke_polyline` or a negative casing.
pub fn road(points: &[P2], width: f32, casing: f32, join: Join, cap: Cap) -> Option<(Mesh2d, Mesh2d)> {
    if casing < 0.0 || !casing.is_finite() {
        return None;
    }
    let outline = stroke_polyline(points, width + 2.0 * casing, join, cap)?;
    let fill = stroke_polyline(points, width, join, cap)?;
    Some((outline, fill))
}

fn in_triangle(p: P2, a: P2, b: P2, c: P2) -> bool {
    cross(sub(b, a), sub(p, a)) >= 0.0 && cross(sub(c, b), sub(p, b)) >= 0.0 && cross(sub(a, c), sub(p, c)) >= 0.0
}

/// Ear-clipping triangulation of a simple polygon (no holes, no self-intersection). Returns triangle indices into
/// `ring` with counter-clockwise winding regardless of the input winding. `None` for fewer than 3 points, zero area, or a
/// ring that cannot be clipped (self-intersecting).
pub fn triangulate_polygon(ring: &[P2]) -> Option<Vec<u32>> {
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let area2: f32 = (0..n).map(|i| cross(ring[i], ring[(i + 1) % n])).sum();
    if area2.abs() < 1e-12 {
        return None;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    if area2 < 0.0 {
        idx.reverse();
    }
    let mut out: Vec<u32> = Vec::with_capacity((n - 2) * 3);
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (i0, i1, i2) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (ring[i0], ring[i1], ring[i2]);
            if cross(sub(b, a), sub(c, a)) <= 1e-12 {
                continue;
            }
            if idx.iter().any(|&j| j != i0 && j != i1 && j != i2 && in_triangle(ring[j], a, b, c)) {
                continue;
            }
            out.extend_from_slice(&[i0 as u32, i1 as u32, i2 as u32]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
    }
    out.extend_from_slice(&[idx[0] as u32, idx[1] as u32, idx[2] as u32]);
    Some(out)
}

fn segment_distance(p: P2, a: P2, b: P2) -> f32 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 < 1e-20 {
        return len(sub(p, a));
    }
    let t = (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0);
    len(sub(p, [a[0] + ab[0] * t, a[1] + ab[1] * t]))
}

/// Douglas-Peucker simplification: the fewest points of the polyline such that every dropped point lies within
/// `tolerance` of the simplified line. The ends are always kept.
pub fn simplify_polyline(points: &[P2], tolerance: f32) -> Vec<P2> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let mut best = 0.0f32;
        let mut best_i = a;
        for i in a + 1..b {
            let d = segment_distance(points[i], points[a], points[b]);
            if d > best {
                best = d;
                best_i = i;
            }
        }
        if best > tolerance {
            keep[best_i] = true;
            stack.push((a, best_i));
            stack.push((best_i, b));
        }
    }
    points.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

/// Length of a polyline.
pub fn polyline_length(points: &[P2]) -> f32 {
    points.windows(2).map(|w| len(sub(w[1], w[0]))).sum()
}

/// The point at `distance` along a polyline (clamped to its ends) and the heading of the segment there in radians
/// (`atan2(dy, dx)`), for placing a road label. `None` for fewer than two points.
pub fn point_at(points: &[P2], distance: f32) -> Option<(P2, f32)> {
    if points.len() < 2 {
        return None;
    }
    let mut left = distance.max(0.0);
    let last = points.len() - 2;
    for (i, w) in points.windows(2).enumerate() {
        let d = sub(w[1], w[0]);
        let l = len(d);
        if left <= l || i == last {
            let t = if l < 1e-12 { 0.0 } else { (left / l).min(1.0) };
            return Some(([w[0][0] + d[0] * t, w[0][1] + d[1] * t], d[1].atan2(d[0])));
        }
        left -= l;
    }
    None
}

/// Fractional slippy-map tile coordinates of a longitude and latitude (degrees) at `zoom` (web mercator). Latitude
/// is clamped to the mercator limit of 85.0511 degrees.
pub fn lonlat_to_tile(lon: f64, lat: f64, zoom: u32) -> (f64, f64) {
    let n = f64::from(1u32 << zoom.min(31));
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: P2, b: P2) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    fn has_vertex(m: &Mesh2d, p: P2) -> bool {
        m.vertices.iter().any(|v| near(*v, p))
    }

    #[test]
    fn a_straight_stroke_covers_length_times_width() {
        let m = stroke_polyline(&[[0.0, 0.0], [10.0, 0.0]], 2.0, Join::Bevel, Cap::Butt).unwrap();
        assert!((m.area() - 20.0).abs() < 1e-4);
        let sq = stroke_polyline(&[[0.0, 0.0], [10.0, 0.0]], 2.0, Join::Bevel, Cap::Square).unwrap();
        assert!((sq.area() - 24.0).abs() < 1e-4);
        let round = stroke_polyline(&[[0.0, 0.0], [10.0, 0.0]], 2.0, Join::Bevel, Cap::Round).unwrap();
        // Two half discs of radius 1 add about pi in total (a polygonal fan slightly less).
        assert!(round.area() > 20.0 + 3.0 && round.area() < 20.0 + PI + 1e-3);
    }

    #[test]
    fn bad_input_gives_none() {
        assert!(stroke_polyline(&[[0.0, 0.0]], 1.0, Join::Bevel, Cap::Butt).is_none());
        assert!(stroke_polyline(&[[0.0, 0.0], [0.0, 0.0]], 1.0, Join::Bevel, Cap::Butt).is_none());
        assert!(stroke_polyline(&[[0.0, 0.0], [1.0, 0.0]], 0.0, Join::Bevel, Cap::Butt).is_none());
        assert!(stroke_polyline(&[[0.0, 0.0], [f32::NAN, 0.0]], 1.0, Join::Bevel, Cap::Butt).is_none());
    }

    #[test]
    fn joins_differ_at_a_right_angle() {
        let path = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        let miter = stroke_polyline(&path, 2.0, Join::Miter { limit: 2.0 }, Cap::Butt).unwrap();
        assert!(has_vertex(&miter, [11.0, -1.0]));
        let cut = stroke_polyline(&path, 2.0, Join::Miter { limit: 1.0 }, Cap::Butt).unwrap();
        assert!(!has_vertex(&cut, [11.0, -1.0]));
        assert!(has_vertex(&cut, [10.0, -1.0]) && has_vertex(&cut, [11.0, 0.0]));
        let bevel = stroke_polyline(&path, 2.0, Join::Bevel, Cap::Butt).unwrap();
        assert_eq!(bevel.indices.len(), 3 * 5);
        let round = stroke_polyline(&path, 2.0, Join::Round, Cap::Butt).unwrap();
        assert!(round.indices.len() > bevel.indices.len());
        // The 45 degree point of the round join's arc around the corner.
        assert!(has_vertex(&round, [10.0 + 0.707_106_8, -0.707_106_8]));
    }

    #[test]
    fn straight_continuation_adds_no_join() {
        let m = stroke_polyline(&[[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]], 2.0, Join::Round, Cap::Butt).unwrap();
        assert_eq!(m.indices.len(), 12);
    }

    #[test]
    fn road_casing_is_wider_than_fill() {
        let (outline, fill) = road(&[[0.0, 0.0], [10.0, 0.0]], 4.0, 1.0, Join::Round, Cap::Butt).unwrap();
        assert!((outline.area() - 60.0).abs() < 1e-3);
        assert!((fill.area() - 40.0).abs() < 1e-3);
        assert!(road(&[[0.0, 0.0], [10.0, 0.0]], 4.0, -1.0, Join::Round, Cap::Butt).is_none());
    }

    fn tri_area(ring: &[P2], idx: &[u32]) -> f32 {
        idx.chunks_exact(3)
            .map(|t| 0.5 * cross(sub(ring[t[1] as usize], ring[t[0] as usize]), sub(ring[t[2] as usize], ring[t[0] as usize])))
            .sum()
    }

    #[test]
    fn triangulates_convex_and_concave_polygons() {
        let sq = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let t = triangulate_polygon(&sq).unwrap();
        assert_eq!(t.len(), 6);
        assert!((tri_area(&sq, &t) - 1.0).abs() < 1e-6);

        let l = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let t = triangulate_polygon(&l).unwrap();
        assert_eq!(t.len(), 12);
        assert!((tri_area(&l, &t) - 3.0).abs() < 1e-5);

        // Clockwise input gives the same triangles, wound counter-clockwise.
        let cw: Vec<P2> = l.iter().rev().copied().collect();
        let t = triangulate_polygon(&cw).unwrap();
        assert!((tri_area(&cw, &t) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn degenerate_polygons_are_refused() {
        assert!(triangulate_polygon(&[[0.0, 0.0], [1.0, 1.0]]).is_none());
        assert!(triangulate_polygon(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]).is_none());
    }

    #[test]
    fn simplification_keeps_corners() {
        let line = [[0.0, 0.0], [1.0, 0.01], [2.0, 0.0], [3.0, 0.0], [3.0, 3.0]];
        let s = simplify_polyline(&line, 0.1);
        assert_eq!(s, vec![[0.0, 0.0], [3.0, 0.0], [3.0, 3.0]]);
        assert_eq!(simplify_polyline(&line, 0.0).len(), 5);
        assert_eq!(simplify_polyline(&line[..2], 1.0).len(), 2);
    }

    #[test]
    fn labels_follow_the_line() {
        let line = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert!((polyline_length(&line) - 20.0).abs() < 1e-5);
        let (p, a) = point_at(&line, 5.0).unwrap();
        assert!(near(p, [5.0, 0.0]) && a.abs() < 1e-6);
        let (p, a) = point_at(&line, 15.0).unwrap();
        assert!(near(p, [10.0, 5.0]) && (a - PI / 2.0).abs() < 1e-5);
        let (p, _) = point_at(&line, 99.0).unwrap();
        assert!(near(p, [10.0, 10.0]));
        let (p, _) = point_at(&line, -3.0).unwrap();
        assert!(near(p, [0.0, 0.0]));
        assert!(point_at(&line[..1], 1.0).is_none());
    }

    #[test]
    fn tile_coordinates_match_the_slippy_map_scheme() {
        let (x, y) = lonlat_to_tile(0.0, 0.0, 1);
        assert!((x - 1.0).abs() < 1e-9 && (y - 1.0).abs() < 1e-9);
        let (x, y) = lonlat_to_tile(-180.0, 85.0511, 3);
        assert!(x.abs() < 1e-9 && y.abs() < 1e-3);
        let (x, _) = lonlat_to_tile(180.0, 0.0, 2);
        assert!((x - 4.0).abs() < 1e-9);
        // Reykjavik at zoom 10 (values from the standard formula).
        let (x, y) = lonlat_to_tile(-21.9426, 64.1466, 10);
        assert_eq!((x.floor() as u32, y.floor() as u32), (449, 272));
    }
}
