//! Store catalogue parsing and the metadata the store shows for each entry: price, age rating, product type,
//! screenshots, release notes and the install-kind rules.
//!
//! Headless: the catalogue is parsed from an in-memory JSON string. No network, cache file, window or device is
//! involved, and the dates are computed from fixed civil days.

use bifrost_ui::product::ProductType;
use springboard::bare::timezone::days_from_civil;
use springboard::shell::store::api::{age_rating_of, parse_catalog, parse_changelog, parse_screenshots, relative_date, update_available, AGE_RATINGS, MAX_SHOTS};
use springboard::shell::store::json;

const SHA: &str = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";

fn catalog() -> String {
    format!(
        r#"{{"apps": [
  {{"id": "org.example.notes", "name": "Notes", "version": "1.2.3", "package": "notes.pkg",
    "summary": "Write things down", "category": "Productivity", "size": 2048, "sha256": "{SHA}",
    "price": 1250, "currency": "EUR", "age_rating": " 12+ ", "product_type": "auto-renewable",
    "screenshots": ["a.png", "  ", "b.png"],
    "changelog": [{{"version": "1.2.3", "date": "2026-10-01", "notes": ["Fixed a crash", " "]}}, {{"version": "1.2.2"}}, {{"notes": ["no version"]}}]}},
  {{"id": "org.example.helper", "name": "Helper", "version": "0.1.0", "package": "helper.pkg", "kind": "service",
    "sha256": "{SHA}", "price": 0, "currency": "EUR"}},
  {{"id": "org.example.badhash", "name": "Bad hash", "version": "1.0.0", "package": "bad.pkg", "sha256": "abc"}},
  {{"id": "org.example.badversion", "name": "Bad version", "version": "one", "package": "bad.pkg", "sha256": "{SHA}"}},
  {{"name": "No id", "version": "1.0.0", "package": "x.pkg", "sha256": "{SHA}"}}
]}}"#
    )
}

#[test]
fn entries_missing_a_required_field_or_with_a_bad_version_or_hash_are_skipped() {
    let apps = parse_catalog(&catalog()).unwrap();
    let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["org.example.notes", "org.example.helper"]);
}

#[test]
fn the_catalogue_must_be_json_with_an_apps_array() {
    assert!(parse_catalog("not json").is_err());
    assert_eq!(parse_catalog(r#"{"apps": 3}"#).unwrap_err(), "catalogue has no \"apps\" array");
    assert!(parse_catalog(r#"{"apps": []}"#).unwrap().is_empty());
}

#[test]
fn a_paid_entry_carries_its_price_rating_product_type_and_screenshots() {
    let apps = parse_catalog(&catalog()).unwrap();
    let notes = &apps[0];
    assert_eq!(notes.name, "Notes");
    assert_eq!(notes.version, "1.2.3");
    assert_eq!(notes.package, "notes.pkg");
    assert_eq!(notes.category, "Productivity");
    assert_eq!(notes.size, 2048);
    assert_eq!(notes.sha256, SHA.to_ascii_lowercase(), "the hash is kept in lower case");
    assert_eq!(notes.description, "Write things down", "the description falls back to the summary");
    assert!(notes.is_paid());
    assert_eq!(notes.price_text(), "12.50 EUR");
    assert_eq!(notes.age_rating, "12+");
    assert_eq!(notes.age_text(), "12+");
    assert!(matches!(notes.product_type, Some(ProductType::AutoRenewable)));
    assert_eq!(notes.screenshots, ["a.png", "b.png"], "blank screenshot entries are dropped");
}

#[test]
fn a_free_service_entry_has_the_defaults_the_store_shows() {
    let apps = parse_catalog(&catalog()).unwrap();
    let helper = &apps[1];
    assert!(!helper.is_paid());
    assert_eq!(helper.price_text(), "Free");
    assert_eq!(helper.age_text(), "-", "an unrated entry shows a dash");
    assert!(helper.product_type.is_none(), "no product type given");
    assert_eq!(helper.category, "Utilities", "the default category");
    assert_eq!(helper.min_shell, "0.0.0");
    assert_eq!(helper.description, "", "no summary means no description");
    assert!(helper.changelog.is_empty());
    assert!(helper.is_service());
    assert!(helper.is_preinstalled(), "a service is installed in the background");
    assert!(!helper.is_system());
}

#[test]
fn release_notes_drop_blank_lines_and_entries_without_a_version() {
    let apps = parse_catalog(&catalog()).unwrap();
    let changelog = &apps[0].changelog;
    assert_eq!(changelog.len(), 2);
    assert_eq!(changelog[0].version, "1.2.3");
    assert_eq!(changelog[0].date, "2026-10-01");
    assert_eq!(changelog[0].notes, ["Fixed a crash"]);
    assert_eq!(changelog[1].version, "1.2.2");
    assert_eq!(changelog[1].date, "", "an entry without a date has an empty date");
    assert!(changelog[1].notes.is_empty());
}

#[test]
fn an_absent_changelog_gives_an_empty_list() {
    assert!(parse_changelog(None).is_empty());
    let not_a_list = json::parse(r#""text""#).unwrap();
    assert!(parse_changelog(Some(&not_a_list)).is_empty());
}

#[test]
fn screenshots_are_trimmed_and_capped_at_the_product_page_limit() {
    let many = json::parse(r#"["a", " ", "b", "c", "d", "e", "f", "g", "h"]"#).unwrap();
    let shots = parse_screenshots(Some(&many));
    assert_eq!(shots.len(), MAX_SHOTS);
    assert_eq!(shots, ["a", "b", "c", "d", "e", "f"]);
    assert!(parse_screenshots(None).is_empty());
}

#[test]
fn only_the_listed_age_ratings_are_kept() {
    assert_eq!(AGE_RATINGS, ["4+", "9+", "12+", "16+", "18+"]);
    assert_eq!(age_rating_of("16+"), "16+");
    assert_eq!(age_rating_of(" 9+ "), "9+", "surrounding spaces are ignored");
    assert_eq!(age_rating_of("13+"), "", "an unknown rating is dropped");
    assert_eq!(age_rating_of(""), "");
}

#[test]
fn a_newer_store_version_is_an_update_and_an_equal_or_older_one_is_not() {
    assert!(update_available("1.2.3", "1.10.0"), "1.10.0 is newer than 1.2.3");
    assert!(!update_available("2.0.0", "1.9.9"));
    assert!(!update_available("1.0.0", "1.0.0"));
}

#[test]
fn release_dates_read_as_today_yesterday_days_ago_or_a_full_date() {
    let today = days_from_civil(2026, 10, 14);
    assert_eq!(relative_date("2026-10-14", today), "Today");
    assert_eq!(relative_date("2026-10-13", today), "Yesterday");
    assert_eq!(relative_date("2026-10-11", today), "3d ago");
    assert_eq!(relative_date("2026-09-01", today), "1 Sep 2026");
    assert_eq!(relative_date("not-a-date", today), "not-a-date", "an unreadable date is shown as it is");
    assert_eq!(relative_date("2026-13-01", today), "2026-13-01", "a month out of range is shown as it is");
}
