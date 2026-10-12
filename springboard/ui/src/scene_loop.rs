//! The lifecycle and frame cycle of a game scene as a pure model. A scene is presented by a view and later dismissed
//! from it, it can change size while presented, and while it runs each frame goes through the same ordered steps: the
//! app's update, scripted actions, physics, constraints, and a final step once everything is settled.
//!
//! The caller passes the time in seconds and runs the steps that `frame` returns. The model reads no clock and runs
//! nothing itself. The time a frame advances the scene by is clamped to `max_step`, so a long stall (a window drag, a
//! breakpoint) does not make the scene jump. While paused, frames run no steps and the scene's clock does not advance,
//! and the first frame after a pause only counts the time since the last paused frame, again clamped.

/// The steps of one frame, in the order a frame runs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The app's own per-frame logic.
    Update,
    /// Scripted actions run.
    EvaluateActions,
    /// The physics world advances.
    SimulatePhysics,
    /// Constraints are applied to the scene's nodes.
    ApplyConstraints,
    /// The frame is settled; the app may read the final state.
    FinishUpdate,
}

/// Every step of an unpaused frame, in order.
pub const FRAME_STEPS: [Step; 5] = [
    Step::Update,
    Step::EvaluateActions,
    Step::SimulatePhysics,
    Step::ApplyConstraints,
    Step::FinishUpdate,
];

/// The longest a frame may advance the scene, in seconds, unless the caller changes it.
pub const DEFAULT_MAX_STEP: f64 = 0.1;

/// What one frame asks the caller to do.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Seconds the scene advanced in this frame, after clamping. Zero for the first frame and while paused.
    pub delta: f64,
    /// Seconds the scene has run since it was presented, not counting paused time.
    pub current_time: f64,
    /// The steps to run, in order. Empty while paused.
    pub steps: &'static [Step],
}

/// The lifecycle and clock of one scene.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneLoop {
    width: f32,
    height: f32,
    presented: bool,
    paused: bool,
    max_step: f64,
    last_time: Option<f64>,
    current_time: f64,
}

impl SceneLoop {
    /// A scene of the given size that is not presented yet. Negative sizes are taken as zero.
    pub fn new(width: f32, height: f32) -> Self {
        SceneLoop {
            width: width.max(0.0),
            height: height.max(0.0),
            presented: false,
            paused: false,
            max_step: DEFAULT_MAX_STEP,
            last_time: None,
            current_time: 0.0,
        }
    }

    /// The scene's size as (width, height).
    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    pub fn is_presented(&self) -> bool {
        self.presented
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Seconds the scene has run since it was presented, not counting paused time.
    pub fn current_time(&self) -> f64 {
        self.current_time
    }

    pub fn max_step(&self) -> f64 {
        self.max_step
    }

    /// Sets the clamp on how far one frame may advance the scene. Returns false and changes nothing unless the value is
    /// finite and above zero.
    pub fn set_max_step(&mut self, seconds: f64) -> bool {
        if seconds.is_finite() && seconds > 0.0 {
            self.max_step = seconds;
            true
        } else {
            false
        }
    }

    /// The scene is presented by a view. The clock starts again from zero. Returns false if it already was.
    pub fn present(&mut self) -> bool {
        if self.presented {
            return false;
        }
        self.presented = true;
        self.last_time = None;
        self.current_time = 0.0;
        true
    }

    /// The scene is removed from its view. Returns false if it was not presented.
    pub fn dismiss(&mut self) -> bool {
        if !self.presented {
            return false;
        }
        self.presented = false;
        self.last_time = None;
        true
    }

    /// The view gave the scene a new size. Returns true when the size actually changed.
    pub fn resize(&mut self, width: f32, height: f32) -> bool {
        let (w, h) = (width.max(0.0), height.max(0.0));
        if w == self.width && h == self.height {
            return false;
        }
        self.width = w;
        self.height = h;
        true
    }

    /// Pauses or resumes the scene. Frames while paused run no steps.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// The frame at time `now` (seconds). None while the scene is not presented, so the caller runs nothing.
    /// A `now` earlier than the previous frame counts as no time passed.
    pub fn frame(&mut self, now: f64) -> Option<Frame> {
        if !self.presented {
            return None;
        }
        let delta = match self.last_time {
            Some(prev) if now > prev => (now - prev).min(self.max_step),
            _ => 0.0,
        };
        self.last_time = Some(now);
        if self.paused {
            return Some(Frame { delta: 0.0, current_time: self.current_time, steps: &[] });
        }
        self.current_time += delta;
        Some(Frame { delta, current_time: self.current_time, steps: &FRAME_STEPS })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn present_and_dismiss_report_each_change_once() {
        let mut s = SceneLoop::new(800.0, 600.0);
        assert!(!s.is_presented());
        assert!(s.present());
        assert!(!s.present(), "a second present changes nothing");
        assert!(s.is_presented());
        assert!(s.dismiss());
        assert!(!s.dismiss(), "a second dismiss changes nothing");
        assert!(!s.is_presented());
    }

    #[test]
    fn no_frame_runs_before_the_scene_is_presented_or_after_it_is_dismissed() {
        let mut s = SceneLoop::new(10.0, 10.0);
        assert_eq!(s.frame(0.0), None);
        s.present();
        assert!(s.frame(1.0).is_some());
        s.dismiss();
        assert_eq!(s.frame(2.0), None);
    }

    #[test]
    fn first_frame_has_no_delta_and_runs_the_full_cycle_in_order() {
        let mut s = SceneLoop::new(10.0, 10.0);
        s.present();
        let f = s.frame(1.0).unwrap();
        assert_eq!(f.delta, 0.0);
        assert_eq!(f.current_time, 0.0);
        assert_eq!(
            f.steps,
            &[Step::Update, Step::EvaluateActions, Step::SimulatePhysics, Step::ApplyConstraints, Step::FinishUpdate]
        );
    }

    #[test]
    fn a_frame_advances_by_the_elapsed_time_clamped_to_the_max_step() {
        let mut s = SceneLoop::new(10.0, 10.0);
        s.present();
        s.frame(1.0);
        // 0.25 s elapsed, clamped to the default 0.1 s.
        let f = s.frame(1.25).unwrap();
        assert!(near(f.delta, 0.1));
        assert!(near(f.current_time, 0.1));
        // 0.05 s elapsed, below the clamp, so it is used as it is.
        let f = s.frame(1.30).unwrap();
        assert!(near(f.delta, 0.05));
        assert!(near(f.current_time, 0.15));
    }

    #[test]
    fn paused_time_does_not_advance_the_scene_clock() {
        let mut s = SceneLoop::new(10.0, 10.0);
        s.present();
        s.frame(1.0);
        s.frame(1.05);
        s.set_paused(true);
        let paused = s.frame(2.0).unwrap();
        assert!(paused.steps.is_empty());
        assert_eq!(paused.delta, 0.0);
        assert!(near(paused.current_time, 0.05));
        s.set_paused(false);
        // Only the 0.05 s since the paused frame counts, not the second spent paused.
        let resumed = s.frame(2.05).unwrap();
        assert!(near(resumed.delta, 0.05));
        assert!(near(resumed.current_time, 0.10));
        assert_eq!(resumed.steps.len(), 5);
    }

    #[test]
    fn a_clock_that_goes_backwards_counts_as_no_time_and_still_runs_the_steps() {
        let mut s = SceneLoop::new(10.0, 10.0);
        s.present();
        s.frame(5.0);
        let f = s.frame(4.0).unwrap();
        assert_eq!(f.delta, 0.0);
        assert_eq!(f.steps.len(), 5);
    }

    #[test]
    fn resize_reports_only_real_changes_and_takes_negative_sizes_as_zero() {
        let mut s = SceneLoop::new(100.0, 50.0);
        assert!(!s.resize(100.0, 50.0));
        assert!(s.resize(200.0, 50.0));
        assert_eq!(s.size(), (200.0, 50.0));
        assert!(s.resize(-5.0, 50.0));
        assert_eq!(s.size(), (0.0, 50.0));
    }

    #[test]
    fn a_new_presentation_starts_the_clock_again_and_max_step_must_be_positive() {
        let mut s = SceneLoop::new(10.0, 10.0);
        s.present();
        s.frame(1.0);
        s.frame(1.05);
        s.dismiss();
        s.present();
        let f = s.frame(10.0).unwrap();
        assert_eq!(f.delta, 0.0);
        assert_eq!(f.current_time, 0.0);
        assert!(!s.set_max_step(0.0));
        assert!(!s.set_max_step(f64::NAN));
        assert!(s.set_max_step(0.5));
        assert!(near(s.max_step(), 0.5));
    }
}
