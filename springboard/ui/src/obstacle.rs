//! Impassable areas that steering code routes around: circles, spheres and polygons.
//! Two-dimensional points are `(f32, f32)` and three-dimensional points are `spatial::Point3`.
//! Each shape answers whether a point is blocked, and gives the nearest free point (`push_out`) or the
//! nearest outline point.

use crate::spatial::Point3;

/// The closest point to `p` on the segment `a`-`b`, clamped to its ends. A zero-length segment gives `a`.
pub fn closest_point_on_segment(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0)
    };
    (a.0 + abx * t, a.1 + aby * t)
}

fn dist2(a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    dx * dx + dy * dy
}

/// A circular impassable area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircleObstacle {
    pub center: (f32, f32),
    pub radius: f32,
}

impl CircleObstacle {
    pub fn new(center: (f32, f32), radius: f32) -> Self {
        CircleObstacle { center, radius }
    }

    /// True inside the circle or on its edge.
    pub fn contains(self, p: (f32, f32)) -> bool {
        dist2(p, self.center) <= self.radius * self.radius
    }

    /// `p` when it is free; otherwise the edge point on the ray from the centre through `p`. A point at
    /// the centre goes to the +X edge.
    pub fn push_out(self, p: (f32, f32)) -> (f32, f32) {
        if !self.contains(p) {
            return p;
        }
        let (dx, dy) = (p.0 - self.center.0, p.1 - self.center.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d == 0.0 {
            return (self.center.0 + self.radius, self.center.1);
        }
        (self.center.0 + dx / d * self.radius, self.center.1 + dy / d * self.radius)
    }

    /// True when the segment `a`-`b` passes through the circle.
    pub fn intersects_segment(self, a: (f32, f32), b: (f32, f32)) -> bool {
        dist2(closest_point_on_segment(a, b, self.center), self.center) <= self.radius * self.radius
    }
}

/// A spherical impassable volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SphereObstacle {
    pub center: Point3,
    pub radius: f32,
}

impl SphereObstacle {
    pub fn new(center: Point3, radius: f32) -> Self {
        SphereObstacle { center, radius }
    }

    /// True inside the sphere or on its surface.
    pub fn contains(self, p: Point3) -> bool {
        self.center.distance(p) <= self.radius
    }

    /// `p` when it is free; otherwise the surface point on the ray from the centre through `p`.
    pub fn push_out(self, p: Point3) -> Point3 {
        if !self.contains(p) {
            return p;
        }
        let v = p.to_vector() - self.center.to_vector();
        let d = v.length();
        if d == 0.0 {
            return Point3::new(self.center.x + self.radius, self.center.y, self.center.z);
        }
        self.center.offset(v * (self.radius / d))
    }
}

/// A polygonal impassable area, given by its vertices in order.
#[derive(Clone, Debug, PartialEq)]
pub struct PolygonObstacle {
    vertices: Vec<(f32, f32)>,
}

impl PolygonObstacle {
    pub fn new(vertices: Vec<(f32, f32)>) -> Self {
        PolygonObstacle { vertices }
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn vertex(&self, index: usize) -> Option<(f32, f32)> {
        self.vertices.get(index).copied()
    }

    /// Even-odd rule. A polygon with fewer than three vertices contains nothing. Points exactly on an
    /// edge may fall either way.
    pub fn contains(&self, p: (f32, f32)) -> bool {
        let n = self.vertices.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (vi, vj) = (self.vertices[i], self.vertices[j]);
            if (vi.1 > p.1) != (vj.1 > p.1) && p.0 < (vj.0 - vi.0) * (p.1 - vi.1) / (vj.1 - vi.1) + vi.0 {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// The point on the outline nearest to `p`, or None with fewer than two vertices.
    pub fn closest_outline_point(&self, p: (f32, f32)) -> Option<(f32, f32)> {
        let n = self.vertices.len();
        if n < 2 {
            return None;
        }
        let mut best: Option<((f32, f32), f32)> = None;
        for i in 0..n {
            let a = self.vertices[i];
            let b = self.vertices[(i + 1) % n];
            let c = closest_point_on_segment(a, b, p);
            let d = dist2(c, p);
            let better = match best {
                Some((_, bd)) => d < bd,
                None => true,
            };
            if better {
                best = Some((c, d));
            }
        }
        best.map(|(c, _)| c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close2(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS
    }

    fn close3(a: Point3, b: Point3) -> bool {
        a.distance(b) < EPS
    }

    #[test]
    fn circle_contains_and_pushes_out() {
        let c = CircleObstacle::new((0.0, 0.0), 2.0);
        assert!(c.contains((1.0, 1.0)));
        assert!(c.contains((2.0, 0.0)));
        assert!(!c.contains((2.0, 2.0)));

        assert!(close2(c.push_out((5.0, 0.0)), (5.0, 0.0)));
        assert!(close2(c.push_out((1.0, 0.0)), (2.0, 0.0)));
        assert!(close2(c.push_out((0.0, -0.5)), (0.0, -2.0)));
        assert!(close2(c.push_out((0.0, 0.0)), (2.0, 0.0)));
    }

    #[test]
    fn circle_segment_test_uses_the_nearest_point() {
        let c = CircleObstacle::new((0.0, 0.0), 1.0);
        assert!(c.intersects_segment((-5.0, 0.5), (5.0, 0.5)));
        assert!(!c.intersects_segment((-5.0, 2.0), (5.0, 2.0)));
    }

    #[test]
    fn sphere_contains_and_pushes_out() {
        let s = SphereObstacle::new(Point3::ORIGIN, 1.0);
        assert!(s.contains(Point3::new(0.0, 0.0, 1.0)));
        assert!(!s.contains(Point3::new(0.0, 0.0, 1.5)));
        assert!(close3(s.push_out(Point3::new(0.0, 0.0, 3.0)), Point3::new(0.0, 0.0, 3.0)));
        assert!(close3(s.push_out(Point3::new(0.0, 0.0, 0.5)), Point3::new(0.0, 0.0, 1.0)));
    }

    #[test]
    fn polygon_contains_by_even_odd_rule() {
        let square = PolygonObstacle::new(vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        assert_eq!(square.vertex_count(), 4);
        assert!(square.contains((2.0, 2.0)));
        assert!(!square.contains((5.0, 2.0)));
        assert!(!square.contains((-1.0, -1.0)));
        assert!(square.vertex(4).is_none());

        let degenerate = PolygonObstacle::new(vec![(0.0, 0.0), (1.0, 1.0)]);
        assert!(!degenerate.contains((0.5, 0.5)));
        assert!(degenerate.closest_outline_point((0.5, 0.5)).is_some());
    }

    #[test]
    fn polygon_outline_point_is_on_the_nearest_edge() {
        let square = PolygonObstacle::new(vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        assert!(close2(square.closest_outline_point((2.0, 6.0)).unwrap(), (2.0, 4.0)));
    }

    #[test]
    fn closest_point_on_segment_clamps_to_the_ends() {
        assert!(close2(closest_point_on_segment((1.0, 1.0), (1.0, 1.0), (3.0, 3.0)), (1.0, 1.0)));
        assert!(close2(closest_point_on_segment((0.0, 0.0), (10.0, 0.0), (4.0, 7.0)), (4.0, 0.0)));
        assert!(close2(closest_point_on_segment((0.0, 0.0), (10.0, 0.0), (-5.0, 0.0)), (0.0, 0.0)));
    }
}
