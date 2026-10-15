//! Parity tests (sdk-tests-08) for the package placement rules and the review prompt policy. Pure host tests: no display,
//! no device and no clock. Each test drives the public API of the module it names.

use bifrost_app::review::{Policy, ReviewRequests};

#[test]
fn review_first_ask_is_allowed_and_recorded() {
    let mut r = ReviewRequests::new();
    assert!(r.request(100, Policy::default()));
    assert_eq!(r.days(), &[100]);
}

#[test]
fn review_minimum_gap_blocks_an_early_ask() {
    let mut r = ReviewRequests::new();
    let p = Policy::default();
    assert!(r.request(100, p));
    assert!(!r.request(150, p), "50 days is less than the 120-day gap");
    assert!(!r.request(100, p), "the same day is never a second ask");
    assert!(r.request(220, p));
    assert_eq!(r.days(), &[100, 220]);
}

#[test]
fn review_max_asks_caps_the_asks_inside_the_window() {
    let mut r = ReviewRequests::new();
    let p = Policy { min_gap: 0, max_asks: 2, window: 30 };
    assert!(r.request(0, p));
    assert!(r.request(1, p));
    assert!(!r.request(2, p), "two asks already fall in the window");
    assert!(r.request(30, p), "the ask on day 0 has left the 30-day window");
}

#[test]
fn review_restored_days_are_sorted_and_checked() {
    let r = ReviewRequests::from_days(vec![300, 100]);
    assert_eq!(r.days(), &[100, 300]);
    let p = Policy::default();
    assert!(!r.allowed(350, p), "only 50 days after the last ask");
    assert!(r.allowed(430, p), "130 days after the last ask, and two asks fall inside the year");
}

#[test]
fn review_zero_max_asks_never_asks() {
    let mut r = ReviewRequests::new();
    assert!(!r.request(5, Policy { min_gap: 0, max_asks: 0, window: 365 }));
    assert!(r.days().is_empty());
}

/// The first app intent end to end at the SDK level: declaring it, its Search shortcut entry, its manifest table and the
/// checks on the parameters a call may send. Plain data only; no app is launched and no socket is opened.
mod first_intent {
    use bifrost_app::intents::{Authentication, Intent, IntentMode, IntentResult, Param, ParamType, Session};
    use bifrost_core::json::Json;

    fn new_note() -> Intent {
        Intent::new("new-note", "New Note", "Creates an empty note")
    }

    fn look_up() -> Intent {
        Intent::new("look-up", "Look Up", "Finds a word").shortcut().param(Param::new("word", "Word", ParamType::String))
    }

    #[test]
    fn a_new_intent_runs_in_the_background_and_is_not_a_shortcut() {
        let intent = new_note();
        assert!(!intent.shortcut);
        assert_eq!(intent.modes, vec![IntentMode::Background]);
        assert!(intent.supports(IntentMode::Background));
        assert!(!intent.supports(IntentMode::Foreground));
        assert_eq!(intent.authentication, Authentication::AlwaysAllowed);
    }

    #[test]
    fn the_rpc_method_of_an_intent_is_intent_and_its_id() {
        assert_eq!(new_note().method(), "intent.new-note");
    }

    #[test]
    fn foreground_is_added_once_and_only_when_asked() {
        let intent = new_note().foreground().foreground();
        assert_eq!(intent.modes, vec![IntentMode::Background, IntentMode::Foreground]);
    }

    #[test]
    fn a_parameterless_shortcut_has_an_empty_parameter_list_in_its_manifest_table() {
        let text = new_note().shortcut().manifest_toml();
        assert_eq!(
            text,
            "[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\ndescription = \"Creates an empty note\"\nshortcut = true\nparams = []\n"
        );
    }

    #[test]
    fn manifest_parameters_carry_their_type_and_an_optional_marker() {
        let intent = Intent::new("add-task", "Add Task", "Adds a task")
            .shortcut()
            .param(Param::new("title", "Title", ParamType::String))
            .param(Param::new("due", "Due", ParamType::Date).optional());
        let text = intent.manifest_toml();
        assert!(text.contains("params = [\"title:string\", \"due:date?\"]\n"), "{text}");
        assert!(text.contains("shortcut = true\n"), "{text}");
    }

    #[test]
    fn a_manifest_table_escapes_quotes_in_its_text_for_toml() {
        let intent = Intent::new("say", "Say \"Hi\"", "Says \"hi\" aloud");
        let text = intent.manifest_toml();
        assert!(text.contains("title = \"Say 'Hi'\"\n"), "{text}");
        assert!(text.contains("description = \"Says 'hi' aloud\"\n"), "{text}");
        assert!(text.contains("shortcut = false\n"), "{text}");
    }

    #[test]
    fn the_json_description_lists_the_shortcut_modes_and_parameters() {
        let json = look_up().to_json();
        assert!(matches!(json.get("shortcut"), Some(Json::Bool(true))));
        assert!(matches!(json.get("authentication"), Some(Json::Str(s)) if s == "always-allowed"));
        match json.get("parameters") {
            Some(Json::Arr(params)) => assert_eq!(params.len(), 1),
            other => panic!("parameters missing: {other:?}"),
        }
        assert!(matches!(json.get("modes"), Some(Json::Arr(modes)) if modes.len() == 1));
    }

    #[test]
    fn an_intent_without_parameters_accepts_no_input_or_an_empty_object() {
        assert_eq!(new_note().validate(&Json::Null), Ok(()));
        assert_eq!(new_note().validate(&Json::Obj(vec![])), Ok(()));
        let err = new_note().validate(&Json::Obj(vec![("word".into(), Json::Str("cat".into()))])).unwrap_err();
        assert!(err.message.contains("unknown parameter word"), "{}", err.message);
    }

    #[test]
    fn a_required_parameter_must_be_present_and_a_string() {
        let intent = look_up();
        let missing = intent.validate(&Json::Null).unwrap_err();
        assert!(missing.message.contains("missing parameter word (string)"), "{}", missing.message);
        assert_eq!(intent.validate(&Json::Obj(vec![("word".into(), Json::Str("cat".into()))])), Ok(()));
        let wrong_type = intent.validate(&Json::Obj(vec![("word".into(), Json::Bool(true))])).unwrap_err();
        assert!(wrong_type.message.contains("parameter word must be string"), "{}", wrong_type.message);
    }

    #[test]
    fn an_optional_parameter_may_be_absent() {
        let intent = Intent::new("add-task", "Add Task", "Adds a task").param(Param::new("due", "Due", ParamType::Date).optional());
        assert_eq!(intent.validate(&Json::Null), Ok(()));
        assert_eq!(intent.validate(&Json::Obj(vec![("due".into(), Json::Null)])), Ok(()));
    }

    #[test]
    fn an_intent_that_needs_the_device_unlocked_runs_only_when_unlocked() {
        let intent = new_note().authentication(Authentication::RequiresLocalDeviceAuthentication);
        assert!(intent.may_run(Session { locked: false, authenticated: false }));
        assert!(!intent.may_run(Session { locked: true, authenticated: true }));
        assert!(new_note().may_run(Session { locked: true, authenticated: false }));
    }

    #[test]
    fn a_continue_in_foreground_result_needs_the_foreground_mode() {
        let declared = vec![new_note()];
        let result = IntentResult::dialog("Created").continue_in_foreground();
        assert!(new_note().check_result(&result, &declared).is_err());
        assert_eq!(new_note().foreground().check_result(&result, &declared), Ok(()));
    }

    #[test]
    fn a_result_may_open_only_a_declared_intent_with_valid_parameters() {
        let declared = vec![new_note(), look_up()];
        let good = IntentResult::dialog("Opening").opens("look-up", Json::Obj(vec![("word".into(), Json::Str("cat".into()))]));
        assert_eq!(new_note().check_result(&good, &declared), Ok(()));

        let undeclared = IntentResult::dialog("Opening").opens("delete-all", Json::Null);
        assert!(new_note().check_result(&undeclared, &declared).is_err());

        let bad_params = IntentResult::dialog("Opening").opens("look-up", Json::Null);
        assert!(new_note().check_result(&bad_params, &declared).is_err());
    }
}
