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

/// A connection between two bodies, by their indices. Anchors are offsets from each body's centre, so a joint keeps
/// its meaning when the bodies move. The world's `add_*` functions take anchors in world points and convert them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Joint {
    /// The two anchor points stay on the same spot.
    Pin { a: usize, b: usize, anchor_a: Vec2, anchor_b: Vec2 },
    /// The bodies keep the offset they had when the joint was made (`offset` is body b minus body a).
    Fixed { a: usize, b: usize, offset: Vec2 },
    /// A rope: the anchor points may not be further apart than `max_length`.
    Limit { a: usize, b: usize, anchor_a: Vec2, anchor_b: Vec2, max_length: f32 },
    /// Hooke spring with damping. Force along the line is `stiffness * (length - rest_length) + damping * closing speed`.
    Spring { a: usize, b: usize, anchor_a: Vec2, anchor_b: Vec2, rest_length: f32, stiffness: f32, damping: f32 },
    /// Body b may only move along `axis` relative to body a, and its displacement along it stays in `lower..=upper`.
    /// The displacement is measured from `offset`, the offset the bodies had when the joint was made.
    Slide { a: usize, b: usize, offset: Vec2, axis: Vec2, lower: f32, upper: f32 },
}

/// The bodies, the joints between them, and the settings that move them.
#[derive(Clone, Debug)]
pub struct World {
    pub gravity: Vec2,
    /// Multiplier on the time step passed to `step`.
    pub speed: f32,
    bodies: Vec<Body>,
    joints: Vec<Joint>,
}

impl World {
    pub fn new() -> World {
        World {
            gravity: Vec2::ZERO,
            speed: 1.0,
            bodies: Vec::new(),
            joints: Vec::new(),
        }
    }

    /// Pins bodies `a` and `b` at the world point `anchor`.
    pub fn add_pin(&mut self, a: usize, b: usize, anchor: Vec2) -> usize {
        let anchor_a = anchor.minus(self.position_of(a));
        let anchor_b = anchor.minus(self.position_of(b));
        self.push_joint(Joint::Pin { a, b, anchor_a, anchor_b })
    }

    /// Locks body `b` to body `a` at their current relative offset.
    pub fn add_fixed(&mut self, a: usize, b: usize) -> usize {
        let offset = self.position_of(b).minus(self.position_of(a));
        self.push_joint(Joint::Fixed { a, b, offset })
    }

    /// A rope of length `max_length` between the world points `anchor_a` and `anchor_b`.
    pub fn add_limit(&mut self, a: usize, b: usize, anchor_a: Vec2, anchor_b: Vec2, max_length: f32) -> usize {
        let anchor_a = anchor_a.minus(self.position_of(a));
        let anchor_b = anchor_b.minus(self.position_of(b));
        let max_length = max_length.max(0.0);
        self.push_joint(Joint::Limit { a, b, anchor_a, anchor_b, max_length })
    }

    /// A spring between the world points `anchor_a` and `anchor_b`. Its rest length is their distance now.
    pub fn add_spring(&mut self, a: usize, b: usize, anchor_a: Vec2, anchor_b: Vec2, stiffness: f32, damping: f32) -> usize {
        let rest_length = anchor_b.minus(anchor_a).length();
        let anchor_a = anchor_a.minus(self.position_of(a));
        let anchor_b = anchor_b.minus(self.position_of(b));
        self.push_joint(Joint::Spring { a, b, anchor_a, anchor_b, rest_length, stiffness, damping })
    }

    /// Body `b` slides along `axis` (normalised here) relative to body `a`. The displacement along the axis, from the
    /// offset the bodies have now, stays in `lower..=upper`. Pass the infinities for no limit.
    pub fn add_slide(&mut self, a: usize, b: usize, axis: Vec2, lower: f32, upper: f32) -> usize {
        let len = axis.length();
        let axis = if len > 0.0 { axis.scaled(1.0 / len) } else { Vec2::new(1.0, 0.0) };
        let offset = self.position_of(b).minus(self.position_of(a));
        self.push_joint(Joint::Slide { a, b, offset, axis, lower, upper })
    }

    /// The joints in the order they were added.
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    /// Mutable access to a body, for the caller to place it or set its velocity between steps.
    pub fn body_mut(&mut self, id: usize) -> Option<&mut Body> {
        self.bodies.get_mut(id)
    }

    fn position_of(&self, id: usize) -> Vec2 {
        self.bodies.get(id).map(|b| b.position).unwrap_or(Vec2::ZERO)
    }

    fn push_joint(&mut self, joint: Joint) -> usize {
        self.joints.push(joint);
        self.joints.len() - 1
    }

    fn valid_pair(&self, a: usize, b: usize) -> bool {
        a != b && a < self.bodies.len() && b < self.bodies.len()
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
        self.apply_springs(dt);
        let mut contacts = Vec::new();
        let n = self.bodies.len();
        for i in 0..n {
            for j in (i + 1)..n {
                if let Some(c) = self.collide(i, j) {
                    contacts.push(c);
                }
            }
        }
        self.solve_joints();
        contacts
    }

    /// Spring forces, as velocity changes over `dt`. Equal and opposite on the two bodies, scaled by inverse mass.
    fn apply_springs(&mut self, dt: f32) {
        for joint in self.joints.clone() {
            if let Joint::Spring { a, b, anchor_a, anchor_b, rest_length, stiffness, damping } = joint {
                if !self.valid_pair(a, b) {
                    continue;
                }
                let (ba, bb) = (self.bodies[a], self.bodies[b]);
                let d = bb.position.plus(anchor_b).minus(ba.position.plus(anchor_a));
                let len = d.length();
                if len <= 0.0 {
                    continue;
                }
                let n = d.scaled(1.0 / len);
                let closing = bb.velocity.minus(ba.velocity).dot(n);
                let force = stiffness * (len - rest_length) + damping * closing;
                let k = force * dt;
                self.bodies[a].velocity = ba.velocity.plus(n.scaled(k * ba.inverse_mass()));
                self.bodies[b].velocity = bb.velocity.minus(n.scaled(k * bb.inverse_mass()));
            }
        }
    }

    /// One positional pass over the pin, fixed, limit and slide joints. Pin, fixed and slide correct the offset to the
    /// target. A limit joint corrects only when stretched, and then also removes the speed that pulls it further apart.
    fn solve_joints(&mut self) {
        for joint in self.joints.clone() {
            match joint {
                Joint::Pin { a, b, anchor_a, anchor_b } if self.valid_pair(a, b) => {
                    let diff = self.relative(a, b).plus(anchor_b.minus(anchor_a));
                    self.correct(a, b, diff);
                }
                Joint::Fixed { a, b, offset } if self.valid_pair(a, b) => {
                    let e = self.relative(a, b).minus(offset);
                    self.correct(a, b, e);
                }
                Joint::Slide { a, b, offset, axis, lower, upper } if self.valid_pair(a, b) => {
                    let r = self.relative(a, b).minus(offset);
                    let s = r.dot(axis);
                    let excess = s - s.max(lower).min(upper);
                    let perp = r.minus(axis.scaled(s));
                    let e = perp.plus(axis.scaled(excess));
                    self.correct(a, b, e);
                }
                Joint::Limit { a, b, anchor_a, anchor_b, max_length } if self.valid_pair(a, b) => {
                    let diff = self.relative(a, b).plus(anchor_b.minus(anchor_a));
                    let dist = diff.length();
                    if dist > max_length && dist > 0.0 {
                        let n = diff.scaled(1.0 / dist);
                        self.correct(a, b, n.scaled(dist - max_length));
                        self.remove_separating_speed(a, b, n);
                    }
                }
                _ => {}
            }
        }
    }

    /// Position of body b minus body a, from their centres.
    fn relative(&self, a: usize, b: usize) -> Vec2 {
        self.bodies[b].position.minus(self.bodies[a].position)
    }

    /// Moves the pair so that `relative(a, b)` drops by `e`, split by inverse mass. A static body does not move.
    fn correct(&mut self, a: usize, b: usize, e: Vec2) {
        let (ba, bb) = (self.bodies[a], self.bodies[b]);
        let (inv_a, inv_b) = (ba.inverse_mass(), bb.inverse_mass());
        let sum = inv_a + inv_b;
        if sum <= 0.0 {
            return;
        }
        self.bodies[a].position = ba.position.plus(e.scaled(inv_a / sum));
        self.bodies[b].position = bb.position.minus(e.scaled(inv_b / sum));
    }

    /// Cancels the speed of body b away from body a along `n`, if there is any. Bodies closing in are left alone.
    fn remove_separating_speed(&mut self, a: usize, b: usize, n: Vec2) {
        let (ba, bb) = (self.bodies[a], self.bodies[b]);
        let separating = bb.velocity.minus(ba.velocity).dot(n);
        let sum = ba.inverse_mass() + bb.inverse_mass();
        if separating <= 0.0 || sum <= 0.0 {
            return;
        }
        let k = separating / sum;
        self.bodies[a].velocity = ba.velocity.plus(n.scaled(k * ba.inverse_mass()));
        self.bodies[b].velocity = bb.velocity.minus(n.scaled(k * bb.inverse_mass()));
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

    #[test]
    fn spring_pulls_stretched_bodies_together() {
        // Rest length 3 (the anchors were 3 apart). b is moved to 5, so the extension is 2.
        // Force 10 * 2 = 20, impulse 20 * 0.1 = 2 per unit inverse mass, so va = +2 and vb = -2 along x.
        let mut w = World::new();
        let a = w.add(Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0));
        let b = w.add(Body::dynamic(Vec2::new(3.0, 0.0), 1.0, 1.0));
        w.add_spring(a, b, Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0), 10.0, 0.0);
        w.body_mut(b).unwrap().position = Vec2::new(5.0, 0.0);
        let contacts = w.step(0.1);
        assert!(contacts.is_empty());
        assert!(close(w.body(a).unwrap().velocity.x, 2.0));
        assert!(close(w.body(b).unwrap().velocity.x, -2.0));
    }

    #[test]
    fn pin_brings_the_two_anchor_points_onto_one_spot() {
        // Anchor at (1, 0) on a and on b. b moves to (2, 1), so its anchor is at (1, 1).
        // The error is (0, 1); each body moves half of it: a to y 0.5, b to y 0.5.
        let mut w = World::new();
        let a = w.add(Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0));
        let b = w.add(Body::dynamic(Vec2::new(2.0, 0.0), 1.0, 1.0));
        w.add_pin(a, b, Vec2::new(1.0, 0.0));
        w.body_mut(b).unwrap().position = Vec2::new(2.0, 1.0);
        let contacts = w.step(0.0);
        assert!(contacts.is_empty());
        let pa = w.body(a).unwrap().position;
        let pb = w.body(b).unwrap().position;
        assert!(close(pa.y, 0.5) && close(pb.y, 0.5));
        // Both anchor points now sit at (1, 0.5): a's is pa + (1, 0) and b's is pb + (-1, 0).
        assert!(close(pa.x + 1.0, 1.0));
        assert!(close(pb.x - 1.0, 1.0));
    }

    #[test]
    fn fixed_joint_keeps_the_offset_it_was_made_with() {
        // Offset (3, 0) when made. b is moved to (3, 1), so the error is (0, 1); each body moves half.
        let mut w = World::new();
        let a = w.add(Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0));
        let b = w.add(Body::dynamic(Vec2::new(3.0, 0.0), 1.0, 1.0));
        w.add_fixed(a, b);
        w.body_mut(b).unwrap().position = Vec2::new(3.0, 1.0);
        w.step(0.0);
        let d = w.relative(a, b);
        assert!(close(d.x, 3.0));
        assert!(close(d.y, 0.0));
    }

    #[test]
    fn limit_joint_is_a_rope_that_stops_speed_away_from_its_partner() {
        // Ends 3 apart, max length 2. The overshoot 1 is shared: a moves to x 0.5, b to x 2.5.
        // b's speed away from a is 1. Equal masses split the removal: a gains 0.5 and b drops by 0.5, so both are 0.5.
        let mut w = World::new();
        let a = w.add(Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0));
        let b = w.add(Body::dynamic(Vec2::new(3.0, 0.0), 1.0, 1.0));
        w.add_limit(a, b, Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0), 2.0);
        w.body_mut(b).unwrap().velocity = Vec2::new(1.0, 0.0);
        let contacts = w.step(0.0);
        assert!(contacts.is_empty());
        assert!(close(w.body(a).unwrap().position.x, 0.5));
        assert!(close(w.body(b).unwrap().position.x, 2.5));
        assert!(close(w.body(a).unwrap().velocity.x, 0.5));
        assert!(close(w.body(b).unwrap().velocity.x, 0.5));
    }

    #[test]
    fn slide_joint_keeps_the_body_on_its_axis_and_within_its_limits() {
        // Made with b at (5, 0). Moved to (8, 0): displacement (3, 0), limit 2 along x, so the excess 1 is shared.
        let mut w = World::new();
        let a = w.add(Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0));
        let b = w.add(Body::dynamic(Vec2::new(5.0, 0.0), 1.0, 1.0));
        w.add_slide(a, b, Vec2::new(1.0, 0.0), f32::NEG_INFINITY, 2.0);
        w.body_mut(b).unwrap().position = Vec2::new(8.0, 0.0);
        w.step(0.0);
        assert!(close(w.body(a).unwrap().position.x, 0.5));
        assert!(close(w.body(b).unwrap().position.x, 7.5));
        assert!(close(w.body(a).unwrap().position.y, 0.0));
    }

    #[test]
    fn slide_joint_removes_movement_off_the_axis() {
        // Made with b at (5, 0), moved to (6, 2). Along x the displacement is 1, off-axis error (0, 2).
        // Each body moves half of the error: a to y 1, b to y 1. Centre to centre is now (6, 0).
        let mut w = World::new();
        let a = w.add(Body::dynamic(Vec2::new(0.0, 0.0), 1.0, 1.0));
        let b = w.add(Body::dynamic(Vec2::new(5.0, 0.0), 1.0, 1.0));
        w.add_slide(a, b, Vec2::new(1.0, 0.0), f32::NEG_INFINITY, f32::INFINITY);
        w.body_mut(b).unwrap().position = Vec2::new(6.0, 2.0);
        w.step(0.0);
        assert!(close(w.body(a).unwrap().position.y, 1.0));
        assert!(close(w.body(b).unwrap().position.y, 1.0));
        assert!(close(w.relative(a, b).x, 6.0));
        assert!(close(w.relative(a, b).y, 0.0));
    }
}
