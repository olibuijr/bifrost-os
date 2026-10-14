//! Parity sdk-rest-11: headless tests of the power activities, the now-playing session the app publishes, and the
//! typed script values an app reads back from a web view. No test opens a device, a display or a socket.

use bifrost_app::js_value::JsValue;
use bifrost_app::now_playing::{
    accept, command_from_event, publish_body, Command, CommandState, MediaContent, MediaDuration, MediaSession,
    MediaSessionError, MediaType, RemoteCommand, COMMAND_EVENT, SERVICE,
};
use bifrost_app::process_info::{ActivityOptions, ActivityTracker};
use bifrost_app::webview_dev::EvalValue;
use bifrost_core::json::parse;
use springboard::shell::services::now_playing::parse_session;

// ---- ProcessInfo activities: the idle sleep bit -------------------------------------------------------------------

#[test]
fn an_idle_sleep_activity_reports_its_bit_to_the_shell_when_it_starts_and_stops() {
    let mut tracker = ActivityTracker::default();
    let (token, changed) = tracker.begin(ActivityOptions::IDLE_SYSTEM_SLEEP_DISABLED, "playing a film");
    assert_eq!(changed, Some(8), "idleSystemSleepDisabled is bit 8 on the wire");
    assert_eq!(tracker.count(), 1);
    assert_eq!(tracker.end(token), Some(0), "the last activity ends, so the union is empty");
    assert_eq!(tracker.count(), 0);
}

#[test]
fn a_second_sleep_activity_does_not_change_the_union_the_shell_hears() {
    let mut tracker = ActivityTracker::default();
    let (first, _) = tracker.begin(ActivityOptions::IDLE_SYSTEM_SLEEP_DISABLED, "download");
    let (second, changed) = tracker.begin(ActivityOptions::IDLE_SYSTEM_SLEEP_DISABLED, "transfer");
    assert_eq!(changed, None, "the union is still 8");
    assert_eq!(tracker.end(first), None, "one holder is left, so nothing changed");
    assert_eq!(tracker.end(second), Some(0));
}

#[test]
fn a_user_initiated_activity_and_a_sleep_activity_keep_their_own_bits() {
    let mut tracker = ActivityTracker::default();
    let (user, changed) = tracker.begin(ActivityOptions::USER_INITIATED, "export");
    assert_eq!(changed, Some(1));
    let (sleep, changed) = tracker.begin(ActivityOptions::IDLE_SYSTEM_SLEEP_DISABLED, "meeting");
    assert_eq!(changed, Some(9));
    assert_eq!(tracker.end(user), Some(8), "only the sleep bit remains");
    assert_eq!(tracker.end(sleep), Some(0));
}

#[test]
fn a_background_activity_does_not_keep_the_machine_awake() {
    let mut tracker = ActivityTracker::default();
    let (token, changed) = tracker.begin(ActivityOptions::BACKGROUND, "sync");
    assert_eq!(changed, Some(4), "background is bit 4, which the shell does not treat as a sleep block");
    assert_eq!(tracker.end(token), Some(0));
}

// ---- now playing: the session an app publishes and the commands it accepts ----------------------------------------

fn song_session() -> MediaSession {
    let mut session = MediaSession::new("main").expect("a session");
    let mut content = MediaContent::new("t1", "Song", MediaType::Audio, MediaDuration::Finite(200.0)).expect("content");
    content.artist = Some("Band".into());
    session.set_content(Some(content));
    session.set_command(RemoteCommand::Play, CommandState::Enabled);
    session.set_command(RemoteCommand::Next, CommandState::Disabled);
    session.set_command(RemoteCommand::SeekTo, CommandState::Enabled);
    session
}

#[test]
fn a_declared_enabled_command_is_accepted_and_maps_to_its_session_command() {
    assert_eq!(accept(&song_session(), Command::Play), Ok(RemoteCommand::Play));
    assert_eq!(accept(&song_session(), Command::Seek(42_000.0)), Ok(RemoteCommand::SeekTo));
}

#[test]
fn an_undeclared_command_is_refused_as_unsupported() {
    assert_eq!(accept(&song_session(), Command::Pause), Err(MediaSessionError::CommandUnsupported));
}

#[test]
fn a_declared_but_disabled_command_is_refused_as_disabled() {
    assert_eq!(accept(&song_session(), Command::Next), Err(MediaSessionError::CommandDisabled));
}

#[test]
fn a_session_without_content_publishes_nothing() {
    let empty = MediaSession::new("main").expect("a session");
    assert!(publish_body(&empty).is_none());
}

#[test]
fn a_video_session_is_published_as_video() {
    let mut session = MediaSession::new("film").expect("a session");
    let film = MediaContent::new("v1", "Film", MediaType::Video, MediaDuration::Finite(60.0)).expect("content");
    session.set_content(Some(film));
    let body = publish_body(&session).expect("a body");
    assert_eq!(body.str_field("type").as_deref(), Some("video"));
    assert_eq!(body.get("duration_ms").and_then(|v| v.number()), Some(60_000.0));
}

#[test]
fn a_published_session_reads_back_in_the_shell_with_its_commands() {
    let body = publish_body(&song_session()).expect("a body");
    let shell_side = parse_session("is.example.music", &body).expect("the shell reads the body");
    assert_eq!(shell_side.id(), "main");
    assert_eq!(shell_side.content().map(|c| c.title.as_str()), Some("Song"));
    assert_eq!(shell_side.command_state(RemoteCommand::Play), CommandState::Enabled);
    assert_eq!(shell_side.command_state(RemoteCommand::Next), CommandState::Disabled);
    assert_eq!(shell_side.command_state(RemoteCommand::Pause), CommandState::Unsupported);
}

#[test]
fn a_seek_command_event_needs_its_position() {
    let with_position = parse(r#"{"command":"seek","position_ms":1500}"#).expect("json");
    assert_eq!(command_from_event(SERVICE, COMMAND_EVENT, &with_position), Some(Command::Seek(1500.0)));
    let without = parse(r#"{"command":"seek"}"#).expect("json");
    assert_eq!(command_from_event(SERVICE, COMMAND_EVENT, &without), None);
}

// ---- script values: what an app reads back from a web view and writes into a script ------------------------------

fn eval(ty: &str, json: &str) -> EvalValue {
    EvalValue { ty: ty.into(), json: json.into() }
}

#[test]
fn a_number_result_reads_as_a_typed_number() {
    assert_eq!(JsValue::from_eval(&eval("number", "4")), Ok(JsValue::Number(4.0)));
}

#[test]
fn an_object_result_keeps_its_fields_in_script_order() {
    let value = JsValue::from_eval(&eval("object", r#"{"b":1,"a":"x"}"#)).expect("an object");
    assert_eq!(value.type_name(), "object");
    assert_eq!(value.get("a"), Some(&JsValue::String("x".into())));
    assert_eq!(value.get("b"), Some(&JsValue::Number(1.0)));
}

#[test]
fn a_result_whose_text_disagrees_with_its_type_is_refused() {
    let err = JsValue::from_eval(&eval("array", r#"{"a":1}"#)).unwrap_err();
    assert!(err.contains("reads as object"), "{err}");
}

#[test]
fn a_dom_element_result_has_no_value_form() {
    let err = JsValue::from_eval(&eval("element", "{}")).unwrap_err();
    assert!(err.contains("DOM element"), "{err}");
}

#[test]
fn an_unknown_result_type_is_named_in_the_error() {
    let err = JsValue::from_eval(&eval("symbol", "null")).unwrap_err();
    assert!(err.contains("symbol"), "{err}");
}

#[test]
fn a_nested_value_becomes_a_script_literal_in_field_order() {
    let value = JsValue::Object(vec![
        ("n".into(), JsValue::Number(1.0)),
        ("list".into(), JsValue::Array(vec![JsValue::String("x".into()), JsValue::Null, JsValue::Undefined])),
        ("ok".into(), JsValue::Bool(true)),
    ]);
    assert_eq!(value.to_literal().as_deref(), Ok(r#"{"n":1,"list":["x",null,undefined],"ok":true}"#));
}

#[test]
fn a_value_that_is_not_finite_has_no_script_literal() {
    let err = JsValue::Array(vec![JsValue::Number(f64::NAN)]).to_literal().unwrap_err();
    assert!(err.contains("has no JavaScript literal"), "{err}");
}
