//! A 3D particle system: an emitter at a point that spawns particles at a birth rate, gives each one a speed along
//! the emitting direction, and moves it under a constant acceleration (gravity) until its life span ends.
//! It is the 3D counterpart of the 2D emitter in `emitter`, and it uses that module's seeded generator.
//! Pure: no clock and no drawing. The caller advances the simulation with `advance(dt)` and draws `particles()`.
//! Units are metres and seconds. Integration is semi-implicit Euler: velocity first, then position.

use crate::emitter::Rng;
use crate::linalg::Vector3;

/// One live particle. `age` counts up from zero and the particle is removed once it reaches `lifetime`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle3 {
    pub position: Vector3,
    pub velocity: Vector3,
    pub age: f32,
    pub lifetime: f32,
}

/// A 3D emitter. The public fields are its settings; change them between `advance` calls.
#[derive(Clone, Debug)]
pub struct ParticleSystem {
    /// Where new particles appear.
    pub position: Vector3,
    /// Direction new particles leave in. A zero or non-finite vector is treated as up (+y).
    pub direction: Vector3,
    /// Particles spawned per second while emitting.
    pub birth_rate: f32,
    /// Mean life span in seconds, and the range of randomized life spans around it.
    pub life_span: f32,
    pub life_span_variation: f32,
    /// Mean speed in metres per second, and the range of randomized speeds around it.
    pub velocity: f32,
    pub velocity_variation: f32,
    /// Constant acceleration applied to every live particle, in metres per second squared.
    pub gravity: Vector3,
    /// Upper bound on live particles. Spawns beyond it are dropped.
    pub max_particles: usize,
    emitting: bool,
    carry: f32,
    rng: Rng,
    particles: Vec<Particle3>,
}

impl ParticleSystem {
    /// A stopped emitter with no particles: birth rate 0, life span 1 s, speed 0, no gravity, up to 1000 particles.
    pub fn new(seed: u64) -> Self {
        Self {
            position: Vector3::ZERO,
            direction: Vector3::new(0.0, 1.0, 0.0),
            birth_rate: 0.0,
            life_span: 1.0,
            life_span_variation: 0.0,
            velocity: 0.0,
            velocity_variation: 0.0,
            gravity: Vector3::ZERO,
            max_particles: 1000,
            emitting: false,
            carry: 0.0,
            rng: Rng::new(seed),
            particles: Vec::new(),
        }
    }

    /// Starts spawning particles at the birth rate.
    pub fn start(&mut self) {
        self.emitting = true;
    }

    /// Stops spawning. Live particles keep moving until their life spans end.
    pub fn stop(&mut self) {
        self.emitting = false;
    }

    pub fn is_emitting(&self) -> bool {
        self.emitting
    }

    /// The live particles, in spawn order.
    pub fn particles(&self) -> &[Particle3] {
        &self.particles
    }

    /// Removes every particle and clears the fractional spawn carry. The emitting state is kept.
    pub fn reset(&mut self) {
        self.particles.clear();
        self.carry = 0.0;
    }

    /// Moves the live particles by `dt` seconds, removes the ones whose life span has ended, then spawns the
    /// particles due in this step while emitting. A non-positive or non-finite `dt` does nothing.
    pub fn advance(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let gravity = self.gravity;
        for p in self.particles.iter_mut() {
            p.age += dt;
            p.velocity = p.velocity + gravity * dt;
            p.position = p.position + p.velocity * dt;
        }
        self.particles.retain(|p| p.age < p.lifetime);

        if self.emitting && self.birth_rate > 0.0 {
            self.carry += self.birth_rate * dt;
            let due = self.carry.floor();
            self.carry -= due;
            let dir = self.unit_direction();
            for _ in 0..(due as usize) {
                if self.particles.len() >= self.max_particles {
                    break;
                }
                let lifetime = self.rng.around(self.life_span, self.life_span_variation).max(0.0);
                let speed = self.rng.around(self.velocity, self.velocity_variation).max(0.0);
                self.particles.push(Particle3 {
                    position: self.position,
                    velocity: dir * speed,
                    age: 0.0,
                    lifetime,
                });
            }
        }
    }

    fn unit_direction(&self) -> Vector3 {
        let d = self.direction;
        let len = d.length();
        if len.is_finite() && len > 1e-6 {
            d * (1.0 / len)
        } else {
            Vector3::new(0.0, 1.0, 0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn birth_rate_spawns_whole_particles_and_carries_the_remainder() {
        let mut s = ParticleSystem::new(1);
        s.birth_rate = 10.0;
        s.life_span = 10.0;
        s.start();
        // 10 per second for 0.25 s is 2.5: two spawn, 0.5 carries over.
        s.advance(0.25);
        assert_eq!(s.particles().len(), 2);
        // 0.5 + 2.5 = 3.0: three more spawn.
        s.advance(0.25);
        assert_eq!(s.particles().len(), 5);
    }

    #[test]
    fn gravity_bends_the_path_and_the_particle_dies_at_its_life_span() {
        let mut s = ParticleSystem::new(2);
        s.birth_rate = 1.0;
        s.life_span = 1.0;
        s.velocity = 4.0;
        s.direction = Vector3::new(1.0, 0.0, 0.0);
        s.gravity = Vector3::new(0.0, -10.0, 0.0);
        s.start();
        // The particle is spawned at the end of this step, so it has not moved yet.
        s.advance(1.0);
        assert_eq!(s.particles().len(), 1);
        assert_eq!(s.particles()[0].velocity, Vector3::new(4.0, 0.0, 0.0));
        s.stop();
        // Half a second: velocity becomes (4, -5, 0), position becomes (4, -5, 0) * 0.5 = (2, -2.5, 0).
        s.advance(0.5);
        let p = s.particles()[0];
        assert!(close(p.age, 0.5, 1e-6));
        assert!(close(p.velocity.y, -5.0, 1e-5));
        assert!(close(p.position.x, 2.0, 1e-5));
        assert!(close(p.position.y, -2.5, 1e-5));
        // Another half second brings age to the 1.0 s life span: the particle is removed.
        s.advance(0.5);
        assert!(s.particles().is_empty());
    }

    #[test]
    fn particles_appear_at_the_emitter_position() {
        let mut s = ParticleSystem::new(3);
        s.position = Vector3::new(5.0, 1.0, -2.0);
        s.birth_rate = 1.0;
        s.start();
        s.advance(1.0);
        assert_eq!(s.particles()[0].position, Vector3::new(5.0, 1.0, -2.0));
    }

    #[test]
    fn population_is_capped_at_max_particles() {
        let mut s = ParticleSystem::new(4);
        s.birth_rate = 10.0;
        s.life_span = 10.0;
        s.max_particles = 2;
        s.start();
        s.advance(1.0);
        assert_eq!(s.particles().len(), 2);
        s.advance(1.0);
        assert_eq!(s.particles().len(), 2);
    }

    #[test]
    fn randomized_life_span_stays_within_its_range_and_repeats_for_a_seed() {
        let make = || {
            let mut s = ParticleSystem::new(77);
            s.birth_rate = 3.0;
            s.life_span = 2.0;
            s.life_span_variation = 1.0;
            s.start();
            s.advance(1.0);
            s
        };
        let a = make();
        let b = make();
        assert_eq!(a.particles(), b.particles());
        for p in a.particles() {
            // Mean 2 with range 1: lifetimes lie in 1.5 .. 2.5.
            assert!(p.lifetime >= 1.5 && p.lifetime < 2.5, "lifetime {}", p.lifetime);
        }
    }

    #[test]
    fn invalid_time_step_and_zero_direction_are_handled() {
        let mut s = ParticleSystem::new(5);
        s.birth_rate = 1.0;
        s.velocity = 2.0;
        s.direction = Vector3::ZERO;
        s.start();
        s.advance(0.0);
        s.advance(f32::NAN);
        assert!(s.particles().is_empty());
        s.advance(1.0);
        // A zero direction falls back to up, so the speed of 2 goes along +y.
        assert_eq!(s.particles()[0].velocity, Vector3::new(0.0, 2.0, 0.0));
    }
}
