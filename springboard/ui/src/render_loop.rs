//! A render loop as a pure model: the timed or on-demand drawing decision for a view, the frame rate it asks for, pausing
//! by the app or by the program, and pausing and resuming with the app's activity. The caller passes the time in seconds
//! and draws when `tick` says a frame is due. The model reads no clock and draws nothing.
//!
//! Frames never catch up: when the caller arrives late, one frame is due now and the next one comes one interval after
//! it, not several frames at once.

/// The frame rate limits: a loop asks for between one and 120 frames per second.
pub const MIN_FPS: u32 = 1;
pub const MAX_FPS: u32 = 120;

#[derive(Clone, Debug, PartialEq)]
pub struct RenderLoop {
    preferred_fps: u32,
    on_demand: bool,
    requested: bool,
    paused_by_program: bool,
    paused_by_app: bool,
    pause_on_resign: bool,
    resume_on_active: bool,
    next_due: Option<f64>,
    frames: u64,
}

impl RenderLoop {
    /// A timed loop, unpaused, at `preferred_fps` clamped to `MIN_FPS..=MAX_FPS`.
    pub fn new(preferred_fps: u32) -> Self {
        RenderLoop {
            preferred_fps: preferred_fps.clamp(MIN_FPS, MAX_FPS),
            on_demand: false,
            requested: false,
            paused_by_program: false,
            paused_by_app: false,
            pause_on_resign: false,
            resume_on_active: false,
            next_due: None,
            frames: 0,
        }
    }

    /// The frame rate the loop asks for, in frames per second.
    pub fn preferred_fps(&self) -> u32 {
        self.preferred_fps
    }

    /// Changes the frame rate, clamped like `new`.
    pub fn set_preferred_fps(&mut self, fps: u32) {
        self.preferred_fps = fps.clamp(MIN_FPS, MAX_FPS);
    }

    /// The time between two frames, in seconds.
    pub fn frame_interval(&self) -> f64 {
        1.0 / self.preferred_fps as f64
    }

    /// On-demand loops draw only after `request_redraw`; timed loops draw at the frame rate.
    pub fn set_on_demand(&mut self, on_demand: bool) {
        self.on_demand = on_demand;
    }

    /// Asks an on-demand loop for one frame. Several requests before the next tick make one frame.
    pub fn request_redraw(&mut self) {
        self.requested = true;
    }

    /// The program pauses or resumes the loop.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused_by_program = paused;
    }

    /// Whether the loop is paused by the program or by the app's activity.
    pub fn is_paused(&self) -> bool {
        self.paused_by_program || self.paused_by_app
    }

    /// Whether the loop pauses itself when the app resigns active.
    pub fn set_pause_on_resign(&mut self, on: bool) {
        self.pause_on_resign = on;
    }

    /// Whether the loop resumes itself when the app becomes active.
    pub fn set_resume_on_active(&mut self, on: bool) {
        self.resume_on_active = on;
    }

    /// The app resigns active. A loop set to pause on resign pauses by the app, which leaves a program pause alone.
    pub fn app_resigned_active(&mut self) {
        if self.pause_on_resign {
            self.paused_by_app = true;
        }
    }

    /// The app becomes active. A loop set to resume on activity clears the app's pause, and a program pause stays.
    pub fn app_became_active(&mut self) {
        if self.resume_on_active {
            self.paused_by_app = false;
        }
    }

    /// Reports the time `now` in seconds and returns `true` when the caller should draw a frame.
    pub fn tick(&mut self, now: f64) -> bool {
        if self.is_paused() {
            self.next_due = None;
            return false;
        }
        if self.on_demand {
            if !self.requested {
                return false;
            }
            self.requested = false;
            self.frames += 1;
            return true;
        }
        match self.next_due {
            Some(due) if now < due => false,
            _ => {
                self.frames += 1;
                self.next_due = Some(now + self.frame_interval());
                true
            }
        }
    }

    /// Frames the loop has asked the caller to draw since it was made.
    pub fn frames_displayed(&self) -> u64 {
        self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rate_is_clamped_and_sets_the_interval() {
        assert_eq!(RenderLoop::new(0).preferred_fps(), 1);
        assert_eq!(RenderLoop::new(500).preferred_fps(), 120);
        let mut l = RenderLoop::new(30);
        assert_eq!(l.preferred_fps(), 30);
        assert_eq!(l.frame_interval(), 1.0 / 30.0);
        l.set_preferred_fps(60);
        assert_eq!(l.frame_interval(), 1.0 / 60.0);
    }

    #[test]
    fn timed_loop_draws_at_the_frame_rate() {
        let mut l = RenderLoop::new(60);
        assert!(l.tick(0.0), "the first tick draws at once");
        assert!(!l.tick(0.01), "too early for the next frame");
        assert!(l.tick(1.0 / 60.0), "one interval later the next frame is due");
        assert_eq!(l.frames_displayed(), 2);
    }

    #[test]
    fn a_late_caller_gets_one_frame_and_no_catch_up() {
        let mut l = RenderLoop::new(10);
        assert!(l.tick(0.0));
        assert!(l.tick(0.35), "3.5 intervals late still draws one frame");
        assert!(!l.tick(0.4), "the next frame is one interval after the late one");
        assert!(l.tick(0.5));
        assert_eq!(l.frames_displayed(), 3);
    }

    #[test]
    fn a_paused_loop_draws_nothing_and_resumes_at_once() {
        let mut l = RenderLoop::new(60);
        l.set_paused(true);
        assert!(!l.tick(0.0));
        assert_eq!(l.frames_displayed(), 0);
        l.set_paused(false);
        assert!(l.tick(5.0), "a resumed loop draws at once, not after a stale interval");
    }

    #[test]
    fn app_activity_pauses_and_resumes_only_when_set_to() {
        let mut l = RenderLoop::new(60);
        l.set_pause_on_resign(true);
        l.set_resume_on_active(true);
        l.app_resigned_active();
        assert!(l.is_paused());
        assert!(!l.tick(0.0));
        l.app_became_active();
        assert!(!l.is_paused());

        // A program pause survives the app becoming active.
        l.set_paused(true);
        l.app_resigned_active();
        l.app_became_active();
        assert!(l.is_paused(), "the program's pause stays");

        // A loop that does not pause on resign keeps drawing.
        let mut plain = RenderLoop::new(60);
        plain.app_resigned_active();
        assert!(!plain.is_paused());
    }

    #[test]
    fn on_demand_loop_draws_once_per_request() {
        let mut l = RenderLoop::new(60);
        l.set_on_demand(true);
        assert!(!l.tick(0.0), "nothing is drawn without a request");
        l.request_redraw();
        l.request_redraw();
        assert!(l.tick(0.0), "two requests make one frame");
        assert!(!l.tick(0.1), "the request is used up");
        assert_eq!(l.frames_displayed(), 1);
    }
}
