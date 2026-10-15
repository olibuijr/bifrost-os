//! Parity tests for the foundation model rows and the route picker rows (sdk-tests-09).
//!
//! These are headless tests of pure models: no drawing, no display, no network and no clock. Each test drives the public
//! API of one module and asserts concrete values.

use bifrost_ui::context_window::{entry_tokens, estimate_tokens, fit, ATTACHMENT_TOKENS, CHARS_PER_TOKEN};
use bifrost_ui::generation_schema::{parse, validate, Kind, ParseError, Property, Value};
use bifrost_ui::model_provider::{run_turn, ModelProvider, ProviderError, Reply, TurnError};
use bifrost_ui::model_session::{
    DynamicInstructions, Profile, PropertyError, PropertyKey, PropertyValue, Session, SessionProperties,
};
use bifrost_ui::model_tool::{Tool, ToolSet};
use bifrost_ui::route_auth::{RouteAuth, RouteError};
use bifrost_ui::route_picker::destinations;
use bifrost_ui::route_picker_view::{ButtonStyle, Event, RoutePickerView};
use bifrost_ui::transcript::{Entry, Segment, ToolCall, ToolOutput, Transcript};
use std::cell::RefCell;
use std::collections::VecDeque;

fn text(s: &str) -> Segment {
    Segment::Text(s.into())
}

/// Forty characters: ten estimated tokens.
fn forty(c: char) -> String {
    std::iter::repeat(c).take(40).collect()
}

fn joined_text(transcript: &Transcript) -> String {
    let segments = transcript.instructions().unwrap_or(&[]);
    segments
        .iter()
        .filter_map(|s| match s {
            Segment::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

// Model session: sessions, prompts, instructions and typed properties.

#[test]
fn a_session_records_prompts_and_responses_in_order() {
    let mut session = Session::new(Profile::new("plain", DynamicInstructions::new()));
    session.prompt(vec![text("high tide?")]);
    session.respond(vec![text("at 14:20")]);
    let history = session.history();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0], Entry::Prompt(vec![text("high tide?")]));
    assert_eq!(history[1], Entry::Response(vec![text("at 14:20")]));
    assert_eq!(session.profile().name(), "plain");
}

#[test]
fn dynamic_instructions_see_the_turn_and_empty_parts_are_left_out() {
    let instructions = DynamicInstructions::new()
        .fixed("Tide clerk.")
        .dynamic(|f| Some(format!("turn {}", f.turn)))
        .dynamic(|_| None)
        .fixed("");
    let mut session = Session::new(Profile::new("tides", instructions));
    assert_eq!(session.instructions(), "Tide clerk.\nturn 0", "no prompts yet: turn 0");
    session.prompt(vec![text("one")]);
    assert_eq!(session.instructions(), "Tide clerk.\nturn 1");
}

#[test]
fn a_property_reads_its_default_and_refuses_a_value_of_another_kind() {
    let brief = PropertyKey::new("brief", PropertyValue::Bool(false));
    assert_eq!(brief.name(), "brief");
    let mut session = Session::new(Profile::new("answers", DynamicInstructions::new()));
    assert_eq!(session.property(&brief), PropertyValue::Bool(false));
    assert_eq!(
        session.set_property(&brief, PropertyValue::Int(2)),
        Err(PropertyError::TypeMismatch { key: "brief" })
    );
    assert_eq!(session.property(&brief), PropertyValue::Bool(false));
    session.set_property(&brief, PropertyValue::Bool(true)).unwrap();
    assert_eq!(session.property(&brief), PropertyValue::Bool(true));
}

#[test]
fn session_properties_store_each_key_separately() {
    let count = PropertyKey::new("count", PropertyValue::Int(0));
    let locale = PropertyKey::new("locale", PropertyValue::Text("en".into()));
    let mut values = SessionProperties::default();
    values.set(&count, PropertyValue::Int(7)).unwrap();
    assert_eq!(values.get(&count), PropertyValue::Int(7));
    assert_eq!(values.get(&locale), PropertyValue::Text("en".into()));
    assert!(values.set(&locale, PropertyValue::Bool(true)).is_err());
}

#[test]
fn switching_profiles_keeps_history_and_changes_the_next_instructions() {
    let mut session = Session::new(Profile::new("plain", DynamicInstructions::new().fixed("Plain.")));
    session.prompt(vec![text("hi")]);
    session.respond(vec![text("hello")]);
    session.switch_profile(Profile::new("formal", DynamicInstructions::new().fixed("Formal.")));
    assert_eq!(session.profile().name(), "formal");
    assert_eq!(session.history().len(), 2);
    assert_eq!(session.instructions(), "Formal.");
}

#[test]
fn prepare_keeps_the_instructions_and_the_newest_turns_that_fit() {
    // "be brief" is two tokens; each forty-character entry is ten tokens. Room for 22 keeps the last prompt and response.
    let mut session = Session::new(Profile::new("short", DynamicInstructions::new().fixed("be brief")));
    session.prompt(vec![text(&forty('a'))]);
    session.respond(vec![text(&forty('b'))]);
    session.prompt(vec![text(&forty('c'))]);
    session.respond(vec![text(&forty('d'))]);
    let fitted = session.prepare(22, 0);
    assert!(!fitted.over_budget);
    assert_eq!(fitted.dropped, 2);
    assert_eq!(fitted.tokens, 22);
    assert_eq!(fitted.transcript.history().len(), 2);
    assert_eq!(joined_text(&fitted.transcript), "be brief");
}

#[test]
fn prepare_reports_over_budget_when_the_instructions_alone_do_not_fit() {
    let mut session = Session::new(Profile::new("long", DynamicInstructions::new().fixed(forty('x'))));
    session.prompt(vec![text("hi")]);
    let fitted = session.prepare(5, 0);
    assert!(fitted.over_budget);
    assert!(fitted.transcript.history().is_empty());
    assert_eq!(fitted.dropped, 1);
}

// Context window: the token estimate and the budget fit.

#[test]
fn the_token_estimate_rounds_up_per_four_characters() {
    assert_eq!(CHARS_PER_TOKEN, 4);
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcde"), 2);
    assert_eq!(estimate_tokens("\u{e5}\u{e4}\u{f6}"), 1, "three characters, one token");
}

#[test]
fn an_attachment_costs_its_name_plus_a_fixed_share() {
    let entry = Entry::Prompt(vec![Segment::Attachment { name: "map".into(), bytes: 1 }]);
    assert_eq!(entry_tokens(&entry), estimate_tokens("map") + ATTACHMENT_TOKENS);
    assert_eq!(entry_tokens(&Entry::Reasoning(forty('r'))), 10);
}

#[test]
fn fit_returns_a_transcript_that_fits_whole() {
    let transcript = Transcript::new(vec![
        Entry::Instructions(vec![text("be brief")]),
        Entry::Prompt(vec![text(&forty('a'))]),
        Entry::Response(vec![text(&forty('b'))]),
    ]);
    let fitted = fit(&transcript, 1000, 100);
    assert_eq!(fitted.transcript, transcript);
    assert_eq!(fitted.dropped, 0);
    assert_eq!(fitted.tokens, 2 + 20);
}

#[test]
fn fit_keeps_the_reserve_free_for_the_answer() {
    let transcript = Transcript::new(vec![
        Entry::Prompt(vec![text(&forty('a'))]),
        Entry::Response(vec![text(&forty('b'))]),
        Entry::Prompt(vec![text(&forty('c'))]),
        Entry::Response(vec![text(&forty('d'))]),
    ]);
    // Room of 25 keeps the last prompt and response (20 tokens). Reserving 10 leaves 15, which keeps only the response,
    // so the history starts at the next prompt and nothing is kept.
    assert_eq!(fit(&transcript, 25, 0).dropped, 2);
    assert_eq!(fit(&transcript, 25, 10).dropped, 4, "a response alone is never kept without its prompt");
}

#[test]
fn fit_never_starts_the_history_with_an_answer() {
    let transcript = Transcript::new(vec![
        Entry::Prompt(vec![text(&forty('a'))]),
        Entry::Response(vec![text(&forty('b'))]),
        Entry::Prompt(vec![text(&forty('c'))]),
        Entry::ToolOutput(ToolOutput { name: "t".into(), text: forty('x') }),
        Entry::Response(vec![text(&forty('d'))]),
    ]);
    let fitted = fit(&transcript, 35, 0);
    assert!(matches!(fitted.transcript.entries()[0], Entry::Prompt(_)));
    assert_eq!(fitted.dropped, 2);
    let tight = fit(&transcript, 25, 0);
    assert!(tight.transcript.history().is_empty());
    assert_eq!(tight.dropped, 5);
}

// Transcript: the text view and the typed view.

#[test]
fn a_transcript_gives_its_text_and_its_typed_entries() {
    let transcript = Transcript::new(vec![
        Entry::Instructions(vec![text("Answer briefly.")]),
        Entry::Prompt(vec![text("What is the weather?")]),
        Entry::ToolCalls(vec![ToolCall { name: "weather".into(), arguments: "{}".into() }]),
        Entry::ToolOutput(ToolOutput { name: "weather".into(), text: "4 C, rain".into() }),
        Entry::Reasoning("the tool answered".into()),
        Entry::Response(vec![text("It is 4 C and raining.")]),
    ]);
    assert_eq!(transcript.history().len(), 5);
    assert_eq!(transcript.text(), "What is the weather?\n\nIt is 4 C and raining.");
    let structured = transcript.structured();
    assert_eq!(structured.tool_calls.len(), 1);
    assert_eq!(structured.tool_calls[0].name, "weather");
    assert_eq!(structured.tool_outputs[0].text, "4 C, rain");
    assert_eq!(structured.responses[0], &[text("It is 4 C and raining.")][..]);
}

// Guided generation: schema validation, description and JSON reading.

fn forecast_schema() -> Kind {
    Kind::Object(vec![
        Property::new("city", Kind::Text).described("city name"),
        Property::new("temp", Kind::number_in(-50.0, 60.0)),
        Property::new("tags", Kind::array_of(Kind::choice(&["rain", "snow", "sun"]))).optional(),
    ])
}

#[test]
fn a_value_that_matches_the_schema_has_no_issues() {
    let value = Value::Object(vec![
        ("city".into(), Value::Text("Reykjavik".into())),
        ("temp".into(), Value::Number(4.0)),
        ("tags".into(), Value::Array(vec![Value::Text("rain".into())])),
    ]);
    assert!(validate(&forecast_schema(), &value).is_empty());
}

#[test]
fn validate_reports_every_problem_with_its_path_in_schema_order() {
    let value = Value::Object(vec![
        ("city".into(), Value::Number(1.0)),
        ("temp".into(), Value::Number(99.0)),
        ("tags".into(), Value::Array(vec![Value::Text("hail".into())])),
        ("wind".into(), Value::Bool(true)),
    ]);
    let issues = validate(&forecast_schema(), &value);
    let found: Vec<(&str, &str)> = issues.iter().map(|i| (i.path.as_str(), i.message.as_str())).collect();
    assert_eq!(
        found,
        vec![
            ("$.city", "expected text, found number"),
            ("$.temp", "99 is outside -50..=60"),
            ("$.tags[0]", "\"hail\" is not one of \"rain\", \"snow\", \"sun\""),
            ("$.wind", "not in the schema"),
        ]
    );
}

#[test]
fn validate_reports_a_missing_required_member_and_a_non_finite_number() {
    let missing = validate(&forecast_schema(), &Value::Object(vec![("temp".into(), Value::Number(1.0))]));
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].path, "$.city");
    assert_eq!(missing[0].message, "missing");
    let nan = validate(&Kind::number(), &Value::Number(f64::NAN));
    assert_eq!(nan[0].message, "number is not finite");
}

#[test]
fn describe_lists_each_member_with_its_type_range_and_description() {
    let schema = Kind::Object(vec![
        Property::new("city", Kind::Text).described("city name"),
        Property::new("temp", Kind::number_in(-50.0, 60.0)).optional(),
    ]);
    assert_eq!(schema.describe(), "object { city: text (city name), temp?: number from -50 to 60 }");
    assert_eq!(Kind::array_of(Kind::Bool).describe(), "array of boolean");
}

#[test]
fn parse_reads_json_values_and_refuses_text_after_the_value() {
    let value = parse(" {\"a\": [1, true, null, \"x\"]} ").unwrap();
    assert_eq!(
        value,
        Value::Object(vec![(
            "a".into(),
            Value::Array(vec![Value::Number(1.0), Value::Bool(true), Value::Null, Value::Text("x".into())]),
        )])
    );
    assert_eq!(parse("1 2"), Err(ParseError { at: 2, message: "text after the value" }));
}

#[test]
fn parse_refuses_nesting_deeper_than_the_limit() {
    let deep = format!("{}{}", "[".repeat(65), "]".repeat(65));
    assert!(parse(&deep).is_err());
}

// Model provider: one turn with tools, against a scripted provider.

/// A provider that returns scripted replies in order and keeps every request it was sent.
struct Scripted {
    replies: RefCell<VecDeque<Result<Reply, ProviderError>>>,
    seen: RefCell<Vec<Transcript>>,
}

impl Scripted {
    fn new(replies: Vec<Result<Reply, ProviderError>>) -> Self {
        Scripted { replies: RefCell::new(replies.into()), seen: RefCell::new(Vec::new()) }
    }
}

impl ModelProvider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn generate(&self, request: &Transcript) -> Result<Reply, ProviderError> {
        self.seen.borrow_mut().push(request.clone());
        self.replies.borrow_mut().pop_front().unwrap_or_else(|| Err(ProviderError::new("no reply left")))
    }
}

fn weather_tools(result: &'static str) -> ToolSet {
    let schema = Kind::Object(vec![Property::new("city", Kind::Text).described("city name")]);
    let mut set = ToolSet::new();
    set.register(Tool::new("weather", "Current weather for a city.", schema, move |_| Ok(result.to_string())))
        .unwrap();
    set
}

fn weather_call() -> Reply {
    Reply::Call(vec![ToolCall { name: "weather".into(), arguments: "{\"city\":\"Reykjavik\"}".into() }])
}

fn chat() -> Session {
    Session::new(Profile::new("brief", DynamicInstructions::new().fixed("Be brief.")))
}

#[test]
fn a_plain_answer_ends_the_turn_and_is_recorded_in_the_session() {
    let provider = Scripted::new(vec![Ok(Reply::Respond(vec![text("Hello")]))]);
    let mut session = chat();
    session.prompt(vec![text("Hi")]);
    let out = run_turn(&mut session, &ToolSet::new(), &provider, 1000, 0, 2);
    assert_eq!(out, Ok(vec![text("Hello")]));
    assert_eq!(session.history().len(), 2);
    assert_eq!(session.history()[1], Entry::Response(vec![text("Hello")]));
}

#[test]
fn a_tool_call_runs_the_tool_records_its_output_and_asks_again() {
    let provider = Scripted::new(vec![Ok(weather_call()), Ok(Reply::Respond(vec![text("It is 4 C")]))]);
    let mut session = chat();
    session.prompt(vec![text("Weather?")]);
    let out = run_turn(&mut session, &weather_tools("4 C, rain"), &provider, 1000, 0, 2);
    assert_eq!(out, Ok(vec![text("It is 4 C")]));
    let history = session.history();
    assert_eq!(history.len(), 4);
    assert!(matches!(history[1], Entry::ToolCalls(_)));
    assert_eq!(
        history[2],
        Entry::ToolOutput(ToolOutput { name: "weather".into(), text: "4 C, rain".into() })
    );
    let seen = provider.seen.borrow();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].entries().iter().any(|e| matches!(e, Entry::ToolOutput(_))));
}

#[test]
fn the_tool_descriptions_are_added_to_the_instructions_of_each_request() {
    let provider = Scripted::new(vec![Ok(Reply::Respond(vec![text("ok")]))]);
    let mut session = chat();
    session.prompt(vec![text("hi")]);
    run_turn(&mut session, &weather_tools("unused"), &provider, 1000, 0, 2).unwrap();
    let seen = provider.seen.borrow();
    let instructions = joined_text(&seen[0]);
    assert!(instructions.starts_with("Be brief."), "the session's own instructions come first");
    assert!(instructions.contains("- weather: Current weather for a city. Arguments: object { city: text (city name) }"));
}

#[test]
fn a_call_to_an_unknown_tool_returns_an_error_output_and_the_turn_goes_on() {
    let unknown = Reply::Call(vec![ToolCall { name: "wind".into(), arguments: String::new() }]);
    let provider = Scripted::new(vec![Ok(unknown), Ok(Reply::Respond(vec![text("I cannot check the wind")]))]);
    let mut session = chat();
    session.prompt(vec![text("Wind?")]);
    let out = run_turn(&mut session, &weather_tools("unused"), &provider, 1000, 0, 2);
    assert_eq!(out, Ok(vec![text("I cannot check the wind")]));
    match &session.history()[2] {
        Entry::ToolOutput(output) => {
            assert_eq!(output.name, "wind");
            assert!(output.text.starts_with("error: "), "got {:?}", output.text);
        }
        other => panic!("expected a tool output, got {other:?}"),
    }
}

#[test]
fn a_turn_stops_when_the_model_keeps_asking_for_tools() {
    let provider = Scripted::new(vec![Ok(weather_call()), Ok(weather_call())]);
    let mut session = chat();
    session.prompt(vec![text("Weather?")]);
    let out = run_turn(&mut session, &weather_tools("rain"), &provider, 1000, 0, 1);
    assert_eq!(out, Err(TurnError::ToolRoundsExceeded { rounds: 1 }));
}

#[test]
fn a_turn_fails_before_asking_when_the_tool_descriptions_do_not_fit() {
    let provider = Scripted::new(vec![Ok(Reply::Respond(vec![text("never sent")]))]);
    let mut session = chat();
    session.prompt(vec![text("hi")]);
    let out = run_turn(&mut session, &weather_tools("rain"), &provider, 5, 0, 2);
    assert_eq!(out, Err(TurnError::OverBudget));
    assert!(provider.seen.borrow().is_empty(), "no request is sent");
}

#[test]
fn a_provider_failure_and_an_empty_tool_request_are_both_reported() {
    let failing = Scripted::new(vec![Err(ProviderError::new("offline"))]);
    let mut session = chat();
    session.prompt(vec![text("hi")]);
    let out = run_turn(&mut session, &ToolSet::new(), &failing, 1000, 0, 2);
    assert_eq!(out, Err(TurnError::Provider(ProviderError::new("offline"))));

    let empty = Scripted::new(vec![Ok(Reply::Call(Vec::new()))]);
    let mut session = chat();
    session.prompt(vec![text("hi")]);
    let out = run_turn(&mut session, &ToolSet::new(), &empty, 1000, 0, 2);
    assert_eq!(out, Err(TurnError::Provider(ProviderError::new("the reply asked for no tool"))));
}

// Route picker rows and the route authorization they read.

#[test]
fn route_rows_list_local_outputs_first_then_receivers_sorted_ignoring_case() {
    let auth = RouteAuth::new();
    let found = [("r2", "bedroom"), ("r1", "Attic"), ("r3", "Kitchen")];
    let rows = destinations(&[("speaker", "Speaker")], &found, &auth);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["Speaker", "Attic", "bedroom", "Kitchen"]);
    assert!(rows[0].local);
    assert!(!rows[1].local);
}

#[test]
fn a_route_row_repeats_no_identifier_and_skips_blank_ones() {
    let auth = RouteAuth::new();
    let found = [("speaker", "Speaker copy"), ("r1", "Attic"), ("r1", "Attic again"), ("  ", "Blank")];
    let rows = destinations(&[("speaker", "Speaker")], &found, &auth);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["Speaker", "Attic"]);
}

#[test]
fn a_route_row_is_active_only_after_the_route_is_authorized_and_activated() {
    let mut auth = RouteAuth::new();
    assert_eq!(auth.set_active("r1", true), Err(RouteError::Unauthorized));
    assert!(auth.authorize("r1"));
    assert!(!auth.authorize("r1"), "a second authorization changes nothing");
    auth.set_active("r1", true).unwrap();
    let rows = destinations(&[], &[("r1", "Attic")], &auth);
    assert!(rows[0].active);
}

#[test]
fn invalidating_a_route_deactivates_it_and_reports_the_change_once() {
    let mut auth = RouteAuth::new();
    auth.authorize("r1");
    auth.set_active("r1", true).unwrap();
    assert!(auth.take_changed());
    assert!(!auth.take_changed(), "reading the flag clears it");
    assert!(auth.invalidate("r1"));
    assert!(!auth.is_active("r1"));
    assert!(!auth.is_authorized("r1"));
    assert!(auth.take_changed());
    assert!(!auth.invalidate("r1"));
}

#[test]
fn the_picker_button_is_highlighted_by_its_style_and_its_route() {
    let mut view = RoutePickerView::new();
    assert_eq!(view.style(), ButtonStyle::System);
    assert!(!view.is_highlighted());
    view.set_route_active(true);
    assert!(view.is_highlighted());
    view.set_style(ButtonStyle::Inactive);
    assert!(!view.is_highlighted(), "a pinned style ignores the route");
}

#[test]
fn the_routing_menu_sends_each_delegate_event_once() {
    let mut view = RoutePickerView::new();
    assert_eq!(view.end_presenting(), None);
    assert_eq!(view.begin_presenting(), Some(Event::WillBeginPresentingRoutes));
    assert_eq!(view.begin_presenting(), None);
    assert!(view.is_presenting());
    assert_eq!(view.end_presenting(), Some(Event::DidEndPresentingRoutes));
    assert!(!view.is_presenting());
}
