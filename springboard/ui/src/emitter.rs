//! Particle emitter model: a source that creates particles at a birth rate and moves each one under its own velocity
//! and acceleration until its lifetime ends. It is the counterpart of the 2D scene framework's emitter node.
//!
//! Pure: no clock and no drawing. The caller advances the simulation with `advance(dt)` and draws `particles()`.
//! Every random value comes from a seeded generator, so a run can be repeated exactly. A "mean and range" pair gives
//! values in `mean - range / 2 .. mean + range / 2`.
//!
//! Angles are in radians (counter-clockwise from +x), speeds and positions in points, lifetimes in seconds. Y grows
//! upwards, as in the rest of the scene model.

/// Seeded xorshift64 generator: the random source of an emitter.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // xorshift must not start at zero, so a zero seed takes a fixed non-zero constant.
        let state = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
        Rng { state }
    }

    /// Next value in `0.0..1.0`, with 24 bits of precision.
    pub fn next_unit(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        (x >> 40) as f32 / 16_777_216.0
    }

    /// A value in `mean - range / 2 .. mean + range / 2` (the upper end is never reached).
    pub fn around(&mut self, mean: f32, range: f32) -> f32 {
        mean + (self.next_unit() - 0.5) * range
    }
}

/// One live particle. `age` counts up from zero; the particle is removed once `age` reaches `lifetime`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub age: f32,
    pub lifetime: f32,
    pub scale: f32,
    pub rotation: f32,
}

/// Emitter settings and the particles it has alive.
#[derive(Clone, Debug)]
pub struct Emitter {
    /// Where new particles start, in points.
    pub position: (f32, f32),
    pub position_range: (f32, f32),
    /// New particles per second.
    pub birth_rate: f32,
    /// Stop after this many particles. `None` keeps emitting.
    pub num_to_emit: Option<u32>,
    pub lifetime: f32,
    pub lifetime_range: f32,
    pub speed: f32,
    pub speed_range: f32,
    /// Direction of the initial velocity, in radians.
    pub angle: f32,
    pub angle_range: f32,
    pub x_acceleration: f32,
    pub y_acceleration: f32,
    pub scale: f32,
    pub scale_range: f32,
    /// Change of scale per second.
    pub scale_speed: f32,
    pub rotation: f32,
    pub rotation_range: f32,
    /// Change of rotation per second, in radians.
    pub rotation_speed: f32,
    rng: Rng,
    particles: Vec<Particle>,
    emitted: u32,
    pending: f32,
}

impl Emitter {
    /// An emitter that makes nothing until its birth rate is set. Lifetime and scale start at 1.
    pub fn new(seed: u64) -> Emitter {
        Emitter {
            position: (0.0, 0.0),
            position_range: (0.0, 0.0),
            birth_rate: 0.0,
            num_to_emit: None,
            lifetime: 1.0,
            lifetime_range: 0.0,
            speed: 0.0,
            speed_range: 0.0,
            angle: 0.0,
            angle_range: 0.0,
            x_acceleration: 0.0,
            y_acceleration: 0.0,
            scale: 1.0,
            scale_range: 0.0,
            scale_speed: 0.0,
            rotation: 0.0,
            rotation_range: 0.0,
            rotation_speed: 0.0,
            rng: Rng::new(seed),
            particles: Vec::new(),
            emitted: 0,
            pending: 0.0,
        }
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    /// How many particles have been born since the last reset.
    pub fn emitted(&self) -> u32 {
        self.emitted
    }

    /// True once the emitter has reached its `num_to_emit` count.
    pub fn is_stopped(&self) -> bool {
        self.num_to_emit.map_or(false, |n| self.emitted >= n)
    }

    /// True when the emitter has stopped and its last particle has died.
    pub fn is_finished(&self) -> bool {
        self.particles.is_empty() && self.is_stopped()
    }

    /// Removes every particle and starts the count again.
    pub fn reset(&mut self) {
        self.particles.clear();
        self.emitted = 0;
        self.pending = 0.0;
    }

    /// Moves the simulation on by `dt` seconds. Existing particles move first and dead ones are dropped. Then new
    /// particles are born at their start position with age zero, so a particle is not moved in the step it is born.
    /// Births that do not fill a whole particle carry over to the next step.
    pub fn advance(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let (ax, ay) = (self.x_acceleration, self.y_acceleration);
        let (scale_speed, rotation_speed) = (self.scale_speed, self.rotation_speed);
        for p in self.particles.iter_mut() {
            p.age += dt;
            p.vx += ax * dt;
            p.vy += ay * dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.scale += scale_speed * dt;
            p.rotation += rotation_speed * dt;
        }
        self.particles.retain(|p| p.age < p.lifetime);

        if self.is_stopped() {
            self.pending = 0.0;
            return;
        }
        self.pending += self.birth_rate.max(0.0) * dt;
        while self.pending >= 1.0 && !self.is_stopped() {
            self.pending -= 1.0;
            let p = self.spawn();
            self.particles.push(p);
        }
        if self.is_stopped() {
            self.pending = 0.0;
        }
    }

    fn spawn(&mut self) -> Particle {
        let x = self.rng.around(self.position.0, self.position_range.0);
        let y = self.rng.around(self.position.1, self.position_range.1);
        let speed = self.rng.around(self.speed, self.speed_range);
        let angle = self.rng.around(self.angle, self.angle_range);
        let lifetime = self.rng.around(self.lifetime, self.lifetime_range).max(0.0);
        let scale = self.rng.around(self.scale, self.scale_range);
        let rotation = self.rng.around(self.rotation, self.rotation_range);
        self.emitted += 1;
        Particle {
            x,
            y,
            vx: speed * angle.cos(),
            vy: speed * angle.sin(),
            age: 0.0,
            lifetime,
            scale,
            rotation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn birth_rate_spawns_whole_particles_and_carries_the_rest() {
        let mut e = Emitter::new(1);
        e.birth_rate = 10.0;
        e.advance(0.25); // 2.5 births: two particles, 0.5 carried over
        assert_eq!(e.particles().len(), 2);
        assert_eq!(e.emitted(), 2);
        e.advance(0.25); // 0.5 + 2.5 = 3.0 births
        assert_eq!(e.particles().len(), 5);
        assert_eq!(e.emitted(), 5);
    }

    #[test]
    fn acceleration_moves_a_particle_only_after_its_birth_step() {
        let mut e = Emitter::new(1);
        e.birth_rate = 1.0;
        e.y_acceleration = -10.0;
        e.advance(1.0); // one particle is born at rest and is not moved in this step
        let p = e.particles()[0];
        assert!(close(p.y, 0.0) && close(p.vy, 0.0));
        e.advance(0.5); // no birth here: the carry is only 0.5
        let p = e.particles()[0];
        assert!(close(p.age, 0.5));
        assert!(close(p.vy, -5.0)); // -10 * 0.5
        assert!(close(p.y, -2.5)); // -5 * 0.5
        assert_eq!(e.particles().len(), 1);
    }

    #[test]
    fn speed_and_angle_split_into_velocity() {
        let mut e = Emitter::new(7);
        e.birth_rate = 1.0;
        e.speed = 10.0;
        e.angle = std::f32::consts::FRAC_PI_2; // straight up
        e.advance(1.0);
        let p = e.particles()[0];
        assert!(close(p.vx, 0.0));
        assert!(close(p.vy, 10.0));
    }

    #[test]
    fn sampled_speed_stays_inside_mean_plus_and_minus_half_the_range() {
        let mut e = Emitter::new(42);
        e.birth_rate = 1000.0;
        e.speed = 10.0;
        e.speed_range = 4.0;
        e.advance(1.0);
        let speeds: Vec<f32> = e.particles().iter().map(|p| (p.vx * p.vx + p.vy * p.vy).sqrt()).collect();
        assert_eq!(speeds.len(), 1000);
        assert!(speeds.iter().all(|s| *s >= 7.999 && *s <= 12.001));
        // The range is actually used: the values are not all the same.
        assert!(speeds.iter().any(|s| (s - speeds[0]).abs() > 0.01));
    }

    #[test]
    fn particles_die_when_their_age_reaches_the_lifetime() {
        let mut e = Emitter::new(3);
        e.birth_rate = 1.0;
        e.lifetime = 0.5;
        e.advance(1.0); // born, age 0
        e.advance(0.25); // age 0.25, alive
        assert_eq!(e.particles().len(), 1);
        e.advance(0.25); // age 0.5 is not less than 0.5, removed
        assert_eq!(e.particles().len(), 0);
    }

    #[test]
    fn num_to_emit_stops_births_and_the_emitter_finishes() {
        let mut e = Emitter::new(5);
        e.birth_rate = 100.0;
        e.num_to_emit = Some(3);
        e.lifetime = 0.5;
        e.advance(1.0);
        assert_eq!(e.emitted(), 3);
        assert_eq!(e.particles().len(), 3);
        assert!(!e.is_finished());
        e.advance(1.0); // all three are older than their lifetime and no more are born
        assert_eq!(e.particles().len(), 0);
        assert_eq!(e.emitted(), 3);
        assert!(e.is_finished());
    }

    #[test]
    fn reset_clears_particles_and_restarts_the_count() {
        let mut e = Emitter::new(9);
        e.birth_rate = 10.0;
        e.advance(1.0);
        assert_eq!(e.particles().len(), 10);
        e.reset();
        assert_eq!(e.particles().len(), 0);
        assert_eq!(e.emitted(), 0);
        e.advance(0.1); // one birth from 1.0 of carry
        assert_eq!(e.particles().len(), 1);
    }
}
