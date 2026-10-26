//! The capture session behind the window: the Capture button, the timer countdown and the status line. It has no
//! dependencies, so the state changes are tested alone; the window turns a [`Step::Take`] into a request to the shell's
//! screenshot service and feeds the reply and the finished picture back.

use crate::options::{Options, Target, Timer};

/// The choices of the Capture control, in the order the window shows them.
pub const TARGETS: [Target; 3] = [Target::Screen, Target::Window, Target::Selection];
/// The choices of the Timer control, in the order the window shows them.
pub const TIMERS: [Timer; 3] = [Timer::Off, Timer::Seconds5, Timer::Seconds10];

/// The status line before anything was captured.
pub const READY: &str = "Pictures are saved to Photos.";

/// What the window does after a state change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing changed.
    Idle,
    /// The window needs a new frame.
    Redraw,
    /// Ask the shell for the picture now.
    Take,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    /// The timer runs. `Armed` waits for the first tick to learn the clock, then `Until` holds the deadline.
    Armed(u32),
    Until(f64),
    /// The request is with the shell and its reply has not arrived.
    Waiting,
}

/// Options plus the state of the current capture.
#[derive(Clone, Debug)]
pub struct Session {
    pub options: Options,
    phase: Phase,
    /// The whole seconds the window last showed, so a tick redraws only when the number changes.
    shown: u32,
    status: String,
}

impl Default for Session {
    fn default() -> Self {
        Session { options: Options::default(), phase: Phase::Idle, shown: 0, status: READY.into() }
    }
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// The line under the button.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// True while the timer counts down, so the button offers Cancel.
    pub fn counting(&self) -> bool {
        matches!(self.phase, Phase::Armed(_) | Phase::Until(_))
    }

    /// True while the shell has not answered the request; the button is dimmed then.
    pub fn busy(&self) -> bool {
        self.phase == Phase::Waiting
    }

    /// The seconds left on the timer, rounded up, while it counts down.
    pub fn remaining(&self, now: f64) -> Option<u32> {
        match self.phase {
            Phase::Armed(secs) => Some(secs),
            Phase::Until(deadline) => Some((deadline - now).max(0.0).ceil() as u32),
            _ => None,
        }
    }

    /// The Capture button (or Return): starts the timer or the capture, or cancels a running countdown.
    pub fn press(&mut self) -> Step {
        match self.phase {
            Phase::Idle => {
                let secs = self.options.delay_seconds();
                if secs == 0 {
                    self.phase = Phase::Waiting;
                    Step::Take
                } else {
                    self.phase = Phase::Armed(secs);
                    self.shown = secs;
                    self.status = format!("Capturing in {secs} seconds.");
                    Step::Redraw
                }
            }
            Phase::Armed(_) | Phase::Until(_) => {
                self.phase = Phase::Idle;
                self.status = "Timer cancelled.".into();
                Step::Redraw
            }
            Phase::Waiting => Step::Idle,
        }
    }

    /// The clock `now` (seconds). Fires the capture when the timer ends.
    pub fn tick(&mut self, now: f64) -> Step {
        match self.phase {
            Phase::Armed(secs) => {
                self.phase = Phase::Until(now + secs as f64);
                Step::Idle
            }
            Phase::Until(deadline) => {
                if now >= deadline {
                    self.phase = Phase::Waiting;
                    self.status = READY.into();
                    return Step::Take;
                }
                let left = self.remaining(now).unwrap_or(0);
                if left == self.shown {
                    return Step::Idle;
                }
                self.shown = left;
                self.status = format!("Capturing in {left} seconds.");
                Step::Redraw
            }
            _ => Step::Idle,
        }
    }

    /// The shell's answer to the request. A refusal (a capture already on screen, screen recording not allowed) is
    /// shown and the button is free again. Cancelling a selection or window sends no event, so the session does not
    /// wait for the picture.
    pub fn started(&mut self, result: Result<(), String>) {
        self.phase = Phase::Idle;
        self.status = match result {
            Ok(()) => "Choose what to capture. Press Escape to cancel.".into(),
            Err(e) => format!("Could not take a screenshot: {e}"),
        };
    }

    /// The `captured` event: the file the shell saved.
    pub fn captured(&mut self, file_name: &str) {
        self.status = format!("Saved {file_name} to Photos.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_without_a_timer_takes_at_once() {
        let mut s = Session::new();
        assert_eq!(s.press(), Step::Take);
        assert!(s.busy());
        assert_eq!(s.press(), Step::Idle, "a second press while waiting does nothing");
        s.started(Ok(()));
        assert!(!s.busy());
        assert!(s.status().starts_with("Choose what to capture"));
    }

    #[test]
    fn the_timer_counts_down_and_takes_at_the_end() {
        let mut s = Session::new();
        s.options.set_timer(Timer::Seconds5);
        assert_eq!(s.press(), Step::Redraw);
        assert!(s.counting());
        assert_eq!(s.remaining(0.0), Some(5));
        assert_eq!(s.tick(100.0), Step::Idle, "the first tick only reads the clock");
        assert_eq!(s.remaining(100.0), Some(5));
        assert_eq!(s.tick(100.4), Step::Idle, "still five seconds on screen");
        assert_eq!(s.tick(101.2), Step::Redraw);
        assert_eq!(s.remaining(101.2), Some(4));
        assert_eq!(s.status(), "Capturing in 4 seconds.");
        assert_eq!(s.tick(104.9), Step::Redraw);
        assert_eq!(s.tick(105.0), Step::Take);
        assert!(!s.counting());
        assert!(s.busy());
    }

    #[test]
    fn pressing_during_the_countdown_cancels_it() {
        let mut s = Session::new();
        s.options.set_timer(Timer::Seconds10);
        s.press();
        s.tick(1.0);
        assert_eq!(s.press(), Step::Redraw);
        assert!(!s.counting());
        assert_eq!(s.status(), "Timer cancelled.");
        assert_eq!(s.tick(20.0), Step::Idle, "a cancelled timer never fires");
    }

    #[test]
    fn a_refused_request_frees_the_button_and_says_why() {
        let mut s = Session::new();
        s.press();
        s.started(Err("a capture is already on screen".into()));
        assert!(!s.busy());
        assert_eq!(s.status(), "Could not take a screenshot: a capture is already on screen");
        assert_eq!(s.press(), Step::Take);
    }

    #[test]
    fn a_finished_picture_is_reported_by_name() {
        let mut s = Session::new();
        s.captured("Screenshot 2026-10-26 at 12.00.00.png");
        assert_eq!(s.status(), "Saved Screenshot 2026-10-26 at 12.00.00.png to Photos.");
    }

    #[test]
    fn the_choices_cover_every_target_and_timer() {
        assert_eq!(TARGETS.len(), 3);
        assert_eq!(TIMERS.map(|t| t.seconds()), [0, 5, 10]);
    }
}
