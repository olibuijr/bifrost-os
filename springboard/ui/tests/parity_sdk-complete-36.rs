//! Parity sdk-complete-36: headless tests for the default calling app settings and routing, and for the audio
//! parameter table of a render unit. Nothing here starts a call or touches an audio device.

use bifrost_ui::audio_parameters::{ParameterError, ParameterTable};
use bifrost_ui::call_apps::{AppError, CallRoute, CallingApps};

fn two_apps() -> CallingApps {
    let mut apps = CallingApps::new();
    apps.register("phone").unwrap();
    apps.register("voip").unwrap();
    apps
}

#[test]
fn a_call_with_no_default_app_goes_to_the_system() {
    let apps = two_apps();
    assert_eq!(apps.route(), CallRoute::System);
}

#[test]
fn a_call_goes_to_the_chosen_default_app() {
    let mut apps = two_apps();
    apps.set_default("voip").unwrap();
    assert_eq!(apps.route(), CallRoute::App("voip"));
    apps.clear_default();
    assert_eq!(apps.route(), CallRoute::System);
}

#[test]
fn removing_the_default_app_routes_calls_to_the_system_again() {
    let mut apps = two_apps();
    apps.set_default("phone").unwrap();
    assert!(apps.unregister("phone"));
    assert_eq!(apps.route(), CallRoute::System);
}

#[test]
fn the_settings_value_is_the_default_app_id_or_empty() {
    let mut apps = two_apps();
    assert_eq!(apps.settings_value(), "");
    apps.set_default("phone").unwrap();
    assert_eq!(apps.settings_value(), "phone");
}

#[test]
fn a_saved_default_is_restored_when_the_app_is_registered() {
    let mut saved = two_apps();
    saved.set_default("voip").unwrap();
    let stored = saved.settings_value().to_string();

    let mut restored = two_apps();
    restored.load_settings_value(&stored).unwrap();
    assert_eq!(restored.default_app(), Some("voip"));
    assert_eq!(restored.route(), CallRoute::App("voip"));
}

#[test]
fn an_empty_settings_value_clears_the_default() {
    let mut apps = two_apps();
    apps.set_default("voip").unwrap();
    apps.load_settings_value("   ").unwrap();
    assert_eq!(apps.default_app(), None);
}

#[test]
fn a_settings_value_for_an_unregistered_app_is_refused_and_keeps_the_default() {
    let mut apps = two_apps();
    apps.set_default("phone").unwrap();
    assert_eq!(apps.load_settings_value("video"), Err(AppError::NotRegistered("video".into())));
    assert_eq!(apps.default_app(), Some("phone"));
}

#[test]
fn a_new_parameter_starts_at_its_default() {
    let mut table = ParameterTable::new();
    table.define(1, "volume", 0.0, 1.0, 0.5).unwrap();
    assert_eq!(table.value(1), Ok(0.5));
    assert_eq!(table.get(1).map(|p| p.name.as_str()), Some("volume"));
}

#[test]
fn a_write_is_clamped_into_the_parameter_range() {
    let mut table = ParameterTable::new();
    table.define(1, "volume", 0.0, 1.0, 0.5).unwrap();
    assert_eq!(table.set(1, 3.0), Ok(1.0));
    assert_eq!(table.value(1), Ok(1.0));
    assert_eq!(table.set(1, -2.0), Ok(0.0));
    assert_eq!(table.set(1, 0.25), Ok(0.25));
}

#[test]
fn a_non_finite_write_is_refused_and_keeps_the_value() {
    let mut table = ParameterTable::new();
    table.define(1, "cutoff", 20.0, 20_000.0, 1_000.0).unwrap();
    assert_eq!(table.set(1, f32::NAN), Err(ParameterError::InvalidValue));
    assert_eq!(table.set(1, f32::INFINITY), Err(ParameterError::InvalidValue));
    assert_eq!(table.value(1), Ok(1_000.0));
}

#[test]
fn reset_returns_a_parameter_to_its_default() {
    let mut table = ParameterTable::new();
    table.define(2, "mix", 0.0, 100.0, 30.0).unwrap();
    table.set(2, 80.0).unwrap();
    table.reset(2).unwrap();
    assert_eq!(table.value(2), Ok(30.0));
}

#[test]
fn an_unknown_parameter_is_refused_for_every_operation() {
    let mut table = ParameterTable::new();
    assert_eq!(table.value(9), Err(ParameterError::Unknown(9)));
    assert_eq!(table.set(9, 1.0), Err(ParameterError::Unknown(9)));
    assert_eq!(table.reset(9), Err(ParameterError::Unknown(9)));
    assert!(table.get(9).is_none());
}

#[test]
fn a_parameter_id_can_be_defined_only_once() {
    let mut table = ParameterTable::new();
    table.define(1, "volume", 0.0, 1.0, 0.5).unwrap();
    assert_eq!(table.define(1, "other", 0.0, 1.0, 0.5), Err(ParameterError::Duplicate(1)));
    assert_eq!(table.parameters().len(), 1);
}

#[test]
fn a_range_must_be_finite_ordered_and_hold_the_default() {
    let mut table = ParameterTable::new();
    assert_eq!(table.define(1, "backwards", 1.0, 0.0, 0.5), Err(ParameterError::InvalidRange));
    assert_eq!(table.define(2, "nan", f32::NAN, 1.0, 0.5), Err(ParameterError::InvalidRange));
    assert_eq!(table.define(3, "outside", 0.0, 1.0, 2.0), Err(ParameterError::InvalidRange));
    assert!(table.parameters().is_empty());
}

#[test]
fn parameters_keep_their_definition_order() {
    let mut table = ParameterTable::new();
    table.define(7, "gain", 0.0, 2.0, 1.0).unwrap();
    table.define(3, "pan", -1.0, 1.0, 0.0).unwrap();
    let ids: Vec<u32> = table.parameters().iter().map(|p| p.id).collect();
    assert_eq!(ids, vec![7, 3]);
}
