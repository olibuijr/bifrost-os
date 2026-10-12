//! The progress model: the bar's position at an elapsed time, from a `BootTiming`. Monotonic (never moves back),
//! eased, clamped below the next stage's checkpoint until that stage is reported done, hidden for boots predicted
//! under 1.5 s. Pure and host-tested. Owner: task 03.

use crate::timing::{BootTiming, Stage};

/// The bar's state across one boot (the loader and Bootweave each hold one, Bootweave seeded from the loader's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub timing: BootTiming,
    /// The furthest position shown so far (0..1).
    pub shown: f32,
    /// The last stage reported done.
    pub done: Option<Stage>,
}

impl Progress {
    pub fn new(timing: BootTiming) -> Progress {
        Progress { timing, shown: 0.0, done: None }
    }

    /// Whether the bar shows at all (predicted total of at least 1.5 s).
    pub fn visible(&self) -> bool {
        self.timing.total_ms() >= 1500
    }

    /// The position at `elapsed_ms` since loader start; updates and returns `shown`.
    pub fn at(&mut self, elapsed_ms: u32) -> f32 {
        let _ = elapsed_ms;
        todo!("boot-03")
    }

    /// Marks a stage done (the bar may then pass its checkpoint).
    pub fn stage_done(&mut self, stage: Stage) {
        self.done = Some(stage);
    }

    /// The fraction of the bar at the end of `stage`.
    pub fn checkpoint(&self, stage: Stage) -> f32 {
        let _ = stage;
        todo!("boot-03")
    }
}
