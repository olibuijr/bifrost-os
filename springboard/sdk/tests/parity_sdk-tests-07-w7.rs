//! Parity sdk-tests-07 (wave 7): headless tests for app intents and the app image files. Intent declarations are checked
//! as the JSON the shell reads, parameters are validated against their declarations, and results are read back from
//! their JSON. The image tests encode pixels into memory and decode them again. No display, no device and no clock is
//! involved.

use bifrost_app::graphics_imageio;
use bifrost_app::intents::{
    Authentication, Entity, Intent, IntentMode, IntentResult, Param, ParamType, Session, Snippet,
};
use bifrost_core::json::Json;
use bifrost_render::cg::imageio::Format;
use bifrost_render::gfx::pack;

fn text(s: &str) -> Json {
    Json::Str(s.into())
}

// ---- intents: declarations, the shortcut flag and the JSON the shell reads ----

#[test]
fn headless_a_new_intent_runs_in_the_background_only_and_is_not_a_shortcut() {
    let intent = Intent::new("open_note", "Open Note", "Opens a note");
    assert!(!intent.shortcut);
    assert_eq!(intent.modes, vec![IntentMode::Background]);
    assert!(intent.supports(IntentMode::Background));
    assert!(!intent.supports(IntentMode::Foreground));
    assert_eq!(intent.authentication, Authentication::AlwaysAllowed);
}

#[test]
fn headless_the_shortcut_flag_is_set_by_the_shortcut_builder_and_written_to_json() {
    let plain = Intent::new("open_note", "Open Note", "Opens a note");
    assert_eq!(plain.to_json().get("shortcut"), Some(&Json::Bool(false)));
    let shortcut = plain.shortcut();
    assert!(shortcut.shortcut);
    assert_eq!(shortcut.to_json().get("shortcut"), Some(&Json::Bool(true)));
}

#[test]
fn headless_foreground_adds_the_mode_once_and_the_method_name_follows_the_id() {
    let intent = Intent::new("open_note", "Open Note", "")
        .foreground()
        .foreground();
    assert_eq!(
        intent.modes,
        vec![IntentMode::Background, IntentMode::Foreground]
    );
    assert!(intent.supports(IntentMode::Foreground));
    assert_eq!(intent.method(), "intent.open_note");
    assert_eq!(
        intent.to_json().get("modes"),
        Some(&Json::Arr(vec![text("background"), text("foreground")]))
    );
}

#[test]
fn headless_the_intent_json_lists_each_parameter_with_its_type_and_optionality() {
    let intent = Intent::new("find", "Find", "Finds notes")
        .param(Param::new("query", "Query", ParamType::String))
        .param(Param::new("limit", "Limit", ParamType::Int).optional());
    let json = intent.to_json();
    let Some(Json::Arr(params)) = json.get("parameters") else {
        panic!("parameters is an array")
    };
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].get("name"), Some(&text("query")));
    assert_eq!(params[0].get("type"), Some(&text("string")));
    assert_eq!(params[0].get("optional"), Some(&Json::Bool(false)));
    assert_eq!(params[1].get("type"), Some(&text("int")));
    assert_eq!(params[1].get("optional"), Some(&Json::Bool(true)));
}

#[test]
fn headless_the_manifest_table_carries_the_shortcut_and_the_typed_parameters() {
    let intent = Intent::new("greet", "Say \"hi\"", "Greets")
        .shortcut()
        .param(Param::new("name", "Name", ParamType::String).optional());
    let toml = intent.manifest_toml();
    assert!(toml.starts_with("[[intent]]\n"));
    assert!(toml.contains("id = \"greet\""));
    assert!(
        toml.contains("title = \"Say 'hi'\""),
        "double quotes in a title are replaced so the TOML stays valid"
    );
    assert!(toml.contains("shortcut = true"));
    assert!(toml.contains("params = [\"name:string?\"]"));
}

#[test]
fn headless_parameter_type_names_round_trip_through_their_text_form() {
    assert_eq!(ParamType::parse("int"), Some(ParamType::Int));
    assert_eq!(ParamType::Int.name(), "int");
    assert_eq!(ParamType::Entity("Note".into()).name(), "entity(Note)");
    assert_eq!(
        ParamType::Enum(vec!["open".into(), "closed".into()]).name(),
        "enum(open|closed)"
    );
    assert!(ParamType::String.accepts(&text("hello")));
    assert!(!ParamType::String.accepts(&Json::Bool(true)));
}

// ---- intents: validating parameters and checking results against the declarations ----

fn note_intent() -> Intent {
    Intent::new("open_note", "Open Note", "Opens a note")
        .param(Param::new("name", "Name", ParamType::String))
        .param(Param::new("tag", "Tag", ParamType::String).optional())
}

#[test]
fn headless_a_call_with_every_required_parameter_is_valid() {
    let call = Json::Obj(vec![("name".into(), text("Groceries"))]);
    assert!(note_intent().validate(&call).is_ok());
}

#[test]
fn headless_a_call_missing_a_required_parameter_names_it_in_the_error() {
    let err = note_intent()
        .validate(&Json::Obj(vec![]))
        .expect_err("name is required");
    assert!(
        err.message.contains("missing parameter name (string)"),
        "got {}",
        err.message
    );
    let null_value = Json::Obj(vec![("name".into(), Json::Null)]);
    assert!(
        note_intent().validate(&null_value).is_err(),
        "a null value counts as missing"
    );
}

#[test]
fn headless_a_call_with_a_parameter_of_the_wrong_type_or_an_unknown_name_is_refused() {
    let wrong = Json::Obj(vec![("name".into(), Json::Bool(true))]);
    let err = note_intent()
        .validate(&wrong)
        .expect_err("name must be a string");
    assert!(err.message.contains("parameter name must be string"));
    let unknown = Json::Obj(vec![
        ("name".into(), text("a")),
        ("colour".into(), text("red")),
    ]);
    let err = note_intent()
        .validate(&unknown)
        .expect_err("colour is not declared");
    assert!(err.message.contains("unknown parameter colour"));
}

#[test]
fn headless_parameters_must_be_an_object_or_absent() {
    let err = note_intent()
        .validate(&text("Groceries"))
        .expect_err("a bare string is not a parameter object");
    assert!(err.message.contains("params must be an object"));
}

#[test]
fn headless_a_continue_in_foreground_result_needs_the_foreground_mode() {
    let declared = vec![note_intent()];
    let result = IntentResult::dialog("Opened").continue_in_foreground();
    assert!(note_intent().check_result(&result, &declared).is_err());
    let foreground = note_intent().foreground();
    assert!(foreground.check_result(&result, &declared).is_ok());
}

#[test]
fn headless_a_result_that_opens_an_undeclared_intent_is_refused() {
    let declared = vec![note_intent()];
    let result = IntentResult::value(Json::Null).opens("delete_note", Json::Null);
    let err = note_intent()
        .check_result(&result, &declared)
        .expect_err("delete_note is not declared");
    assert!(err
        .message
        .contains("opens an undeclared intent delete_note"));
}

#[test]
fn headless_a_result_that_opens_a_declared_intent_with_valid_parameters_is_accepted() {
    let declared = vec![note_intent()];
    let call = Json::Obj(vec![("name".into(), text("Groceries"))]);
    let result = IntentResult::value(Json::Null).opens("open_note", call);
    assert!(note_intent().check_result(&result, &declared).is_ok());
    let bad = IntentResult::value(Json::Null).opens("open_note", Json::Obj(vec![]));
    assert!(
        note_intent().check_result(&bad, &declared).is_err(),
        "the follow-on call still needs its name"
    );
}

// ---- intents: authentication and results ----

#[test]
fn headless_each_authentication_policy_checks_the_device_state_it_names() {
    let locked = Session {
        locked: true,
        authenticated: false,
    };
    let unlocked = Session {
        locked: false,
        authenticated: false,
    };
    let signed_in = Session {
        locked: false,
        authenticated: true,
    };
    assert!(Authentication::AlwaysAllowed.allows(locked));
    assert!(!Authentication::RequiresLocalDeviceAuthentication.allows(locked));
    assert!(Authentication::RequiresLocalDeviceAuthentication.allows(unlocked));
    assert!(!Authentication::RequiresAuthentication.allows(unlocked));
    assert!(Authentication::RequiresAuthentication.allows(signed_in));
    let guarded = note_intent().authentication(Authentication::RequiresAuthentication);
    assert!(!guarded.may_run(unlocked));
    assert!(guarded.may_run(signed_in));
}

#[test]
fn headless_a_result_round_trips_through_its_json_reply() {
    let result = IntentResult::value(text("saved"))
        .with_dialog("Saved")
        .continue_in_foreground();
    assert_eq!(IntentResult::from_json(&result.to_json()), Some(result));
    let plain = IntentResult::dialog("Done");
    assert_eq!(IntentResult::from_json(&plain.to_json()), Some(plain));
}

#[test]
fn headless_a_result_without_a_dialog_or_a_follow_on_leaves_those_keys_out_of_its_json() {
    let json = IntentResult::value(Json::Null).to_json();
    assert_eq!(json.get("dialog"), Some(&Json::Null));
    assert!(json.get("continueInForeground").is_none());
    assert!(json.get("opens").is_none());
    assert!(json.get("snippet").is_none());
}

#[test]
fn headless_a_snippet_is_written_into_the_result_json() {
    let result =
        IntentResult::value(Json::Null).with_snippet(Snippet::new("Summary").row("Items", "3"));
    assert!(result.to_json().get("snippet").is_some());
}

#[test]
fn headless_a_reply_with_a_non_text_dialog_is_not_a_result() {
    assert!(IntentResult::from_json(&text("not a reply")).is_none());
    let bad = Json::Obj(vec![
        ("value".into(), Json::Null),
        ("dialog".into(), Json::Bool(true)),
    ]);
    assert!(IntentResult::from_json(&bad).is_none());
}

#[test]
fn headless_an_entity_names_its_type_and_shows_its_subtitle_when_it_has_one() {
    let note = Entity::new("Note", "n1", "Groceries").subtitle("Today");
    let json = note.to_json();
    assert_eq!(json.get("type"), Some(&text("Note")));
    assert_eq!(json.get("subtitle"), Some(&text("Today")));
    assert!(Entity::new("Note", "n2", "Plans")
        .to_json()
        .get("subtitle")
        .is_none());
}

// ---- app image files: detection, pixel size, decoding and encoding ----

#[test]
fn headless_an_app_detects_the_picture_format_from_its_bytes() {
    let png = graphics_imageio::encode(Format::Png, 1, 1, &[1, 2, 3, 255]).expect("a PNG encodes");
    assert_eq!(graphics_imageio::detect(&png), Some(Format::Png));
    assert_eq!(graphics_imageio::detect(&[]), None);
    assert_eq!(graphics_imageio::pixel_size(&png), Some((1, 1)));
}

#[test]
fn headless_an_app_decodes_what_it_encoded_as_premultiplied_pixels() {
    let rgba: [u8; 8] = [10, 20, 30, 255, 200, 100, 50, 255];
    let png = graphics_imageio::encode(Format::Png, 2, 1, &rgba).expect("encodes");
    let picture = graphics_imageio::decode(&png).expect("decodes");
    assert_eq!((picture.w, picture.h), (2, 1));
    assert_eq!(
        picture.px,
        vec![pack(10, 20, 30, 255), pack(200, 100, 50, 255)]
    );
}

#[test]
fn headless_an_app_cannot_write_webp_and_refuses_a_pixel_buffer_of_the_wrong_length() {
    assert!(!graphics_imageio::can_write(Format::Webp));
    assert!(graphics_imageio::encode(Format::Webp, 1, 1, &[0, 0, 0, 255]).is_none());
    assert!(
        graphics_imageio::encode(Format::Png, 2, 2, &[0; 4]).is_none(),
        "2 by 2 needs 16 bytes"
    );
    assert!(
        graphics_imageio::encode(Format::Png, 0, 1, &[]).is_none(),
        "an empty picture is refused"
    );
    assert!(graphics_imageio::can_write(Format::Png));
}

#[test]
fn headless_an_app_jpeg_quality_is_clamped_and_a_nan_quality_uses_the_default() {
    let rgba = vec![90u8; 8 * 8 * 4];
    let low =
        graphics_imageio::encode_jpeg(8, 8, &rgba, -4.0).expect("a clamped quality still encodes");
    let high =
        graphics_imageio::encode_jpeg(8, 8, &rgba, 9.0).expect("a clamped quality still encodes");
    let nan = graphics_imageio::encode_jpeg(8, 8, &rgba, f32::NAN)
        .expect("NaN falls back to the default quality");
    assert_eq!(graphics_imageio::detect(&nan), Some(Format::Jpeg));
    assert!(
        low.len() <= high.len(),
        "a lower quality never writes a larger file"
    );
}
