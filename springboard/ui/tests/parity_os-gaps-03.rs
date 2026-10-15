//! Parity os-gaps-03: headless tests for the ambient light model, the account restriction policy and the route
//! suggestions. No test reads a sensor, a clock or a route.

use bifrost_ui::account_policy::{Denial, Feature, Policy, PolicyError, PolicyTable};
use bifrost_ui::ambient_light::{brightness_for_lux, lux_from_raw, ReadingError, Smoother, BRIGHT_LUX, DARK_LUX, MIN_BRIGHTNESS};
use bifrost_ui::route_picker::Destination;
use bifrost_ui::route_prediction::suggest;

fn destination(id: &str) -> Destination {
    Destination { id: id.into(), name: id.into(), local: true, active: false }
}

#[test]
fn a_raw_count_with_no_scale_or_offset_is_already_lux() {
    assert_eq!(lux_from_raw("250", "", ""), Ok(250.0));
}

#[test]
fn the_scale_and_offset_follow_the_iio_rule() {
    // (raw + offset) * scale
    assert_eq!(lux_from_raw("100", "20", "0.5"), Ok(60.0));
    assert_eq!(lux_from_raw(" 40 ", " 0 ", " 2.5 "), Ok(100.0), "surrounding spaces are ignored");
}

#[test]
fn a_negative_result_is_clamped_to_zero_lux() {
    assert_eq!(lux_from_raw("-30", "0", "1"), Ok(0.0));
}

#[test]
fn a_reading_that_is_not_a_number_is_malformed() {
    assert_eq!(lux_from_raw("bright", "", ""), Err(ReadingError::Malformed));
    assert_eq!(lux_from_raw("", "", ""), Err(ReadingError::Malformed), "the raw count is required");
}

#[test]
fn a_zero_or_negative_scale_is_out_of_range() {
    assert_eq!(lux_from_raw("10", "0", "0"), Err(ReadingError::OutOfRange));
    assert_eq!(lux_from_raw("10", "0", "-1"), Err(ReadingError::OutOfRange));
}

#[test]
fn a_non_finite_value_is_out_of_range() {
    assert_eq!(lux_from_raw("inf", "0", "1"), Err(ReadingError::OutOfRange));
}

#[test]
fn a_lux_value_that_cannot_fit_in_the_public_float_type_is_out_of_range() {
    assert_eq!(lux_from_raw("1e300", "0", "1"), Err(ReadingError::OutOfRange));
}

#[test]
fn the_smoother_takes_the_first_reading_as_it_is() {
    let mut smoother = Smoother::new(0.5).expect("0.5 is a valid weight");
    assert_eq!(smoother.value(), None);
    assert_eq!(smoother.push(100.0), 100.0);
}

#[test]
fn the_smoother_moves_by_its_weight_towards_each_new_reading() {
    let mut smoother = Smoother::new(0.25).unwrap();
    smoother.push(100.0);
    assert_eq!(smoother.push(200.0), 125.0);
    assert_eq!(smoother.value(), Some(125.0));
}

#[test]
fn invalid_smoother_readings_do_not_poison_the_next_valid_value() {
    let mut smoother = Smoother::new(0.5).unwrap();
    assert_eq!(smoother.push(f32::NAN), DARK_LUX);
    assert_eq!(smoother.value(), None);
    assert_eq!(smoother.push(100.0), 100.0);
    assert_eq!(smoother.push(f32::INFINITY), 100.0);
    assert_eq!(smoother.push(-1.0), 100.0);
    assert_eq!(smoother.value(), Some(100.0));
}

#[test]
fn a_smoother_weight_must_be_in_zero_to_one() {
    assert!(Smoother::new(0.0).is_none());
    assert!(Smoother::new(1.5).is_none());
    assert!(Smoother::new(f32::NAN).is_none());
    assert!(Smoother::new(1.0).is_some());
}

#[test]
fn a_dark_room_gets_the_minimum_brightness() {
    assert_eq!(brightness_for_lux(0.0), MIN_BRIGHTNESS);
    assert_eq!(brightness_for_lux(DARK_LUX), MIN_BRIGHTNESS);
    assert_eq!(brightness_for_lux(f32::NAN), MIN_BRIGHTNESS);
}

#[test]
fn full_daylight_gets_full_brightness() {
    assert!((brightness_for_lux(BRIGHT_LUX) - 1.0).abs() < 1e-6);
    assert!((brightness_for_lux(1_000_000.0) - 1.0).abs() < 1e-6);
}

#[test]
fn brightness_rises_with_the_logarithm_of_the_light_level() {
    // Each tenfold step of light adds the same brightness: 1 to 100 lux and 100 to 10000 lux add the same amount.
    let low = brightness_for_lux(100.0) - brightness_for_lux(1.0);
    let high = brightness_for_lux(10_000.0) - brightness_for_lux(100.0);
    assert!((low - high).abs() < 1e-5);
    assert!(brightness_for_lux(500.0) > brightness_for_lux(50.0));
}

#[test]
fn an_account_with_no_policy_may_open_any_app_and_use_any_feature() {
    let table = PolicyTable::new();
    assert_eq!(table.check_app("kid", "com.example.game"), Ok(()));
    assert_eq!(table.check_feature("kid", Feature::Camera), Ok(()));
}

#[test]
fn a_blocked_app_is_refused_for_that_account_only() {
    let mut policy = Policy::unrestricted();
    policy.block_app("video").unwrap();
    let mut table = PolicyTable::new();
    table.set("kid", policy).unwrap();
    assert_eq!(table.check_app("kid", "video"), Err(Denial::AppBlocked("video".into())));
    assert_eq!(table.check_app("parent", "video"), Ok(()));
}

#[test]
fn unblocking_an_app_lifts_the_refusal() {
    let mut policy = Policy::unrestricted();
    policy.block_app("video").unwrap();
    assert!(policy.unblock_app("video"));
    assert!(!policy.unblock_app("video"), "a second unblock reports nothing was blocked");
    assert_eq!(policy.check_app("video"), Ok(()));
}

#[test]
fn a_disabled_feature_is_refused_until_it_is_enabled_again() {
    let mut policy = Policy::unrestricted();
    policy.disable(Feature::Purchases);
    assert_eq!(policy.check_feature(Feature::Purchases), Err(Denial::FeatureDisabled(Feature::Purchases)));
    assert_eq!(policy.check_feature(Feature::Location), Ok(()));
    policy.enable(Feature::Purchases);
    assert_eq!(policy.check_feature(Feature::Purchases), Ok(()));
}

#[test]
fn an_empty_app_or_account_identifier_is_refused() {
    let mut policy = Policy::unrestricted();
    assert_eq!(policy.block_app("   "), Err(PolicyError::EmptyId));
    let mut table = PolicyTable::new();
    assert_eq!(table.set("", Policy::unrestricted()), Err(PolicyError::EmptyId));
    assert_eq!(table.check_app("", "video"), Err(Denial::InvalidId));
    assert_eq!(table.check_app("kid", "  "), Err(Denial::InvalidId));
}

#[test]
fn account_and_app_identifiers_are_trimmed_consistently() {
    let mut policy = Policy::unrestricted();
    policy.block_app(" video ").unwrap();
    let mut table = PolicyTable::new();
    table.set(" kid ", policy).unwrap();
    assert_eq!(table.check_app("kid", "video"), Err(Denial::AppBlocked("video".into())));
    assert!(table.remove(" kid ").is_some());
}

#[test]
fn removing_a_policy_makes_the_account_unrestricted_again() {
    let mut policy = Policy::unrestricted();
    policy.disable(Feature::Microphone);
    let mut table = PolicyTable::new();
    table.set("kid", policy.clone()).unwrap();
    assert_eq!(table.remove("kid"), Some(policy));
    assert_eq!(table.check_feature("kid", Feature::Microphone), Ok(()));
}

#[test]
fn suggestions_are_the_used_destinations_most_used_first() {
    let destinations = vec![destination("speakers"), destination("headset"), destination("tv")];
    let history = ["speakers", "tv", "headset", "tv"];
    let ids: Vec<&str> = suggest(&destinations, &history, 5).iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["tv", "headset", "speakers"]);
}

#[test]
fn equally_used_destinations_rank_the_most_recent_first() {
    let destinations = vec![destination("speakers"), destination("headset")];
    let history = ["headset", "speakers"];
    let ids: Vec<&str> = suggest(&destinations, &history, 2).iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["speakers", "headset"]);
}

#[test]
fn a_destination_never_chosen_is_not_suggested_and_the_limit_is_kept() {
    let destinations = vec![destination("speakers"), destination("headset"), destination("tv")];
    let history = ["tv", "speakers", "tv"];
    let ids: Vec<&str> = suggest(&destinations, &history, 1).iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["tv"]);
    assert!(suggest(&destinations, &[], 3).is_empty());
}
