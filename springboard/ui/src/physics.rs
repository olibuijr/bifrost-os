//! Rigid-body physics world: circular bodies under gravity that push apart and bounce off each other, with the contacts
//! of each step reported back. It is the counterpart of the 2D scene framework's physics world and bodies.
//!
//! Pure: no clock and no thread. The caller steps the world with `step(dt)`. A step adds gravity to the velocity of
//! the dynamic bodies, moves them, then resolves every overlapping pair once. The pair is pushed apart in proportion
//! to the inverse masses, and the closing speed along the contact normal is reflected by the restitution, the smaller
//! of the two bodies' values. Static bodies never move. Only circles exist here; boxes and edges come later.
//!
//! Units: points, seconds and points per second, Y upwards. `speed` scales the time step, as the framework does.

/// A two-dimensional vector in points or points per second.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Vec2 {
        Vec2 { x, y }
    }

    pub fn dot(self, other: Vec2) -> f32 {
        self.x * other.x + self.y * other.y
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn scaled(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }

    pub fn plus(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x + other.x, self.y + other.y)
    }

    pub fn minus(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x - other.x, self.y - other.y)
    }
}

/// One circular body. A dynamic body moves under forces and collisions; a static one stays where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub position: Vec2,
    pub velocity: Vec2,
    pub radius: f32,
    /// Mass of a dynamic body. A dynamic body with mass zero or less counts as mass 1.
    pub mass: f32,
    pub is_dynamic: bool,
    /// Whether the world's gravity applies (dynamic bodies only).
    pub affected_by_gravity: bool,
    /// 0 is perfectly inelastic, 1 is perfectly elastic. Values outside 0..=1 are clamped.
    pub restitution: f32,
}

impl Body {
    /// A dynamic body, affected by gravity, with restitution 0.2.
    pub fn dynamic(position: Vec2, radius: f32, mass: f32) -> Body {
        Body {
            position,
            velocity: Vec2::ZERO,
            radius,
            mass,
            is_dynamic: true,
            affected_by_gravity: true,
            restitution: 0.2,
        }
    }

    /// A static body with restitution 0.5.
    pub fn fixed(position: Vec2, radius: f32) -> Body {
        Body {
            position,
            velocity: Vec2::ZERO,
            radius,
            mass: 0.0,
            is_dynamic: false,
            affected_by_gravity: false,
            restitution: 0.5,
        }
    }

    fn inverse_mass(&self) -> f32 {
        if !self.is_dynamic {
            0.0
        } else if self.mass > 0.0 {
            1.0 / self.mass
        } else {
            1.0
        }
    }
}

/// One overlap found in a step. `normal` points from body `a` to body `b`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub a: usize,
    pub b: usize,
    pub normal: Vec2,
    /// How far the two circles overlapped before they were pushed apart.
    pub penetration: f32,
}

/// The bodies and the settings that move them.
#[derive(Clone, Debug)]
pub struct World {
    pub gravity: Vec2,
    /// Multiplier on the time step passed to `step`.
    pub speed: f32,
    bodies: Vec<Body>,
}

impl World {
    pub fn new() -> World {
        World {
            gravity: Vec2::ZERO,
            speed: 1.0,
            bodies: Vec::new(),
        }
    }

    /// Adds a body and returns its id, which is its index.
    pub fn add(&mut self, body: Body) -> usize {
        self.bodies.push(body);
        self.bodies.len() - 1
    }

    pub fn body(&self, id: usize) -> Option<&Body> {
        self.bodies.get(id)
    }

    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    /// The first body (lowest id) whose circle contains `point`.
    pub fn body_at(&self, point: Vec2) -> Option<usize> {
        self.bodies
            .iter()
            .position(|b| b.position.minus(point).length() <= b.radius)
    }

    /// Advances the world by `dt` seconds (scaled by `speed`) and returns the contacts found in this step.
    pub fn step(&mut self, dt: f32) -> Vec<Contact> {
        let dt = (dt * self.speed).max(0.0);
        let gravity = self.gravity;
        for b in self.bodies.iter_mut() {
            if b.is_dynamic {
                if b.affected_by_gravity {
                    b.velocity = b.velocity.plus(gravity.scaled(dt));
                }
                b.position = b.position.plus(b.velocity.scaled(dt));
            }
        }
        let mut contacts = Vec::new();
        let n = self.bodies.len();
        for i in 0..n {
            for j in (i + 1)..n {
                if let Some(c) = self.collide(i, j) {
                    contacts.push(c);
                }
            }
        }
        contacts
    }

    fn collide(&mut self, i: usize, j: usize) -> Option<Contact> {
        let a = self.bodies[i];
        let b = self.bodies[j];
        if !a.is_dynamic && !b.is_dynamic {
            return None;
        }
        let delta = b.position.minus(a.position);
        let dist = delta.length();
        let min_dist = a.radius + b.radius;
        if dist >= min_dist {
            return None;
        }
        let normal = if dist > 0.0 {
            delta.scaled(1.0 / dist)
        } else {
            Vec2::new(1.0, 0.0)
        };
        let penetration = min_dist - dist;

        let inv_a = a.inverse_mass();
        let inv_b = b.inverse_mass();
        let inv_sum = inv_a + inv_b;

        // Push the pair apart in proportion to the inverse masses. A static body gets no share.
        if a.is_dynamic {
            let push = penetration * inv_a / inv_sum;
            self.bodies[i].position = a.position.minus(normal.scaled(push));
        }
        if b.is_dynamic {
            let push = penetration * inv_b / inv_sum;
            self.bodies[j].position = b.position.plus(normal.scaled(push));
        }

        // Reflect the closing speed along the normal, with the smaller restitution.
        let closing = b.velocity.minus(a.velocity).dot(normal);
        if closing < 0.0 {
            let e = a.restitution.max(0.0).min(1.0).min(b.restitution.max(0.0).min(1.0));
            let impulse = -(1.0 + e) * closing / inv_sum;
            if a.is_dynamic {
                self.bodies[i].velocity = a.velocity.minus(normal.scaled(impulse * inv_a));
            }
            if b.is_dynamic {
                self.bodies[j].velocity = b.velocity.plus(normal.scaled(impulse * inv_b));
            }
        }

        Some(Contact {
            a: i,
            b: j,
            normal,
            penetration,
        })
    }
}

impl Default for World {
    fn default() -> World {
        World::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn gravity_accelerates_a_dynamic_body_then_moves_it() {
        let mut w = World::new();
        w.gravity = Vec2::new(0.0, -10.0);
        let id = w.add(Body::dynamic(Vec2::new(0.0, 10.0), 1.0, 1.0));
        let contacts = w.step(0.5);
        let b = w.body(id).unwrap();
        assert!(close(b.velocity.y, -5.0)); // -10 * 0.5
        assert!(close(b.position.y, 7.5)); // 10 + (-5 * 0.5)
        assert!(contacts.is_empty());
    }

    #[test]
    fn static_bodies_and_bodies_without_gravity_do_not_fall() {
        let mut w = World::new();
        w.gravity = Vec2::new(0.0, -10.0);
        let fixed = w.add(Body::fixed(Vec2::new(0.0, 0.0), 1.0));
        let mut floating = Body::dynamic(Vec2::new(5.0, 5.0), 1.0, 1.0);
        floating.affected_by_gravity = false;
        let id = w.add(floating);
        w.step(0.5);
        assert_eq!(w.body(fixed).unwrap().position, Vec2::new(0.0, 0.0));
        assert_eq!(w.body(id).unwrap().position, Vec2::new(5.0, 5.0));
        assert_eq!(w.body(id).unwrap().velocity, Vec2::ZERO);
    }

    #[test]
    fn speed_scales_the_time_step() {
        // speed 2 with dt 0.5 is the same as speed 1 with dt 1.0: velocity -10, position 10 - 10 = 0
        let mut fast = World::new();
        fast.gravity = Vec2::new(0.0, -10.0);
        fast.speed = 2.0;
        let f = fast.add(Body::dynamic(Vec2::new(0.0, 10.0), 1.0, 1.0));
        fast.step(0.5);

        let mut normal = World::new();
        normal.gravity = Vec2::new(0.0, -10.0);
        let n = normal.add(Body::dynamic(Vec2::new(0.0, 10.0), 1.0, 1.0));
        normal.step(1.0);

        assert!(close(fast.body(f).unwrap().position.y, 0.0));
        assert!(close(normal.body(n).unwrap().position.y, 0.0));
        assert!(close(fast.body(f).unwrap().velocity.y, -10.0));
    }

    #[test]
    fn falling_body_bounces_off_a_static_body() {
        let mut w = World::new();
        let floor = w.add(Body::fixed(Vec2::new(0.0, 0.0), 1.0));
        let mut ball = Body::dynamic(Vec2::new(0.0, 1.5), 1.0, 1.0);
        ball.velocity = Vec2::new(0.0, -10.0);
        ball.restitution = 0.5;
        let id = w.add(ball);
        // dt 0.1: y moves 1.5 -> 0.5, overlap 2 - 0.5 = 1.5, pushed back to y = 2.0
        let contacts = w.step(0.1);
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].a, floor);
        assert_eq!(contacts[0].b, id);
        assert!(close(contacts[0].penetration, 1.5));
        assert!(close(contacts[0].normal.y, 1.0));
        let b = w.body(id).unwrap();
        assert!(close(b.position.y, 2.0));
        // closing speed -10 reflected with e = 0.5 gives +5
        assert!(close(b.velocity.y, 5.0));
        assert!(close(w.body(floor).unwrap().position.y, 0.0));
    }

    #[test]
    fn equal_masses_split_the_overlap_and_swap_velocities_when_elastic() {
        let mut w = World::new();
        let mut left = Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0);
        left.velocity = Vec2::new(1.0, 0.0);
        left.restitution = 1.0;
        let mut right = Body::dynamic(Vec2::new(1.5, 0.0), 1.0, 1.0);
        right.velocity = Vec2::new(-1.0, 0.0);
        right.restitution = 1.0;
        let a = w.add(left);
        let b = w.add(right);
        // dt 0: no motion, only the overlap of 2 - 1.5 = 0.5 is resolved
        let contacts = w.step(0.0);
        assert_eq!(contacts.len(), 1);
        assert!(close(contacts[0].penetration, 0.5));
        assert!(close(w.body(a).unwrap().position.x, -0.25));
        assert!(close(w.body(b).unwrap().position.x, 1.75));
        assert!(close(w.body(a).unwrap().velocity.x, -1.0));
        assert!(close(w.body(b).unwrap().velocity.x, 1.0));
    }

    #[test]
    fn separated_bodies_produce_no_contact() {
        let mut w = World::new();
        let a = w.add(Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0));
        let b = w.add(Body::dynamic(Vec2::new(3.0, 0.0), 1.0, 1.0));
        let contacts = w.step(0.0);
        assert!(contacts.is_empty());
        assert!(close(w.body(a).unwrap().position.x, 0.0));
        assert!(close(w.body(b).unwrap().position.x, 3.0));
    }

    #[test]
    fn body_at_returns_the_first_circle_containing_the_point() {
        let mut w = World::new();
        w.add(Body::fixed(Vec2::new(0.0, 0.0), 1.0));
        w.add(Body::fixed(Vec2::new(5.0, 0.0), 1.0));
        assert_eq!(w.body_at(Vec2::new(0.5, 0.0)), Some(0));
        assert_eq!(w.body_at(Vec2::new(5.9, 0.0)), Some(1));
        assert_eq!(w.body_at(Vec2::new(5.0, 1.5)), None);
    }
}
