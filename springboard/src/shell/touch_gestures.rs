//! Touch gestures of the shell, run on the `bifrost_ui` recognizers.
//!
//! A touchscreen reports each finger as an `input::Event::Contact` (the Wayland path, `bare/wl_touch.rs`). This module
//! turns a contact into a [`TouchSample`] and feeds a [`GestureArbiter`] that the shell owns. The first system gesture
//! is the swipe up from the bottom edge: a [`ScreenEdgePan`] that starts inside the home strip. Its states are the
//! only output, and the shell applies them to the go-home swipe it already draws (`switcher.rs`).
//!
//! Positions are logical frame pixels (the shell applies its orientation first) and the thresholds of the recognizers
//! are in points, so every sample is divided by the output scale. Times come in milliseconds, the shell's clock, and
//! go out in seconds, the recognizers' clock. The module is pure: the caller passes the frame, the scale and the time.

use crate::bare::input::TouchPhase;
use bifrost_ui::recognizer::{EdgeMask, GestureArbiter, GestureEvent, GestureState, ScreenEdgePan, TouchSample};

/// The height of the home strip in points: the strip that starts a go-home swipe (`render::ui::home_zone`, 28 pt).
pub const HOME_STRIP_PT: f32 = 28.0;

/// The output scale: pixels per point, guarded so a bad value falls back to one pixel per point.
fn scale_or_one(points_px: f32) -> f32 {
    if points_px.is_finite() && points_px > 0.0 {
        points_px
    } else {
        1.0
    }
}

/// The recognizer sample of one contact. `logical` is the position in logical frame pixels, `t_ms` the shell's time
/// in milliseconds and `points_px` the output scale.
pub fn sample(phase: TouchPhase, id: i32, logical: (f32, f32), t_ms: f64, points_px: f32) -> TouchSample {
    let k = scale_or_one(points_px);
    let (x, y) = (logical.0 / k, logical.1 / k);
    let (id, t) = (id as u64, t_ms / 1000.0);
    match phase {
        TouchPhase::Down => TouchSample::began(id, x, y, t),
        TouchPhase::Move => TouchSample::moved(id, x, y, t),
        TouchPhase::Up => TouchSample::ended(id, x, y, t),
        TouchPhase::Cancel => TouchSample::cancelled(id, x, y, t),
    }
}

/// The frame a touch is judged in: its size in logical pixels and the output scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub size_px: (f32, f32),
    pub points_px: f32,
}

impl Frame {
    /// The frame size in points, the coordinate space the recognizers measure in.
    pub fn size_pt(self) -> (f32, f32) {
        let k = scale_or_one(self.points_px);
        (self.size_px.0 / k, self.size_px.1 / k)
    }
}

/// The tablet's system gestures, run together by one arbiter.
pub struct SystemGestures {
    arbiter: GestureArbiter,
    home: usize,
}

impl SystemGestures {
    /// `home_margin_pt` is how far above the bottom edge a swipe may start (the shell's home strip, in points).
    pub fn new(home_margin_pt: f32) -> Self {
        let mut arbiter = GestureArbiter::new();
        let home = arbiter.add(ScreenEdgePan::new(EdgeMask::BOTTOM, (1.0, 1.0)).with_margin(home_margin_pt));
        SystemGestures { arbiter, home }
    }

    /// The frame changed size (a rotation, a resized surface): the edges the recognizers test move with it. A swipe
    /// that starts on the new bottom edge is judged against it.
    pub fn set_frame(&mut self, frame: Frame) {
        if let Some(pan) = self.arbiter.get_mut::<ScreenEdgePan>(self.home) {
            pan.set_screen(frame.size_pt());
        }
    }

    /// Consumes one contact and returns the states the home swipe went through, in order.
    pub fn feed(&mut self, phase: TouchPhase, id: i32, logical: (f32, f32), t_ms: f64, frame: Frame) -> Vec<GestureState> {
        self.set_frame(frame);
        let s = sample(phase, id, logical, t_ms, frame.points_px);
        let events = self.arbiter.feed(&s);
        self.home_states(events)
    }

    /// Lets time pass (long presses and the like); returns the home swipe's new states.
    pub fn tick(&mut self, t_ms: f64) -> Vec<GestureState> {
        let events = self.arbiter.tick(t_ms / 1000.0);
        self.home_states(events)
    }

    /// The time, in milliseconds, at which a tick can change a recognizer.
    pub fn next_deadline_ms(&self) -> Option<f64> {
        self.arbiter.next_deadline().map(|s| s * 1000.0)
    }

    /// The touch was taken away (focus lost, the system cancelled it): everything in progress is cancelled.
    pub fn cancel_all(&mut self) -> Vec<GestureState> {
        let events = self.arbiter.cancel_all();
        self.home_states(events)
    }

    /// The state of the home swipe as the observer sees it.
    pub fn home_state(&self) -> GestureState {
        self.arbiter.state(self.home)
    }

    fn home_states(&self, events: Vec<GestureEvent>) -> Vec<GestureState> {
        events.into_iter().filter(|e| e.id == self.home).map(|e| e.state).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::recognizer::TouchPhase as Rec;

    /// A 1200 x 1920 panel at two pixels per point.
    const FRAME: Frame = Frame { size_px: (1200.0, 1920.0), points_px: 2.0 };
    const HOME_MARGIN: f32 = 28.0;

    #[test]
    fn phases_map_to_recognizer_phases_with_ids_and_seconds() {
        let s = sample(TouchPhase::Down, 7, (20.0, 40.0), 1500.0, 1.0);
        assert_eq!(s.id, 7);
        assert_eq!(s.phase, Rec::Began);
        assert_eq!((s.x, s.y), (20.0, 40.0));
        assert!((s.t - 1.5).abs() < 1e-12);
        assert_eq!(sample(TouchPhase::Move, 1, (0.0, 0.0), 0.0, 1.0).phase, Rec::Moved);
        assert_eq!(sample(TouchPhase::Up, 1, (0.0, 0.0), 0.0, 1.0).phase, Rec::Ended);
        assert_eq!(sample(TouchPhase::Cancel, 1, (0.0, 0.0), 0.0, 1.0).phase, Rec::Cancelled);
    }

    #[test]
    fn negative_ids_keep_their_bits_and_stay_distinct() {
        let a = sample(TouchPhase::Down, -1, (0.0, 0.0), 0.0, 1.0);
        let b = sample(TouchPhase::Down, 1, (0.0, 0.0), 0.0, 1.0);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn positions_are_divided_by_the_output_scale() {
        let s = sample(TouchPhase::Move, 1, (300.0, 600.0), 0.0, 2.0);
        assert_eq!((s.x, s.y), (150.0, 300.0));
    }

    #[test]
    fn a_bad_scale_falls_back_to_one() {
        for bad in [0.0, -3.0, f32::NAN, f32::INFINITY] {
            let s = sample(TouchPhase::Down, 1, (30.0, 60.0), 0.0, bad);
            assert_eq!((s.x, s.y), (30.0, 60.0), "scale {bad}");
        }
    }

    #[test]
    fn frame_size_is_in_points() {
        assert_eq!(FRAME.size_pt(), (600.0, 960.0));
    }

    /// A swipe from 5 pt above the bottom edge (960 pt high) up to 850 pt, lifted at the end.
    fn swipe_up_from_bottom(g: &mut SystemGestures, id: i32) -> Vec<GestureState> {
        let mut out = Vec::new();
        out.extend(g.feed(TouchPhase::Down, id, (300.0, 1910.0), 0.0, FRAME));
        out.extend(g.feed(TouchPhase::Move, id, (300.0, 1880.0), 16.0, FRAME));
        out.extend(g.feed(TouchPhase::Move, id, (300.0, 1800.0), 32.0, FRAME));
        out.extend(g.feed(TouchPhase::Move, id, (300.0, 1700.0), 48.0, FRAME));
        out.extend(g.feed(TouchPhase::Up, id, (300.0, 1700.0), 64.0, FRAME));
        out
    }

    #[test]
    fn a_swipe_up_from_the_bottom_edge_runs_the_home_gesture_to_its_end() {
        let mut g = SystemGestures::new(HOME_MARGIN);
        let states = swipe_up_from_bottom(&mut g, 1);
        assert!(states.contains(&GestureState::Began), "{states:?}");
        assert_eq!(states.last(), Some(&GestureState::Ended), "{states:?}");
        assert_eq!(g.home_state(), GestureState::Ended);
    }

    #[test]
    fn a_touch_that_starts_mid_screen_never_begins() {
        let mut g = SystemGestures::new(HOME_MARGIN);
        let mut states = g.feed(TouchPhase::Down, 2, (300.0, 1800.0), 0.0, FRAME);
        states.extend(g.feed(TouchPhase::Move, 2, (300.0, 1600.0), 20.0, FRAME));
        states.extend(g.feed(TouchPhase::Up, 2, (300.0, 1600.0), 40.0, FRAME));
        assert!(!states.contains(&GestureState::Began), "{states:?}");
        assert_ne!(g.home_state(), GestureState::Began);
    }

    #[test]
    fn a_touch_just_outside_the_strip_fails_at_once() {
        // 40 pt above the bottom edge is outside a 28 pt strip
        let mut g = SystemGestures::new(HOME_MARGIN);
        let states = g.feed(TouchPhase::Down, 6, (300.0, 1840.0), 0.0, FRAME);
        assert_eq!(states, vec![GestureState::Failed]);
    }

    #[test]
    fn a_cancel_during_the_swipe_cancels_the_home_gesture() {
        let mut g = SystemGestures::new(HOME_MARGIN);
        g.feed(TouchPhase::Down, 3, (300.0, 1910.0), 0.0, FRAME);
        g.feed(TouchPhase::Move, 3, (300.0, 1880.0), 16.0, FRAME);
        g.feed(TouchPhase::Move, 3, (300.0, 1800.0), 32.0, FRAME);
        let states = g.feed(TouchPhase::Cancel, 3, (300.0, 1800.0), 40.0, FRAME);
        assert_eq!(states, vec![GestureState::Cancelled]);
    }

    #[test]
    fn cancel_all_ends_a_swipe_in_progress_and_starts_over() {
        let mut g = SystemGestures::new(HOME_MARGIN);
        g.feed(TouchPhase::Down, 4, (300.0, 1910.0), 0.0, FRAME);
        g.feed(TouchPhase::Move, 4, (300.0, 1880.0), 16.0, FRAME);
        g.feed(TouchPhase::Move, 4, (300.0, 1800.0), 32.0, FRAME);
        assert_eq!(g.cancel_all(), vec![GestureState::Cancelled]);
        assert_eq!(g.home_state(), GestureState::Possible);
        let states = swipe_up_from_bottom(&mut g, 5);
        assert_eq!(states.last(), Some(&GestureState::Ended), "a new touch after the cancel works: {states:?}");
    }

    #[test]
    fn a_rotated_frame_moves_the_bottom_edge_the_home_swipe_starts_on() {
        // 960 x 600 pt after a turn: the portrait bottom edge (y 960 pt) is now mid-screen, the landscape one is 600 pt
        const LANDSCAPE: Frame = Frame { size_px: (1920.0, 1200.0), points_px: 2.0 };
        let mut g = SystemGestures::new(HOME_MARGIN);
        g.set_frame(LANDSCAPE);
        let mut states = g.feed(TouchPhase::Down, 8, (300.0, 1180.0), 0.0, LANDSCAPE);
        states.extend(g.feed(TouchPhase::Move, 8, (300.0, 1100.0), 16.0, LANDSCAPE));
        states.extend(g.feed(TouchPhase::Move, 8, (300.0, 1000.0), 32.0, LANDSCAPE));
        states.extend(g.feed(TouchPhase::Up, 8, (300.0, 1000.0), 48.0, LANDSCAPE));
        assert!(states.contains(&GestureState::Began), "{states:?}");
        assert_eq!(states.last(), Some(&GestureState::Ended), "{states:?}");
    }

    #[test]
    fn idle_has_no_deadline_and_ticks_are_quiet() {
        let mut g = SystemGestures::new(HOME_MARGIN);
        assert_eq!(g.next_deadline_ms(), None);
        assert!(g.tick(1000.0).is_empty());
        assert!(g.cancel_all().is_empty());
    }
}
