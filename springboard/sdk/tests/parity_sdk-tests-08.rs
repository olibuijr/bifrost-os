//! Parity tests (sdk-tests-08) for the package placement rules and the review prompt policy. Pure host tests: no display,
//! no device and no clock. Each test drives the public API of the module it names.

use bifrost_app::review::{Policy, ReviewRequests};

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
