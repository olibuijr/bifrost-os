//! Parity sdk-complete-33: headless tests for when the idle monitor next needs a check. No test reads an input device.

use bifrost_core::idle_monitor::{IdleEvent, IdleMonitor};

#[test]
fn the_next_check_is_the_time_the_gap_reaches_the_timeout() {
    let mut monitor = IdleMonitor::new(1_000, 500);
    assert_eq!(monitor.next_check_at(), Some(1_500));
    assert_eq!(monitor.poll(1_499), None);
    assert_eq!(monitor.next_check_at(), Some(1_500), "polling early does not move the check");
    assert_eq!(monitor.poll(1_500), Some(IdleEvent::BecameIdle));
}

#[test]
fn input_moves_the_next_check_later() {
    let mut monitor = IdleMonitor::new(1_000, 500);
    monitor.note_activity(800);
    assert_eq!(monitor.next_check_at(), Some(1_800));
}

#[test]
fn an_idle_monitor_has_no_next_check_until_input_returns() {
    let mut monitor = IdleMonitor::new(1_000, 0);
    monitor.poll(1_000);
    assert_eq!(monitor.next_check_at(), None);
    monitor.note_activity(1_200);
    assert_eq!(monitor.next_check_at(), Some(2_200));
}

#[test]
fn a_zero_timeout_has_no_next_check() {
    assert_eq!(IdleMonitor::new(0, 0).next_check_at(), None);
}
