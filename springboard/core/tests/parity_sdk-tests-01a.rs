//! Parity sdk-tests-01a: headless tests for the idle monitor. The monitor is driven with explicit millisecond clocks,
//! so no test reads the wall clock or needs an input device.

use bifrost_core::idle_monitor::{IdleEvent, IdleMonitor, IdleState};

#[test]
fn idle_begins_exactly_at_the_timeout_and_is_reported_once() {
    let mut m = IdleMonitor::new(1000, 0);
    assert_eq!(m.state(), IdleState::Active);
    assert_eq!(m.poll(999), None);
    assert_eq!(m.poll(1000), Some(IdleEvent::BecameIdle));
    assert_eq!(m.state(), IdleState::Idle);
    assert_eq!(m.poll(5000), None, "the idle event is reported once");
}

#[test]
fn input_ends_an_idle_period_and_restarts_the_timer() {
    let mut m = IdleMonitor::new(1000, 0);
    assert_eq!(m.poll(1000), Some(IdleEvent::BecameIdle));
    assert_eq!(m.note_activity(2500), Some(IdleEvent::BecameActive));
    assert_eq!(m.state(), IdleState::Active);
    assert_eq!(m.idle_for_ms(2600), 100);
    assert_eq!(m.poll(3499), None);
    assert_eq!(m.poll(3500), Some(IdleEvent::BecameIdle));
}

#[test]
fn input_while_active_moves_the_deadline_without_an_event() {
    let mut m = IdleMonitor::new(1000, 0);
    assert_eq!(m.note_activity(600), None);
    assert_eq!(m.poll(1599), None, "the deadline moved to 1600");
    assert_eq!(m.poll(1600), Some(IdleEvent::BecameIdle));
}

#[test]
fn a_zero_timeout_never_goes_idle() {
    let mut m = IdleMonitor::new(0, 0);
    assert_eq!(m.poll(10_000_000), None);
    assert_eq!(m.state(), IdleState::Active);
    m.set_timeout(500);
    assert_eq!(m.timeout_ms(), 500);
    assert_eq!(m.poll(500), Some(IdleEvent::BecameIdle));
}

#[test]
fn a_clock_behind_the_last_input_counts_as_no_idle_time() {
    let mut m = IdleMonitor::new(1000, 500);
    assert_eq!(m.idle_for_ms(0), 0);
    assert_eq!(m.note_activity(800), None);
    assert_eq!(m.note_activity(100), None, "an older input does not rewind the last input");
    assert_eq!(m.idle_for_ms(1700), 900);
    assert_eq!(m.poll(1700), None);
    assert_eq!(m.poll(1800), Some(IdleEvent::BecameIdle));
}
