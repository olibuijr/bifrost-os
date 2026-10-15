//! Parity sdk-tests-16: headless tests for file and content-type icons, the developer-tools line protocol of web views,
//! web view configuration and page messages, and the location manager. The folder icon is drawn offscreen into an
//! in-memory pixel buffer and its pixels are asserted. No engine process, shell, display or device is started, and no
//! clock is read.

use bifrost_app::controls::location_auth::{Level, RequestOutcome, UsageDescriptions, UsageError, ERRORS};
use bifrost_app::controls::Rect;
use bifrost_app::core_location::{
    accuracy, decode_error, decode_location, decode_locations, decode_placemark, error_named, AuthorizationStatus, Coordinate,
    Delegate, Location, LocationError, LocationManager, DISTANCE_FILTER_NONE, EVENT_FAIL, EVENT_UPDATE, SERVICE,
};
use bifrost_app::kit::{Font, Surface, Ui};
use bifrost_app::proto::{Configure, ToShell};
use bifrost_app::web_config::{
    HandlerError, InjectionTime, ScriptMessage, ScriptMessageHandler, UserContentController, UserScript, WebPreferences,
    WebViewConfiguration,
};
use bifrost_app::webview::{unescape_line, CONSOLE_LINE_MAX};
use bifrost_app::webview_dev::{escape_full, parse_dev_line, DevEvent, EvalValue, ShotMode};
use bifrost_app::widgets::folder_icon;
use bifrost_app::workspace::{icon_for_content_type, icon_for_file};
use bifrost_app::Ctx;
use bifrost_core::json::Json;
use std::cell::RefCell;
use std::rc::Rc;

// ---------------------------------------------------------------------------------------------------------------------
// Icons for content types and file names (`workspace`)
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_content_type_gets_the_glyph_of_its_branch_of_the_type_tree() {
    assert_eq!(icon_for_content_type("public.png"), "photo");
    assert_eq!(icon_for_content_type("public.svg-image"), "photo", "an SVG image conforms to image");
    assert_eq!(icon_for_content_type("public.movie"), "film");
    assert_eq!(icon_for_content_type("public.mpeg-4"), "film");
    assert_eq!(icon_for_content_type("public.mp3"), "music.note");
    assert_eq!(icon_for_content_type("public.zip-archive"), "archivebox");
    assert_eq!(icon_for_content_type("public.plain-text"), "doc.text");
    assert_eq!(icon_for_content_type("public.json"), "doc.text", "JSON conforms to text");
    assert_eq!(icon_for_content_type("public.folder"), "folder");
}

#[test]
fn a_content_type_no_branch_covers_gets_the_plain_document_glyph() {
    assert_eq!(icon_for_content_type("com.example.unlisted"), "doc");
    assert_eq!(icon_for_content_type("public.data"), "doc");
}

#[test]
fn a_file_name_gets_the_glyph_of_its_extension_in_any_letter_case() {
    assert_eq!(icon_for_file("holiday.PNG"), "photo");
    assert_eq!(icon_for_file("clip.mov"), "film");
    assert_eq!(icon_for_file("song.flac"), "music.note");
    assert_eq!(icon_for_file("paper.pdf"), "doc.richtext");
    assert_eq!(icon_for_file("bundle.ZIP"), "archivebox");
    assert_eq!(icon_for_file("readme.TXT"), "doc.text");
    assert_eq!(icon_for_file("notes.md"), "doc.text");
}

#[test]
fn a_file_name_without_a_known_extension_gets_the_plain_document_glyph() {
    assert_eq!(icon_for_file("Makefile"), "doc");
    assert_eq!(icon_for_file("archive.unknownext"), "doc");
}

// ---------------------------------------------------------------------------------------------------------------------
// The folder icon, drawn offscreen (`widgets::folder_icon`)
// ---------------------------------------------------------------------------------------------------------------------

fn ui(scale: f32) -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), scale)
}

/// Draws the folder icon into an offscreen buffer of `w` x `h` pixels and returns the packed pixels.
fn draw_folder(ui: &Ui, w: usize, h: usize, bounds: Rect) -> Vec<u32> {
    let mut px = vec![0u32; w * h];
    {
        let mut surface = Surface { w, h, stride: w, px: px.as_mut_slice(), y0: 0 };
        folder_icon(&mut surface, ui, bounds, None);
    }
    px
}

/// The pixel at (x, y) of a buffer `w` pixels wide. Packed as 0xAABBGGRR for red, green, blue and alpha bytes.
fn at(px: &[u32], w: usize, x: usize, y: usize) -> u32 {
    px[y * w + x]
}

fn red(p: u32) -> u32 {
    p & 0xFF
}

fn blue(p: u32) -> u32 {
    (p >> 16) & 0xFF
}

#[test]
fn headless_the_folder_icon_paints_blue_inside_its_square_and_leaves_the_corners_empty() {
    let px = draw_folder(&ui(1.0), 64, 64, Rect { x: 0.0, y: 0.0, w: 64.0, h: 64.0 });
    let body = at(&px, 64, 32, 40);
    assert!(blue(body) > red(body) && blue(body) > 0, "the front body is blue: {body:#010x}");
    assert_eq!(at(&px, 64, 0, 0), 0, "the top left corner is outside the icon");
    assert_eq!(at(&px, 64, 63, 63), 0, "the bottom right corner is outside the icon");
}

#[test]
fn headless_the_folder_icon_is_centred_in_a_wide_frame_with_its_side_as_the_height() {
    let px = draw_folder(&ui(1.0), 120, 40, Rect { x: 0.0, y: 0.0, w: 120.0, h: 40.0 });
    assert_eq!(at(&px, 120, 5, 20), 0, "left of the icon stays empty");
    assert_eq!(at(&px, 120, 115, 20), 0, "right of the icon stays empty");
    let middle = at(&px, 120, 60, 25);
    assert!(blue(middle) > red(middle), "the body fills the middle of the frame: {middle:#010x}");
}

#[test]
fn headless_the_folder_icon_scales_its_drawing_with_the_point_size() {
    // Two pixels per point: the same layout is twice as large, so the body centre moves from (32, 40) to (64, 80).
    let px = draw_folder(&ui(2.0), 128, 128, Rect { x: 0.0, y: 0.0, w: 64.0, h: 64.0 });
    let body = at(&px, 128, 64, 80);
    assert!(blue(body) > red(body), "the body is drawn at the scaled position: {body:#010x}");
    assert_eq!(at(&px, 128, 0, 0), 0);
}

#[test]
fn headless_the_folder_icon_renders_the_same_pixels_twice_for_the_same_bounds() {
    let bounds = Rect { x: 4.0, y: 2.0, w: 48.0, h: 48.0 };
    assert_eq!(draw_folder(&ui(1.0), 64, 64, bounds), draw_folder(&ui(1.0), 64, 64, bounds));
}

// ---------------------------------------------------------------------------------------------------------------------
// The developer-tools line protocol of a web view (`webview_dev`, `webview` escapes)
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_dev_eval_answer_parses_its_type_and_its_json_value() {
    assert_eq!(
        parse_dev_line("eval 12 ok number 42"),
        Some(DevEvent::Eval { token: 12, result: Ok(EvalValue { ty: "number".into(), json: "42".into() }) })
    );
    assert_eq!(
        parse_dev_line("eval 13 ok undefined"),
        Some(DevEvent::Eval { token: 13, result: Ok(EvalValue { ty: "undefined".into(), json: "null".into() }) }),
        "an answer with no JSON text reads as null"
    );
}

#[test]
fn a_dev_eval_error_keeps_the_thrown_text_and_its_line_breaks() {
    assert_eq!(
        parse_dev_line("eval 5 err ReferenceError: value is not defined"),
        Some(DevEvent::Eval { token: 5, result: Err("ReferenceError: value is not defined".into()) })
    );
    assert_eq!(
        parse_dev_line("eval 6 err line one\\nline two"),
        Some(DevEvent::Eval { token: 6, result: Err("line one\nline two".into()) })
    );
}

#[test]
fn a_dev_screenshot_answer_reports_the_pixel_size_or_the_failure() {
    assert_eq!(parse_dev_line("shot 3 ok 1280 800"), Some(DevEvent::Shot { token: 3, result: Ok((1280, 800)) }));
    assert_eq!(
        parse_dev_line("shot 4 err no view to capture"),
        Some(DevEvent::Shot { token: 4, result: Err("no view to capture".into()) })
    );
    assert_eq!(parse_dev_line("shot 5 ok 1280"), None, "a size needs both sides");
}

#[test]
fn a_malformed_or_unknown_dev_line_is_ignored() {
    assert_eq!(parse_dev_line("eval x ok number 1"), None, "the token must be a number");
    assert_eq!(parse_dev_line("eval 7"), None, "a status is required");
    assert_eq!(parse_dev_line("collector 7 ok"), None, "only eval and shot answers are read");
    assert_eq!(parse_dev_line(""), None);
}

#[test]
fn an_escaped_script_keeps_its_line_breaks_and_its_backslashes() {
    let script = "const a = 1;\nreturn a\\b\r";
    let escaped = escape_full(script);
    assert_eq!(escaped, "const a = 1;\\nreturn a\\\\b\\r");
    assert_eq!(unescape_line(&escaped), script);
}

#[test]
fn a_script_escape_has_no_length_cap_while_console_text_does() {
    let long = "x\n".repeat(CONSOLE_LINE_MAX);
    let escaped = escape_full(&long);
    assert!(escaped.len() > CONSOLE_LINE_MAX, "a script is sent whole");
    assert_eq!(unescape_line(&escaped), long);
}

#[test]
fn a_screenshot_mode_names_the_view_the_rectangle_or_the_whole_document() {
    let rect = ShotMode::Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 };
    assert_eq!(rect, ShotMode::Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 });
    assert_ne!(rect, ShotMode::Rect { x: 1.0, y: 2.0, w: 6.0, h: 4.0 });
    assert_ne!(ShotMode::View, ShotMode::Full);
}

// ---------------------------------------------------------------------------------------------------------------------
// Page scripts and messages (`web_config`)
// ---------------------------------------------------------------------------------------------------------------------

struct Inbox(Rc<RefCell<Vec<ScriptMessage>>>);

impl ScriptMessageHandler for Inbox {
    fn did_receive(&mut self, message: &ScriptMessage) {
        self.0.borrow_mut().push(message.clone());
    }
}

fn inbox() -> (Box<dyn ScriptMessageHandler>, Rc<RefCell<Vec<ScriptMessage>>>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    (Box::new(Inbox(log.clone())), log)
}

fn message(name: &str, body: &str) -> ScriptMessage {
    ScriptMessage { name: name.into(), body: body.into(), is_main_frame: true }
}

#[test]
fn a_handler_name_must_be_a_usable_script_property_name() {
    let mut c = UserContentController::new();
    assert_eq!(c.add(inbox().0, "bridge"), Ok(()));
    assert_eq!(c.add(inbox().0, "$ok_1"), Ok(()), "dollar signs and digits after the first character are allowed");
    assert_eq!(c.add(inbox().0, "1st"), Err(HandlerError::InvalidName));
    assert_eq!(c.add(inbox().0, ""), Err(HandlerError::InvalidName));
    assert_eq!(c.add(inbox().0, "has space"), Err(HandlerError::InvalidName));
}

#[test]
fn a_second_handler_under_a_taken_name_is_a_duplicate() {
    let mut c = UserContentController::new();
    c.add(inbox().0, "bridge").unwrap();
    assert_eq!(c.add(inbox().0, "bridge"), Err(HandlerError::Duplicate));
}

#[test]
fn a_page_message_reaches_only_the_handler_registered_under_its_name() {
    let mut c = UserContentController::new();
    let (handler, log) = inbox();
    c.add(handler, "bridge").unwrap();
    assert!(c.deliver(&message("bridge", "{\"n\":1}")));
    assert!(!c.deliver(&message("other", "{}")), "no handler under that name");
    let got = log.borrow();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].name.as_str(), got[0].body.as_str()), ("bridge", "{\"n\":1}"));
}

#[test]
fn a_removed_handler_receives_nothing_more() {
    let mut c = UserContentController::new();
    let (handler, log) = inbox();
    c.add(handler, "bridge").unwrap();
    assert!(c.remove_script_message_handler("bridge"));
    assert!(!c.remove_script_message_handler("bridge"));
    assert!(!c.deliver(&message("bridge", "{}")));
    assert!(log.borrow().is_empty());
}

#[test]
fn the_bootstrap_script_defines_one_poster_per_handler_in_name_order() {
    let mut c = UserContentController::new();
    c.add(inbox().0, "beta").unwrap();
    c.add(inbox().0, "alpha").unwrap();
    let script = c.bootstrap_script("post");
    assert_eq!(script.injection_time, InjectionTime::AtDocumentStart);
    assert!(!script.for_main_frame_only, "the bridge runs in every frame");
    let src = &script.source;
    let alpha = src.find("h[\"alpha\"]").expect("alpha is defined");
    let beta = src.find("h[\"beta\"]").expect("beta is defined");
    assert!(alpha < beta, "names are defined in sorted order");
    assert!(src.contains("post(\"alpha\",JSON.stringify(b))"), "a post goes to the engine through the named function");
    assert!(src.contains("window.bifrost.messageHandlers=h"));
}

#[test]
fn user_scripts_run_in_the_order_they_were_added_and_can_be_cleared() {
    let mut c = UserContentController::new();
    c.add_user_script(UserScript::new("window.a=1", InjectionTime::AtDocumentStart, true));
    c.add_user_script(UserScript::new("window.b=2", InjectionTime::AtDocumentEnd, false));
    let sources: Vec<&str> = c.user_scripts().iter().map(|s| s.source.as_str()).collect();
    assert_eq!(sources, ["window.a=1", "window.b=2"]);
    c.remove_all_user_scripts();
    assert!(c.user_scripts().is_empty());
}

#[test]
fn a_configuration_injects_the_message_bridge_before_the_apps_own_scripts() {
    let mut cfg = WebViewConfiguration::new();
    cfg.user_content_controller.add_user_script(UserScript::new("window.x=1", InjectionTime::AtDocumentEnd, true));
    let injected = cfg.injected_scripts("post");
    assert_eq!(injected.len(), 2);
    assert!(injected[0].source.contains("messageHandlers"));
    assert_eq!(injected[1], UserScript::new("window.x=1", InjectionTime::AtDocumentEnd, true));
}

#[test]
fn the_user_agent_is_the_default_with_the_application_name_or_the_custom_value() {
    let mut cfg = WebViewConfiguration::new();
    assert_eq!(cfg.effective_user_agent("Engine/1.0"), "Engine/1.0");
    cfg.application_name_for_user_agent = Some("Notes/2".into());
    assert_eq!(cfg.effective_user_agent("Engine/1.0"), "Engine/1.0 Notes/2");
    cfg.custom_user_agent = Some("Custom/3".into());
    assert_eq!(cfg.effective_user_agent("Engine/1.0"), "Custom/3", "a custom agent replaces the default");
    cfg.custom_user_agent = Some(String::new());
    assert_eq!(cfg.effective_user_agent("Engine/1.0"), "Engine/1.0 Notes/2", "an empty custom agent is ignored");
}

#[test]
fn the_default_preferences_enable_scripts_and_block_no_pop_ups_by_default() {
    let p = WebPreferences::default();
    assert!(p.javascript_enabled);
    assert!(!p.javascript_can_open_windows_automatically);
    assert_eq!(p.minimum_font_size, 0.0);
    assert!(p.fraudulent_website_warning_enabled);
}

// ---------------------------------------------------------------------------------------------------------------------
// Location manager (`core_location`)
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Spy {
    updates: Vec<Vec<Location>>,
    errors: Vec<LocationError>,
    statuses: Vec<AuthorizationStatus>,
}

impl Delegate for Spy {
    fn did_update_locations(&mut self, locations: &[Location]) {
        self.updates.push(locations.to_vec());
    }
    fn did_fail_with_error(&mut self, error: LocationError) {
        self.errors.push(error);
    }
    fn did_change_authorization(&mut self, status: AuthorizationStatus) {
        self.statuses.push(status);
    }
}

fn ctx() -> Ctx {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
}

fn fix(lat: f64, lon: f64) -> Json {
    Json::Obj(vec![
        ("latitude".into(), Json::Num(lat)),
        ("longitude".into(), Json::Num(lon)),
        ("altitude".into(), Json::Null),
        ("horizontal_accuracy".into(), Json::Num(100.0)),
        ("timestamp".into(), Json::Num(1000.0)),
        ("source".into(), Json::Str("manual".into())),
        ("city".into(), Json::Str("Akureyri".into())),
        ("country".into(), Json::Str("Iceland".into())),
    ])
}

fn update(lat: f64, lon: f64) -> Json {
    Json::Obj(vec![("locations".into(), Json::Arr(vec![fix(lat, lon)]))])
}

#[test]
fn a_new_manager_is_not_determined_idle_and_asks_for_the_best_accuracy() {
    let m = LocationManager::new();
    assert_eq!(m.authorization_status(), AuthorizationStatus::NotDetermined);
    assert!(!m.is_updating());
    assert!(m.location().is_none());
    assert_eq!((m.desired_accuracy, m.distance_filter), (accuracy::BEST, DISTANCE_FILTER_NONE));
}

#[test]
fn the_accuracy_levels_run_from_the_best_to_the_coarsest() {
    let levels = [
        accuracy::BEST_FOR_NAVIGATION,
        accuracy::BEST,
        accuracy::NEAREST_TEN_METERS,
        accuracy::HUNDRED_METERS,
        accuracy::KILOMETER,
        accuracy::THREE_KILOMETERS,
    ];
    assert!(levels.windows(2).all(|pair| pair[0] < pair[1]), "{levels:?}");
}

#[test]
fn a_coordinate_is_valid_only_inside_the_latitude_and_longitude_ranges() {
    assert!(Coordinate::new(90.0, 180.0).is_valid());
    assert!(Coordinate::new(-90.0, -180.0).is_valid());
    assert!(!Coordinate::new(90.1, 0.0).is_valid());
    assert!(!Coordinate::new(0.0, -180.5).is_valid());
}

#[test]
fn the_distance_between_two_points_is_great_circle_metres() {
    let origin = Coordinate::new(0.0, 0.0);
    assert_eq!(origin.distance_to(&origin), 0.0);
    // one degree of longitude on the equator is 2 * pi * 6371 km / 360, about 111.2 km
    let degree = origin.distance_to(&Coordinate::new(0.0, 1.0));
    assert!((degree - 111_195.0).abs() < 100.0, "{degree}");
}

#[test]
fn a_status_reply_reaches_the_delegate_once_per_change() {
    let (mut m, mut d) = (LocationManager::new(), Spy::default());
    let authorized = Ok(Json::Obj(vec![("status".into(), Json::Str("authorized_when_in_use".into()))]));
    assert!(m.handle_reply(&authorized, &mut d));
    assert!(m.handle_reply(&authorized, &mut d));
    assert_eq!(m.authorization_status(), AuthorizationStatus::AuthorizedWhenInUse);
    assert_eq!(d.statuses, vec![AuthorizationStatus::AuthorizedWhenInUse]);
    let denied = Ok(Json::Obj(vec![("status".into(), Json::Str("denied".into()))]));
    m.handle_reply(&denied, &mut d);
    assert_eq!(d.statuses.len(), 2);
    assert_eq!(m.authorization_status(), AuthorizationStatus::Denied);
}

#[test]
fn an_error_reply_that_names_a_case_reaches_the_delegate_and_other_text_does_not() {
    let (mut m, mut d) = (LocationManager::new(), Spy::default());
    assert!(m.handle_reply(&Err("Denied".into()), &mut d));
    assert!(!m.handle_reply(&Err("something else".into()), &mut d));
    assert_eq!(d.errors, vec![LocationError::Denied]);
}

#[test]
fn updates_are_delivered_only_while_the_manager_is_updating() {
    let ctx = ctx();
    let (mut m, mut d) = (LocationManager::new(), Spy::default());
    assert!(m.handle_event(SERVICE, EVENT_UPDATE, &update(65.68, -18.10), &mut d), "a location event is ours");
    assert!(d.updates.is_empty(), "nothing is delivered before the manager starts");

    m.start_updating_location(&ctx);
    assert!(m.is_updating());
    let sent = ctx.take_sent();
    assert!(
        sent.iter().any(|msg| matches!(msg, ToShell::ServiceCall { service, method, .. } if service.as_str() == SERVICE && method.as_str() == "start")),
        "starting asks the location service to start"
    );
    m.handle_event(SERVICE, EVENT_UPDATE, &update(65.68, -18.10), &mut d);
    assert_eq!(d.updates.len(), 1);
    let got = &d.updates[0][0];
    assert_eq!(got.coordinate, Coordinate::new(65.68, -18.10));
    assert_eq!((got.horizontal_accuracy, got.timestamp, got.altitude), (100.0, 1000.0, None));
    assert_eq!((got.source.as_str(), got.city.as_str(), got.country.as_str()), ("manual", "Akureyri", "Iceland"));
    assert_eq!(m.location(), Some(got));

    m.stop_updating_location(&ctx);
    assert!(!m.is_updating());
}

#[test]
fn an_event_of_another_service_is_not_handled_by_the_location_manager() {
    let (mut m, mut d) = (LocationManager::new(), Spy::default());
    m.handle_event(SERVICE, EVENT_UPDATE, &update(65.0, -18.0), &mut d);
    assert!(!m.handle_event("network", EVENT_UPDATE, &update(65.0, -18.0), &mut d));
    assert!(!m.handle_event(SERVICE, "other", &Json::Null, &mut d));
}

#[test]
fn the_distance_filter_drops_updates_that_moved_less_than_the_filter() {
    let ctx = ctx();
    let (mut m, mut d) = (LocationManager::new(), Spy::default());
    m.distance_filter = 5_000.0;
    m.start_updating_location(&ctx);
    m.handle_event(SERVICE, EVENT_UPDATE, &update(65.68, -18.10), &mut d);
    m.handle_event(SERVICE, EVENT_UPDATE, &update(65.69, -18.10), &mut d);
    assert_eq!(d.updates.len(), 1, "about 110 m is under the filter");
    m.handle_event(SERVICE, EVENT_UPDATE, &update(64.14, -21.94), &mut d);
    assert_eq!(d.updates.len(), 2, "a move of about 250 km passes the filter");
}

#[test]
fn a_failure_event_reaches_the_delegate_only_while_updating() {
    let ctx = ctx();
    let (mut m, mut d) = (LocationManager::new(), Spy::default());
    let body = Json::Obj(vec![("code".into(), Json::Num(LocationError::Denied.code() as f64))]);
    m.handle_event(SERVICE, EVENT_FAIL, &body, &mut d);
    assert!(d.errors.is_empty());
    m.start_updating_location(&ctx);
    m.handle_event(SERVICE, EVENT_FAIL, &body, &mut d);
    assert_eq!(d.errors, vec![LocationError::Denied]);
}

#[test]
fn a_request_for_one_location_sends_the_current_call_to_the_service() {
    let ctx = ctx();
    let m = LocationManager::new();
    m.request_location(&ctx);
    let sent = ctx.take_sent();
    assert!(
        sent.iter().any(|msg| matches!(msg, ToShell::ServiceCall { service, method, .. } if service.as_str() == SERVICE && method.as_str() == "current")),
        "requestLocation asks for the current location"
    );
}

#[test]
fn decoding_a_location_rejects_an_invalid_coordinate_or_a_missing_accuracy() {
    assert!(decode_location(&fix(95.0, 0.0)).is_none());
    assert!(decode_location(&Json::Null).is_none());
    let without_accuracy = Json::Obj(vec![
        ("latitude".into(), Json::Num(65.68)),
        ("longitude".into(), Json::Num(-18.10)),
        ("timestamp".into(), Json::Num(1000.0)),
    ]);
    assert!(decode_location(&without_accuracy).is_none());
    let ok = decode_location(&fix(65.68, -18.10)).expect("a valid fix decodes");
    assert_eq!(ok.altitude, None, "no altitude source");
}

#[test]
fn decoding_updates_skips_entries_of_another_shape() {
    let body = Json::Obj(vec![(
        "locations".into(),
        Json::Arr(vec![fix(65.68, -18.10), Json::Str("not a fix".into()), fix(64.14, -21.94)]),
    )]);
    let locations = decode_locations(&body);
    assert_eq!(locations.len(), 2);
    assert_eq!(locations[1].coordinate, Coordinate::new(64.14, -21.94));
    assert!(decode_locations(&Json::Null).is_empty());
}

#[test]
fn decoding_a_placemark_needs_a_city_and_reads_its_country() {
    let place = Json::Obj(vec![
        ("city".into(), Json::Str("Akureyri".into())),
        ("country".into(), Json::Str("Iceland".into())),
        ("latitude".into(), Json::Num(65.68)),
        ("longitude".into(), Json::Num(-18.10)),
    ]);
    let p = decode_placemark(&place).expect("a placemark decodes");
    assert_eq!((p.locality.as_str(), p.country.as_str()), ("Akureyri", "Iceland"));
    assert_eq!(p.coordinate, Coordinate::new(65.68, -18.10));
    let no_city = Json::Obj(vec![("latitude".into(), Json::Num(65.68)), ("longitude".into(), Json::Num(-18.10))]);
    assert!(decode_placemark(&no_city).is_none());
}

#[test]
fn decoding_a_failure_maps_only_the_codes_the_service_defines() {
    let code = |c: f64| Json::Obj(vec![("code".into(), Json::Num(c))]);
    assert_eq!(decode_error(&code(LocationError::Denied.code() as f64)), Some(LocationError::Denied));
    assert_eq!(decode_error(&code(999.0)), None);
    assert_eq!(decode_error(&code(-1.0)), None);
    assert_eq!(decode_error(&Json::Null), None);
}

#[test]
fn an_error_name_matches_the_case_name_exactly() {
    assert_eq!(error_named("GeocodeFoundNoResult"), Some(LocationError::GeocodeFoundNoResult));
    assert_eq!(error_named("geocodefoundnoresult"), None);
}

#[test]
fn every_error_case_round_trips_through_its_numeric_code() {
    for e in ERRORS {
        assert_eq!(LocationError::from_code(e.code()), Some(e));
    }
    assert_eq!(LocationError::from_code(ERRORS.len() as u32), None);
}

#[test]
fn only_the_permission_errors_need_the_person_to_act_and_only_the_passing_ones_are_transient() {
    assert!(LocationError::Denied.needs_user_action());
    assert!(LocationError::PromptDeclined.needs_user_action());
    assert!(!LocationError::LocationUnknown.needs_user_action());
    assert!(LocationError::LocationUnknown.is_transient());
    assert!(LocationError::Network.is_transient());
    assert!(!LocationError::Denied.is_transient());
}

#[test]
fn location_may_start_only_when_authorized_and_in_use_for_when_in_use() {
    assert!(AuthorizationStatus::AuthorizedAlways.may_start(false));
    assert!(!AuthorizationStatus::AuthorizedWhenInUse.may_start(false));
    assert!(AuthorizationStatus::AuthorizedWhenInUse.may_start(true));
    assert!(!AuthorizationStatus::Denied.may_start(true));
    assert!(AuthorizationStatus::AuthorizedWhenInUse.is_authorized());
    assert!(!AuthorizationStatus::NotDetermined.is_authorized());
}

#[test]
fn a_request_prompts_only_when_the_person_has_not_answered() {
    assert_eq!(AuthorizationStatus::NotDetermined.request_outcome(), RequestOutcome::Prompt);
    assert_eq!(AuthorizationStatus::AuthorizedAlways.request_outcome(), RequestOutcome::AlreadyGranted);
    assert_eq!(AuthorizationStatus::Restricted.request_outcome(), RequestOutcome::Refused);
    assert_eq!(AuthorizationStatus::Denied.request_outcome(), RequestOutcome::Refused);
}

#[test]
fn a_usage_text_is_trimmed_and_a_blank_one_is_missing() {
    let texts = UsageDescriptions::new("  Use nearby places  ", "   ");
    assert_eq!(texts.for_level(Level::WhenInUse), Ok("Use nearby places"));
    assert_eq!(texts.for_level(Level::Always), Err(UsageError::Missing(Level::Always)));
}
