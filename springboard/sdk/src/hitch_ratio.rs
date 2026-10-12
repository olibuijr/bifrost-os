//! Animation hitch measurement as a pure model. A frame hitches when it takes longer than its budget. The time over the
//! budget is the hitch time of that frame, and the ratio is the hitch time divided by the total time spent on frames.
//!
//! All durations are whole microseconds, supplied by the caller, so nothing here reads a clock. The ratio is a plain
//! fraction of the measured frame time. The official reference normalises the ratio against the animation duration and
//! applies perceptual weighting that this model does not reproduce, so the value is a lower-level measure than the
//! official one and should not be compared with it directly.

/// The frame budget of a 60 Hz panel, in microseconds (1 000 000 / 60, rounded up).
pub const BUDGET_60HZ_US: u64 = 16_667;

/// Running totals of frame time and hitch time for one tracked animation or for the whole session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HitchMeter {
    total_animation_us: u64,
    total_hitch_us: u64,
    frames: u32,
    hitched_frames: u32,
}

impl HitchMeter {
    pub fn new() -> HitchMeter {
        HitchMeter::default()
    }

    /// Records one frame that took `duration_us` against a budget of `budget_us`. A frame at or under its budget adds
    /// only to the animation time. A frame over it adds the excess to the hitch time and counts as hitched.
    pub fn record_frame(&mut self, duration_us: u64, budget_us: u64) {
        self.total_animation_us = self.total_animation_us.saturating_add(duration_us);
        self.frames = self.frames.saturating_add(1);
        if duration_us > budget_us {
            self.total_hitch_us = self.total_hitch_us.saturating_add(duration_us - budget_us);
            self.hitched_frames = self.hitched_frames.saturating_add(1);
        }
    }

    /// The total time spent on recorded frames, in microseconds.
    pub fn total_animation_time_us(&self) -> u64 {
        self.total_animation_us
    }

    /// The total time frames spent over their budget, in microseconds.
    pub fn total_hitch_time_us(&self) -> u64 {
        self.total_hitch_us
    }

    /// The number of frames recorded.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The number of frames that went over their budget.
    pub fn hitched_frames(&self) -> u32 {
        self.hitched_frames
    }

    /// Hitch time divided by animation time, as a fraction from 0.0 to 1.0. With no animation time recorded the ratio
    /// is 0.0, so an idle meter reports no hitching.
    pub fn ratio(&self) -> f64 {
        if self.total_animation_us == 0 {
            return 0.0;
        }
        self.total_hitch_us as f64 / self.total_animation_us as f64
    }

    /// Clears every total, as when a new animation starts.
    pub fn reset(&mut self) {
        *self = HitchMeter::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn frames_under_budget_add_no_hitch() {
        let mut m = HitchMeter::new();
        m.record_frame(16_000, BUDGET_60HZ_US);
        m.record_frame(10_000, BUDGET_60HZ_US);
        assert_eq!(m.total_animation_time_us(), 26_000);
        assert_eq!(m.total_hitch_time_us(), 0);
        assert_eq!(m.hitched_frames(), 0);
        assert_eq!(m.ratio(), 0.0);
    }

    #[test]
    fn a_frame_exactly_on_budget_is_not_a_hitch() {
        let mut m = HitchMeter::new();
        m.record_frame(16_667, BUDGET_60HZ_US);
        assert_eq!(m.frames(), 1);
        assert_eq!(m.hitched_frames(), 0);
        assert_eq!(m.total_hitch_time_us(), 0);
    }

    #[test]
    fn the_ratio_is_hitch_time_over_animation_time() {
        // Frames: 16 000 (no hitch), 20 000 (hitch 3 333), 50 000 (hitch 33 333).
        let mut m = HitchMeter::new();
        m.record_frame(16_000, BUDGET_60HZ_US);
        m.record_frame(20_000, BUDGET_60HZ_US);
        m.record_frame(50_000, BUDGET_60HZ_US);
        assert_eq!(m.total_animation_time_us(), 86_000);
        assert_eq!(m.total_hitch_time_us(), 36_666);
        assert_eq!(m.hitched_frames(), 2);
        // 36 666 / 86 000 = 0.42634883...
        assert!(close(m.ratio(), 36_666.0 / 86_000.0));
    }

    #[test]
    fn an_idle_meter_reports_zero_ratio() {
        let m = HitchMeter::new();
        assert_eq!(m.ratio(), 0.0);
        assert_eq!(m.frames(), 0);
    }

    #[test]
    fn a_fully_hitched_run_has_ratio_of_hitch_to_frame_time() {
        // Two frames of 33 000 against a budget of 16 667: each hitches 16 333, so the ratio is 32 666 / 66 000.
        let mut m = HitchMeter::new();
        m.record_frame(33_000, BUDGET_60HZ_US);
        m.record_frame(33_000, BUDGET_60HZ_US);
        assert_eq!(m.total_hitch_time_us(), 32_666);
        assert!(close(m.ratio(), 32_666.0 / 66_000.0));
    }

    #[test]
    fn reset_clears_every_total() {
        let mut m = HitchMeter::new();
        m.record_frame(40_000, BUDGET_60HZ_US);
        m.reset();
        assert_eq!(m, HitchMeter::new());
        assert_eq!(m.ratio(), 0.0);
    }
}
