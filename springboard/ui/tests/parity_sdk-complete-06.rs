//! Parity sdk-complete-06: guided generation. The schema becomes the instruction a model gets, and a model's reply is
//! read back against it. Headless: no model is called, and the replies are fixed text.

use bifrost_ui::generation_schema::{instructions, read_reply, Kind, Property, ReplyError, Value};

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
