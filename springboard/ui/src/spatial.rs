//! Points, sizes, boxes, axes, rays and spherical coordinates in 3D space, built on `linalg::Vector3`.
//! Plain `f32` values with no allocation. Angles are radians.

use crate::linalg::Vector3;

/// A position in 3D space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Point3 {
    pub const ORIGIN: Point3 = Point3 { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Point3 { x, y, z }
    }

    /// The offset from the origin to this point.
    pub fn to_vector(self) -> Vector3 {
        Vector3::new(self.x, self.y, self.z)
    }

    pub fn from_vector(v: Vector3) -> Self {
        Point3::new(v.x, v.y, v.z)
    }

    /// This point moved by `offset`.
    pub fn offset(self, offset: Vector3) -> Self {
        Point3::from_vector(self.to_vector() + offset)
    }

    /// Straight-line distance to `other`.
    pub fn distance(self, other: Point3) -> f32 {
        (other.to_vector() - self.to_vector()).length()
    }
}

/// Width, height and depth of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size3 {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
}

impl Size3 {
    pub fn new(width: f32, height: f32, depth: f32) -> Self {
        Size3 { width, height, depth }
    }

    /// True when any extent is zero or negative.
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0 || self.depth <= 0.0
    }

    pub fn volume(self) -> f32 {
        self.width * self.height * self.depth
    }
}

/// An axis-aligned box given by its minimum corner and its size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect3 {
    pub origin: Point3,
    pub size: Size3,
}

impl Rect3 {
    pub fn new(origin: Point3, size: Size3) -> Self {
        Rect3 { origin, size }
    }

    /// The box spanning two opposite corners, given in either order.
    pub fn from_corners(a: Point3, b: Point3) -> Self {
        let lo = Point3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
        let hi = Point3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
        Rect3::new(lo, Size3::new(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z))
    }

    /// The corner with the largest coordinates.
    pub fn max_corner(self) -> Point3 {
        Point3::new(
            self.origin.x + self.size.width,
            self.origin.y + self.size.height,
            self.origin.z + self.size.depth,
        )
    }

    pub fn center(self) -> Point3 {
        Point3::new(
            self.origin.x + self.size.width * 0.5,
            self.origin.y + self.size.height * 0.5,
            self.origin.z + self.size.depth * 0.5,
        )
    }

    /// True when `p` is inside the box or on its faces.
    pub fn contains(self, p: Point3) -> bool {
        let hi = self.max_corner();
        p.x >= self.origin.x
            && p.x <= hi.x
            && p.y >= self.origin.y
            && p.y <= hi.y
            && p.z >= self.origin.z
            && p.z <= hi.z
    }

    /// The overlap of two boxes, or None when they are apart. Boxes that only touch give a flat overlap.
    pub fn intersection(self, other: Rect3) -> Option<Rect3> {
        let (a, b) = (self.max_corner(), other.max_corner());
        let lo = Point3::new(self.origin.x.max(other.origin.x), self.origin.y.max(other.origin.y), self.origin.z.max(other.origin.z));
        let hi = Point3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
        if hi.x < lo.x || hi.y < lo.y || hi.z < lo.z {
            return None;
        }
        Some(Rect3::from_corners(lo, hi))
    }

    /// The smallest box containing both boxes.
    pub fn union(self, other: Rect3) -> Rect3 {
        let (a, b) = (self.max_corner(), other.max_corner());
        let lo = Point3::new(self.origin.x.min(other.origin.x), self.origin.y.min(other.origin.y), self.origin.z.min(other.origin.z));
        let hi = Point3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
        Rect3::from_corners(lo, hi)
    }
}

/// One of the three coordinate axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis3 {
    X,
    Y,
    Z,
}

impl Axis3 {
    /// The unit vector along this axis.
    pub fn unit(self) -> Vector3 {
        match self {
            Axis3::X => Vector3::new(1.0, 0.0, 0.0),
            Axis3::Y => Vector3::new(0.0, 1.0, 0.0),
            Axis3::Z => Vector3::new(0.0, 0.0, 1.0),
        }
    }

    /// The component of `v` along this axis.
    pub fn component(self, v: Vector3) -> f32 {
        match self {
            Axis3::X => v.x,
            Axis3::Y => v.y,
            Axis3::Z => v.z,
        }
    }

    /// 0 for X, 1 for Y, 2 for Z.
    pub fn index(self) -> usize {
        match self {
            Axis3::X => 0,
            Axis3::Y => 1,
            Axis3::Z => 2,
        }
    }
}

/// A half-line from `origin` along a unit `direction`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray3 {
    pub origin: Point3,
    pub direction: Vector3,
}

impl Ray3 {
    /// A ray from `origin` along `direction`, which is normalized. A zero direction stays zero.
    pub fn new(origin: Point3, direction: Vector3) -> Self {
        Ray3 { origin, direction: direction.normalized() }
    }

    /// The point at distance `t` along the ray.
    pub fn point_at(self, t: f32) -> Point3 {
        self.origin.offset(self.direction * t)
    }

    /// The distance along the ray to the first hit with a sphere, or None. A ray that starts inside
    /// the sphere returns the distance to its exit.
    pub fn intersect_sphere(self, center: Point3, radius: f32) -> Option<f32> {
        let a = self.direction.dot(self.direction);
        if a == 0.0 || radius < 0.0 {
            return None;
        }
        let oc = self.origin.to_vector() - center.to_vector();
        let half_b = oc.dot(self.direction);
        let c = oc.dot(oc) - radius * radius;
        let disc = half_b * half_b - a * c;
        if disc < 0.0 {
            return None;
        }
        let root = disc.sqrt();
        let near = (-half_b - root) / a;
        let far = (-half_b + root) / a;
        if near >= 0.0 {
            Some(near)
        } else if far >= 0.0 {
            Some(far)
        } else {
            None
        }
    }

    /// The distance along the ray where it enters `rect` (slab test), or None. A ray that starts
    /// inside the box returns 0. A zero direction hits only when the origin is inside.
    pub fn intersect_rect(self, rect: Rect3) -> Option<f32> {
        let hi = rect.max_corner();
        let lo_c = [rect.origin.x, rect.origin.y, rect.origin.z];
        let hi_c = [hi.x, hi.y, hi.z];
        let o = [self.origin.x, self.origin.y, self.origin.z];
        let d = [self.direction.x, self.direction.y, self.direction.z];
        let mut t_min = 0.0f32;
        let mut t_max = f32::INFINITY;
        for k in 0..3 {
            if d[k] == 0.0 {
                if o[k] < lo_c[k] || o[k] > hi_c[k] {
                    return None;
                }
                continue;
            }
            let t1 = (lo_c[k] - o[k]) / d[k];
            let t2 = (hi_c[k] - o[k]) / d[k];
            t_min = t_min.max(t1.min(t2));
            t_max = t_max.min(t1.max(t2));
        }
        if t_min <= t_max {
            Some(t_min)
        } else {
            None
        }
    }
}

/// Spherical coordinates. `inclination` is measured from the +Z axis (0 at +Z, pi at -Z) and `azimuth`
/// in the XY plane from +X towards +Y. This is this crate's convention; the reference page's summary
/// does not state its own, so conversions are only as portable as this convention is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spherical3 {
    pub radius: f32,
    pub inclination: f32,
    pub azimuth: f32,
}

impl Spherical3 {
    pub fn new(radius: f32, inclination: f32, azimuth: f32) -> Self {
        Spherical3 { radius, inclination, azimuth }
    }

    /// The spherical coordinates of a Cartesian point. The origin maps to radius 0 with angles 0.
    pub fn from_point(p: Point3) -> Self {
        let r = p.to_vector().length();
        if r == 0.0 {
            return Spherical3::default();
        }
        Spherical3 {
            radius: r,
            inclination: (p.z / r).clamp(-1.0, 1.0).acos(),
            azimuth: p.y.atan2(p.x),
        }
    }

    /// The Cartesian point for these coordinates.
    pub fn to_point(self) -> Point3 {
        let s = self.inclination.sin();
        Point3::new(
            self.radius * s * self.azimuth.cos(),
            self.radius * s * self.azimuth.sin(),
            self.radius * self.inclination.cos(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_point(actual: Point3, x: f32, y: f32, z: f32) {
        assert!(
            close(actual.x, x) && close(actual.y, y) && close(actual.z, z),
            "expected ({x}, {y}, {z}), got {actual:?}"
        );
    }

    #[test]
    fn distance_and_offset() {
        let p = Point3::new(1.0, 2.0, 2.0);
        assert!(close(p.distance(Point3::ORIGIN), 3.0));
        assert_point(p.offset(Vector3::new(-1.0, 0.0, 1.0)), 0.0, 2.0, 3.0);
    }

    #[test]
    fn rect_contains_is_inclusive_and_center_is_midpoint() {
        let r = Rect3::new(Point3::new(0.0, 0.0, 0.0), Size3::new(2.0, 4.0, 6.0));
        assert!(r.contains(Point3::new(2.0, 4.0, 6.0)));
        assert!(!r.contains(Point3::new(2.1, 1.0, 1.0)));
        assert_point(r.center(), 1.0, 2.0, 3.0);
        assert!(close(r.size.volume(), 48.0));
    }

    #[test]
    fn rect_intersection_and_union() {
        let a = Rect3::new(Point3::new(0.0, 0.0, 0.0), Size3::new(2.0, 2.0, 2.0));
        let b = Rect3::new(Point3::new(1.0, 1.0, 1.0), Size3::new(2.0, 2.0, 2.0));
        let overlap = a.intersection(b).expect("boxes overlap");
        assert_point(overlap.origin, 1.0, 1.0, 1.0);
        assert_point(overlap.max_corner(), 2.0, 2.0, 2.0);

        let apart = Rect3::new(Point3::new(3.0, 0.0, 0.0), Size3::new(1.0, 1.0, 1.0));
        assert!(a.intersection(apart).is_none());

        let c = Rect3::new(Point3::new(2.0, -1.0, 0.0), Size3::new(1.0, 1.0, 1.0));
        let hull = a.union(c);
        assert_point(hull.origin, 0.0, -1.0, 0.0);
        assert_point(hull.max_corner(), 3.0, 2.0, 2.0); // y reaches 2 from a
    }

    #[test]
    fn ray_hits_sphere_at_near_surface() {
        let ray = Ray3::new(Point3::new(0.0, 0.0, -5.0), Vector3::new(0.0, 0.0, 5.0));
        let t = ray.intersect_sphere(Point3::ORIGIN, 1.0).expect("ray hits");
        assert!(close(t, 4.0));
        assert_point(ray.point_at(t), 0.0, 0.0, -1.0);

        let miss = Ray3::new(Point3::new(0.0, 2.0, -5.0), Vector3::new(0.0, 0.0, 1.0));
        assert!(miss.intersect_sphere(Point3::ORIGIN, 1.0).is_none());

        let inside = Ray3::new(Point3::ORIGIN, Vector3::new(0.0, 0.0, 1.0));
        assert!(close(inside.intersect_sphere(Point3::ORIGIN, 1.0).unwrap(), 1.0));
    }

    #[test]
    fn ray_hits_box_by_slab_test() {
        let cube = Rect3::new(Point3::new(-1.0, -1.0, -1.0), Size3::new(2.0, 2.0, 2.0));
        let ahead = Ray3::new(Point3::new(0.0, 0.0, -5.0), Vector3::new(0.0, 0.0, 1.0));
        assert!(close(ahead.intersect_rect(cube).unwrap(), 4.0));

        let beside = Ray3::new(Point3::new(3.0, 0.0, -5.0), Vector3::new(0.0, 0.0, 1.0));
        assert!(beside.intersect_rect(cube).is_none());

        let behind = Ray3::new(Point3::new(0.0, 0.0, 5.0), Vector3::new(0.0, 0.0, 1.0));
        assert!(behind.intersect_rect(cube).is_none());
    }

    #[test]
    fn spherical_round_trip_and_known_points() {
        assert_point(Spherical3::new(2.0, FRAC_PI_2, 0.0).to_point(), 2.0, 0.0, 0.0);

        let up = Spherical3::from_point(Point3::new(0.0, 3.0, 0.0));
        assert!(close(up.radius, 3.0));
        assert!(close(up.inclination, FRAC_PI_2));
        assert!(close(up.azimuth, FRAC_PI_2));

        let p = Point3::new(1.0, 2.0, 2.0);
        assert_point(Spherical3::from_point(p).to_point(), 1.0, 2.0, 2.0);

        let pole = Spherical3::from_point(Point3::new(0.0, 0.0, -2.0));
        assert!(close(pole.inclination, PI));
    }

    #[test]
    fn axis_unit_and_component() {
        assert_eq!(Axis3::Y.unit(), Vector3::new(0.0, 1.0, 0.0));
        assert!(close(Axis3::Y.component(Vector3::new(1.0, 2.0, 3.0)), 2.0));
        assert_eq!(Axis3::Z.index(), 2);
    }
}
