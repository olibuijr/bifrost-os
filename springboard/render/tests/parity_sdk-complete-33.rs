//! Parity sdk-complete-33: headless tests for the saver fade timeline and the filter input checks. No test renders pixels.

use bifrost_render::filter_input::{check_input, resolve_inputs, InputError};
use bifrost_render::gamma_fade::FadeCycle;

#[test]
fn a_cycle_is_dark_before_it_starts() {
    let cycle = FadeCycle::new(0.5, true);
    assert_eq!(cycle.level(10.0), 0.0);
}

#[test]
fn a_start_fades_in_over_the_fade_length() {
    let mut cycle = FadeCycle::new(0.5, true);
    cycle.start(1.0);
    assert_eq!(cycle.level(1.25), 0.5);
    assert_eq!(cycle.level(1.5), 1.0);
    assert_eq!(cycle.level(3.0), 1.0, "a running saver stays at full brightness");
}

#[test]
fn a_stop_fades_out_from_full_brightness() {
    let mut cycle = FadeCycle::new(0.5, true);
    cycle.start(0.0);
    cycle.stop(2.0);
    assert_eq!(cycle.level(2.0), 1.0);
    assert_eq!(cycle.level(2.125), 0.75);
    assert_eq!(cycle.level(2.5), 0.0);
    assert_eq!(cycle.level(9.0), 0.0);
}

#[test]
fn a_start_after_a_stop_fades_in_again() {
    let mut cycle = FadeCycle::new(0.5, true);
    cycle.start(0.0);
    cycle.stop(1.0);
    cycle.start(5.0);
    assert_eq!(cycle.level(5.0), 0.0);
    assert_eq!(cycle.level(5.25), 0.5);
}

#[test]
fn a_stop_before_any_start_does_nothing() {
    let mut cycle = FadeCycle::new(0.5, true);
    cycle.stop(1.0);
    assert_eq!(cycle.level(2.0), 0.0);
}

#[test]
fn a_disabled_cycle_is_always_fully_bright() {
    let mut cycle = FadeCycle::new(0.5, false);
    assert!(!cycle.enabled());
    assert_eq!(cycle.level(0.0), 1.0);
    cycle.start(0.0);
    cycle.stop(1.0);
    assert_eq!(cycle.level(1.1), 1.0);
}

#[test]
fn a_fade_with_no_length_jumps_to_full_brightness() {
    let mut cycle = FadeCycle::new(0.0, true);
    cycle.start(1.0);
    assert_eq!(cycle.level(1.0), 1.0);
}

#[test]
fn a_value_inside_the_attribute_range_is_accepted() {
    assert_eq!(check_input("CIGaussianBlur", "inputRadius", 5.0), Ok(5.0));
}

#[test]
fn a_value_outside_the_attribute_range_is_refused() {
    assert_eq!(
        check_input("CIGaussianBlur", "inputRadius", 150.0),
        Err(InputError::OutOfRange { min: 0.0, max: 100.0 })
    );
}

#[test]
fn an_unknown_filter_or_key_is_refused() {
    assert_eq!(
        check_input("CINoSuchFilter", "inputRadius", 1.0),
        Err(InputError::UnknownFilter("CINoSuchFilter".to_string()))
    );
    assert_eq!(
        check_input("CIGaussianBlur", "inputAngle", 1.0),
        Err(InputError::UnknownKey { filter: "CIGaussianBlur".to_string(), key: "inputAngle".to_string() })
    );
}

#[test]
fn a_non_finite_value_is_refused() {
    assert_eq!(check_input("CIGaussianBlur", "inputRadius", f32::NAN), Err(InputError::NotFinite));
    assert_eq!(check_input("CIGaussianBlur", "inputRadius", f32::INFINITY), Err(InputError::NotFinite));
}

#[test]
fn resolve_inputs_fills_the_defaults_for_missing_keys() {
    assert_eq!(
        resolve_inputs("CIUnsharpMask", &[("inputIntensity", 0.25)]),
        Ok(vec![("inputRadius", 2.5), ("inputIntensity", 0.25)])
    );
}

#[test]
fn resolve_inputs_refuses_a_bad_value_anywhere_in_the_list() {
    assert_eq!(
        resolve_inputs("CIUnsharpMask", &[("inputRadius", 500.0)]),
        Err(InputError::OutOfRange { min: 0.0, max: 100.0 })
    );
}

#[test]
fn a_filter_without_inputs_resolves_to_an_empty_list() {
    assert_eq!(resolve_inputs("CIAffineTile", &[]), Ok(Vec::new()));
}
