//! Parity os-gaps-03: headless tests for the intent parameter form. No test runs an intent or touches a window.

use bifrost_app::intent_form::{FormError, IntentForm};
use bifrost_app::intents::{Intent, Param, ParamType};
use bifrost_core::json::Json;

fn note_intent() -> Intent {
    Intent::new("add_note", "Add Note", "Adds a note.")
        .param(Param::new("title", "Title", ParamType::String))
        .param(Param::new("priority", "Priority", ParamType::Enum(vec!["low".into(), "high".into()])))
        .param(Param::new("count", "Count", ParamType::Int).optional())
}

#[test]
fn a_new_form_lists_every_declared_field_in_order_and_has_no_values() {
    let form = IntentForm::new(note_intent());
    let names: Vec<&str> = form.fields().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["title", "priority", "count"]);
    assert_eq!(form.value("title"), None);
}

#[test]
fn setting_a_field_with_the_declared_type_stores_the_value() {
    let mut form = IntentForm::new(note_intent());
    assert_eq!(form.set("title", Json::Str("Milk".into())), Ok(()));
    assert_eq!(form.value("title"), Some(&Json::Str("Milk".into())));
}

#[test]
fn a_value_of_the_wrong_type_is_refused_and_names_the_expected_type() {
    let mut form = IntentForm::new(note_intent());
    let error = form.set("count", Json::Str("three".into())).unwrap_err();
    assert_eq!(error, FormError::WrongType { name: "count".into(), expected: "int".into() });
    assert_eq!(form.value("count"), None, "a refused value is not kept");
}

#[test]
fn an_enum_field_accepts_only_its_cases() {
    let mut form = IntentForm::new(note_intent());
    assert!(form.set("priority", Json::Str("high".into())).is_ok());
    assert!(form.set("priority", Json::Str("urgent".into())).is_err());
}

#[test]
fn a_field_name_the_intent_does_not_declare_is_unknown() {
    let mut form = IntentForm::new(note_intent());
    assert_eq!(form.set("colour", Json::Str("red".into())), Err(FormError::UnknownParameter("colour".into())));
    assert_eq!(form.value("colour"), None);
}

#[test]
fn setting_null_clears_a_field() {
    let mut form = IntentForm::new(note_intent());
    form.set("title", Json::Str("Milk".into())).unwrap();
    form.set("title", Json::Null).unwrap();
    assert_eq!(form.value("title"), None);
}

#[test]
fn missing_lists_the_required_fields_that_are_still_empty() {
    let mut form = IntentForm::new(note_intent());
    assert_eq!(form.missing(), vec!["title", "priority"], "the optional count is not listed");
    form.set("title", Json::Str("Milk".into())).unwrap();
    assert_eq!(form.missing(), vec!["priority"]);
}

#[test]
fn arguments_refuse_to_build_while_a_required_field_is_empty() {
    let mut form = IntentForm::new(note_intent());
    form.set("title", Json::Str("Milk".into())).unwrap();
    assert_eq!(form.arguments(), Err(FormError::Missing("priority".into())));
}

#[test]
fn arguments_hold_the_filled_fields_and_pass_the_intent_validation() {
    let intent = note_intent();
    let mut form = IntentForm::new(intent.clone());
    form.set("priority", Json::Str("low".into())).unwrap();
    form.set("title", Json::Str("Milk".into())).unwrap();
    let arguments = form.arguments().expect("both required fields are set");
    assert_eq!(arguments.get("title"), Some(&Json::Str("Milk".into())));
    assert_eq!(arguments.get("count"), None, "an empty optional field is left out");
    assert!(intent.validate(&arguments).is_ok(), "the form never makes arguments the intent refuses");
}

#[test]
fn an_intent_without_parameters_makes_empty_arguments() {
    let form = IntentForm::new(Intent::new("ping", "Ping", "Does nothing."));
    let arguments = form.arguments().expect("nothing is required");
    assert_eq!(arguments, Json::Obj(Vec::new()));
}
