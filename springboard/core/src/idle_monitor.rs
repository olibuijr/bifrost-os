//! Inactivity tracking that starts a screen saver. The monitor keeps the time of the last user input and reports
//! one transition when the gap reaches the timeout (`BecameIdle`), and one when input returns (`BecameActive`).
//! Times are milliseconds on the caller's clock, so the monitor owns no timer, no input device and no thread. The
//! input source calls [`IdleMonitor::note_activity`] and the frame or idle tick calls [`IdleMonitor::poll`].
//!
//! A timeout of zero disables idle detection: the monitor never reports idle. A clock that moves backwards counts as
//! no time passing, so a stale timestamp can never make the monitor idle early.

/// Whether the user has been inactive for the timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleState {
    Active,
    Idle,
}

/// A change of state reported by [`IdleMonitor::poll`] or [`IdleMonitor::note_activity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleEvent {
    BecameIdle,
    BecameActive,
}

/// Tracks inactivity against a timeout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleMonitor {
    timeout_ms: u64,
    last_activity_ms: u64,
    idle: bool,
}

impl IdleMonitor {
    /// Starts active, with the last input at `now_ms`.
    pub fn new(timeout_ms: u64, now_ms: u64) -> Self {
        IdleMonitor { timeout_ms, last_activity_ms: now_ms, idle: false }
    }

    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Changes the timeout. A zero timeout turns idle detection off; the state is kept until the next poll.
    pub fn set_timeout(&mut self, timeout_ms: u64) {
        self.timeout_ms = timeout_ms;
    }

    pub fn state(&self) -> IdleState {
        if self.idle {
            IdleState::Idle
        } else {
            IdleState::Active
        }
    }

    /// Milliseconds since the last input, or zero when the clock is behind the last input.
    pub fn idle_for_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.last_activity_ms)
    }

    /// Records user input at `now_ms`. Returns `BecameActive` when input ends an idle period.
    pub fn note_activity(&mut self, now_ms: u64) -> Option<IdleEvent> {
        self.last_activity_ms = self.last_activity_ms.max(now_ms);
        if self.idle {
            self.idle = false;
            Some(IdleEvent::BecameActive)
        } else {
            None
        }
    }

    /// Checks the timeout at `now_ms`. Returns `BecameIdle` once, when the gap first reaches the timeout.
    pub fn poll(&mut self, now_ms: u64) -> Option<IdleEvent> {
        if self.timeout_ms == 0 || self.idle {
            return None;
        }
        if self.idle_for_ms(now_ms) >= self.timeout_ms {
            self.idle = true;
            Some(IdleEvent::BecameIdle)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_active_and_stays_active_before_the_timeout() {
        let mut m = IdleMonitor::new(1_000, 0);
        assert_eq!(m.state(), IdleState::Active);
        assert_eq!(m.poll(999), None);
        assert_eq!(m.state(), IdleState::Active);
    }

    #[test]
    fn becomes_idle_exactly_at_the_timeout_and_reports_it_once() {
        let mut m = IdleMonitor::new(1_000, 0);
        assert_eq!(m.poll(1_000), Some(IdleEvent::BecameIdle));
        assert_eq!(m.state(), IdleState::Idle);
        assert_eq!(m.poll(5_000), None, "a second poll while idle reports nothing");
    }

    #[test]
    fn input_while_idle_reports_active_and_restarts_the_timer() {
        let mut m = IdleMonitor::new(1_000, 0);
        m.poll(2_000);
        assert_eq!(m.note_activity(2_500), Some(IdleEvent::BecameActive));
        assert_eq!(m.state(), IdleState::Active);
        assert_eq!(m.poll(3_499), None, "500 ms of gap is below the 1000 ms timeout");
        assert_eq!(m.poll(3_500), Some(IdleEvent::BecameIdle));
    }

    #[test]
    fn input_while_active_resets_the_timer_without_an_event() {
        let mut m = IdleMonitor::new(1_000, 0);
        assert_eq!(m.note_activity(900), None);
        assert_eq!(m.poll(1_500), None, "the last input was at 900, so the gap is 600");
        assert_eq!(m.idle_for_ms(1_500), 600);
    }

    #[test]
    fn zero_timeout_never_goes_idle() {
        let mut m = IdleMonitor::new(0, 0);
        assert_eq!(m.poll(u64::MAX), None);
        assert_eq!(m.state(), IdleState::Active);
    }

    #[test]
    fn a_clock_behind_the_last_input_counts_as_no_time() {
        let mut m = IdleMonitor::new(1_000, 5_000);
        assert_eq!(m.idle_for_ms(4_000), 0);
        assert_eq!(m.poll(4_000), None);
        m.note_activity(3_000);
        assert_eq!(m.idle_for_ms(5_500), 500, "the late input at 3000 does not move the last input back");
    }
}
