//! A particle emitter model. Each cell emits particles at a rate that accumulates across steps, and every particle
//! takes a lifetime and a velocity from its cell's ranges. A step first advances the live particles and expires the
//! ones past their lifetime, then spawns new ones at the emitter position. Randomness comes from a seeded generator,
//! so one seed always gives the same particles. This is a pure simulation; the caller draws the particles.

/// The emission parameters of one cell. Angles are in radians; a range is the full width around the centre value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// Particles per second before the emitter multiplier.
    pub birth_rate: f32,
    /// Seconds a particle lives, before the emitter multiplier, plus or minus half of `lifetime_range`.
    pub lifetime: f32,
    pub lifetime_range: f32,
    /// Units per second, plus or minus half of `velocity_range`.
    pub velocity: f32,
    pub velocity_range: f32,
    /// The centre direction of emission.
    pub emission_longitude: f32,
    /// The full width of the emission cone around `emission_longitude`.
    pub emission_range: f32,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            birth_rate: 0.0,
            lifetime: 1.0,
            lifetime_range: 0.0,
            velocity: 0.0,
            velocity_range: 0.0,
            emission_longitude: 0.0,
            emission_range: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub age: f32,
    pub life: f32,
    /// The index of the cell that emitted this particle.
    pub cell: usize,
}

#[derive(Debug)]
pub struct Emitter {
    pub position: (f32, f32),
    pub cells: Vec<Cell>,
    /// Multipliers applied to every cell's birth rate, velocity and lifetime.
    pub birth_multiplier: f32,
    pub velocity_multiplier: f32,
    pub lifetime_multiplier: f32,
    pub particles: Vec<Particle>,
    /// Fractional particles carried over from the last step, per cell.
    accum: Vec<f32>,
    state: u32,
}

impl Emitter {
    pub fn new(cells: Vec<Cell>, seed: u32) -> Self {
        let accum = vec![0.0; cells.len()];
        Emitter {
            position: (0.0, 0.0),
            cells,
            birth_multiplier: 1.0,
            velocity_multiplier: 1.0,
            lifetime_multiplier: 1.0,
            particles: Vec::new(),
            accum,
            state: seed,
        }
    }

    /// A uniform value in [0, 1) from a linear congruential generator.
    fn next_unit(&mut self) -> f32 {
        self.state = self.state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.state >> 8) as f32 / 16_777_216.0
    }

    /// Advances the simulation by `dt` seconds. A negative `dt` is treated as zero.
    pub fn step(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        for p in self.particles.iter_mut() {
            p.age += dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
        }
        self.particles.retain(|p| p.age < p.life);
        for c in 0..self.cells.len() {
            let rate = self.cells[c].birth_rate * self.birth_multiplier;
            self.accum[c] += rate * dt;
            let n = self.accum[c].floor();
            self.accum[c] -= n;
            for _ in 0..(n as usize) {
                self.spawn(c);
            }
        }
    }

    fn spawn(&mut self, c: usize) {
        let cell = self.cells[c];
        let life = ((cell.lifetime + (self.next_unit() - 0.5) * cell.lifetime_range) * self.lifetime_multiplier).max(0.0);
        let speed = (cell.velocity + (self.next_unit() - 0.5) * cell.velocity_range) * self.velocity_multiplier;
        let angle = cell.emission_longitude + (self.next_unit() - 0.5) * cell.emission_range;
        let (x, y) = self.position;
        self.particles.push(Particle {
            x,
            y,
            vx: speed * angle.cos(),
            vy: speed * angle.sin(),
            age: 0.0,
            life,
            cell: c,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate_cell(rate: f32) -> Cell {
        Cell { birth_rate: rate, ..Default::default() }
    }

    #[test]
    fn particles_spawn_at_the_rate_and_expire_after_their_lifetime() {
        // 8 per second over steps of 0.25 s: 2 per step. A particle lives 1 s, so the first group expires at step 5.
        let mut e = Emitter::new(vec![rate_cell(8.0)], 1);
        for _ in 0..4 {
            e.step(0.25);
        }
        assert_eq!(e.particles.len(), 8);
        e.step(0.25);
        assert_eq!(e.particles.len(), 8);
        assert!(e.particles.iter().all(|p| p.age < 1.0));
    }

    #[test]
    fn fractional_births_carry_over_between_steps() {
        // 1 per second over steps of 0.5 s gives half a particle per step: one spawn every two steps.
        let mut e = Emitter::new(vec![rate_cell(1.0)], 7);
        e.step(0.5);
        assert_eq!(e.particles.len(), 0);
        e.step(0.5);
        assert_eq!(e.particles.len(), 1);
    }

    #[test]
    fn particles_move_by_their_velocity_after_they_spawn() {
        // Rate 2 per second over 0.5 s spawns one particle at the origin with velocity (10, 0).
        let mut e = Emitter::new(vec![Cell { birth_rate: 2.0, velocity: 10.0, ..Default::default() }], 3);
        e.step(0.5);
        e.step(0.5);
        assert_eq!(e.particles.len(), 2);
        // The first particle moved 10 * 0.5 = 5 units in the second step; the second spawned after the move.
        assert_eq!(e.particles[0].x, 5.0);
        assert_eq!(e.particles[1].x, 0.0);
    }

    #[test]
    fn the_emission_direction_follows_the_longitude() {
        let cell = Cell { birth_rate: 1.0, velocity: 10.0, emission_longitude: std::f32::consts::FRAC_PI_2, ..Default::default() };
        let mut e = Emitter::new(vec![cell], 5);
        e.step(1.0);
        let p = e.particles[0];
        assert!(p.vx.abs() < 1e-5);
        assert!((p.vy - 10.0).abs() < 1e-5);
    }

    #[test]
    fn multipliers_scale_birth_velocity_and_lifetime() {
        // Base life 1 s with steps of 0.25 s and one particle per step: 4 are alive after 4 steps, and after 5 steps
        // the first has aged 1.0 s and expires. A lifetime multiplier of 2 keeps it, so all 5 stay alive.
        let mut base = Emitter::new(vec![rate_cell(4.0)], 2);
        let mut long = Emitter::new(vec![rate_cell(4.0)], 2);
        long.lifetime_multiplier = 2.0;
        for _ in 0..5 {
            base.step(0.25);
            long.step(0.25);
        }
        assert_eq!(base.particles.len(), 4);
        assert_eq!(long.particles.len(), 5);

        let mut fast = Emitter::new(vec![Cell { birth_rate: 8.0, velocity: 5.0, ..Default::default() }], 4);
        fast.velocity_multiplier = 2.0;
        fast.step(0.25);
        assert_eq!(fast.particles[0].vx, 10.0);
        assert_eq!(fast.particles[0].vy, 0.0);
    }

    #[test]
    fn a_seed_fixes_the_random_values_and_ranges_bound_them() {
        let cell = Cell {
            birth_rate: 40.0,
            lifetime: 1.0,
            lifetime_range: 0.5,
            velocity: 5.0,
            velocity_range: 1.0,
            emission_range: 0.5,
            ..Default::default()
        };
        let mut a = Emitter::new(vec![cell], 42);
        let mut b = Emitter::new(vec![cell], 42);
        a.step(1.0);
        b.step(1.0);
        assert_eq!(a.particles, b.particles);
        assert_eq!(a.particles.len(), 40);
        for p in &a.particles {
            assert!(p.life >= 0.75 && p.life <= 1.25);
            let speed = (p.vx * p.vx + p.vy * p.vy).sqrt();
            assert!(speed >= 4.499 && speed <= 5.501);
        }
    }

    #[test]
    fn a_zero_birth_rate_or_a_negative_step_spawns_nothing() {
        let mut e = Emitter::new(vec![rate_cell(0.0)], 9);
        e.step(1.0);
        assert!(e.particles.is_empty());
        let mut f = Emitter::new(vec![rate_cell(8.0)], 9);
        f.step(-1.0);
        assert!(f.particles.is_empty());
    }
}
