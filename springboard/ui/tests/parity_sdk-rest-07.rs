//! Parity sdk-rest-07: headless tests for the notification content height registry. No test draws a view.

use bifrost_ui::notification_content::{clamp_height, ContentError, ContentLayout, ContentRegistry, MAX_CATEGORY_LEN, MAX_HEIGHT, MIN_HEIGHT};

#[test]
fn a_height_inside_the_limits_is_kept_as_requested() {
    let mut registry = ContentRegistry::default();
    assert_eq!(registry.register("mail.reply", 120.0), Ok(ContentLayout { height: 120.0 }));
    assert_eq!(registry.layout_for("mail.reply"), Some(ContentLayout { height: 120.0 }));
}

#[test]
fn a_height_below_the_minimum_is_raised_to_the_minimum() {
    assert_eq!(clamp_height(10.0), MIN_HEIGHT);
    assert_eq!(clamp_height(-5.0), MIN_HEIGHT);
}

#[test]
fn a_height_above_the_maximum_is_lowered_to_the_maximum() {
    assert_eq!(clamp_height(900.0), MAX_HEIGHT);
    assert_eq!(MIN_HEIGHT, 44.0);
    assert_eq!(MAX_HEIGHT, 320.0);
}

#[test]
fn a_registered_height_is_clamped_before_it_is_kept() {
    let mut registry = ContentRegistry::default();
    assert_eq!(registry.register("photos.shared", 1000.0), Ok(ContentLayout { height: MAX_HEIGHT }));
    assert_eq!(registry.layout_for("photos.shared").map(|l| l.height), Some(MAX_HEIGHT));
}

#[test]
fn a_non_finite_height_is_refused() {
    let mut registry = ContentRegistry::default();
    assert_eq!(registry.register("mail.reply", f64::NAN), Err(ContentError::HeightNotFinite));
    assert_eq!(registry.register("mail.reply", f64::INFINITY), Err(ContentError::HeightNotFinite));
    assert_eq!(registry.layout_for("mail.reply"), None, "a refused registration keeps nothing");
}

#[test]
fn an_empty_category_is_refused() {
    let mut registry = ContentRegistry::default();
    assert_eq!(registry.register("", 80.0), Err(ContentError::EmptyCategory));
}

#[test]
fn a_category_with_a_space_or_another_symbol_is_refused() {
    let mut registry = ContentRegistry::default();
    assert_eq!(registry.register("mail reply", 80.0), Err(ContentError::BadCategory("mail reply".into())));
    assert_eq!(registry.register("mail/reply", 80.0), Err(ContentError::BadCategory("mail/reply".into())));
}

#[test]
fn a_category_longer_than_the_limit_is_refused() {
    let mut registry = ContentRegistry::default();
    let long = "a".repeat(MAX_CATEGORY_LEN + 1);
    assert_eq!(registry.register(&long, 80.0), Err(ContentError::BadCategory(long.clone())));
    let at_limit = "a".repeat(MAX_CATEGORY_LEN);
    assert!(registry.register(&at_limit, 80.0).is_ok());
}

#[test]
fn a_category_with_no_registration_has_no_layout() {
    let registry = ContentRegistry::default();
    assert_eq!(registry.layout_for("calendar.invite"), None);
}

#[test]
fn registering_a_category_again_replaces_its_height() {
    let mut registry = ContentRegistry::default();
    registry.register("calendar.invite", 80.0).unwrap();
    registry.register("calendar.invite", 200.0).unwrap();
    assert_eq!(registry.layout_for("calendar.invite"), Some(ContentLayout { height: 200.0 }));
}

#[test]
fn unregistering_removes_the_layout() {
    let mut registry = ContentRegistry::default();
    registry.register("calendar.invite", 80.0).unwrap();
    assert_eq!(registry.unregister("calendar.invite"), Ok(()));
    assert_eq!(registry.layout_for("calendar.invite"), None);
}

#[test]
fn unregistering_an_unknown_category_is_an_error_that_names_it() {
    let mut registry = ContentRegistry::default();
    let err = registry.unregister("calendar.invite").unwrap_err();
    assert_eq!(err, ContentError::UnknownCategory("calendar.invite".into()));
    assert_eq!(err.to_string(), "no content view is registered for category calendar.invite");
}

#[test]
fn categories_keep_their_heights_apart() {
    let mut registry = ContentRegistry::default();
    registry.register("mail.reply", 60.0).unwrap();
    registry.register("mail.flag", 150.0).unwrap();
    assert_eq!(registry.layout_for("mail.reply").map(|l| l.height), Some(60.0));
    assert_eq!(registry.layout_for("mail.flag").map(|l| l.height), Some(150.0));
}
