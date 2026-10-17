//! Parity sdk-os-06: headless tests for the system capability models in `bifrost_ui::os_services`. Each test drives the
//! public API with plain values. No test touches a device, a file, a clock or a display.

use bifrost_ui::os_services::alarm::{weekday, Alarm, EVERY_DAY, WEEKDAYS};
use bifrost_ui::os_services::drawable::{DrawableConfig, DrawableError, PixelFormat, DEFAULT_MAX_EDGE};
use bifrost_ui::os_services::idle::{
    effective_auto_lock_ms, should_auto_lock, system_sleep_after_ms, system_sleep_pref_value, LOW_POWER_LOCK_MS,
    SUSPEND_AFTER_MS, SYSTEM_SLEEP_PREF,
};
use bifrost_ui::os_services::output_route::{Router, Route, RouteChange, HEADPHONE_CAP};
use bifrost_ui::os_services::permissions::{Denied, Grants, Permission, UnknownPermission};
use bifrost_ui::os_services::time_limits::{next_limit_step, Downtime, Ignore, Policy, Reason, UnsupportedLimit};
use bifrost_ui::os_services::usage_record::{window_days, UsageDay};
use bifrost_ui::os_services::usb::{
    parse_code, parse_id, Device, DeviceClass, Interface, Matching, AUDIO_SUBCLASS_STREAMING, HID_PROTOCOL_KEYBOARD,
    HID_PROTOCOL_MOUSE, VIDEO_SUBCLASS_STREAMING,
};

/// Local day 20 000 (UTC midnight), used as the base of every clock value below.
const DAY: i64 = 20_000 * 86_400;

fn at(minute: i64) -> i64 {
    DAY + minute * 60
}

fn keyboard() -> Device {
    Device {
        vendor_id: 0x046d,
        product_id: 0xc31c,
        class: DeviceClass::Other(0),
        interfaces: vec![Interface { class: DeviceClass::Hid, subclass: 1, protocol: HID_PROTOCOL_KEYBOARD }],
    }
}

fn mouse() -> Device {
    Device {
        vendor_id: 0x046d,
        product_id: 0xc077,
        class: DeviceClass::Other(0),
        interfaces: vec![Interface { class: DeviceClass::Hid, subclass: 1, protocol: HID_PROTOCOL_MOUSE }],
    }
}

fn camera() -> Device {
    Device {
        vendor_id: 0x0bda,
        product_id: 0x5830,
        class: DeviceClass::Other(0),
        interfaces: vec![
            Interface { class: DeviceClass::Video, subclass: 1, protocol: 0 },
            Interface { class: DeviceClass::Video, subclass: VIDEO_SUBCLASS_STREAMING, protocol: 0 },
        ],
    }
}

fn headset() -> Device {
    Device {
        vendor_id: 0x08bb,
        product_id: 0x2704,
        class: DeviceClass::Other(0),
        interfaces: vec![
            Interface { class: DeviceClass::Audio, subclass: 1, protocol: 0 },
            Interface { class: DeviceClass::Audio, subclass: AUDIO_SUBCLASS_STREAMING, protocol: 0 },
        ],
    }
}

// --- USB device classes, descriptors and matching -------------------------------------------------------------------

#[test]
fn a_descriptor_class_code_maps_to_its_class_and_back() {
    assert_eq!(DeviceClass::from_code(0x03), DeviceClass::Hid);
    assert_eq!(DeviceClass::from_code(0x0e), DeviceClass::Video);
    assert_eq!(DeviceClass::Video.code(), 0x0e);
    assert_eq!(DeviceClass::from_code(0x42), DeviceClass::Other(0x42));
    assert_eq!(DeviceClass::Other(0x42).code(), 0x42);
}

#[test]
fn sysfs_identifiers_parse_as_hexadecimal() {
    assert_eq!(parse_id("1d6b\n"), Some(0x1d6b));
    assert_eq!(parse_id("zz"), None);
    assert_eq!(parse_code("03"), Some(3));
    assert_eq!(parse_code("100"), None, "a class code is one byte");
}

#[test]
fn a_keyboard_interface_is_not_a_pointer() {
    assert!(keyboard().is_keyboard());
    assert!(!keyboard().is_pointer());
    assert!(mouse().is_pointer());
    assert!(!mouse().is_keyboard());
}

#[test]
fn a_camera_has_a_video_streaming_interface_and_a_headset_an_audio_one() {
    assert!(camera().is_camera());
    assert!(!headset().is_camera());
    assert!(headset().is_audio());
    assert!(!camera().is_audio());
}

#[test]
fn an_empty_matching_selects_every_device() {
    assert!(Matching::default().matches(&keyboard()));
    assert!(Matching::default().matches(&camera()));
}

#[test]
fn a_vendor_matching_selects_only_that_vendor() {
    let by_vendor = Matching { vendor_id: Some(0x046d), ..Matching::default() };
    assert!(by_vendor.matches(&keyboard()));
    assert!(by_vendor.matches(&mouse()));
    assert!(!by_vendor.matches(&camera()));
}

#[test]
fn a_product_matching_needs_the_product_identifier_too() {
    let exact = Matching { vendor_id: Some(0x046d), product_id: Some(0xc077), ..Matching::default() };
    assert!(exact.matches(&mouse()));
    assert!(!exact.matches(&keyboard()));
}

#[test]
fn a_class_matching_selects_devices_with_an_interface_of_that_class() {
    let hid = Matching { class: Some(DeviceClass::Hid), ..Matching::default() };
    assert!(hid.matches(&keyboard()));
    assert!(hid.matches(&mouse()));
    assert!(!hid.matches(&camera()));
}

#[test]
fn a_device_level_class_matches_without_an_interface() {
    let hub = Device { vendor_id: 1, product_id: 2, class: DeviceClass::Hub, interfaces: vec![] };
    assert!(Matching { class: Some(DeviceClass::Hub), ..Matching::default() }.matches(&hub));
    assert!(!Matching { class: Some(DeviceClass::Hid), ..Matching::default() }.matches(&hub));
}

#[test]
fn a_protocol_matching_selects_the_mouse_and_not_the_keyboard() {
    let pointer = Matching { class: Some(DeviceClass::Hid), interface_protocol: Some(HID_PROTOCOL_MOUSE), ..Matching::default() };
    assert!(pointer.matches(&mouse()));
    assert!(!pointer.matches(&keyboard()));
}

#[test]
fn a_subclass_matching_needs_one_interface_with_both_terms() {
    let streaming = Matching {
        class: Some(DeviceClass::Video),
        interface_subclass: Some(VIDEO_SUBCLASS_STREAMING),
        ..Matching::default()
    };
    assert!(streaming.matches(&camera()));
    let audio_streaming = Matching { class: Some(DeviceClass::Audio), interface_subclass: Some(AUDIO_SUBCLASS_STREAMING), ..Matching::default() };
    assert!(!audio_streaming.matches(&camera()), "a camera has no audio interface");
    assert!(audio_streaming.matches(&headset()));
}

// --- Daily limits and Downtime -----------------------------------------------------------------------------------------

#[test]
fn the_limit_cycle_steps_through_the_choices_and_wraps_to_none() {
    assert_eq!(next_limit_step(0), 15);
    assert_eq!(next_limit_step(60), 120);
    assert_eq!(next_limit_step(240), 0);
    assert_eq!(next_limit_step(20), 15, "an unknown limit starts the cycle at the first step");
}

#[test]
fn a_downtime_window_must_use_valid_minutes() {
    assert_eq!(Downtime::new(true, 1440, 0), None);
    assert_eq!(Downtime::new(true, 0, 1439), Some(Downtime { on: true, start: 0, end: 1439 }));
}

#[test]
fn a_downtime_window_may_cross_midnight() {
    let night = Downtime::new(true, 22 * 60, 7 * 60).unwrap();
    assert!(night.active(23 * 60));
    assert!(night.active(0));
    assert!(night.active(6 * 60 + 59));
    assert!(!night.active(7 * 60));
    assert!(!night.active(12 * 60));
}

#[test]
fn a_switched_off_or_empty_downtime_window_covers_nothing() {
    assert!(!Downtime::new(false, 22 * 60, 7 * 60).unwrap().active(23 * 60));
    assert!(!Downtime::new(true, 600, 600).unwrap().active(600));
}

#[test]
fn a_daily_limit_accepts_only_the_limit_steps() {
    let mut policy = Policy::default();
    assert_eq!(policy.set_limit("mail", 45), Err(UnsupportedLimit(45)));
    assert_eq!(policy.limit("mail"), None);
    policy.set_limit("mail", 60).unwrap();
    assert_eq!(policy.limit("mail"), Some(60));
    policy.set_limit("mail", 0).unwrap();
    assert_eq!(policy.limit("mail"), None, "0 removes the limit");
}

#[test]
fn an_app_is_covered_once_its_used_time_reaches_the_limit() {
    let mut policy = Policy::default();
    policy.set_limit("mail", 60).unwrap();
    assert_eq!(policy.block("mail", 3599, at(600), 0), None);
    assert_eq!(policy.block("mail", 3600, at(600), 0), Some(Reason::Limit));
}

#[test]
fn settings_is_never_covered_and_an_unset_clock_blocks_nothing() {
    let mut policy = Policy::default();
    policy.set_limit("settings", 15).unwrap();
    assert_eq!(policy.block("settings", 99_999, at(600), 0), None);
    policy.set_limit("mail", 15).unwrap();
    assert_eq!(policy.block("mail", 99_999, 0, 0), None);
}

#[test]
fn downtime_covers_every_app_except_the_always_allowed_ones() {
    let mut policy = Policy::default();
    policy.set_downtime(Downtime::new(true, 22 * 60, 7 * 60).unwrap());
    assert_eq!(policy.block("mail", 0, at(23 * 60), 0), Some(Reason::Downtime));
    assert_eq!(policy.block("clock", 0, at(23 * 60), 0), None, "clock is allowed by default");
    assert_eq!(policy.block("settings", 0, at(23 * 60), 0), None);
    policy.set_allowed("mail", true);
    assert_eq!(policy.block("mail", 0, at(23 * 60), 0), None);
    policy.set_allowed("mail", false);
    assert_eq!(policy.block("mail", 0, at(12 * 60), 0), None, "outside the window nothing is covered");
}

#[test]
fn a_fifteen_minute_grant_lets_an_app_run_past_its_limit_for_that_long() {
    let mut policy = Policy::default();
    policy.set_limit("mail", 15).unwrap();
    let grant = at(600);
    policy.grant_ignore("mail", Ignore::Fifteen, grant, 0);
    assert_eq!(policy.block("mail", 900, grant + 899, 0), None);
    assert_eq!(policy.block("mail", 900, grant + 900, 0), Some(Reason::Limit));
}

#[test]
fn a_today_grant_lasts_until_the_local_day_changes() {
    let mut policy = Policy::default();
    policy.set_limit("mail", 15).unwrap();
    policy.grant_ignore("mail", Ignore::Today, at(10 * 60), 0);
    assert_eq!(policy.block("mail", 900, at(23 * 60), 0), None);
    assert_eq!(policy.block("mail", 900, at(24 * 60 + 30), 0), Some(Reason::Limit));
}

#[test]
fn a_grant_overrides_downtime_for_the_granted_app() {
    let mut policy = Policy::default();
    policy.set_downtime(Downtime::new(true, 22 * 60, 7 * 60).unwrap());
    policy.grant_ignore("mail", Ignore::Fifteen, at(23 * 60), 0);
    assert_eq!(policy.block("mail", 0, at(23 * 60 + 5), 0), None);
}

// --- Usage records -----------------------------------------------------------------------------------------------------

#[test]
fn a_usage_day_round_trips_through_its_day_file_text() {
    let mut day = UsageDay { pickups: 4, ..UsageDay::default() };
    day.secs.insert("mail".into(), 90);
    day.hours[9] = 90;
    day.notifs.insert("mail".into(), 2);
    assert_eq!(UsageDay::parse(&day.to_text()), day);
}

#[test]
fn a_usage_day_file_skips_lines_it_does_not_understand() {
    let day = UsageDay::parse("nonsense\napp.files=x\nhour.30=5\napp.files=12\n");
    assert_eq!(day.app_secs("files"), 12);
    assert_eq!(day.total(), 12);
    assert_eq!(day.hours.iter().sum::<u32>(), 0);
}

#[test]
fn apps_rank_by_foreground_time_then_by_key() {
    let mut day = UsageDay::default();
    day.secs.insert("notes".into(), 60);
    day.secs.insert("calendar".into(), 60);
    day.secs.insert("browser".into(), 300);
    day.secs.insert("idle".into(), 0);
    assert_eq!(day.ranked(), vec![("browser", 300), ("calendar", 60), ("notes", 60)]);
}

#[test]
fn adding_two_usage_days_sums_every_counter() {
    let mut a = UsageDay { pickups: 1, ..UsageDay::default() };
    a.secs.insert("mail".into(), 10);
    let mut b = UsageDay { pickups: 2, ..UsageDay::default() };
    b.secs.insert("mail".into(), 5);
    b.notifs.insert("mail".into(), 3);
    b.hours[0] = 15;
    a.add(&b);
    assert_eq!(a.pickups, 3);
    assert_eq!(a.app_secs("mail"), 15);
    assert_eq!(a.notification_total(), 3);
    assert_eq!(a.hours[0], 15);
}

#[test]
fn a_seven_day_history_ends_at_today_oldest_first() {
    assert_eq!(window_days(10, 3), vec![8, 9, 10]);
    assert_eq!(window_days(10, 7).len(), 7);
}

// --- Drawables ---------------------------------------------------------------------------------------------------------

#[test]
fn a_drawable_is_the_layer_size_in_points_times_the_scale() {
    let config = DrawableConfig::default();
    assert_eq!(config.drawable_size((1200.0, 780.0), 2.0), Ok((2400, 1560)));
}

#[test]
fn a_drawable_keeps_its_aspect_ratio_under_the_edge_limit() {
    let config = DrawableConfig::new(PixelFormat::Bgra8, 2, 2000).unwrap();
    assert_eq!(config.drawable_size((1200.0, 780.0), 2.0), Ok((2000, 1300)));
}

#[test]
fn a_drawable_rejects_a_scale_that_is_not_positive_and_finite() {
    let config = DrawableConfig::default();
    assert_eq!(config.drawable_size((10.0, 10.0), 0.0), Err(DrawableError::Scale));
    assert_eq!(config.drawable_size((10.0, 10.0), f32::NAN), Err(DrawableError::Scale));
    assert_eq!(config.drawable_size((-1.0, 10.0), 2.0), Err(DrawableError::Size));
}

#[test]
fn a_swap_set_has_two_or_three_drawables_and_a_nonzero_edge_limit() {
    assert_eq!(DrawableConfig::new(PixelFormat::Bgra8, 1, 100), Err(DrawableError::DrawableCount(1)));
    assert_eq!(DrawableConfig::new(PixelFormat::Bgra8, 4, 100), Err(DrawableError::DrawableCount(4)));
    assert_eq!(DrawableConfig::new(PixelFormat::Bgra8, 2, 0), Err(DrawableError::MaxEdge));
    assert!(DrawableConfig::new(PixelFormat::Rgba16Float, 3, 100).is_ok());
}

#[test]
fn a_swap_set_reports_the_memory_all_its_drawables_take() {
    let bgra = DrawableConfig::new(PixelFormat::Bgra8, 3, DEFAULT_MAX_EDGE).unwrap();
    assert_eq!(bgra.memory_bytes((1000, 500)), 6_000_000);
    let float = DrawableConfig::new(PixelFormat::Rgba16Float, 2, DEFAULT_MAX_EDGE).unwrap();
    assert_eq!(float.memory_bytes((10, 10)), 1600);
}

// --- Idle, auto-lock and system sleep ----------------------------------------------------------------------------------

#[test]
fn low_power_mode_shortens_auto_lock_and_gives_a_zero_delay_the_low_power_value() {
    assert_eq!(effective_auto_lock_ms(0, true), LOW_POWER_LOCK_MS);
    assert_eq!(effective_auto_lock_ms(120_000, true), LOW_POWER_LOCK_MS);
    assert_eq!(effective_auto_lock_ms(10_000, true), 10_000);
    assert_eq!(effective_auto_lock_ms(0, false), 0);
    assert_eq!(effective_auto_lock_ms(120_000, false), 120_000);
}

#[test]
fn the_screen_locks_once_idle_time_reaches_the_delay_and_never_when_the_delay_is_zero() {
    assert!(!should_auto_lock(59_999, 60_000));
    assert!(should_auto_lock(60_000, 60_000));
    assert!(!should_auto_lock(u64::MAX, 0));
}

#[test]
fn a_system_sleep_preference_in_minutes_becomes_milliseconds() {
    assert_eq!(system_sleep_after_ms(Some("15")), Some(900_000.0));
    assert_eq!(system_sleep_after_ms(Some(" 5 ")), Some(300_000.0));
}

#[test]
fn a_system_sleep_preference_that_is_never_or_invalid_means_never_sleep() {
    assert_eq!(system_sleep_after_ms(Some("never")), None);
    assert_eq!(system_sleep_after_ms(Some("0")), None);
    assert_eq!(system_sleep_after_ms(None), None);
    assert_eq!(system_sleep_pref_value(0), "never");
    assert_eq!(system_sleep_pref_value(30), "30");
}

#[test]
fn the_sleep_constants_match_the_shell_values() {
    assert_eq!(SUSPEND_AFTER_MS, 30_000.0);
    assert_eq!(SYSTEM_SLEEP_PREF, "system_sleep_minutes");
}

// --- Output routing ----------------------------------------------------------------------------------------------------

#[test]
fn plugging_in_headphones_moves_sound_and_caps_a_loud_headphone_volume() {
    let mut router = Router::new(0.5, 0.9, false, false);
    assert_eq!(router.route(), Route::Speaker);
    assert_eq!(router.set_jack(true), Some(RouteChange { route: Route::Headphones, pause: false }));
    assert_eq!(router.volume(), HEADPHONE_CAP);
    assert_eq!(router.set_jack(true), None, "a plug event that changes nothing returns None");
}

#[test]
fn unplugging_headphones_returns_to_the_speaker_and_asks_for_a_pause() {
    let mut router = Router::new(0.5, 0.6, false, true);
    assert_eq!(router.set_jack(false), Some(RouteChange { route: Route::Speaker, pause: true }));
    assert_eq!(router.volume(), 0.5);
}

#[test]
fn a_router_that_starts_with_the_jack_plugged_selects_headphones_at_once() {
    let router = Router::new(0.5, 0.9, false, true);
    assert_eq!(router.route(), Route::Headphones);
    assert_eq!(router.volume(), HEADPHONE_CAP);
    assert_eq!(router.volume_of(Route::Speaker), 0.5);
}

#[test]
fn going_above_the_cap_on_headphones_lifts_it_for_good() {
    let mut router = Router::new(0.5, 0.2, false, true);
    router.set_volume(HEADPHONE_CAP);
    assert!(!router.cap_lifted(), "reaching the cap does not lift it");
    router.set_volume(0.95);
    assert!(router.cap_lifted());
    router.set_jack(false);
    router.set_jack(true);
    assert_eq!(router.volume(), 0.95, "a lifted cap is not applied again on plug-in");
}

#[test]
fn a_route_volume_is_clamped_to_the_unit_range() {
    let mut router = Router::new(0.5, 0.5, false, false);
    router.set_volume(2.0);
    assert_eq!(router.volume(), 1.0);
    router.set_volume(f32::NAN);
    assert_eq!(router.volume(), 0.0);
    assert_eq!(Route::Headphones.pref_key(), "volume_headphones");
    assert_eq!(Route::Speaker.label(), "Speaker");
}

// --- Alarm schedules ---------------------------------------------------------------------------------------------------

#[test]
fn an_alarm_needs_a_minute_of_the_day_and_known_weekdays() {
    assert_eq!(Alarm::new(1440, 0), None);
    assert_eq!(Alarm::new(420, 0x80), None, "bit 7 is not a weekday");
    assert_eq!(Alarm::new(420, WEEKDAYS), Some(Alarm { enabled: true, minute: 420, repeat: WEEKDAYS }));
    assert_eq!(EVERY_DAY.count_ones(), 7);
}

#[test]
fn day_zero_was_a_thursday_and_the_day_before_a_wednesday() {
    assert_eq!(weekday(0), 4);
    assert_eq!(weekday(3), 0);
    assert_eq!(weekday(-1), 3);
}

#[test]
fn an_alarm_rings_once_within_the_grace_window_after_its_time() {
    let at_seven = 4 * 86_400 + 7 * 3600;
    let alarm = Alarm::new(420, EVERY_DAY).unwrap();
    assert!(alarm.occurs_in(at_seven - 60, at_seven + 30, 0));
    assert!(!alarm.occurs_in(at_seven, at_seven + 30, 0), "a check at the alarm time does not ring it twice");
    assert!(!alarm.occurs_in(at_seven - 1, at_seven + 601, 0), "an occurrence older than the grace window is dropped");
}

#[test]
fn a_disabled_alarm_never_rings() {
    let alarm = Alarm { enabled: false, minute: 420, repeat: 0 };
    assert!(!alarm.occurs_in(0, 4 * 86_400 + 7 * 3600 + 30, 0));
    assert_eq!(alarm.next_occurrence(0, 0), None);
}

#[test]
fn a_weekday_alarm_skips_the_days_outside_its_mask() {
    let alarm = Alarm::new(420, WEEKDAYS).unwrap();
    let saturday = 2 * 86_400 + 7 * 3600;
    let monday = 4 * 86_400 + 7 * 3600;
    assert!(!alarm.occurs_in(saturday - 1, saturday + 30, 0));
    assert!(alarm.occurs_in(monday - 1, monday + 30, 0));
    assert_eq!(alarm.next_occurrence(saturday + 3600, 0), Some(monday));
}

#[test]
fn the_next_occurrence_is_the_first_instant_after_now() {
    let alarm = Alarm::new(420, EVERY_DAY).unwrap();
    let monday_eight = 4 * 86_400 + 8 * 3600;
    assert_eq!(alarm.next_occurrence(monday_eight, 0), Some(5 * 86_400 + 7 * 3600));
}

#[test]
fn an_alarm_follows_the_local_offset_from_utc() {
    let alarm = Alarm::new(420, EVERY_DAY).unwrap();
    let utc_six = 4 * 86_400 + 6 * 3600;
    assert!(alarm.occurs_in(utc_six - 60, utc_six + 30, 3600), "07:00 local at UTC+1 is 06:00 UTC");
}

// --- App permissions ---------------------------------------------------------------------------------------------------

#[test]
fn every_permission_round_trips_through_its_manifest_name() {
    for permission in Permission::ALL {
        assert_eq!(Permission::from_manifest_name(permission.manifest_name()), Some(permission));
    }
    assert_eq!(Permission::from_manifest_name("display-config"), Some(Permission::DisplayConfig));
    assert_eq!(Permission::from_manifest_name("remote-desktop"), None, "a first-party entitlement is not a manifest permission");
}

#[test]
fn manifest_permissions_become_grants_once_each() {
    let grants = Grants::from_manifest(["network", "storage", "network"]).unwrap();
    assert_eq!(grants.granted().collect::<Vec<_>>(), vec![Permission::Network, Permission::Storage]);
}

#[test]
fn a_misspelt_manifest_permission_is_an_error() {
    assert_eq!(Grants::from_manifest(["netwrk"]), Err(UnknownPermission("netwrk".into())));
}

#[test]
fn a_feature_needs_its_permission_to_be_granted() {
    let grants = Grants::from_manifest(["network"]).unwrap();
    assert!(grants.allows(Permission::Network));
    assert_eq!(grants.require(Permission::Network), Ok(()));
    assert_eq!(grants.require(Permission::Audio), Err(Denied(Permission::Audio)));
}

#[test]
fn a_stored_grant_is_restored_as_it_was_written() {
    let mut p = Policy::default();
    p.restore_ignore("files", Some(1_000 + 900), None);
    assert!(p.ignored("files", 1_500, 0), "the fifteen-minute pass is still running");
    assert!(!p.ignored("files", 2_000, 0), "and it has ended after its stored end");
    p.restore_ignore("files", None, None);
    assert!(!p.ignored("files", 1_500, 0), "clearing the grant removes it");
}
