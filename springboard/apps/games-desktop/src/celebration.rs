//! The win celebration: a short burst of coloured dots over the board. The dots come from the 3D particle model
//! (`bifrost_app::controls::particle_system3d`) and are projected to the window with a simple perspective. The board
//! is flat, so depth only moves a dot away from the centre and makes it smaller.
//! Three emitters fan out for `BURST_S` seconds; each dot fades and shrinks as it ages.

use bifrost_app::controls::linalg::Vector3;
use bifrost_app::controls::particle_system3d::ParticleSystem;
use bifrost_app::kit::ui::{SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE};
use bifrost_app::kit::{Surface, Ui};

/// Distance from the eye to the board plane, in metres. A point at depth z is scaled by FOCAL / (FOCAL + z).
const FOCAL_M: f32 = 4.0;
/// How many points one metre of the model is on screen, before perspective.
const METRE_PT: f32 = 40.0;
/// Diameter of a dot in the board plane, in points.
const DOT_PT: f32 = 6.0;
/// How long the emitters keep spawning, in seconds.
const BURST_S: f32 = 0.6;
const BIRTH_RATE: f32 = 60.0;
const LIFE_S: f32 = 1.2;
/// Life spans lie in `LIFE_S - LIFE_RANGE_S / 2 .. LIFE_S + LIFE_RANGE_S / 2`.
const LIFE_RANGE_S: f32 = 0.4;
const SPEED_MS: f32 = 3.0;
const SPEED_RANGE_MS: f32 = 1.0;
const GRAVITY_MS2: f32 = -6.0;
const MAX_LIVE: usize = 200;
/// Emission direction of each emitter (x to the right, y up, z towards the eye). The three fan out.
const AIMS: [(f32, f32, f32); 3] = [(-0.6, 1.0, 0.0), (0.0, 1.0, 0.4), (0.6, 1.0, 0.0)];
const COLOURS: [u32; 3] = [SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE];
/// The longest step the burst takes at once, in seconds, so a long pause does not skip the animation.
const MAX_STEP_S: f32 = 0.1;

/// One burst of confetti, centred on a window pixel.
pub struct Celebration {
    emitters: Vec<ParticleSystem>,
    origin: (f32, f32),
    /// Seconds of emission left; the emitters stop when it reaches zero.
    firing_s: f32,
    /// The app clock (ms) at the last step.
    last_ms: f64,
}

impl Celebration {
    /// A burst centred on `origin` (window pixels), starting at app time `t_ms`. `seed` makes the dots repeat.
    pub fn start(seed: u64, origin: (f32, f32), t_ms: f64) -> Self {
        let emitters = AIMS
            .iter()
            .enumerate()
            .map(|(k, &(x, y, z))| {
                let mut e = ParticleSystem::new(seed.wrapping_add(k as u64));
                e.direction = Vector3::new(x, y, z);
                e.birth_rate = BIRTH_RATE;
                e.life_span = LIFE_S;
                e.life_span_variation = LIFE_RANGE_S;
                e.velocity = SPEED_MS;
                e.velocity_variation = SPEED_RANGE_MS;
                e.gravity = Vector3::new(0.0, GRAVITY_MS2, 0.0);
                e.max_particles = MAX_LIVE;
                e.start();
                e
            })
            .collect();
        Self { emitters, origin, firing_s: BURST_S, last_ms: t_ms }
    }

    /// Moves the burst to app time `t_ms`. Stops the emitters when the firing time is up. A non-finite time is ignored.
    pub fn advance(&mut self, t_ms: f64) {
        if !t_ms.is_finite() {
            return;
        }
        let raw = ((t_ms - self.last_ms) / 1000.0) as f32;
        self.last_ms = t_ms;
        let step = if raw.is_finite() { raw.clamp(0.0, MAX_STEP_S) } else { 0.0 };
        self.firing_s -= step;
        if self.firing_s <= 0.0 {
            for e in self.emitters.iter_mut() {
                e.stop();
            }
        }
        for e in self.emitters.iter_mut() {
            e.advance(step);
        }
    }

    /// Live particles across all three emitters.
    pub fn live(&self) -> usize {
        self.emitters.iter().map(|e| e.particles().len()).sum()
    }

    /// True once the emitters have stopped and the last particle has died.
    pub fn is_done(&self) -> bool {
        self.firing_s <= 0.0 && self.live() == 0
    }

    /// Draws every live particle as a dot, fading and shrinking as it ages.
    pub fn draw(&self, s: &mut Surface, ui: &Ui) {
        let px_per_m = ui.pt(METRE_PT);
        let dot = ui.pt(DOT_PT);
        for (e, colour) in self.emitters.iter().zip(COLOURS) {
            for p in e.particles() {
                let Some((x, y, f)) = project(p.position, self.origin, px_per_m) else { continue };
                let left = if p.lifetime > 0.0 { (1.0 - p.age / p.lifetime).clamp(0.0, 1.0) } else { 0.0 };
                let r = dot * 0.5 * f * left;
                if r >= 0.5 {
                    s.fill_rrect(x - r, y - r, 2.0 * r, 2.0 * r, r, colour);
                }
            }
        }
    }
}

/// Projects a point of the model to window pixels about `origin`. Returns (x, y, f), where f is the perspective scale,
/// or None for a point at or behind the eye. The model's y axis points up; window pixels point down.
pub fn project(at: Vector3, origin: (f32, f32), px_per_m: f32) -> Option<(f32, f32, f32)> {
    let depth = FOCAL_M + at.z;
    if !(depth > 0.1) {
        return None;
    }
    let f = FOCAL_M / depth;
    Some((origin.0 + at.x * px_per_m * f, origin.1 - at.y * px_per_m * f, f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_in_the_board_plane_keeps_the_metre_scale_and_depth_shrinks_it() {
        // In the plane (z = 0) the scale is 1: 1 m right at 10 px/m is 10 px right, and 2 m up is 20 px up.
        assert_eq!(project(Vector3::new(1.0, 2.0, 0.0), (100.0, 100.0), 10.0), Some((110.0, 80.0, 1.0)));
        // At z = 4 the depth is 8, so the scale halves: 5 px right and 10 px up.
        assert_eq!(project(Vector3::new(1.0, 2.0, 4.0), (100.0, 100.0), 10.0), Some((105.0, 90.0, 0.5)));
        // At or behind the eye there is no projection.
        assert_eq!(project(Vector3::new(0.0, 0.0, -4.0), (0.0, 0.0), 10.0), None);
        assert_eq!(project(Vector3::new(0.0, 0.0, -5.0), (0.0, 0.0), 10.0), None);
    }

    #[test]
    fn the_burst_fires_for_its_time_then_dies_out() {
        let mut c = Celebration::start(9, (0.0, 0.0), 0.0);
        // 60 a second for 0.1 s is six from each of the three emitters: 18 in all.
        c.advance(100.0);
        assert_eq!(c.live(), 18);
        assert!(!c.is_done());
        // Still alive at 1 s: the particles spawned up to 0.6 s have life spans of at least 1.0 s.
        for k in 2..=10 {
            c.advance(k as f64 * 100.0);
        }
        assert!(!c.is_done());
        // The last spawn is at 0.6 s at the latest and lives at most 1.4 s, so everything is gone by 2.0 s.
        for k in 11..=21 {
            c.advance(k as f64 * 100.0);
        }
        assert!(c.is_done());
    }

    #[test]
    fn a_long_pause_moves_the_burst_by_at_most_a_tenth_of_a_second() {
        let mut c = Celebration::start(3, (0.0, 0.0), 0.0);
        c.advance(5000.0);
        // The step is clamped to 0.1 s, so the burst is still firing and has spawned 18 dots, not a full second's worth.
        assert_eq!(c.live(), 18);
        assert!(!c.is_done());
    }

    #[test]
    fn a_non_finite_time_is_ignored() {
        let mut c = Celebration::start(4, (0.0, 0.0), 0.0);
        c.advance(f64::NAN);
        assert_eq!(c.live(), 0);
        c.advance(100.0);
        assert_eq!(c.live(), 18);
    }
}
