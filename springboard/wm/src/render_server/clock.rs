//! Per-output frame clock (the display-link side of the server): learns the refresh interval and phase from vblank
//! (page-flip) timestamps, predicts the next presentation time, and starts composition at
//! `next_vblank - budget`, where the budget is the 95th percentile of recent compose+submit times plus 1 ms
//! (starting at 4 ms, clamped to 2..12 ms). Sends `wp_presentation` feedback and fires frame callbacks with the
//! predicted presentation time. Pure scheduling logic lives in `FrameClock` (host-tested); the wiring into the event
//! loop is in `drm.rs`/`nested.rs`.
//!
//! Refresh refinement: a vblank delta within 10% of the current refresh is an inlier and moves the estimate by one
//! eighth of the error (an exponential average). A delta of about two periods is a missed vblank: the phase anchor
//! moves to it but the refresh does not change. Non-increasing timestamps are ignored.
//!
//! Owner: wave G1 agent `wm-clock`.

use std::time::Duration;

use super::stats::percentile;

/// Frames of compose+submit time kept for the budget.
const RECENT_MAX: usize = 120;
/// Budget used before any frame has been measured.
const START_BUDGET: Duration = Duration::from_millis(4);
/// Safety margin added to the 95th percentile.
const BUDGET_MARGIN: Duration = Duration::from_millis(1);
const BUDGET_MIN: Duration = Duration::from_millis(2);
const BUDGET_MAX: Duration = Duration::from_millis(12);
/// Refresh used when the caller passes zero (60 Hz nominal).
const FALLBACK_REFRESH: Duration = Duration::from_nanos(16_666_667);

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
        let refresh = if refresh.is_zero() { FALLBACK_REFRESH } else { refresh };
        FrameClock { refresh, last_vblank: None, recent: Vec::new() }
    }

    /// Records a vblank (page flip completed) at monotonic time `t`.
    pub fn vblank(&mut self, t: Duration) {
        if let Some(last) = self.last_vblank {
            if t <= last {
                return;
            }
            let delta = t - last;
            if is_inlier(delta, self.refresh) {
                self.refresh = refine(self.refresh, delta);
            }
        }
        self.last_vblank = Some(t);
    }

    /// Records how long a frame took from start of composition to submission.
    pub fn frame_took(&mut self, d: Duration) {
        self.recent.push(d);
        if self.recent.len() > RECENT_MAX {
            let excess = self.recent.len() - RECENT_MAX;
            self.recent.drain(..excess);
        }
    }

    /// The current budget (see the module doc).
    pub fn budget(&self) -> Duration {
        if self.recent.is_empty() {
            return START_BUDGET;
        }
        (percentile(&self.recent, 95.0) + BUDGET_MARGIN).clamp(BUDGET_MIN, BUDGET_MAX)
    }

    /// The presentation time of the next vblank after `now`.
    pub fn next_presentation(&self, now: Duration) -> Duration {
        match self.last_vblank {
            // No phase yet: assume the next vblank is one period away.
            None => now.saturating_add(self.refresh),
            Some(last) => {
                if last > now {
                    return last;
                }
                let period = self.period_nanos();
                let elapsed = (now - last).as_nanos();
                let periods = elapsed / period + 1;
                last.saturating_add(Duration::from_nanos((periods * period) as u64))
            }
        }
    }

    /// When to start composing the frame for the next vblank after `now` (never earlier than `now`).
    pub fn start_at(&self, now: Duration) -> Duration {
        let next = self.next_presentation(now);
        next.saturating_sub(self.budget()).max(now)
    }

    /// The refresh in nanoseconds, never zero (a public field may have been set to zero).
    fn period_nanos(&self) -> u128 {
        self.refresh.as_nanos().max(1)
    }
}

/// True when `delta` is within 10% of `refresh`.
fn is_inlier(delta: Duration, refresh: Duration) -> bool {
    let d = delta.as_nanos() * 10;
    let r = refresh.as_nanos() * 10;
    d >= r / 10 * 9 && d <= r / 10 * 11
}

/// Moves `refresh` one eighth of the way toward `delta`.
fn refine(refresh: Duration, delta: Duration) -> Duration {
    let r = refresh.as_nanos() as i128;
    let d = delta.as_nanos() as i128;
    Duration::from_nanos((r + (d - r) / 8) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    fn us(v: u64) -> Duration {
        Duration::from_micros(v)
    }

    #[test]
    fn budget_starts_at_four_ms() {
        let c = FrameClock::new(ms(16));
        assert_eq!(c.budget(), ms(4));
    }

    #[test]
    fn zero_refresh_falls_back_to_sixty_hz() {
        let c = FrameClock::new(Duration::ZERO);
        assert_eq!(c.refresh, Duration::from_nanos(16_666_667));
    }

    #[test]
    fn no_vblank_predicts_one_period_ahead() {
        let c = FrameClock::new(ms(16));
        assert_eq!(c.next_presentation(ms(100)), ms(116));
        // Without phase, start is one period minus the 4 ms budget.
        assert_eq!(c.start_at(ms(100)), ms(112));
    }

    #[test]
    fn steady_vblanks_keep_the_period_and_phase() {
        let mut c = FrameClock::new(ms(16));
        c.vblank(ms(100));
        c.vblank(ms(116));
        c.vblank(ms(132));
        assert_eq!(c.refresh, ms(16));
        assert_eq!(c.last_vblank, Some(ms(132)));
        // now=140 is 8 ms after the vblank at 132: next is 148.
        assert_eq!(c.next_presentation(ms(140)), ms(148));
        // Exactly on a vblank, the next one is a full period later.
        assert_eq!(c.next_presentation(ms(132)), ms(148));
        // Before the anchor, the anchor itself is the next one.
        assert_eq!(c.next_presentation(ms(120)), ms(132));
    }

    #[test]
    fn one_inlier_delta_moves_refresh_by_an_eighth_of_the_error() {
        // Start at 16 ms, one 17 ms delta: 16 + (17 - 16) / 8 = 16.125 ms exactly.
        let mut c = FrameClock::new(ms(16));
        c.vblank(ms(100));
        c.vblank(ms(117));
        assert_eq!(c.refresh, Duration::from_nanos(16_125_000));
    }

    #[test]
    fn refresh_converges_toward_the_measured_period() {
        let mut c = FrameClock::new(ms(16));
        let mut t = ms(0);
        c.vblank(t);
        for _ in 0..200 {
            t += ms(17);
            c.vblank(t);
        }
        let err = c.refresh.as_nanos() as i128 - 17_000_000;
        assert!(err.abs() < 1_000, "refresh {:?} should be within 1 us of 17 ms", c.refresh);
    }

    #[test]
    fn missed_vblank_moves_phase_but_not_refresh() {
        let mut c = FrameClock::new(ms(16));
        c.vblank(ms(100));
        // A 33 ms delta is two periods: an outlier for the refresh estimate.
        c.vblank(ms(133));
        assert_eq!(c.refresh, ms(16));
        assert_eq!(c.last_vblank, Some(ms(133)));
        assert_eq!(c.next_presentation(ms(134)), ms(149));
    }

    #[test]
    fn non_increasing_timestamps_are_ignored() {
        let mut c = FrameClock::new(ms(16));
        c.vblank(ms(100));
        c.vblank(ms(100));
        c.vblank(ms(50));
        assert_eq!(c.last_vblank, Some(ms(100)));
        assert_eq!(c.refresh, ms(16));
    }

    #[test]
    fn long_idle_gap_re_anchors_without_touching_refresh() {
        let mut c = FrameClock::new(ms(16));
        c.vblank(ms(100));
        c.vblank(ms(5_000));
        assert_eq!(c.last_vblank, Some(ms(5_000)));
        assert_eq!(c.refresh, ms(16));
    }

    #[test]
    fn budget_is_p95_plus_one_ms() {
        let mut c = FrameClock::new(ms(16));
        for _ in 0..20 {
            c.frame_took(ms(5));
        }
        assert_eq!(c.budget(), ms(6));
    }

    #[test]
    fn budget_clamps_to_two_and_twelve_ms() {
        let mut low = FrameClock::new(ms(16));
        low.frame_took(us(500));
        assert_eq!(low.budget(), ms(2));
        let mut high = FrameClock::new(ms(16));
        for _ in 0..10 {
            high.frame_took(ms(30));
        }
        assert_eq!(high.budget(), ms(12));
    }

    #[test]
    fn budget_follows_the_95th_percentile_not_the_mean() {
        // 19 frames at 2 ms and one at 10 ms: nearest rank 95 of 20 is the 19th value (2 ms), so budget 3 ms.
        let mut c = FrameClock::new(ms(16));
        for _ in 0..19 {
            c.frame_took(ms(2));
        }
        c.frame_took(ms(10));
        assert_eq!(c.budget(), ms(3));
    }

    #[test]
    fn recent_keeps_only_the_newest_120() {
        let mut c = FrameClock::new(ms(16));
        for i in 0..130u64 {
            c.frame_took(ms(i));
        }
        assert_eq!(c.recent.len(), 120);
        assert_eq!(c.recent[0], ms(10));
        assert_eq!(c.recent[119], ms(129));
    }

    #[test]
    fn start_at_is_budget_before_the_next_vblank() {
        let mut c = FrameClock::new(ms(16));
        c.vblank(ms(100));
        // Budget 4 ms (no samples), next vblank at 116, start at 112.
        assert_eq!(c.start_at(ms(105)), ms(112));
        // Later than the start: compose now, never in the past.
        assert_eq!(c.start_at(ms(113)), ms(113));
    }
}
