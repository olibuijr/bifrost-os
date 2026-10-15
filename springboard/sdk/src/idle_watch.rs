//! App-facing idle tracking. [`IdleWatch`] tells an app when the user has been inactive for a timeout and when input
//! returns, so an app can run its own idle work (for example a screen saver style animation of its own). It wraps
//! `bifrost_core::idle_monitor::IdleMonitor`, which owns no timer: the caller passes the time of each input with
//! [`IdleWatch::input`] and the time of each check with [`IdleWatch::poll`].
//!
//! The shell's last-input time is not yet delivered to apps (no control message carries it), so an app sees idle
//! changes only from the input and clock values it is given. Events are queued and read with
//! [`IdleWatch::drain_events`], so a check that runs between frames never loses a change.

pub use bifrost_core::idle_monitor::{IdleEvent, IdleState};
use bifrost_core::idle_monitor::IdleMonitor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleWatch {
    monitor: IdleMonitor,
    pending: Vec<IdleEvent>,
}

impl IdleWatch {
    /// A watch that starts active, with the last input at `now_ms`. A zero timeout never reports idle.
    pub fn new(timeout_ms: u64, now_ms: u64) -> Self {
        IdleWatch { monitor: IdleMonitor::new(timeout_ms, now_ms), pending: Vec::new() }
    }

    pub fn timeout_ms(&self) -> u64 {
        self.monitor.timeout_ms()
    }

    pub fn set_timeout(&mut self, timeout_ms: u64) {
        self.monitor.set_timeout(timeout_ms);
    }

    pub fn state(&self) -> IdleState {
        self.monitor.state()
    }

    /// Milliseconds since the last input at `now_ms`.
    pub fn idle_for_ms(&self, now_ms: u64) -> u64 {
        self.monitor.idle_for_ms(now_ms)
    }

    /// The time in milliseconds at which the next `poll` reports idle, or None when it will not (idle detection is off, or
    /// the watch is already idle). An app can schedule its check for this time.
    pub fn next_check_at(&self) -> Option<u64> {
        self.monitor.next_check_at()
    }

    /// Records user input at `now_ms`. Input that ends an idle period queues `BecameActive`.
    pub fn input(&mut self, now_ms: u64) {
        if let Some(event) = self.monitor.note_activity(now_ms) {
            self.pending.push(event);
        }
    }

    /// Checks the timeout at `now_ms`. The first check past the timeout queues `BecameIdle`.
    pub fn poll(&mut self, now_ms: u64) {
        if let Some(event) = self.monitor.poll(now_ms) {
            self.pending.push(event);
        }
    }

    /// The changes queued since the last call, oldest first. The queue is emptied.
    pub fn drain_events(&mut self) -> Vec<IdleEvent> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn going_idle_queues_one_event_for_the_app() {
        let mut w = IdleWatch::new(1_000, 0);
        w.poll(999);
        assert!(w.drain_events().is_empty());
        w.poll(1_000);
        assert_eq!(w.state(), IdleState::Idle);
        assert_eq!(w.drain_events(), vec![IdleEvent::BecameIdle]);
        w.poll(5_000);
        assert!(w.drain_events().is_empty(), "the change is reported once");
    }

    #[test]
    fn input_while_idle_queues_became_active() {
        let mut w = IdleWatch::new(1_000, 0);
        w.poll(2_000);
        w.input(2_500);
        assert_eq!(w.drain_events(), vec![IdleEvent::BecameIdle, IdleEvent::BecameActive]);
        assert_eq!(w.state(), IdleState::Active);
    }

    #[test]
    fn input_while_active_queues_nothing() {
        let mut w = IdleWatch::new(1_000, 0);
        w.input(500);
        assert!(w.drain_events().is_empty());
        assert_eq!(w.idle_for_ms(900), 400);
    }

    #[test]
    fn zero_timeout_never_queues_idle() {
        let mut w = IdleWatch::new(0, 0);
        w.poll(u64::MAX);
        assert!(w.drain_events().is_empty());
        assert_eq!(w.state(), IdleState::Active);
    }

    #[test]
    fn changing_the_timeout_applies_to_the_next_check() {
        let mut w = IdleWatch::new(0, 0);
        w.set_timeout(500);
        assert_eq!(w.timeout_ms(), 500);
        w.poll(500);
        assert_eq!(w.drain_events(), vec![IdleEvent::BecameIdle]);
    }
}
