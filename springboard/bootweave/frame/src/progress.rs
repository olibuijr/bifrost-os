//! The progress model: the bar's position at an elapsed time, from a `BootTiming`. Monotonic (never moves back),
//! each stage owns a segment of the bar, a stage's segment is held below its end until the stage is reported done,
//! and the bar creeps on a stage that overruns its prediction. Hidden for boots predicted under 1.5 s. Pure and
//! host-tested. Owner: task 03.
//!
//! Positions are fractions of the bar (0.0 to 1.0). The checkpoints are the end of each stage:
//! `checkpoint(Loader) = loader_ms / total`, `checkpoint(Kernel) = (loader_ms + kernel_ms) / total`,
//! `checkpoint(Userspace) = 1.0`. Before any stage is reported done the running stage is `Loader`; the running stage
//! is always the one after the last reported one.

use crate::timing::{BootTiming, Stage};

/// Before the final stage is reported done the bar never shows more than this (the last 2 % is the done mark).
const HOLD: f32 = 0.98;

/// The bar's state across one boot (the loader and Bootweave each hold one, Bootweave seeded from the loader's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub timing: BootTiming,
    /// The furthest position shown so far (0..1).
    pub shown: f32,
    /// The last stage reported done.
    pub done: Option<Stage>,
}

/// The stage whose end the bar moves towards next: `None` means nothing has finished, so `Loader` runs.
fn running_after(done: Option<Stage>) -> Option<Stage> {
    match done {
        None => Some(Stage::Loader),
        Some(Stage::Loader) => Some(Stage::Kernel),
        Some(Stage::Kernel) => Some(Stage::Userspace),
        Some(Stage::Userspace) => None,
    }
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
    ///
    /// The running stage owns the segment from its start checkpoint to its end checkpoint (capped at `HOLD`). Inside
    /// the segment the bar follows the stage's predicted duration: 90 % of the segment at the predicted end. Past
    /// that the bar creeps, `0.9 + 0.1 * (x - 1) / x` of the segment for `x` stage durations elapsed, so it keeps
    /// moving on a slow stage and never reaches the stage's end before `stage_done`. A reported stage snaps the bar
    /// up to its checkpoint, and the position never moves back.
    pub fn at(&mut self, elapsed_ms: u32) -> f32 {
        let target = match running_after(self.done) {
            None => 1.0,
            Some(running) => {
                let total = self.timing.total_ms().max(1) as f32;
                let start_ms = match running {
                    Stage::Loader => 0,
                    Stage::Kernel => self.timing.stage_end_ms(Stage::Loader),
                    Stage::Userspace => self.timing.stage_end_ms(Stage::Kernel),
                };
                let duration = self.timing.stage_end_ms(running).saturating_sub(start_ms).max(1) as f32;
                let start = start_ms as f32 / total;
                let end = self.checkpoint(running).min(HOLD);
                let x = elapsed_ms.saturating_sub(start_ms) as f32 / duration;
                let fraction = if x <= 1.0 { 0.9 * x } else { 0.9 + 0.1 * (x - 1.0) / x };
                start + (end - start) * fraction
            }
        };
        let floor = match self.done {
            Some(stage) => self.checkpoint(stage),
            None => 0.0,
        };
        let target = if target > floor { target } else { floor };
        if target > self.shown {
            self.shown = target;
        }
        self.shown
    }

    /// Marks a stage done (the bar may then pass its checkpoint).
    pub fn stage_done(&mut self, stage: Stage) {
        self.done = Some(stage);
    }

    /// The fraction of the bar at the end of `stage`.
    pub fn checkpoint(&self, stage: Stage) -> f32 {
        if stage == Stage::Userspace {
            return 1.0;
        }
        let total = self.timing.total_ms().max(1) as f32;
        self.timing.stage_end_ms(stage) as f32 / total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Appearance;

    /// 300 + 1500 + 2000 = 3800 ms. Checkpoints: loader 300 / 3800 = 3/38, kernel 1800 / 3800 = 9/19.
    fn default_progress() -> Progress {
        Progress::new(BootTiming::DEFAULT)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn checkpoints_are_the_stage_ends() {
        let p = default_progress();
        assert!(close(p.checkpoint(Stage::Loader), 3.0 / 38.0));
        assert!(close(p.checkpoint(Stage::Kernel), 9.0 / 19.0));
        assert_eq!(p.checkpoint(Stage::Userspace), 1.0);
    }

    #[test]
    fn loader_segment_follows_its_predicted_duration() {
        let mut p = default_progress();
        assert_eq!(p.at(0), 0.0);
        // 150 ms of a 300 ms loader is half the stage: 0.45 of its segment (0 to 3/38).
        assert!(close(p.at(150), 0.45 * 3.0 / 38.0));
        // At the predicted end the bar is at 90 % of the loader segment.
        let mut q = default_progress();
        assert!(close(q.at(300), 0.9 * 3.0 / 38.0));
    }

    #[test]
    fn never_moves_backwards() {
        let mut p = default_progress();
        let later = p.at(1000);
        // An earlier time reports the furthest position already shown.
        assert_eq!(p.at(500), later);
        assert!(later > 0.0);
    }

    #[test]
    fn overrun_creeps_but_never_reaches_the_checkpoint_before_done() {
        let mut p = default_progress();
        let a = p.at(3000);
        let b = p.at(9000);
        let cap = p.checkpoint(Stage::Loader);
        assert!(a > 0.9 * cap, "a slow stage keeps moving past its predicted end");
        assert!(b > a, "the creep keeps going");
        assert!(b < cap, "held below the loader checkpoint until the loader is reported done");
    }

    #[test]
    fn stage_done_snaps_to_the_checkpoint_and_the_next_stage_continues() {
        let mut p = default_progress();
        p.at(300);
        p.stage_done(Stage::Loader);
        // The bar snaps to the loader checkpoint, 3/38.
        assert!(close(p.at(0), 3.0 / 38.0));
        // Kernel segment: starts at 300 ms, lasts 1500 ms. At 2500 ms, x = 2200 / 1500, fraction = 0.9 + 0.1 * (0.4667 / 1.4667).
        let at_2500 = p.at(2500);
        let segment = 9.0 / 19.0 - 3.0 / 38.0;
        let fraction = 0.9 + 0.1 * ((2200.0 / 1500.0) - 1.0) / (2200.0 / 1500.0);
        assert!(close(at_2500, 3.0 / 38.0 + segment * fraction));
        assert!(at_2500 < p.checkpoint(Stage::Kernel), "below the kernel checkpoint until done");
    }

    #[test]
    fn hides_boots_predicted_under_one_and_a_half_seconds() {
        let short = Progress::new(BootTiming { loader_ms: 300, kernel_ms: 600, userspace_ms: 599, samples: 2, appearance: Appearance::Dark });
        assert_eq!(short.timing.total_ms(), 1499);
        assert!(!short.visible());
        let shown = Progress::new(BootTiming { userspace_ms: 600, ..short.timing });
        assert_eq!(shown.timing.total_ms(), 1500);
        assert!(shown.visible());
    }

    #[test]
    fn all_stages_done_completes_the_bar() {
        let mut p = default_progress();
        p.stage_done(Stage::Loader);
        p.stage_done(Stage::Kernel);
        p.stage_done(Stage::Userspace);
        assert_eq!(p.at(0), 1.0);
    }

    #[test]
    fn userspace_segment_ends_at_the_hold_until_desktop_ready() {
        let mut p = default_progress();
        p.stage_done(Stage::Loader);
        p.stage_done(Stage::Kernel);
        // Running Userspace: segment 9/19 to 0.98; at its predicted duration (2000 ms) the bar is at 90 % of it.
        let start = 9.0 / 19.0;
        assert!(close(p.at(3800), start + (0.98 - start) * 0.9));
        assert!(p.at(60_000) < 0.98);
    }

    #[test]
    fn a_stage_that_overruns_into_the_hold_never_exceeds_it() {
        let mut p = default_progress();
        p.stage_done(Stage::Loader);
        p.stage_done(Stage::Kernel);
        assert!(p.at(10_000_000) <= 0.98 + 1e-6);
    }
}
