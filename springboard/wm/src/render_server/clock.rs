//! Per-output frame clock (the display-link side of the server): learns the refresh interval and phase from vblank
//! (page-flip) timestamps, predicts the next presentation time, and starts composition at
//! `next_vblank - budget`, where the budget is the 95th percentile of recent compose+submit times plus 1 ms
//! (starting at 4 ms, clamped to 2..12 ms). Sends `wp_presentation` feedback and fires frame callbacks with the
//! predicted presentation time. Pure scheduling logic lives in `FrameClock` (host-tested); the wiring into the event
//! loop is in `drm.rs`/`nested.rs`.
//!
//! Owner: wave G1 agent `wm-clock`.

use std::time::Duration;

/// Scheduling state for one output.
#[derive(Debug, Clone)]
pub struct FrameClock {
    /// The refresh interval (from the mode, refined from vblank deltas).
    pub refresh: Duration,
    /// The last vblank time, as a monotonic clock reading.
    pub last_vblank: Option<Duration>,
    /// Recent compose+submit durations, newest last (at most 120).
    pub recent: Vec<Duration>,
}

impl FrameClock {
    pub fn new(refresh: Duration) -> FrameClock {
        FrameClock { refresh, last_vblank: None, recent: Vec::new() }
    }

    /// Records a vblank (page flip completed) at monotonic time `t`.
    pub fn vblank(&mut self, t: Duration) {
        let _ = t;
        todo!("wm-clock")
    }

    /// Records how long a frame took from start of composition to submission.
    pub fn frame_took(&mut self, d: Duration) {
        let _ = d;
        todo!("wm-clock")
    }

    /// The current budget (see the module doc).
    pub fn budget(&self) -> Duration {
        todo!("wm-clock")
    }

    /// The presentation time of the next vblank after `now`.
    pub fn next_presentation(&self, now: Duration) -> Duration {
        let _ = now;
        todo!("wm-clock")
    }

    /// When to start composing the frame for the next vblank after `now` (never earlier than `now`).
    pub fn start_at(&self, now: Duration) -> Duration {
        let _ = now;
        todo!("wm-clock")
    }
}
