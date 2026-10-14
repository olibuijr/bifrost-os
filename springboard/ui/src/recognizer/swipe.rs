//! Swipe: a quick, mostly straight movement in one of the allowed directions.

use super::*;
use std::ops::BitOr;

/// A swipe direction on screen (y grows downwards).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Right,
    Left,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 4] = [Direction::Right, Direction::Left, Direction::Up, Direction::Down];

    pub fn mask(self) -> DirectionMask {
        match self {
            Direction::Right => DirectionMask::RIGHT,
            Direction::Left => DirectionMask::LEFT,
            Direction::Up => DirectionMask::UP,
            Direction::Down => DirectionMask::DOWN,
        }
    }

    /// Splits a displacement into the distance travelled in this direction (negative when going the other way)
    /// and the absolute distance off that axis.
    pub fn measure(self, dx: f32, dy: f32) -> (f32, f32) {
        match self {
            Direction::Right => (dx, dy.abs()),
            Direction::Left => (-dx, dy.abs()),
            Direction::Up => (-dy, dx.abs()),
            Direction::Down => (dy, dx.abs()),
        }
    }
}

/// A set of swipe directions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectionMask(u8);

impl DirectionMask {
    pub const NONE: DirectionMask = DirectionMask(0);
    pub const RIGHT: DirectionMask = DirectionMask(1);
    pub const LEFT: DirectionMask = DirectionMask(2);
    pub const UP: DirectionMask = DirectionMask(4);
    pub const DOWN: DirectionMask = DirectionMask(8);
    pub const ANY: DirectionMask = DirectionMask(15);

    pub fn contains(self, d: Direction) -> bool {
        self.0 & d.mask().0 != 0
    }
}

impl BitOr for DirectionMask {
    type Output = DirectionMask;
    fn bitor(self, rhs: DirectionMask) -> DirectionMask {
        DirectionMask(self.0 | rhs.0)
    }
}

/// Discrete: `Possible -> Recognized` the moment the fingers have travelled `min_distance` in an allowed direction
/// fast enough and straight enough; `Failed` if they lift, wander off axis or take too long first.
#[derive(Debug)]
pub struct Swipe {
    core: Core,
    pub directions: DirectionMask,
    pub touches_required: usize,
    /// Travel needed along the direction (points).
    pub min_distance: f32,
    /// Average speed needed from touch-down (points per second).
    pub min_speed: f32,
    /// Give up after this long (seconds).
    pub max_duration: f64,
    /// Largest off-axis travel as a fraction of the travel along the axis.
    pub max_off_axis: f32,
    touches: TouchSet,
    start: Option<((f32, f32), f64)>,
    direction: Option<Direction>,
    location: (f32, f32),
}

impl Swipe {
    pub fn new(directions: DirectionMask, touches_required: usize) -> Self {
        Swipe {
            core: Core::new(),
            directions,
            touches_required: touches_required.max(1),
            min_distance: 50.0,
            min_speed: 120.0,
            max_duration: 1.0,
            max_off_axis: 0.6,
            touches: TouchSet::default(),
            start: None,
            direction: None,
            location: (0.0, 0.0),
        }
    }

    /// The direction that was recognized.
    pub fn direction(&self) -> Option<Direction> {
        self.direction
    }

    /// Where the fingers were when it was recognized.
    pub fn location(&self) -> (f32, f32) {
        self.location
    }

    fn clear(&mut self) {
        self.core.reset();
        self.touches.clear();
        self.start = None;
        self.direction = None;
    }

    fn fail(&mut self) {
        self.core.go(GestureState::Failed);
    }

    fn evaluate(&mut self, t: f64) {
        let Some((origin, t0)) = self.start else { return };
        let c = self.touches.centroid();
        let (dx, dy) = (c.0 - origin.0, c.1 - origin.1);
        let elapsed = (t - t0) as f32;
        let mut on_axis = false;
        for d in Direction::ALL {
            if !self.directions.contains(d) {
                continue;
            }
            let (along, cross) = d.measure(dx, dy);
            if along > 0.0 && cross <= along * self.max_off_axis {
                on_axis = true;
                if along >= self.min_distance && elapsed > 0.0 && along / elapsed >= self.min_speed {
                    self.direction = Some(d);
                    self.location = c;
                    self.core.go(GestureState::Recognized);
                    return;
                }
            }
        }
        if dx.hypot(dy) >= self.min_distance && !on_axis {
            self.fail();
        }
    }
}

impl Recognizer for Swipe {
    recognizer_plumbing!();

    fn feed(&mut self, s: &TouchSample) {
        self.tick(s.t);
        if needs_reset(&self.core, &self.touches, s) {
            self.clear();
        }
        if !self.touches.apply(s) {
            return;
        }
        if !self.core.current().is_terminal() {
            let n = self.touches.len();
            match s.phase {
                TouchPhase::Began => {
                    if n > self.touches_required {
                        self.fail();
                    } else if n == self.touches_required {
                        self.start = Some((self.touches.centroid(), s.t));
                    }
                }
                TouchPhase::Moved | TouchPhase::Ended => {
                    if n == self.touches_required {
                        self.evaluate(s.t);
                    }
                    if s.phase == TouchPhase::Ended {
                        self.fail();
                    }
                }
                TouchPhase::Cancelled => self.fail(),
            }
        }
        self.touches.finish(s);
    }

    fn tick(&mut self, t: f64) {
        if self.core.current().is_terminal() {
            return;
        }
        if self.start.is_some_and(|(_, t0)| t - t0 > self.max_duration) {
            self.fail();
        }
    }

    fn deadline(&self) -> Option<f64> {
        if self.core.current().is_terminal() {
            return None;
        }
        self.start.map(|(_, t0)| t0 + self.max_duration)
    }

    fn reset(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognizer::testutil::*;

    fn fling(r: &mut Swipe, from: (f32, f32), to: (f32, f32), secs: f64) {
        r.feed(&TouchSample::began(1, from.0, from.1, 0.0));
        run(r, &drag(1, from, to, 0.0, secs, 5));
    }

    #[test]
    fn direction_measure_follows_screen_axes() {
        assert_eq!(Direction::Right.measure(10.0, -3.0), (10.0, 3.0));
        assert_eq!(Direction::Left.measure(10.0, -3.0), (-10.0, 3.0));
        assert_eq!(Direction::Up.measure(2.0, -30.0), (30.0, 2.0));
        assert_eq!(Direction::Down.measure(2.0, -30.0), (-30.0, 2.0));
        let m = DirectionMask::LEFT | DirectionMask::UP;
        assert!(m.contains(Direction::Left) && m.contains(Direction::Up));
        assert!(!m.contains(Direction::Right) && !DirectionMask::NONE.contains(Direction::Down));
        assert!(Direction::ALL.iter().all(|d| DirectionMask::ANY.contains(*d)));
    }

    #[test]
    fn a_quick_straight_swipe_is_recognized_mid_gesture() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        fling(&mut r, (100.0, 100.0), (220.0, 108.0), 0.15);
        assert_eq!(r.state(), GestureState::Recognized);
        assert_eq!(r.direction(), Some(Direction::Right));
    }

    #[test]
    fn recognized_as_soon_as_the_distance_is_reached() {
        let mut r = Swipe::new(DirectionMask::DOWN, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::moved(1, 0.0, 30.0, 0.05));
        assert_eq!(r.state(), GestureState::Possible);
        r.feed(&TouchSample::moved(1, 2.0, 60.0, 0.1));
        assert_eq!(r.state(), GestureState::Recognized);
        assert_eq!(r.location(), (2.0, 60.0));
    }

    #[test]
    fn each_direction_is_recognized() {
        for (dir, to) in [
            (Direction::Right, (100.0, 0.0)),
            (Direction::Left, (-100.0, 0.0)),
            (Direction::Up, (0.0, -100.0)),
            (Direction::Down, (0.0, 100.0)),
        ] {
            let mut r = Swipe::new(DirectionMask::ANY, 1);
            fling(&mut r, (0.0, 0.0), to, 0.1);
            assert_eq!(r.direction(), Some(dir));
        }
    }

    #[test]
    fn a_direction_mask_only_accepts_its_directions() {
        let mut r = Swipe::new(DirectionMask::LEFT | DirectionMask::RIGHT, 1);
        fling(&mut r, (0.0, 0.0), (0.0, -100.0), 0.1);
        assert_eq!(r.state(), GestureState::Failed, "up is not allowed");

        let mut l = Swipe::new(DirectionMask::RIGHT, 1);
        fling(&mut l, (0.0, 0.0), (-100.0, 0.0), 0.1);
        assert_eq!(l.state(), GestureState::Failed, "left is not allowed");
    }

    #[test]
    fn a_diagonal_stroke_fails() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        fling(&mut r, (0.0, 0.0), (80.0, 80.0), 0.1);
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn a_slow_stroke_is_not_a_swipe() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        run(&mut r, &drag(1, (0.0, 0.0), (60.0, 0.0), 0.0, 0.9, 6));
        assert_eq!(r.state(), GestureState::Possible, "60 points in 0.9 s is 66 points per second");
        r.feed(&TouchSample::ended(1, 60.0, 0.0, 0.95));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn taking_too_long_fails_on_tick() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 2.0));
        assert_eq!(r.deadline(), Some(3.0));
        r.tick(2.9);
        assert_eq!(r.state(), GestureState::Possible);
        r.tick(3.1);
        assert_eq!(r.state(), GestureState::Failed);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn a_short_flick_does_not_count() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        fling(&mut r, (0.0, 0.0), (30.0, 0.0), 0.05);
        assert_eq!(r.state(), GestureState::Possible);
        r.feed(&TouchSample::ended(1, 30.0, 0.0, 0.06));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn a_tunable_distance_and_speed() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        r.min_distance = 20.0;
        r.min_speed = 50.0;
        fling(&mut r, (0.0, 0.0), (25.0, 0.0), 0.3);
        assert_eq!(r.state(), GestureState::Recognized);
    }

    #[test]
    fn two_finger_swipe_needs_both_fingers() {
        let mut r = Swipe::new(DirectionMask::LEFT, 2);
        r.feed(&TouchSample::began(1, 200.0, 100.0, 0.0));
        r.feed(&TouchSample::moved(1, 100.0, 100.0, 0.05));
        assert_eq!(r.state(), GestureState::Possible, "one finger does not satisfy a two finger swipe");
        r.feed(&TouchSample::began(2, 200.0, 140.0, 0.06));
        r.feed(&TouchSample::moved(1, 60.0, 100.0, 0.08));
        assert_eq!(r.state(), GestureState::Possible, "the centroid has not travelled far enough yet");
        r.feed(&TouchSample::moved(2, 60.0, 140.0, 0.1));
        assert_eq!(r.state(), GestureState::Recognized);
        assert_eq!(r.direction(), Some(Direction::Left));
    }

    #[test]
    fn extra_fingers_fail() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::began(2, 50.0, 0.0, 0.01));
        assert_eq!(r.state(), GestureState::Failed);
    }

    #[test]
    fn cancel_fails_and_the_next_touch_can_swipe() {
        let mut r = Swipe::new(DirectionMask::RIGHT, 1);
        r.feed(&TouchSample::began(1, 0.0, 0.0, 0.0));
        r.feed(&TouchSample::cancelled(1, 0.0, 0.0, 0.01));
        assert_eq!(r.state(), GestureState::Failed);
        r.feed(&TouchSample::began(2, 0.0, 0.0, 1.0));
        run(&mut r, &drag(2, (0.0, 0.0), (100.0, 0.0), 1.0, 1.1, 4));
        assert_eq!(r.state(), GestureState::Recognized);
        assert_eq!(r.direction(), Some(Direction::Right));
    }

    #[test]
    fn works_with_a_pointer() {
        let mut p = PointerAdapter::new();
        let mut r = Swipe::new(DirectionMask::UP, 1);
        r.feed(&p.press(10.0, 200.0, 0.0));
        r.feed(&p.motion(12.0, 120.0, 0.1).unwrap());
        assert_eq!(r.state(), GestureState::Recognized);
        assert_eq!(r.direction(), Some(Direction::Up));
    }
}
