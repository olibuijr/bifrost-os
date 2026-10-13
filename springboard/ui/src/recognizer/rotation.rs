//! Rotation: two fingers turning around each other.

use super::*;
use std::f32::consts::{PI, TAU};

/// Fingers closer than this give no stable angle (points).
const MIN_SEPARATION: f32 = 10.0;

/// Smallest turn that begins the gesture (radians, about 3 degrees).
pub const ROTATION_SLOP: f32 = 0.05;

/// Wraps an angle difference into (-PI, PI].
fn wrap(a: f32) -> f32 {
    let mut a = a % TAU;
    if a > PI {
        a -= TAU;
    } else if a <= -PI {
        a += TAU;
    }
    a
}

/// Continuous: follows the first two fingers. `rotation` is the accumulated turn of the line between them in
/// radians, positive clockwise on a screen whose y axis points down. It is not wrapped, so several turns add up.
#[derive(Debug)]
pub struct Rotation {
    core: Core,
    /// Turn before the gesture begins (radians).
    pub slop: f32,
    touches: TouchSet,
    pair: TwoTouch,
    last_angle: Option<f32>,
    rotation: f32,
    focal: (f32, f32),
    smoother: Option<Smoother>,
    velocity: f32,
}

impl Rotation {
    pub fn new() -> Self {
        Rotation {
            core: Core::new(),
            slop: ROTATION_SLOP,
            touches: TouchSet::default(),
            pair: TwoTouch::default(),
            last_angle: None,
            rotation: 0.0,
            focal: (0.0, 0.0),
            smoother: None,
            velocity: 0.0,
        }
    }

    /// Accumulated turn in radians.
    pub fn rotation(&self) -> f32 {
        self.rotation
    }

    pub fn rotation_degrees(&self) -> f32 {
        self.rotation.to_degrees()
    }

    /// Angular velocity in radians per second as of the last sample.
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Midpoint between the two fingers.
    pub fn focal(&self) -> (f32, f32) {
        self.focal
    }

    fn clear(&mut self) {
        self.core.reset();
        self.touches.clear();
        self.pair = TwoTouch::default();
        self.last_angle = None;
        self.rotation = 0.0;
        self.smoother = None;
        self.velocity = 0.0;
    }
}

impl Default for Rotation {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer for Rotation {
    recognizer_plumbing!();

    fn feed(&mut self, s: &TouchSample) {
        if needs_reset(&self.core, &self.touches, s) {
            self.clear();
        }
        if !self.touches.apply(s) {
            return;
        }
        if s.phase == TouchPhase::Began {
            self.pair.add(s.id);
        }
        let tracked = self.pair.has(s.id);
        let cur = self.core.current();
        if !cur.is_terminal() {
            if let Some((a, b)) = self.pair.positions(&self.touches) {
                self.focal = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                if dist(a, b) >= MIN_SEPARATION {
                    let angle = (b.1 - a.1).atan2(b.0 - a.0);
                    let before = self.rotation;
                    if let Some(last) = self.last_angle {
                        self.rotation += wrap(angle - last);
                    }
                    self.last_angle = Some(angle);
                    let rest = s.phase == TouchPhase::Ended && self.rotation == before;
                    if !rest {
                        self.smoother.get_or_insert_with(|| Smoother::new(before, s.t)).push(self.rotation, s.t);
                    }
                    if let Some(sm) = &self.smoother {
                        self.velocity = sm.at(s.t);
                    }
                }
            }
            match s.phase {
                TouchPhase::Cancelled if tracked => {
                    self.core.go(if cur == GestureState::Possible { GestureState::Failed } else { GestureState::Cancelled });
                }
                TouchPhase::Ended if tracked => {
                    self.core.go(if cur == GestureState::Possible { GestureState::Failed } else { GestureState::Ended });
                }
                TouchPhase::Began | TouchPhase::Moved => {
                    if self.last_angle.is_some() {
                        if cur == GestureState::Possible {
                            if self.rotation.abs() >= self.slop {
                                self.core.go(GestureState::Began);
                            }
                        } else if tracked {
                            self.core.go(GestureState::Changed);
                        }
                    }
                }
                _ => {}
            }
        }
        self.touches.finish(s);
    }

    fn tick(&mut self, _t: f64) {}

    fn reset(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finger 1 at the origin; finger 2 at `angle` radians, 100 points away.
    fn turn_to(r: &mut Rotation, angle: f32, t: f64) {
        r.feed(&TouchSample::moved(2, 100.0 * angle.cos(), 100.0 * angle.sin(), t));
    }

    fn down(r: &mut Rotation) {
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 100.0, 0.0, 0.0));
    }

    #[test]
    fn wrap_keeps_differences_short() {
        assert!((wrap(0.1) - 0.1).abs() < 1e-6);
        assert!((wrap(TAU + 0.1) - 0.1).abs() < 1e-5);
        assert!((wrap(PI + 0.1) - (0.1 - PI)).abs() < 1e-5);
        assert!((wrap(-PI - 0.1) - (PI - 0.1)).abs() < 1e-5);
    }

    #[test]
    fn a_small_wobble_does_not_begin() {
        let mut r = Rotation::new();
        down(&mut r);
        turn_to(&mut r, 0.02, 0.01);
        assert_eq!(r.state(), GestureState::Possible);
    }

    #[test]
    fn clockwise_turn_is_positive_and_begins_past_the_slop() {
        let mut r = Rotation::new();
        down(&mut r);
        turn_to(&mut r, 0.1, 0.01);
        assert_eq!(r.state(), GestureState::Began);
        turn_to(&mut r, PI / 2.0, 0.02);
        assert_eq!(r.state(), GestureState::Changed);
        assert!((r.rotation() - PI / 2.0).abs() < 1e-4);
        assert!((r.rotation_degrees() - 90.0).abs() < 0.01);
        let (fx, fy) = r.focal();
        assert!(fx.abs() < 1e-3 && (fy - 50.0).abs() < 1e-3, "the midpoint follows the moving finger");
    }

    #[test]
    fn counter_clockwise_turn_is_negative() {
        let mut r = Rotation::new();
        down(&mut r);
        turn_to(&mut r, -0.5, 0.02);
        assert_eq!(r.state(), GestureState::Began);
        assert!((r.rotation() + 0.5).abs() < 1e-5);
    }

    #[test]
    fn turns_accumulate_past_a_half_turn() {
        let mut r = Rotation::new();
        down(&mut r);
        let mut angle = 0.0f32;
        for i in 1..=14 {
            angle += 0.3;
            turn_to(&mut r, angle, 0.02 * i as f64);
        }
        assert!((r.rotation() - 4.2).abs() < 1e-3, "got {}", r.rotation());
    }

    #[test]
    fn velocity_is_the_smoothed_angular_rate() {
        let mut r = Rotation::new();
        down(&mut r);
        // 0.04 rad every 20 ms is 2 rad/s
        for i in 1..=25 {
            turn_to(&mut r, 0.04 * i as f32, 0.02 * i as f64);
        }
        let end_angle = 0.04 * 25 as f32;
        assert!((r.velocity() - 2.0).abs() < 0.1, "got {}", r.velocity());
        r.feed(&TouchSample::ended(2, 100.0 * end_angle.cos(), 100.0 * end_angle.sin(), 0.51));
        assert_eq!(r.state(), GestureState::Ended);
        assert!((r.velocity() - 2.0).abs() < 0.1, "the release velocity stays");
        assert!((r.rotation() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn opposite_turn_has_negative_velocity() {
        let mut r = Rotation::new();
        down(&mut r);
        for i in 1..=10 {
            turn_to(&mut r, -0.04 * i as f32, 0.02 * i as f64);
        }
        assert!(r.velocity() < -1.0);
    }

    #[test]
    fn fingers_too_close_give_no_angle() {
        let mut r = Rotation::new();
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 4.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(2, 0.0, 4.0, 0.01));
        assert_eq!(r.state(), GestureState::Possible);
        assert_eq!(r.rotation(), 0.0);
    }

    #[test]
    fn lifting_early_fails_and_lifting_later_ends() {
        let mut early = Rotation::new();
        down(&mut early);
        early.feed(&TouchSample::ended(1, 0.0, 0.0, 0.02));
        assert_eq!(early.state(), GestureState::Failed);

        let mut r = Rotation::new();
        down(&mut r);
        turn_to(&mut r, 0.4, 0.02);
        r.feed(&TouchSample::ended(1, 0.0, 0.0, 0.03));
        assert_eq!(r.state(), GestureState::Ended);
    }

    #[test]
    fn cancel_and_restart() {
        let mut r = Rotation::new();
        down(&mut r);
        turn_to(&mut r, 0.4, 0.02);
        r.feed(&TouchSample::cancelled(2, 100.0, 40.0, 0.03));
        assert_eq!(r.state(), GestureState::Cancelled);
        r.feed(&TouchSample::ended(1, 0.0, 0.0, 0.04));
        r.feed(&TouchSample::began(7, 0.0, 0.0, 1.0));
        r.feed(&TouchSample::began(8, 100.0, 0.0, 1.0));
        assert_eq!(r.state(), GestureState::Possible);
        assert_eq!(r.rotation(), 0.0, "a new pair starts from zero");
    }

    #[test]
    fn a_third_finger_is_ignored() {
        let mut r = Rotation::new();
        down(&mut r);
        r.feed(&TouchSample::began(3, 300.0, 300.0, 0.01));
        r.feed(&TouchSample::moved(3, 0.0, 0.0, 0.02));
        assert_eq!(r.state(), GestureState::Possible);
        assert_eq!(r.rotation(), 0.0);
    }
}
