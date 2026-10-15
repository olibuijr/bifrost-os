//! Parity os-gaps-07: headless tests for the low disk space classifier and the activity eligibility model. Capacities
//! and times are given as values; no filesystem, clock or notification is touched.

use bifrost_core::low_space::{level, message, notification_name, Level, CRITICAL_BYTES, LOW_BYTES};
use bifrost_core::notify_center::VOLUME_LOW_DISK_SPACE;
use bifrost_core::volume::Capacity;

const GIB: u64 = 1024 * 1024 * 1024;

fn volume(total: u64, available: u64) -> Capacity {
    Capacity { total, free: available, available }
}

#[test]
fn a_large_volume_with_six_gibibytes_available_is_ok() {
    assert_eq!(level(&volume(1000 * GIB, 6 * GIB)), Level::Ok);
}

#[test]
fn a_large_volume_with_four_gibibytes_available_is_low() {
    assert_eq!(level(&volume(1000 * GIB, 4 * GIB)), Level::Low);
}

#[test]
fn a_large_volume_with_a_quarter_gibibyte_available_is_critical() {
    assert_eq!(level(&volume(1000 * GIB, GIB / 4)), Level::Critical);
}

#[test]
fn a_small_volume_is_judged_by_its_fraction_not_by_the_byte_limit() {
    // Ten gibibytes: the low limit is one gibibyte (a tenth), so one gibibyte less one byte is short.
    assert_eq!(level(&volume(10 * GIB, GIB - 1)), Level::Low);
    assert_eq!(level(&volume(10 * GIB, GIB)), Level::Ok);
}

#[test]
fn exactly_the_critical_limit_is_low_not_critical() {
    let total = 10 * GIB;
    let critical_limit = (total as f64 * 0.02) as u64;
    assert!(critical_limit < CRITICAL_BYTES);
    assert_eq!(level(&volume(total, critical_limit)), Level::Low);
    assert_eq!(level(&volume(total, critical_limit - 1)), Level::Critical);
}

#[test]
fn the_byte_limits_apply_to_a_large_volume() {
    assert_eq!(level(&volume(1000 * GIB, LOW_BYTES)), Level::Ok);
    assert_eq!(level(&volume(1000 * GIB, LOW_BYTES - 1)), Level::Low);
}

#[test]
fn a_zero_size_volume_has_no_shortage() {
    assert_eq!(level(&volume(0, 0)), Level::Ok, "a zero-size volume has no limit to fall below");
}

#[test]
fn levels_order_from_ok_to_critical() {
    assert!(Level::Ok < Level::Low);
    assert!(Level::Low < Level::Critical);
}

#[test]
fn only_a_shortage_posts_the_low_disk_space_notification() {
    assert_eq!(notification_name(Level::Ok), None);
    assert_eq!(notification_name(Level::Low), Some(VOLUME_LOW_DISK_SPACE));
    assert_eq!(notification_name(Level::Critical), Some(VOLUME_LOW_DISK_SPACE));
}

#[test]
fn the_warning_names_the_volume_and_says_how_urgent_it_is() {
    assert_eq!(message(Level::Ok, "Data"), None);
    let low = message(Level::Low, "Data").unwrap();
    assert!(low.starts_with("Data is low on disk space."));
    let critical = message(Level::Critical, "Data").unwrap();
    assert!(critical.starts_with("Data is almost out of disk space."));
}
