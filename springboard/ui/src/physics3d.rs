//! Three-dimensional rigid-body world: spheres under gravity, a ground plane at y = 0, and sphere-to-sphere contacts.
//! It is the 3D counterpart of the 2D physics world in `physics.rs`, with dynamic and fixed bodies.
//!
//! Pure: no clock and no thread. The caller steps the world with `step(dt)`. A step applies gravity to the velocity of
//! the dynamic bodies, moves them, stops any dynamic body that sinks below the ground, then resolves every overlapping
//! pair once. The pair is pushed apart in proportion to the inverse masses, and the closing speed along the contact
//! normal is reflected by the restitution, the smaller of the two bodies' values. Fixed bodies never move.
//! A step with `dt == 0` only resolves the overlaps that already exist.
//!
//! Units: metres, seconds and metres per second, with +Y up and the ground at y = 0.
//! `speed` scales the time step, as the framework does.

use crate::linalg::Vector3;

/// One spherical body. A dynamic body moves under gravity and collisions; a fixed one stays where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body3 {
    pub position: Vector3,
    pub velocity: Vector3,
    pub radius: f32,
    /// Mass of a dynamic body. A dynamic body with mass zero or less counts as mass 1.
    pub mass: f32,
    pub is_dynamic: bool,
    /// 0 is perfectly inelastic, 1 is perfectly elastic. Values outside 0..=1 are clamped when used.
    pub restitution: f32,
}

impl Body3 {
    /// A dynamic body with restitution 0.2.
    pub fn dynamic(position: Vector3, radius: f32, mass: f32) -> Body3 {
        Body3 { position, velocity: Vector3::ZERO, radius, mass, is_dynamic: true, restitution: 0.2 }
    }

    /// A fixed body with restitution 0.5.
    pub fn fixed(position: Vector3, radius: f32) -> Body3 {
        Body3 { position, velocity: Vector3::ZERO, radius, mass: 0.0, is_dynamic: false, restitution: 0.5 }
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

/// One overlap between two spheres found in a step. `normal` points from body `a` to body `b`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact3 {
    pub a: usize,
    pub b: usize,
    pub normal: Vector3,
    /// How far the two spheres overlapped before they were pushed apart.
    pub penetration: f32,
}

/// What one step found: the sphere overlaps, and the dynamic bodies that touched the ground.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepReport {
    pub contacts: Vec<Contact3>,
    pub ground: Vec<usize>,
}

/// The bodies and the settings that move them.
#[derive(Clone, Debug)]
pub struct World3 {
    pub gravity: Vector3,
    /// Multiplier on the time step passed to `step`.
    pub speed: f32,
    bodies: Vec<Body3>,
}

impl World3 {
    /// An empty world with no gravity and speed 1.
    pub fn new() -> World3 {
        World3 { gravity: Vector3::ZERO, speed: 1.0, bodies: Vec::new() }
    }

    /// Adds a body and returns its index.
    pub fn add_body(&mut self, body: Body3) -> usize {
        self.bodies.push(body);
        self.bodies.len() - 1
    }

    /// The bodies in the order they were added.
    pub fn bodies(&self) -> &[Body3] {
        &self.bodies
    }

    /// Mutable access to a body, for the caller to place it or set its velocity between steps.
    pub fn body_mut(&mut self, id: usize) -> Option<&mut Body3> {
        self.bodies.get_mut(id)
    }

    /// Advances the world by `dt` seconds times `speed`. A negative or non-finite step does nothing.
    pub fn step(&mut self, dt: f32) -> StepReport {
        let mut report = StepReport::default();
        let dt = dt * self.speed;
        if !(dt >= 0.0) || !dt.is_finite() {
            return report;
        }

        let gravity = self.gravity * dt;
        for body in self.bodies.iter_mut().filter(|b| b.is_dynamic) {
            body.velocity = body.velocity + gravity;
            body.position = body.position + body.velocity * dt;
        }

        for (id, body) in self.bodies.iter_mut().enumerate() {
            if body.is_dynamic && body.position.y < body.radius {
                body.position.y = body.radius;
                if body.velocity.y < 0.0 {
                    body.velocity.y = -body.velocity.y * body.restitution.clamp(0.0, 1.0);
                }
                report.ground.push(id);
            }
        }

        let count = self.bodies.len();
        for i in 0..count {
            for j in (i + 1)..count {
                let a = self.bodies[i];
                let b = self.bodies[j];
                if !a.is_dynamic && !b.is_dynamic {
                    continue;
                }
                let delta = b.position - a.position;
                let distance = delta.length();
                let min_distance = a.radius + b.radius;
                if !(distance < min_distance) {
                    continue;
                }
                let normal = if distance > 1e-6 {
                    delta * (1.0 / distance)
                } else {
                    Vector3::new(0.0, 1.0, 0.0)
                };
                let penetration = min_distance - distance;
                report.contacts.push(Contact3 { a: i, b: j, normal, penetration });

                let inv_a = a.inverse_mass();
                let inv_b = b.inverse_mass();
                let sum = inv_a + inv_b;
                if sum <= 0.0 {
                    continue;
                }
                let position_a = a.position - normal * (penetration * inv_a / sum);
                let position_b = b.position + normal * (penetration * inv_b / sum);
                let mut velocity_a = a.velocity;
                let mut velocity_b = b.velocity;

                let closing = (b.velocity - a.velocity).dot(normal);
                if closing < 0.0 {
                    let restitution = a.restitution.min(b.restitution).clamp(0.0, 1.0);
                    let impulse = -(1.0 + restitution) * closing / sum;
                    velocity_a = velocity_a - normal * (impulse * inv_a);
                    velocity_b = velocity_b + normal * (impulse * inv_b);
                }

                self.bodies[i].position = position_a;
                self.bodies[i].velocity = velocity_a;
                self.bodies[j].position = position_b;
                self.bodies[j].velocity = velocity_b;
            }
        }
        report
    }
}

impl Default for World3 {
    fn default() -> Self {
        World3::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn a_falling_body_follows_semi_implicit_euler() {
        let mut world = World3::new();
        world.gravity = Vector3::new(0.0, -10.0, 0.0);
        let id = world.add_body(Body3::dynamic(Vector3::new(0.0, 100.0, 0.0), 1.0, 1.0));
        let report = world.step(0.5);
        // Velocity first: -10 * 0.5 = -5. Then position: 100 + (-5) * 0.5 = 97.5.
        let body = world.bodies()[id];
        assert_eq!(body.velocity, Vector3::new(0.0, -5.0, 0.0));
        assert_eq!(body.position.y, 97.5);
        assert!(report.contacts.is_empty());
        assert!(report.ground.is_empty());
    }

    #[test]
    fn speed_scales_the_time_step() {
        let mut slow = World3::new();
        slow.gravity = Vector3::new(0.0, -10.0, 0.0);
        slow.add_body(Body3::dynamic(Vector3::new(0.0, 100.0, 0.0), 1.0, 1.0));
        slow.speed = 2.0;
        slow.step(0.25);

        let mut normal = World3::new();
        normal.gravity = Vector3::new(0.0, -10.0, 0.0);
        normal.add_body(Body3::dynamic(Vector3::new(0.0, 100.0, 0.0), 1.0, 1.0));
        normal.step(0.5);

        assert_eq!(slow.bodies()[0], normal.bodies()[0]);
    }

    #[test]
    fn the_ground_stops_a_sinking_body_and_reflects_its_speed_by_restitution() {
        let mut world = World3::new();
        let mut body = Body3::dynamic(Vector3::new(0.0, 1.0, 0.0), 1.0, 1.0);
        body.velocity = Vector3::new(0.0, -4.0, 0.0);
        body.restitution = 0.5;
        let id = world.add_body(body);
        // The step moves the body to y = 1 + (-4) * 0.25 = 0, which is 1 below its resting height of 1.
        let report = world.step(0.25);
        let after = world.bodies()[id];
        assert_eq!(after.position.y, 1.0);
        assert_eq!(after.velocity.y, 2.0);
        assert_eq!(report.ground, vec![id]);
    }

    #[test]
    fn a_fixed_body_does_not_move_and_pushes_a_dynamic_one_out() {
        let mut world = World3::new();
        let wall = world.add_body(Body3::fixed(Vector3::new(0.0, 1.0, 0.0), 1.0));
        let ball = world.add_body(Body3::dynamic(Vector3::new(1.5, 1.0, 0.0), 1.0, 1.0));
        let report = world.step(1.0);

        assert_eq!(world.bodies()[wall].position, Vector3::new(0.0, 1.0, 0.0));
        assert_eq!(report.contacts.len(), 1);
        assert_eq!(report.contacts[0].a, wall);
        assert_eq!(report.contacts[0].b, ball);
        assert!(close(report.contacts[0].penetration, 0.5));
        // Gravity is zero and the ball had no velocity, so only the overlap correction moves it: 1.5 + 0.5 = 2.0.
        assert!(close(world.bodies()[ball].position.x, 2.0));
        assert!(close(world.bodies()[ball].position.y, 1.0));
    }

    #[test]
    fn two_equal_elastic_spheres_swap_their_velocities_and_separate() {
        let mut world = World3::new();
        let mut left = Body3::dynamic(Vector3::new(0.0, 0.0, 0.0), 1.0, 1.0);
        left.velocity = Vector3::new(1.0, 0.0, 0.0);
        left.restitution = 1.0;
        let mut right = Body3::dynamic(Vector3::new(1.5, 0.0, 0.0), 1.0, 1.0);
        right.velocity = Vector3::new(-1.0, 0.0, 0.0);
        right.restitution = 1.0;
        let a = world.add_body(left);
        let b = world.add_body(right);
        // A zero step only resolves the overlap that is already there.
        world.step(0.0);

        let va = world.bodies()[a].velocity;
        let vb = world.bodies()[b].velocity;
        assert!(close(va.x, -1.0));
        assert!(close(vb.x, 1.0));
        // Each sphere moves half of the 0.5 overlap away from the other: the centres end 2.0 apart, just touching.
        assert!(close(world.bodies()[a].position.x, -0.25));
        assert!(close(world.bodies()[b].position.x, 1.75));
    }

    #[test]
    fn a_non_finite_step_leaves_the_world_unchanged() {
        let mut world = World3::new();
        world.gravity = Vector3::new(0.0, -10.0, 0.0);
        let id = world.add_body(Body3::dynamic(Vector3::new(0.0, 5.0, 0.0), 1.0, 1.0));
        let before = world.bodies()[id];
        let report = world.step(f32::NAN);
        assert_eq!(world.bodies()[id], before);
        assert_eq!(report, StepReport::default());
        let negative = world.step(-1.0);
        assert_eq!(world.bodies()[id], before);
        assert_eq!(negative, StepReport::default());
    }
}
