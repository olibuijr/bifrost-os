//! Parity sdk-complete-24: headless tests for the notification observer tokens and user info limits, the
//! authentication error codes, the keychain status codes, the locale accessors, date intervals and the ISO 8601
//! formatter. Every test runs on the host with no shell, no keychain store and no clock.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use bifrost_app::calendar_system::CalendarIdentifier;
use bifrost_app::date_interval::DateInterval;
use bifrost_app::formatters::iso8601;
use bifrost_app::keychain::Error as KeychainError;
use bifrost_app::local_auth::{LAError, ERROR_DOMAIN};
use bifrost_app::locale::{currency_symbol, Locale, MeasurementSystem};
use bifrost_app::notification::{
    distributed_user_info, NotificationCenter, MAX_USER_INFO_ENTRIES, MAX_USER_INFO_VALUE_CHARS,
};

#[test]
fn a_suspended_observer_misses_posts_and_hears_again_after_resume() {
    let hits = Rc::new(Cell::new(0));
    let h = hits.clone();
    let token = NotificationCenter.add_observer("parity.sdk-complete-24.suspend", None, move |_| h.set(h.get() + 1));
    token.suspend();
    NotificationCenter.post("parity.sdk-complete-24.suspend", None, HashMap::new());
    assert_eq!(hits.get(), 0, "a suspended observer hears nothing");
    token.resume();
    NotificationCenter.post("parity.sdk-complete-24.suspend", None, HashMap::new());
    assert_eq!(hits.get(), 1, "the post made after resume is heard");
}

#[test]
fn suspending_one_observer_leaves_the_others_on_the_name_running() {
    let quiet = Rc::new(Cell::new(0));
    let loud = Rc::new(Cell::new(0));
    let (q, l) = (quiet.clone(), loud.clone());
    let muted = NotificationCenter.add_observer("parity.sdk-complete-24.one", None, move |_| q.set(q.get() + 1));
    let _open = NotificationCenter.add_observer("parity.sdk-complete-24.one", None, move |_| l.set(l.get() + 1));
    muted.suspend();
    NotificationCenter.post("parity.sdk-complete-24.one", None, HashMap::new());
    assert_eq!((quiet.get(), loud.get()), (0, 1));
}

#[test]
fn distributed_user_info_is_sorted_and_keeps_at_most_thirty_two_entries() {
    let info: HashMap<String, String> = (0..40).map(|i| (format!("k{i:02}"), i.to_string())).collect();
    let kept = distributed_user_info(&info);
    assert_eq!(kept.len(), MAX_USER_INFO_ENTRIES);
    assert_eq!(kept.first(), Some(&("k00".to_string(), "0".to_string())));
    assert_eq!(kept.last(), Some(&("k31".to_string(), "31".to_string())), "the first 32 keys in sorted order are kept");
}

#[test]
fn a_long_user_info_value_is_cut_to_1024_characters_not_bytes() {
    let long: String = "é".repeat(MAX_USER_INFO_VALUE_CHARS + 76);
    let kept = distributed_user_info(&HashMap::from([("body".to_string(), long)]));
    assert_eq!(kept[0].1.chars().count(), MAX_USER_INFO_VALUE_CHARS);
    assert_eq!(kept[0].1.len(), MAX_USER_INFO_VALUE_CHARS * 2, "each accented letter is two bytes");
}

#[test]
fn small_distributed_user_info_passes_through_unchanged() {
    assert!(distributed_user_info(&HashMap::new()).is_empty());
    let kept = distributed_user_info(&HashMap::from([("who".to_string(), "ada".to_string())]));
    assert_eq!(kept, vec![("who".to_string(), "ada".to_string())]);
}

#[test]
fn every_authentication_error_has_a_numeric_code_that_maps_back() {
    let all = [
        LAError::AuthenticationFailed,
        LAError::UserCancel,
        LAError::UserFallback,
        LAError::SystemCancel,
        LAError::PasscodeNotSet,
        LAError::BiometryNotAvailable,
        LAError::BiometryNotEnrolled,
        LAError::BiometryLockout,
        LAError::AppCancel,
        LAError::InvalidContext,
        LAError::NotInteractive,
    ];
    for e in all {
        assert_eq!(LAError::from_code(e.code()), Some(e), "{e:?} round trips through its code");
    }
}

#[test]
fn the_authentication_error_codes_follow_the_reference_numbering() {
    assert_eq!(LAError::AuthenticationFailed.code(), -1);
    assert_eq!(LAError::PasscodeNotSet.code(), -5);
    assert_eq!(LAError::BiometryLockout.code(), -8);
    assert_eq!(LAError::NotInteractive.code(), -1004);
    assert_eq!(LAError::from_code(0), None, "zero is not an error");
    assert_eq!(LAError::from_code(-11), None, "an undefined code maps to no error");
}

#[test]
fn the_authentication_error_domain_is_a_bifrost_name() {
    assert!(ERROR_DOMAIN.starts_with("bifrost."));
}

#[test]
fn every_keychain_status_code_maps_back_to_its_error() {
    assert_eq!(KeychainError::from_code(KeychainError::DuplicateItem.code()), Some(KeychainError::DuplicateItem));
    assert_eq!(KeychainError::from_code(KeychainError::ItemNotFound.code()), Some(KeychainError::ItemNotFound));
    assert_eq!(KeychainError::from_code(KeychainError::NotPermitted.code()), Some(KeychainError::NotPermitted));
    assert_eq!(KeychainError::from_code(KeychainError::Param.code()), Some(KeychainError::Param));
    assert!(matches!(KeychainError::from_code(-36), Some(KeychainError::Io(_))), "an unnamed status is an I/O error");
    assert_eq!(KeychainError::from_code(0), None, "errSecSuccess is not an error");
}

#[test]
fn a_locale_reports_its_region_and_measurement_system() {
    let us = Locale::new("en_US");
    assert_eq!(us.region_code(), Some("US"));
    assert_eq!(us.measurement_system(), MeasurementSystem::UsCustomary);
    let is = Locale::new("is_IS");
    assert_eq!(is.region_code(), Some("IS"));
    assert_eq!(is.measurement_system(), MeasurementSystem::Metric);
    assert_eq!(Locale::new("en").region_code(), None, "a locale without a region reports none");
}

#[test]
fn a_locale_expects_the_calendar_of_its_language() {
    assert_eq!(Locale::new("th_TH").calendar_identifier(), CalendarIdentifier::Buddhist);
    assert_eq!(Locale::new("ja_JP").calendar_identifier(), CalendarIdentifier::Japanese);
    assert_eq!(Locale::new("ar_SA").calendar_identifier(), CalendarIdentifier::Islamic);
    assert_eq!(Locale::new("he_IL").calendar_identifier(), CalendarIdentifier::Hebrew);
    assert_eq!(Locale::new("is_IS").calendar_identifier(), CalendarIdentifier::Gregorian);
}

#[test]
fn common_currencies_have_their_symbols_and_others_keep_their_code() {
    assert_eq!(currency_symbol("USD"), "$");
    assert_eq!(currency_symbol("EUR"), "\u{20ac}");
    assert_eq!(currency_symbol("CAD"), "CA$");
    assert_eq!(currency_symbol("INR"), "\u{20b9}");
    assert_eq!(currency_symbol("CHF"), "CHF", "a code without a symbol is written as its code");
}

#[test]
fn a_date_interval_runs_from_its_start_up_to_but_not_including_its_end() {
    let span = DateInterval::new(100, 200).expect("an interval that runs forwards");
    assert_eq!((span.start(), span.end(), span.duration()), (100, 200, 100));
    assert!(span.contains(100));
    assert!(span.contains(199));
    assert!(!span.contains(200));
    assert!(!span.contains(99));
}

#[test]
fn a_date_interval_never_runs_backwards_and_takes_a_duration() {
    assert_eq!(DateInterval::new(10, 5), None);
    assert_eq!(DateInterval::new(5, 5).map(|d| d.duration()), Some(0), "an empty interval is allowed");
    assert_eq!(DateInterval::with_duration(10, -1), None);
    assert_eq!(DateInterval::with_duration(10, 5).map(|d| d.end()), Some(15));
    assert_eq!(DateInterval::with_duration(i64::MAX, 1), None, "an end past the largest instant is refused");
}

#[test]
fn two_date_intervals_intersect_only_when_they_share_time() {
    let a = DateInterval::new(0, 10).expect("valid");
    let b = DateInterval::new(5, 20).expect("valid");
    assert_eq!(a.intersection(&b), DateInterval::new(5, 10));
    assert!(a.intersects(&b));
    let touching = DateInterval::new(10, 20).expect("valid");
    assert_eq!(a.intersection(&touching), None, "intervals that only touch share no time");
    assert!(!a.intersects(&touching));
    let far = DateInterval::new(50, 60).expect("valid");
    assert!(!a.intersects(&far));
}

#[test]
fn the_iso_8601_form_writes_utc_with_a_z_and_local_time_with_an_offset() {
    assert_eq!(iso8601::string(0, 0), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601::string(0, 3600), "1970-01-01T01:00:00+01:00");
    assert_eq!(iso8601::string(0, -19_800), "1969-12-31T18:30:00-05:30");
}

#[test]
fn the_iso_8601_reader_takes_z_or_an_offset_and_refuses_bad_dates() {
    assert_eq!(iso8601::parse("1970-01-02T00:00:00Z"), Some(86_400));
    assert_eq!(iso8601::parse("1970-01-01T01:00:00+01:00"), Some(0));
    assert!(iso8601::parse("2024-02-29T00:00:00Z").is_some(), "2024 is a leap year");
    assert_eq!(iso8601::parse("2023-02-29T00:00:00Z"), None, "2023 has no February 29");
    assert_eq!(iso8601::parse("1970-13-01T00:00:00Z"), None);
    assert_eq!(iso8601::parse("1970-01-01 00:00:00Z"), None, "a space is not the T separator");
    assert_eq!(iso8601::parse("1970-01-01T00:00:00"), None, "a zone designator is required");
    assert_eq!(iso8601::parse("1970-01-01T00:00:00+0100"), None, "the offset needs its colon");
}

#[test]
fn the_iso_8601_form_reads_back_what_it_writes() {
    for t in [0_i64, 86_399, 951_782_400, 1_000_000_000] {
        assert_eq!(iso8601::parse(&iso8601::string(t, 0)), Some(t));
        assert_eq!(iso8601::parse(&iso8601::string(t, 7200)), Some(t));
    }
}
