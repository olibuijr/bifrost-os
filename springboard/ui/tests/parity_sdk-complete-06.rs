//! Parity sdk-complete-06: guided generation (the schema becomes the instruction a model gets, and a model's reply is
//! read back against it) and the audio input picker's rows and choice. Headless: no model is called, no device is
//! opened, and the replies and inputs are fixed data.

use bifrost_ui::generation_schema::{instructions, read_reply, Kind, Property, ReplyError, Value};
use bifrost_ui::input_picker::{Input, Picker};

fn recipe() -> Kind {
    Kind::Object(vec![
        Property::new("title", Kind::Text).described("the dish name"),
        Property::new("minutes", Kind::number_in(1.0, 240.0)),
        Property::new("difficulty", Kind::choice(&["easy", "medium", "hard"])),
    ])
}

fn soup() -> Value {
    Value::Object(vec![
        ("title".to_string(), Value::Text("Soup".to_string())),
        ("minutes".to_string(), Value::Number(25.0)),
        ("difficulty".to_string(), Value::Text("easy".to_string())),
    ])
}

const SOUP_JSON: &str = r#"{"title": "Soup", "minutes": 25, "difficulty": "easy"}"#;

#[test]
fn the_instruction_names_every_member_and_forbids_text_around_the_json() {
    let text = instructions(&recipe());
    assert!(text.contains("title: text (the dish name)"), "a member keeps its description: {text}");
    assert!(text.contains("minutes: number from 1 to 240"), "a range is stated: {text}");
    assert!(text.contains("one of \"easy\", \"medium\", \"hard\""), "the choices are listed: {text}");
    assert!(text.ends_with("Put no text before or after the JSON."), "the reply must be bare JSON: {text}");
}

#[test]
fn a_bare_json_reply_that_fits_is_read_in_member_order() {
    assert_eq!(read_reply(&recipe(), SOUP_JSON), Ok(soup()));
}

#[test]
fn a_reply_in_a_json_code_fence_is_read() {
    let reply = format!("```json\n{SOUP_JSON}\n```");
    assert_eq!(read_reply(&recipe(), &reply), Ok(soup()));
}

#[test]
fn a_reply_in_a_fence_without_a_language_is_read() {
    let reply = format!("```\n{SOUP_JSON}\n```\n");
    assert_eq!(read_reply(&recipe(), &reply), Ok(soup()));
}

#[test]
fn a_fence_that_was_never_closed_is_still_read() {
    let reply = format!("```json\n{SOUP_JSON}");
    assert_eq!(read_reply(&recipe(), &reply), Ok(soup()));
}

#[test]
fn prose_around_the_json_is_refused_as_not_json() {
    let reply = format!("Here is the recipe: {SOUP_JSON}");
    assert!(matches!(read_reply(&recipe(), &reply), Err(ReplyError::NotJson(_))));
}

#[test]
fn text_after_a_closed_fence_is_refused_as_not_json() {
    let reply = format!("```json\n{SOUP_JSON}\n```\nEnjoy the soup.");
    assert!(matches!(read_reply(&recipe(), &reply), Err(ReplyError::NotJson(_))));
}

#[test]
fn json_that_breaks_a_range_is_refused_with_the_issue() {
    let reply = r#"{"title": "Soup", "minutes": 500, "difficulty": "easy"}"#;
    match read_reply(&recipe(), reply) {
        Err(ReplyError::DoesNotFit(issues)) => {
            assert_eq!(issues.len(), 1, "only the minutes member is out of range");
            assert!(issues[0].path.contains("minutes"), "the issue names the member: {:?}", issues[0]);
        }
        other => panic!("expected a fit problem, got {other:?}"),
    }
}

#[test]
fn a_reply_missing_a_required_member_does_not_fit() {
    let reply = r#"{"title": "Soup", "minutes": 25}"#;
    assert!(matches!(read_reply(&recipe(), reply), Err(ReplyError::DoesNotFit(issues)) if !issues.is_empty()));
}

fn microphone(id: &str, name: &str) -> Input {
    Input { id: id.to_string(), name: name.to_string() }
}

fn two_microphones() -> Vec<Input> {
    vec![microphone("builtin", "Built-in Microphone"), microphone("usb", "USB Headset")]
}

#[test]
fn the_rows_check_only_the_chosen_input() {
    let offered = two_microphones();
    let mut picker = Picker::new();
    picker.select(&offered, "usb").unwrap();
    let rows = picker.rows(&offered);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0.name, "Built-in Microphone");
    assert!(!rows[0].1);
    assert_eq!(rows[1].0.name, "USB Headset");
    assert!(rows[1].1);
}

#[test]
fn no_row_is_checked_while_the_system_default_is_in_use() {
    let offered = two_microphones();
    let picker = Picker::new();
    assert!(picker.rows(&offered).iter().all(|(_, checked)| !checked));
}

#[test]
fn using_the_system_default_again_clears_the_choice() {
    let offered = two_microphones();
    let mut picker = Picker::new();
    picker.select(&offered, "builtin").unwrap();
    picker.use_system_default();
    assert_eq!(picker.selected_in(&offered), None);
    assert!(picker.rows(&offered).iter().all(|(_, checked)| !checked));
}

#[test]
fn a_choice_whose_microphone_disappeared_checks_no_row() {
    let offered = two_microphones();
    let mut picker = Picker::new();
    picker.select(&offered, "usb").unwrap();
    let remaining = vec![microphone("builtin", "Built-in Microphone")];
    let rows = picker.rows(&remaining);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].1, "the built-in input is not the chosen one");
}
