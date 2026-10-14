//! Parity tests (sdk-tests-08) for the package placement rules and the review prompt policy. Pure host tests: no display,
//! no device and no clock. Each test drives the public API of the module it names.

use bifrost_app::package_layout::{check_package, classify, Placement, Refusal, MANIFEST};
use bifrost_app::review::{Policy, ReviewRequests};

#[test]
fn package_layout_manifest_name_is_fixed() {
    assert_eq!(MANIFEST, "manifest.toml");
}

#[test]
fn package_layout_root_files_are_placed_at_the_root() {
    assert_eq!(classify("manifest.toml"), Ok(Placement::Root));
    assert_eq!(classify("notes"), Ok(Placement::Root));
}

#[test]
fn package_layout_libraries_are_flat_under_libs() {
    assert_eq!(classify("libs/core.so"), Ok(Placement::Library("core".to_string())));
    assert_eq!(classify("libs/sub/core.so"), Err(Refusal::Unplaced("libs/sub/core.so".to_string())));
    assert_eq!(classify("libs/core.dylib"), Err(Refusal::Unplaced("libs/core.dylib".to_string())));
    assert_eq!(classify("libs/.so"), Err(Refusal::Unplaced("libs/.so".to_string())));
}

#[test]
fn package_layout_plugins_are_flat_under_plugins() {
    assert_eq!(classify("plugins/codec.so"), Ok(Placement::Plugin("codec".to_string())));
    assert_eq!(classify("plugins/a/codec.so"), Err(Refusal::Unplaced("plugins/a/codec.so".to_string())));
}

#[test]
fn package_layout_resources_keep_their_path_below_assets() {
    assert_eq!(classify("assets/icons/star.png"), Ok(Placement::Resource("icons/star.png".to_string())));
    assert_eq!(classify("assets/readme.txt"), Ok(Placement::Resource("readme.txt".to_string())));
}

#[test]
fn package_layout_refuses_paths_outside_the_layout() {
    assert_eq!(classify("frameworks/x.so"), Err(Refusal::Unplaced("frameworks/x.so".to_string())));
    assert_eq!(classify("data/x.bin"), Err(Refusal::Unplaced("data/x.bin".to_string())));
}

#[test]
fn package_layout_refuses_unsafe_paths() {
    assert_eq!(classify(""), Err(Refusal::Unsafe(String::new())));
    assert_eq!(classify("/etc/passwd"), Err(Refusal::Unsafe("/etc/passwd".to_string())));
    assert_eq!(classify("../evil"), Err(Refusal::Unsafe("../evil".to_string())));
    assert_eq!(classify("assets/../../x"), Err(Refusal::Unsafe("assets/../../x".to_string())));
    assert_eq!(classify("assets//x"), Err(Refusal::Unsafe("assets//x".to_string())));
    assert_eq!(classify("assets/./x"), Err(Refusal::Unsafe("assets/./x".to_string())));
    assert_eq!(classify("assets\\x"), Err(Refusal::Unsafe("assets\\x".to_string())));
}

#[test]
fn package_check_accepts_a_placed_package_with_a_manifest() {
    let entries = ["manifest.toml", "app", "icon.png", "assets/a.txt", "libs/x.so", "plugins/p.so"];
    assert_eq!(check_package(&entries), Ok(()));
}

#[test]
fn package_check_requires_the_manifest_at_the_root() {
    assert_eq!(check_package(&["app", "icon.png"]), Err(Refusal::NoManifest));
    assert_eq!(check_package(&["manifest.toml/", "app"]), Err(Refusal::Unsafe("manifest.toml/".to_string())));
}

#[test]
fn package_check_returns_the_first_unplaced_entry() {
    assert_eq!(
        check_package(&["manifest.toml", "data/x.bin", "../bad"]),
        Err(Refusal::Unplaced("data/x.bin".to_string()))
    );
}

#[test]
fn review_first_ask_is_allowed_and_recorded() {
    let mut r = ReviewRequests::new();
    assert!(r.request(100, Policy::default()));
    assert_eq!(r.days(), &[100]);
}

#[test]
fn review_minimum_gap_blocks_an_early_ask() {
    let mut r = ReviewRequests::new();
    let p = Policy::default();
    assert!(r.request(100, p));
    assert!(!r.request(150, p), "50 days is less than the 120-day gap");
    assert!(!r.request(100, p), "the same day is never a second ask");
    assert!(r.request(220, p));
    assert_eq!(r.days(), &[100, 220]);
}

#[test]
fn review_max_asks_caps_the_asks_inside_the_window() {
    let mut r = ReviewRequests::new();
    let p = Policy { min_gap: 0, max_asks: 2, window: 30 };
    assert!(r.request(0, p));
    assert!(r.request(1, p));
    assert!(!r.request(2, p), "two asks already fall in the window");
    assert!(r.request(30, p), "the ask on day 0 has left the 30-day window");
}

#[test]
fn review_restored_days_are_sorted_and_checked() {
    let r = ReviewRequests::from_days(vec![300, 100]);
    assert_eq!(r.days(), &[100, 300]);
    let p = Policy::default();
    assert!(!r.allowed(350, p), "only 50 days after the last ask");
    assert!(r.allowed(430, p), "130 days after the last ask, and two asks fall inside the year");
}

#[test]
fn review_zero_max_asks_never_asks() {
    let mut r = ReviewRequests::new();
    assert!(!r.request(5, Policy { min_gap: 0, max_asks: 0, window: 365 }));
    assert!(r.days().is_empty());
}
