//! Parity sdk-tests-10: headless tests of the symbol glyphs (`bifrost_app::glyphs`) and of the App Intents model
//! (`bifrost_app::intents`), including the manifest text that Search reads.
//!
//! Headless: glyphs are drawn into an in-memory surface of packed pixels, and the intents are plain values. No display,
//! device or app process is involved.

use bifrost_app::control::INVALID_PARAMS;
use bifrost_app::controls::symbol_effect::Pose;
use bifrost_app::controls::Rect;
use bifrost_app::glyphs::{glyph, has, posed_bounds, posed_color, NAMES};
use bifrost_app::intents::{Authentication, Entity, Intent, IntentMode, IntentResult, Param, ParamType, Session};
use bifrost_app::kit::{Font, Surface, Ui};
use bifrost_core::json::Json;

const WHITE: u32 = 0xffff_ffff;

fn ui_at(scale: f32) -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf").into_boxed_slice());
    Ui::new(Font::parse(data).expect("the packed face parses"), scale)
}

fn inked(px: &[u32]) -> usize {
    px.iter().filter(|p| **p != 0).count()
}

// --- glyphs ------------------------------------------------------------------------------------------------------------

#[test]
fn headless_the_glyph_names_are_unique_and_all_have_a_drawing() {
    assert_eq!(NAMES.len(), 60);
    let mut seen = std::collections::HashSet::new();
    for name in NAMES {
        assert!(seen.insert(name), "{name} is listed twice");
        assert!(has(name), "{name} has a drawing");
    }
}

#[test]
fn headless_a_name_outside_the_list_has_no_drawing() {
    assert!(!has("wifi"), "the wireless glyph is not in the set");
    assert!(!has(""));
    assert!(!has("Folder"), "names are case sensitive");
}

#[test]
fn headless_every_named_glyph_leaves_ink_in_its_surface() {
    let ui = ui_at(2.0);
    for name in NAMES {
        let mut px = vec![0u32; 48 * 48];
        let drew = glyph(&mut Surface { w: 48, h: 48, stride: 48, px: &mut px, y0: 0 }, &ui, name, Rect { x: 2.0, y: 2.0, w: 20.0, h: 20.0 }, WHITE);
        assert!(drew, "{name} is drawn");
        assert!(inked(&px) > 20, "{name} puts ink on the surface: {} pixels", inked(&px));
    }
}

#[test]
fn headless_an_unknown_glyph_name_draws_nothing_and_says_so() {
    let ui = ui_at(1.0);
    let mut px = vec![0u32; 32 * 32];
    let drew = glyph(&mut Surface { w: 32, h: 32, stride: 32, px: &mut px, y0: 0 }, &ui, "no.such.symbol", Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 }, WHITE);
    assert!(!drew);
    assert_eq!(inked(&px), 0, "not one pixel is written");
}

#[test]
fn headless_a_glyph_stays_inside_its_bounds_on_the_surface() {
    let ui = ui_at(1.0);
    let mut px = vec![0u32; 64 * 64];
    // a glyph centred in a 20 by 20 box at (22, 22) must not reach the outer ring of the surface
    glyph(&mut Surface { w: 64, h: 64, stride: 64, px: &mut px, y0: 0 }, &ui, "star", Rect { x: 22.0, y: 22.0, w: 20.0, h: 20.0 }, WHITE);
    let border = (0..64).flat_map(|i| [px[i], px[63 * 64 + i], px[i * 64], px[i * 64 + 63]]).filter(|p| *p != 0).count();
    assert_eq!(border, 0, "nothing is drawn on the surface edge");
    assert!(inked(&px) > 20);
}

#[test]
fn headless_a_rest_pose_leaves_bounds_and_colour_unchanged() {
    let b = Rect { x: 10.0, y: 20.0, w: 40.0, h: 30.0 };
    let r = posed_bounds(b, Pose::REST);
    assert_eq!((r.x, r.y, r.w, r.h), (10.0, 20.0, 40.0, 30.0));
    assert_eq!(posed_color(0xff12_3456, 1.0), 0xff12_3456);
}

#[test]
fn headless_a_pose_scale_keeps_the_centre_of_the_glyph_fixed() {
    let b = Rect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 };
    let r = posed_bounds(b, Pose { scale: 0.5, ..Pose::REST });
    assert_eq!((r.w, r.h), (20.0, 10.0));
    assert_eq!((r.x + r.w / 2.0, r.y + r.h / 2.0), (20.0, 10.0));
}

#[test]
fn headless_a_pose_offset_moves_the_glyph_by_a_fraction_of_its_size() {
    let b = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
    let r = posed_bounds(b, Pose { dx: 0.25, dy: -0.5, ..Pose::REST });
    assert_eq!((r.x + r.w / 2.0, r.y + r.h / 2.0), (30.0, 0.0));
}

#[test]
fn headless_opacity_scales_only_the_alpha_byte_and_is_clamped() {
    assert_eq!(posed_color(0xff00_ff00, 0.5), 0x8000_ff00);
    assert_eq!(posed_color(0xff00_ff00, 0.0), 0x0000_ff00);
    assert_eq!(posed_color(0xff00_ff00, 3.0), 0xff00_ff00, "opacity above one is clamped, not wrapped");
}

// --- App Intents: parameter types --------------------------------------------------------------------------------------

#[test]
fn headless_a_parameter_type_parses_its_own_name() {
    for name in ["string", "int", "bool", "date", "url", "double", "currency", "placemark", "measurement", "file"] {
        let ty = ParamType::parse(name).unwrap_or_else(|| panic!("{name} parses"));
        assert_eq!(ty.name(), name);
    }
    assert_eq!(ParamType::parse("  date  "), Some(ParamType::Date), "surrounding space is ignored");
}

#[test]
fn headless_entity_and_enum_parameter_types_keep_their_argument() {
    assert_eq!(ParamType::parse("entity(Note)"), Some(ParamType::Entity("Note".into())));
    assert_eq!(ParamType::Entity("Note".into()).name(), "entity(Note)");
    let open = ParamType::parse("enum(open|closed)").expect("enum parses");
    assert_eq!(open, ParamType::Enum(vec!["open".into(), "closed".into()]));
    assert_eq!(open.name(), "enum(open|closed)");
}

#[test]
fn headless_a_malformed_parameter_type_is_refused() {
    assert!(ParamType::parse("entity()").is_none(), "an entity needs a type name");
    assert!(ParamType::parse("enum(a||b)").is_none(), "an empty case is refused");
    assert!(ParamType::parse("enum(a)b").is_none());
    assert!(ParamType::parse("number").is_none());
}

#[test]
fn headless_an_int_parameter_takes_whole_numbers_only() {
    assert!(ParamType::Int.accepts(&Json::Num(3.0)));
    assert!(!ParamType::Int.accepts(&Json::Num(3.5)));
    assert!(!ParamType::Int.accepts(&Json::Str("3".into())));
}

#[test]
fn headless_a_date_parameter_takes_a_day_or_a_day_with_a_time() {
    assert!(ParamType::Date.accepts(&Json::Str("2026-10-10".into())));
    assert!(ParamType::Date.accepts(&Json::Str("2026-10-10T14:30:00".into())));
    assert!(!ParamType::Date.accepts(&Json::Str("10-10-2026".into())));
    assert!(!ParamType::Date.accepts(&Json::Str("2026-13-10".into())), "month 13 does not exist");
    assert!(!ParamType::Date.accepts(&Json::Str("2026-10-10T25:00".into())), "hour 25 does not exist");
}

#[test]
fn headless_a_url_parameter_needs_a_scheme_and_a_rest() {
    assert!(ParamType::Url.accepts(&Json::Str("https://example.com/a".into())));
    assert!(!ParamType::Url.accepts(&Json::Str("example.com".into())));
    assert!(!ParamType::Url.accepts(&Json::Str(": x".into())));
}

#[test]
fn headless_a_currency_parameter_needs_an_amount_and_a_three_capital_code() {
    let good = Json::Obj(vec![("amount".into(), Json::Num(12.5)), ("code".into(), Json::Str("ISK".into()))]);
    assert!(ParamType::Currency.accepts(&good));
    let lower = Json::Obj(vec![("amount".into(), Json::Num(12.5)), ("code".into(), Json::Str("isk".into()))]);
    assert!(!ParamType::Currency.accepts(&lower));
}

#[test]
fn headless_a_placemark_parameter_bounds_its_coordinates() {
    let place = |lat: f64, lon: f64| Json::Obj(vec![("latitude".into(), Json::Num(lat)), ("longitude".into(), Json::Num(lon))]);
    assert!(ParamType::Placemark.accepts(&place(64.1, -21.9)));
    assert!(!ParamType::Placemark.accepts(&place(91.0, 0.0)), "latitude stops at 90");
    assert!(!ParamType::Placemark.accepts(&place(0.0, 181.0)), "longitude stops at 180");
}

#[test]
fn headless_a_measurement_needs_a_value_and_a_unit() {
    let ok = Json::Obj(vec![("value".into(), Json::Num(5.0)), ("unit".into(), Json::Str("km".into()))]);
    assert!(ParamType::Measurement.accepts(&ok));
    let no_unit = Json::Obj(vec![("value".into(), Json::Num(5.0)), ("unit".into(), Json::Str("  ".into()))]);
    assert!(!ParamType::Measurement.accepts(&no_unit));
}

#[test]
fn headless_an_enum_parameter_takes_only_its_listed_cases() {
    let open = ParamType::parse("enum(open|closed)").expect("enum parses");
    assert!(open.accepts(&Json::Str("closed".into())));
    assert!(!open.accepts(&Json::Str("paused".into())));
}

// --- App Intents: declarations -----------------------------------------------------------------------------------------

#[test]
fn headless_a_new_intent_runs_in_the_background_with_no_authentication_and_no_shortcut() {
    let i = Intent::new("new-note", "New Note", "Creates a note");
    assert_eq!(i.method(), "intent.new-note");
    assert!(!i.shortcut);
    assert!(i.supports(IntentMode::Background));
    assert!(!i.supports(IntentMode::Foreground));
    assert_eq!(i.authentication, Authentication::AlwaysAllowed);
}

#[test]
fn headless_foreground_mode_is_added_once_however_often_it_is_asked_for() {
    let i = Intent::new("open", "Open", "").foreground().foreground();
    assert_eq!(i.modes, vec![IntentMode::Background, IntentMode::Foreground]);
    assert_eq!(IntentMode::Foreground.name(), "foreground");
}

#[test]
fn headless_the_manifest_text_lists_the_shortcut_and_its_parameters_for_search() {
    let i = Intent::new("new-note", "New Note", "Creates a note")
        .shortcut()
        .param(Param::new("due", "Due", ParamType::Date).optional())
        .param(Param::new("text", "Text", ParamType::String));
    let text = i.manifest_toml();
    assert_eq!(
        text,
        "[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\ndescription = \"Creates a note\"\nshortcut = true\nparams = [\"due:date?\", \"text:string\"]\n"
    );
}

#[test]
fn headless_the_manifest_text_swaps_double_quotes_for_single_ones() {
    let text = Intent::new("say", "Say \"hi\"", "Speaks \"now\"").manifest_toml();
    assert!(text.contains("title = \"Say 'hi'\""));
    assert!(text.contains("description = \"Speaks 'now'\""));
    assert!(text.contains("shortcut = false"));
    assert!(text.contains("params = []"));
}

#[test]
fn headless_the_json_declaration_names_each_parameter_and_mode() {
    let i = Intent::new("find", "Find", "").param(Param::new("word", "Word", ParamType::String)).foreground();
    let j = i.to_json();
    assert_eq!(j.get("id").and_then(|v| v.as_str()), Some("find"));
    assert_eq!(j.get("authentication").and_then(|v| v.as_str()), Some("always-allowed"));
    let params = match j.get("parameters") {
        Some(Json::Arr(p)) => p,
        _ => panic!("parameters is an array"),
    };
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].get("type").and_then(|v| v.as_str()), Some("string"));
    assert!(matches!(j.get("modes"), Some(Json::Arr(m)) if m.len() == 2));
}

// --- App Intents: validation -------------------------------------------------------------------------------------------

fn note_intent() -> Intent {
    Intent::new("note", "Note", "").param(Param::new("text", "Text", ParamType::String)).param(Param::new("count", "Count", ParamType::Int).optional())
}

#[test]
fn headless_validation_accepts_the_required_parameters_and_leaves_out_the_optional_ones() {
    let params = Json::Obj(vec![("text".into(), Json::Str("milk".into()))]);
    assert!(note_intent().validate(&params).is_ok());
    assert!(note_intent().validate(&Json::Null).is_err(), "text is required");
}

#[test]
fn headless_validation_names_a_missing_required_parameter_and_its_type() {
    let err = note_intent().validate(&Json::Obj(vec![])).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
    assert_eq!(err.message, "missing parameter text (string)");
}

#[test]
fn headless_validation_refuses_a_parameter_the_intent_does_not_declare() {
    let params = Json::Obj(vec![("text".into(), Json::Str("a".into())), ("colour".into(), Json::Str("red".into()))]);
    let err = note_intent().validate(&params).unwrap_err();
    assert_eq!(err.message, "unknown parameter colour");
}

#[test]
fn headless_validation_refuses_a_value_of_the_wrong_type_and_treats_null_as_absent() {
    let wrong = Json::Obj(vec![("text".into(), Json::Str("a".into())), ("count".into(), Json::Str("two".into()))]);
    assert_eq!(note_intent().validate(&wrong).unwrap_err().message, "parameter count must be int");
    let null_optional = Json::Obj(vec![("text".into(), Json::Str("a".into())), ("count".into(), Json::Null)]);
    assert!(note_intent().validate(&null_optional).is_ok());
}

#[test]
fn headless_validation_needs_its_parameters_as_an_object() {
    let err = note_intent().validate(&Json::Num(1.0)).unwrap_err();
    assert_eq!(err.message, "params must be an object");
}

// --- App Intents: authentication, results and follow-on intents ----------------------------------------------------

#[test]
fn headless_an_authentication_policy_checks_the_device_session() {
    let locked = Session { locked: true, authenticated: false };
    let unlocked = Session { locked: false, authenticated: false };
    let authed = Session { locked: true, authenticated: true };
    assert!(Authentication::AlwaysAllowed.allows(locked));
    assert!(!Authentication::RequiresAuthentication.allows(locked));
    assert!(Authentication::RequiresAuthentication.allows(authed));
    assert!(!Authentication::RequiresLocalDeviceAuthentication.allows(locked));
    assert!(Authentication::RequiresLocalDeviceAuthentication.allows(unlocked));
    assert_eq!(Authentication::RequiresLocalDeviceAuthentication.name(), "requires-local-device-authentication");
    let guarded = Intent::new("pay", "Pay", "").authentication(Authentication::RequiresAuthentication);
    assert!(!guarded.may_run(unlocked));
    assert!(guarded.may_run(authed));
}

#[test]
fn headless_a_background_intent_cannot_ask_to_continue_in_the_foreground() {
    let background = Intent::new("sync", "Sync", "");
    let declared = vec![background.clone()];
    let asks = IntentResult::dialog("Syncing").continue_in_foreground();
    let err = background.check_result(&asks, &declared).unwrap_err();
    assert!(err.message.contains("cannot continue in the foreground"));
    let foreground = Intent::new("open", "Open", "").foreground();
    assert!(foreground.check_result(&asks, &[foreground.clone()]).is_ok());
}

#[test]
fn headless_a_follow_on_intent_must_be_declared_and_receive_valid_parameters() {
    let start = Intent::new("start", "Start", "");
    let next = Intent::new("note", "Note", "").param(Param::new("text", "Text", ParamType::String));
    let declared = vec![start.clone(), next];
    let good = IntentResult::value(Json::Null).opens("note", Json::Obj(vec![("text".into(), Json::Str("hi".into()))]));
    assert!(start.check_result(&good, &declared).is_ok());
    let undeclared = IntentResult::value(Json::Null).opens("missing", Json::Null);
    assert!(start.check_result(&undeclared, &declared).unwrap_err().message.contains("undeclared"));
    let bad = IntentResult::value(Json::Null).opens("note", Json::Obj(vec![]));
    assert!(start.check_result(&bad, &declared).unwrap_err().message.contains("missing parameter"));
}

#[test]
fn headless_an_intent_result_reads_back_its_dialog_and_its_value() {
    let result = IntentResult::value(Json::Num(7.0)).with_dialog("Seven");
    let back = IntentResult::from_json(&result.to_json()).expect("the reply parses");
    assert_eq!(back.dialog.as_deref(), Some("Seven"));
    assert!(!back.continue_foreground);
    assert!(back.opens.is_none());
    assert_eq!(IntentResult::from_json(&Json::Num(1.0)).map(|r| r.dialog), None, "a value that is not a reply is refused");
    let keyed = IntentResult::value(Json::Obj(vec![("count".into(), Json::Num(7.0))]));
    assert_eq!(keyed.at("count").and_then(|v| v.number()), Some(7.0));
    assert!(keyed.at("missing").is_none());
}

#[test]
fn headless_an_entity_carries_its_type_id_title_and_subtitle() {
    let e = Entity::new("Note", "n1", "Milk").subtitle("Groceries");
    let j = e.to_json();
    assert_eq!(j.get("id").and_then(|v| v.as_str()), Some("n1"));
    assert_eq!(j.get("title").and_then(|v| v.as_str()), Some("Milk"));
    assert_eq!(j.get("subtitle").and_then(|v| v.as_str()), Some("Groceries"));
    assert_eq!(j.get("type").and_then(|v| v.as_str()), Some("Note"));
}
