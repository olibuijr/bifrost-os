//! Parity sdk-complete-24: headless tests for the notification token table. The system names are fixed constants,
//! a change in the mounted volume count names a mount or an unmount, and polled, suspended and state values behave
//! per name. No test touches a socket shared with another process or a clock.

use bifrost_core::notify_center::{
    volume_change, NotifyCenter, CALENDAR_CHANGE, CLOCK_SET, CONTACTS_CHANGE, DISK_CACHE_INVALIDATE, HOSTNAME_CHANGE,
    LOG_DB_UPDATE, NETWORK_CHANGE, SYSTEM_EVENTS, TIME_ZONE_CHANGE, VOLUME_LOW_DISK_SPACE, VOLUME_MOUNT, VOLUME_UNMOUNT,
};

#[test]
fn every_system_event_constant_is_listed_once_under_the_system_namespace() {
    let named = [
        TIME_ZONE_CHANGE,
        CLOCK_SET,
        VOLUME_MOUNT,
        VOLUME_UNMOUNT,
        VOLUME_LOW_DISK_SPACE,
        NETWORK_CHANGE,
        CONTACTS_CHANGE,
        CALENDAR_CHANGE,
        HOSTNAME_CHANGE,
        DISK_CACHE_INVALIDATE,
        LOG_DB_UPDATE,
    ];
    assert_eq!(SYSTEM_EVENTS.len(), named.len());
    for name in named {
        assert!(name.starts_with("system."), "{name} is in the system namespace");
        assert_eq!(SYSTEM_EVENTS.iter().filter(|n| **n == name).count(), 1, "{name} is listed once");
    }
}

#[test]
fn the_time_zone_host_name_and_volume_names_have_their_wire_spelling() {
    assert_eq!(TIME_ZONE_CHANGE, "system.timezone.change");
    assert_eq!(HOSTNAME_CHANGE, "system.hostname.change");
    assert_eq!(VOLUME_MOUNT, "system.volume.mount");
    assert_eq!(VOLUME_UNMOUNT, "system.volume.unmount");
}

#[test]
fn a_rise_in_the_volume_count_is_a_mount_and_a_fall_is_an_unmount() {
    assert_eq!(volume_change(1, 2), Some(VOLUME_MOUNT));
    assert_eq!(volume_change(3, 2), Some(VOLUME_UNMOUNT));
    assert_eq!(volume_change(0, 0), None, "no change posts nothing");
    assert_eq!(volume_change(2, 2), None);
}

#[test]
fn a_poll_token_is_marked_by_each_post_and_each_check_clears_its_mark() {
    let mut c = NotifyCenter::new();
    let t = c.register(NETWORK_CHANGE);
    assert_eq!(c.post(NETWORK_CHANGE), 1);
    assert_eq!(c.post(NETWORK_CHANGE), 1, "a second post still reaches the token");
    assert!(c.check(t), "the mark is set after posts");
    assert!(!c.check(t), "one check clears it until the next post");
}

#[test]
fn a_post_to_one_name_does_not_mark_a_token_on_another_name() {
    let mut c = NotifyCenter::new();
    let clock = c.register(CLOCK_SET);
    assert_eq!(c.post(TIME_ZONE_CHANGE), 0);
    assert!(!c.check(clock));
}

#[test]
fn a_cancelled_token_is_no_longer_valid_and_gets_no_mark() {
    let mut c = NotifyCenter::new();
    let t = c.register(HOSTNAME_CHANGE);
    assert!(c.is_valid(t));
    assert!(c.cancel(t));
    assert!(!c.is_valid(t));
    assert_eq!(c.post(HOSTNAME_CHANGE), 0);
    assert!(!c.check(t));
}

#[test]
fn a_suspended_token_is_still_valid_but_misses_posts_until_resumed() {
    let mut c = NotifyCenter::new();
    let t = c.register(TIME_ZONE_CHANGE);
    assert!(c.suspend(t));
    assert!(c.is_valid(t), "suspension keeps the token");
    assert_eq!(c.post(TIME_ZONE_CHANGE), 0);
    assert!(c.resume(t));
    assert_eq!(c.post(TIME_ZONE_CHANGE), 1);
    assert!(c.check(t));
}

#[test]
fn lifecycle_calls_on_an_unknown_token_report_failure() {
    let mut c = NotifyCenter::new();
    assert!(!c.suspend(42));
    assert!(!c.resume(42));
    assert!(!c.cancel(42));
    assert!(!c.is_valid(42));
}

#[test]
fn name_state_is_shared_by_every_token_on_the_name_and_zero_by_default() {
    let mut c = NotifyCenter::new();
    let _a = c.register(CONTACTS_CHANGE);
    let _b = c.register(CONTACTS_CHANGE);
    assert_eq!(c.state(CONTACTS_CHANGE), 0);
    c.set_state(CONTACTS_CHANGE, 9);
    assert_eq!(c.state(CONTACTS_CHANGE), 9, "the value belongs to the name, not to one token");
    c.set_state(CONTACTS_CHANGE, u64::MAX);
    assert_eq!(c.state(CONTACTS_CHANGE), u64::MAX, "the full 64-bit range is kept");
}
