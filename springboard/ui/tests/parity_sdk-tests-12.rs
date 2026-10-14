//! Parity sdk-tests-12: headless tests of the widget rendering modes, widget actions, permission questions, symbol
//! effects and the 2D scene model. Each test drives the public API of one module and draws nothing: the models take
//! colours, times and points as plain values, so no window, device or display server is involved.

use bifrost_ui::permission_question::{
    AskCenter, AskError, ButtonState, Handle, PermissionButton, PermissionQuestion, PermissionResponse, QuestionTopic,
    MAX_CHOICES, MAX_HANDLES, MAX_HANDLE_LEN, MAX_PENDING,
};
use bifrost_ui::scene2d::{positional_mix, FrameClock, FramePhase, Mix, ScaleMode, Scene2d, FRAME_PHASES};
use bifrost_ui::symbol_effect::{replace_poses, Effect, Options, Pose, Repeat, Replace, SymbolEffect};
use bifrost_ui::widget_action::{hit_test, Action, Hit, Rect, Toggle};
use bifrost_ui::widget_link::{Link, LinkError};
use bifrost_ui::widget_render::{luminance, resolve_mode, tint, RenderingMode, Rgba};

const BLUE: Rgba = Rgba(10, 132, 255, 255);
const WHITE: Rgba = Rgba(255, 255, 255, 255);
const ORANGE: Rgba = Rgba(255, 149, 0, 255);

// ---- widget rendering modes ----

#[test]
fn headless_a_widget_draws_in_full_colour_unless_the_look_is_tinted() {
    assert_eq!(RenderingMode::default(), RenderingMode::FullColor);
    assert_eq!(resolve_mode(RenderingMode::Accented, false), RenderingMode::FullColor);
    assert_eq!(resolve_mode(RenderingMode::Vibrant, false), RenderingMode::FullColor);
    assert_eq!(resolve_mode(RenderingMode::Accented, true), RenderingMode::Accented);
    assert_eq!(resolve_mode(RenderingMode::Vibrant, true), RenderingMode::Vibrant);
    assert_eq!(resolve_mode(RenderingMode::FullColor, true), RenderingMode::FullColor);
}

#[test]
fn headless_full_colour_mode_leaves_every_element_as_drawn() {
    assert_eq!(tint(BLUE, true, ORANGE, RenderingMode::FullColor), BLUE);
    assert_eq!(tint(BLUE, false, ORANGE, RenderingMode::FullColor), BLUE);
}

#[test]
fn headless_the_accented_mode_gives_only_accentable_elements_the_accent() {
    assert_eq!(tint(BLUE, true, ORANGE, RenderingMode::Accented), ORANGE);
    assert_eq!(tint(BLUE, false, ORANGE, RenderingMode::Accented), BLUE, "a plain element keeps its colour");
}

#[test]
fn headless_the_accented_mode_keeps_the_alpha_of_each_element() {
    let faint = Rgba(10, 132, 255, 200);
    assert_eq!(tint(faint, true, ORANGE, RenderingMode::Accented), Rgba(255, 149, 0, 200));
    let clear = Rgba(10, 132, 255, 0);
    assert_eq!(tint(clear, true, ORANGE, RenderingMode::Accented), Rgba(255, 149, 0, 0));
}

#[test]
fn headless_the_vibrant_mode_reduces_each_element_to_a_grey_of_its_luminance() {
    assert_eq!(tint(WHITE, false, BLUE, RenderingMode::Vibrant), WHITE);
    assert_eq!(tint(Rgba(255, 0, 0, 255), false, BLUE, RenderingMode::Vibrant), Rgba(76, 76, 76, 255));
    assert_eq!(tint(Rgba(0, 0, 255, 255), true, BLUE, RenderingMode::Vibrant), Rgba(29, 29, 29, 255));
}

#[test]
fn headless_the_vibrant_mode_keeps_the_alpha_of_each_pixel() {
    let faded = Rgba(255, 255, 255, 40);
    assert_eq!(tint(faded, false, BLUE, RenderingMode::Vibrant), Rgba(255, 255, 255, 40));
}

#[test]
fn headless_pixel_luminance_uses_the_integer_rec601_weights() {
    assert_eq!(luminance(Rgba(0, 0, 0, 255)), 0);
    assert_eq!(luminance(WHITE), 255);
    assert_eq!(luminance(Rgba(255, 0, 0, 255)), 76, "299 * 255 / 1000");
    assert_eq!(luminance(Rgba(0, 255, 0, 255)), 149, "587 * 255 / 1000");
    assert_eq!(luminance(Rgba(0, 0, 255, 255)), 29, "114 * 255 / 1000");
}

// ---- widget actions and taps ----

fn tile(x: f32, y: f32, w: f32, h: f32, action: Action) -> Hit {
    Hit { frame: Rect { x, y, w, h }, action }
}

#[test]
fn headless_a_rectangle_includes_its_left_and_top_edges_only() {
    let r = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
    assert!(r.contains(0.0, 0.0));
    assert!(r.contains(9.5, 9.5));
    assert!(!r.contains(10.0, 5.0));
    assert!(!r.contains(5.0, 10.0));
    assert!(!r.contains(-0.1, 5.0));
}

#[test]
fn headless_the_topmost_tapped_region_gives_its_action() {
    let hits = vec![
        tile(0.0, 0.0, 100.0, 100.0, Action::RunIntent { id: "under".into() }),
        tile(10.0, 10.0, 20.0, 20.0, Action::SetToggle { id: "wifi".into(), on: true }),
    ];
    assert_eq!(hit_test(&hits, 15.0, 15.0), Some(&Action::SetToggle { id: "wifi".into(), on: true }));
    assert_eq!(hit_test(&hits, 50.0, 50.0), Some(&Action::RunIntent { id: "under".into() }));
    assert_eq!(hit_test(&hits, 150.0, 50.0), None, "a tap outside every region does nothing");
}

#[test]
fn headless_a_tapped_link_region_opens_its_app_and_path() {
    let link = Link::parse("bifrost-app://notes/today").unwrap();
    let hits = vec![tile(0.0, 0.0, 50.0, 50.0, Action::OpenLink(link))];
    match hit_test(&hits, 1.0, 1.0) {
        Some(Action::OpenLink(l)) => {
            assert_eq!(l.app(), "notes");
            assert_eq!(l.path(), "/today");
            assert_eq!(l.to_url(), "bifrost-app://notes/today");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn headless_a_widget_link_must_use_the_app_scheme_and_a_safe_path() {
    assert_eq!(Link::parse("https://example.org/a"), Err(LinkError::NotAppScheme));
    assert_eq!(Link::parse("bifrost-app:///a"), Err(LinkError::MissingApp));
    assert_eq!(Link::parse("bifrost-app://notes/a/../b"), Err(LinkError::BadPath));
    assert_eq!(Link::parse("bifrost-app://notes").unwrap().path(), "/", "no path goes to the app root");
}

#[test]
fn headless_a_toggle_shows_the_new_state_at_once_and_waits_for_the_app() {
    let mut t = Toggle::new(false);
    assert_eq!(t.tap(), Some(true));
    assert!(t.on());
    assert!(t.is_pending());
    assert_eq!(t.tap(), None, "a second tap waits for the first change");
    assert!(t.on(), "the shown state does not flip while a change is pending");
}

#[test]
fn headless_a_refused_toggle_change_puts_the_committed_state_back() {
    let mut t = Toggle::new(true);
    t.tap();
    assert!(!t.on());
    t.settle(false);
    assert!(t.on());
    assert!(!t.is_pending());
    assert_eq!(t.tap(), Some(false));
}

#[test]
fn headless_an_accepted_toggle_change_is_committed_and_a_settle_with_nothing_pending_does_nothing() {
    let mut t = Toggle::new(false);
    t.settle(false);
    assert!(!t.on());
    assert_eq!(t.tap(), Some(true));
    t.settle(true);
    assert!(t.on());
    t.settle(false);
    assert!(t.on(), "a settle with no change pending changes nothing");
}

// ---- permission questions: creating a question ----

fn handle(text: &str) -> Handle {
    Handle::new(text).expect("a valid contact handle")
}

fn two_contacts() -> Vec<Handle> {
    vec![handle("parent@example.org"), handle("+3545550100")]
}

fn choice_ids<T: QuestionTopic>(q: &PermissionQuestion<T>) -> Vec<&str> {
    q.choices().iter().map(String::as_str).collect()
}

#[test]
fn headless_a_communication_question_needs_one_to_thirty_two_contacts() {
    assert!(matches!(PermissionQuestion::communication(1, &[], None), Err(AskError::InvalidQuestion)));
    let many: Vec<Handle> = (0..MAX_HANDLES + 1).map(|i| handle(&format!("contact{i}"))).collect();
    assert!(matches!(PermissionQuestion::communication(1, &many, None), Err(AskError::InvalidQuestion)));
    let q = PermissionQuestion::communication(1, &many[..MAX_HANDLES], None).unwrap();
    assert_eq!(q.handles().len(), MAX_HANDLES);
    assert_eq!(q.handles()[0].as_str(), "contact0");
}

#[test]
fn headless_a_new_question_offers_approve_and_decline_with_decline_selected() {
    let q = PermissionQuestion::communication(2, &two_contacts(), None).unwrap();
    assert_eq!(choice_ids(&q), vec!["approve", "decline"]);
    assert_eq!(q.default_choice(), "decline", "a child never gets a yes by accident");
    let update = PermissionQuestion::significant_app_update(3, Some(900));
    assert_eq!(choice_ids(&update), vec!["approve", "decline"]);
    assert_eq!(update.default_choice(), "decline");
    assert_eq!(update.expires_at(), Some(900));
}

#[test]
fn headless_a_contact_handle_is_short_non_blank_and_free_of_control_characters() {
    assert!(Handle::new("   ").is_err());
    assert!(Handle::new("a\nb").is_err());
    assert!(Handle::new(&"x".repeat(MAX_HANDLE_LEN + 1)).is_err());
    assert!(Handle::new(&"x".repeat(MAX_HANDLE_LEN)).is_ok());
    assert_eq!(Handle::new("  kid  ").unwrap().as_str(), "  kid  ", "the handle is kept as given");
}

#[test]
fn headless_custom_choices_need_unique_non_blank_ids_and_a_valid_default() {
    let base = || PermissionQuestion::significant_app_update(4, None);
    assert!(base().with_choices(&[], 0).is_err());
    assert!(base().with_choices(&["a", "a"], 0).is_err(), "ids are unique");
    assert!(base().with_choices(&["a", " "], 0).is_err(), "ids are not blank");
    assert!(base().with_choices(&["a", "b"], 2).is_err(), "the default is one of the choices");
    let too_many: Vec<String> = (0..=MAX_CHOICES).map(|i| format!("c{i}")).collect();
    let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert!(base().with_choices(&refs, 0).is_err(), "at most {MAX_CHOICES} choices");
    let yes_no = base().with_choices(&["yes", "no"], 0).unwrap();
    assert_eq!(yes_no.default_choice(), "yes");
    assert_eq!(choice_ids(&yes_no), vec!["yes", "no"]);
}

#[test]
fn headless_ask_errors_describe_themselves_in_english() {
    assert_eq!(AskError::SystemError("disk full".into()).description(), "System error: disk full");
    assert_eq!(AskError::NotAvailable.description(), "Permission questions are not available.");
    assert_eq!(AskError::ContactSyncNotSetup.description(), "Contact sync is not set up.");
}

// ---- permission questions: the queue and the responses ----

#[test]
fn headless_an_asked_question_is_answered_once_and_leaves_the_queue() {
    let mut center = AskCenter::new();
    let q = PermissionQuestion::communication(9, &two_contacts(), Some(100)).unwrap();
    center.ask(&q, 10).unwrap();
    assert!(center.is_pending(9));
    assert_eq!(center.pending_count(), 1);
    let r = center.answer(9, "approve", 20).unwrap();
    assert_eq!(
        r,
        PermissionResponse { question_id: 9, topic: "communication", choice: "approve".to_string(), answered_at: 20 }
    );
    assert!(!center.is_pending(9));
    assert_eq!(center.pending_count(), 0);
    assert_eq!(center.answered_count(), 1);
    assert!(matches!(center.answer(9, "approve", 21), Err(AskError::InvalidQuestion)));
}

#[test]
fn headless_an_expired_question_is_refused_on_ask_and_on_answer() {
    let mut center = AskCenter::new();
    let late = PermissionQuestion::communication(1, &two_contacts(), Some(100)).unwrap();
    assert!(matches!(center.ask(&late, 100), Err(AskError::InvalidQuestion)));
    let q = PermissionQuestion::communication(2, &two_contacts(), Some(60)).unwrap();
    center.ask(&q, 50).unwrap();
    assert!(matches!(center.answer(2, "approve", 60), Err(AskError::InvalidQuestion)));
    assert!(!center.is_pending(2), "an expired question is dropped when answered late");
}

#[test]
fn headless_a_choice_the_question_does_not_offer_keeps_it_waiting() {
    let mut center = AskCenter::new();
    let q = PermissionQuestion::communication(5, &two_contacts(), None).unwrap();
    center.ask(&q, 0).unwrap();
    assert!(matches!(center.answer(5, "maybe", 1), Err(AskError::InvalidQuestion)));
    assert!(center.is_pending(5));
    assert_eq!(center.answer(5, "decline", 2).unwrap().choice, "decline");
}

#[test]
fn headless_a_question_id_that_is_already_waiting_is_refused() {
    let mut center = AskCenter::new();
    let q = PermissionQuestion::communication(6, &two_contacts(), None).unwrap();
    center.ask(&q, 0).unwrap();
    assert!(matches!(center.ask(&q, 1), Err(AskError::InvalidQuestion)));
    assert_eq!(center.pending_count(), 1);
}

#[test]
fn headless_the_queue_holds_at_most_sixteen_questions() {
    let mut center = AskCenter::new();
    for i in 0..MAX_PENDING as u64 {
        let q = PermissionQuestion::significant_app_update(100 + i, None);
        center.ask(&q, 0).unwrap();
    }
    assert_eq!(center.pending_count(), MAX_PENDING);
    let extra = PermissionQuestion::significant_app_update(999, None);
    assert!(matches!(center.ask(&extra, 0), Err(AskError::SystemError(_))));
    assert_eq!(center.pending_count(), MAX_PENDING);
}

#[test]
fn headless_expire_drops_only_the_questions_past_their_deadline() {
    let mut center = AskCenter::new();
    center.ask(&PermissionQuestion::significant_app_update(1, Some(10)), 0).unwrap();
    center.ask(&PermissionQuestion::significant_app_update(2, None), 0).unwrap();
    center.ask(&PermissionQuestion::significant_app_update(3, Some(50)), 0).unwrap();
    assert_eq!(center.expire(10), 1);
    assert!(!center.is_pending(1));
    assert!(center.is_pending(2) && center.is_pending(3));
    assert_eq!(center.expire(50), 1);
    assert_eq!(center.pending_count(), 1);
}

#[test]
fn headless_responses_are_read_per_topic_in_arrival_order() {
    let mut center = AskCenter::new();
    center.ask(&PermissionQuestion::communication(1, &two_contacts(), None).unwrap(), 0).unwrap();
    center.ask(&PermissionQuestion::communication(2, &two_contacts(), None).unwrap(), 0).unwrap();
    center.ask(&PermissionQuestion::significant_app_update(3, None), 0).unwrap();
    center.answer(2, "decline", 5).unwrap();
    center.answer(3, "approve", 6).unwrap();
    center.answer(1, "approve", 7).unwrap();

    let communication = center.take_responses("communication");
    let ids: Vec<u64> = communication.iter().map(|r| r.question_id).collect();
    assert_eq!(ids, vec![2, 1], "answers come back in the order they were given");
    let update = center.take_responses("significant-app-update");
    assert_eq!(update.len(), 1);
    assert_eq!(update[0].choice, "approve");
    assert!(center.take_responses("communication").is_empty(), "a taken answer is not returned twice");
    assert_eq!(center.answered_count(), 0);
}

#[test]
fn headless_a_single_answer_can_be_read_and_then_taken_by_its_question() {
    let mut center = AskCenter::new();
    center.ask(&PermissionQuestion::significant_app_update(4, None), 0).unwrap();
    center.answer(4, "approve", 3).unwrap();
    assert_eq!(center.response_for(4).map(|r| r.choice.as_str()), Some("approve"));
    assert_eq!(center.answered_count(), 1, "reading does not remove the answer");
    let taken = center.take_response(4).unwrap();
    assert_eq!(taken.answered_at, 3);
    assert!(center.take_response(4).is_none());
    assert!(center.response_for(4).is_none());
}

struct LocationTopic;

impl QuestionTopic for LocationTopic {
    fn topic_name(&self) -> &'static str {
        "location"
    }
}

#[test]
fn headless_a_custom_topic_question_is_answered_and_routed_by_its_topic_name() {
    let mut center = AskCenter::new();
    let q = PermissionQuestion::for_topic(LocationTopic, 5, None).with_choices(&["allow", "dont_allow"], 1).unwrap();
    assert_eq!(q.default_choice(), "dont_allow");
    center.ask(&q, 0).unwrap();
    center.answer(5, "allow", 1).unwrap();
    assert!(center.take_responses("communication").is_empty());
    let routed = center.take_responses("location");
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].topic, "location");
    assert_eq!(routed[0].choice, "allow");
}

// ---- permission buttons ----

#[test]
fn headless_a_permission_button_sends_its_question_once_and_reads_the_answer_back() {
    let mut center = AskCenter::new();
    let question = PermissionQuestion::significant_app_update(11, Some(100)).with_choices(&["yes", "no"], 0).unwrap();
    let mut button = PermissionButton::new(question, "Ask a Parent");
    assert_eq!(button.label(), "Ask a Parent");
    assert_eq!(button.state(&center, 0), ButtonState::Ready);
    button.press(&mut center, 0).unwrap();
    assert_eq!(button.state(&center, 1), ButtonState::Waiting);
    assert!(button.press(&mut center, 2).is_err(), "a button sends its question once");
    center.answer(11, "no", 5).unwrap();
    assert_eq!(button.state(&center, 6), ButtonState::Answered("no".to_string()));
    let got = button.collect(&mut center).unwrap();
    assert_eq!(got.choice, "no");
    assert!(button.collect(&mut center).is_none(), "the answer is collected once");
    assert_eq!(button.state(&center, 7), ButtonState::Answered("no".to_string()));
}

#[test]
fn headless_a_permission_button_whose_question_expires_unanswered_shows_expired() {
    let mut center = AskCenter::new();
    let question = PermissionQuestion::significant_app_update(12, Some(10));
    let mut button = PermissionButton::new(question, "Ask");
    button.press(&mut center, 0).unwrap();
    assert_eq!(button.state(&center, 9), ButtonState::Waiting);
    assert_eq!(button.state(&center, 10), ButtonState::Expired);
}

#[test]
fn headless_a_permission_button_refuses_to_send_an_expired_question() {
    let mut center = AskCenter::new();
    let question = PermissionQuestion::significant_app_update(13, Some(10));
    let mut button = PermissionButton::new(question, "Ask");
    assert!(matches!(button.press(&mut center, 10), Err(AskError::InvalidQuestion)));
    assert!(!center.is_pending(13));
}

// ---- symbol effects ----

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn headless_a_bounce_swells_to_one_point_one_five_at_mid_cycle_then_rests() {
    let e = SymbolEffect::new(Effect::Bounce, Options::default(), 0.0);
    assert_eq!(e.pose(0.0), Pose::REST);
    assert!(close(e.pose(0.3).scale, 1.15), "scale {}", e.pose(0.3).scale);
    assert!(!e.is_done(0.5));
    assert!(e.is_done(0.7));
    assert_eq!(e.pose(0.7), Pose::REST, "after its cycle a bounce rests");
}

#[test]
fn headless_a_bounce_at_double_speed_reaches_mid_cycle_in_half_the_time() {
    let e = SymbolEffect::new(Effect::Bounce, Options::default().speed(2.0), 1.0);
    assert!(close(e.pose(1.15).scale, 1.15));
    assert_eq!(e.pose(0.5), Pose::REST, "nothing plays before the start");
}

#[test]
fn headless_a_pulse_dims_to_half_opacity_and_returns_to_full() {
    let e = SymbolEffect::new(Effect::Pulse, Options::default(), 0.0);
    assert!(close(e.pose(0.0).opacity, 1.0));
    assert!(close(e.pose(0.45).opacity, 0.5));
    assert!(close(e.pose(0.9).opacity, 1.0), "the cycle has ended");
    assert_eq!(e.pose(0.2).scale, 1.0, "a pulse changes opacity only");
}

#[test]
fn headless_a_repeating_effect_never_finishes_and_a_counted_one_plays_its_cycles() {
    let forever = SymbolEffect::new(Effect::Wiggle, Options::repeating(), 0.0);
    assert_eq!(forever.cycles(), None);
    assert!(!forever.is_done(1.0e6));
    let counted = SymbolEffect::new(Effect::Pulse, Options::repeat(2), 0.0);
    assert_eq!(counted.cycles(), Some(3), "one cycle plus two repeats");
    assert!(!counted.is_done(2.6));
    assert!(counted.is_done(2.8));
    assert_eq!(Options::repeat(4).repeat, Repeat::Count(4));
}

#[test]
fn headless_a_speed_that_is_not_positive_or_finite_falls_back_to_one() {
    assert_eq!(Options::default().speed(0.0).speed, 1.0);
    assert_eq!(Options::default().speed(f32::INFINITY).speed, 1.0);
    assert_eq!(Options::default().speed(3.0).speed, 3.0);
}

#[test]
fn headless_disappear_ends_hidden_and_appear_starts_small_and_ends_drawn() {
    let gone = SymbolEffect::new(Effect::Disappear, Options::default(), 0.0);
    assert_eq!(gone.pose(5.0), Pose::HIDDEN);
    assert!(close(gone.pose(0.0).scale, 1.0));
    let shown = SymbolEffect::new(Effect::Appear, Options::default(), 0.0);
    assert_eq!(shown.pose(5.0), Pose::REST);
    assert!(close(shown.pose(0.0).scale, 0.6), "appear starts at 0.6 scale");
    assert!(close(shown.pose(0.0).opacity, 0.0), "and fully transparent");
}

#[test]
fn headless_a_breathing_effect_stays_within_eight_percent_of_full_size() {
    let e = SymbolEffect::new(Effect::Breathe, Options::repeating(), 0.0);
    for k in 0..400 {
        let s = e.pose(k as f64 * 0.05).scale;
        assert!((0.919..=1.081).contains(&s), "scale {s} at step {k}");
    }
}

#[test]
fn headless_a_replace_draws_one_symbol_at_a_time_and_hands_over_at_half_way() {
    for variant in [Replace::DownUp, Replace::OffUp, Replace::UpUp] {
        let (out, incoming) = replace_poses(variant, 0.25);
        assert!(out.opacity > 0.0 && incoming == Pose::HIDDEN, "{variant:?} before the hand-over");
        let (out, incoming) = replace_poses(variant, 0.75);
        assert!(out == Pose::HIDDEN && incoming.opacity > 0.0, "{variant:?} after the hand-over");
    }
    let (out0, in0) = replace_poses(Replace::DownUp, 0.0);
    assert_eq!((out0.opacity, in0), (1.0, Pose::HIDDEN));
    let (out1, in1) = replace_poses(Replace::DownUp, 1.0);
    assert_eq!(out1, Pose::HIDDEN);
    assert!(close(in1.opacity, 1.0) && close(in1.scale, 1.0));
}

// ---- the 2D scene model ----

#[test]
fn headless_the_scale_modes_map_a_400_by_200_scene_into_an_800_square_view() {
    let mut s = Scene2d::new(400.0, 200.0);
    s.set_view_size(800.0, 800.0);
    s.set_scale_mode(ScaleMode::Fill);
    assert_eq!(s.scale(), (2.0, 4.0), "each axis on its own");
    s.set_scale_mode(ScaleMode::AspectFill);
    assert_eq!(s.scale(), (4.0, 4.0), "one factor, the larger of 2 and 4");
    s.set_scale_mode(ScaleMode::AspectFit);
    assert_eq!(s.scale(), (2.0, 2.0), "one factor, the smaller of 2 and 4");
    s.set_scale_mode(ScaleMode::ResizeFill);
    assert_eq!(s.scale(), (1.0, 1.0));
    assert_eq!(s.size(), (800.0, 800.0), "resize takes the view's size");
    assert_eq!(s.scale_mode(), ScaleMode::ResizeFill);
}

#[test]
fn headless_an_aspect_fit_scene_is_centred_by_the_anchor() {
    let mut s = Scene2d::new(400.0, 200.0);
    s.set_view_size(800.0, 800.0);
    s.set_scale_mode(ScaleMode::AspectFit);
    s.set_anchor(0.5, 0.5);
    assert_eq!(s.origin(), (0.0, 200.0), "the free height of 400 is split in half");
    assert_eq!(s.scene_to_view(0.0, 0.0), (0.0, 200.0));
}

#[test]
fn headless_an_aspect_fill_scene_is_cropped_by_the_anchor_and_maps_back() {
    let mut s = Scene2d::new(400.0, 200.0);
    s.set_view_size(800.0, 800.0);
    s.set_scale_mode(ScaleMode::AspectFill);
    s.set_anchor(0.5, 0.5);
    assert_eq!(s.origin(), (-400.0, 0.0), "the scaled scene is 800 wider than the view");
    assert_eq!(s.scene_to_view(100.0, 0.0), (0.0, 0.0));
    assert_eq!(s.view_to_scene(0.0, 0.0), (100.0, 0.0));
}

#[test]
fn headless_view_and_scene_points_convert_both_ways() {
    let mut s = Scene2d::new(300.0, 150.0);
    s.set_view_size(600.0, 600.0);
    s.set_scale_mode(ScaleMode::AspectFit);
    s.set_anchor(0.25, 0.75);
    let (vx, vy) = s.scene_to_view(37.5, 12.25);
    let (sx, sy) = s.view_to_scene(vx, vy);
    assert!((sx - 37.5).abs() < 1e-3 && (sy - 12.25).abs() < 1e-3, "round trip gave {sx}, {sy}");
}

#[test]
fn headless_the_anchor_is_kept_between_zero_and_one_and_nan_reads_as_zero() {
    let mut s = Scene2d::new(100.0, 100.0);
    s.set_view_size(200.0, 300.0);
    s.set_scale_mode(ScaleMode::AspectFit);
    s.set_anchor(5.0, -3.0);
    assert_eq!(s.origin(), (0.0, 0.0));
    s.set_anchor(-1.0, 9.0);
    assert_eq!(s.origin(), (0.0, 100.0));
    s.set_anchor(f32::NAN, 0.5);
    assert_eq!(s.origin(), (0.0, 50.0));
}

#[test]
fn headless_an_empty_or_negative_scene_never_divides_by_zero() {
    let mut empty = Scene2d::new(0.0, 0.0);
    empty.set_view_size(100.0, 50.0);
    assert_eq!(empty.scale(), (1.0, 1.0));
    let (x, y) = empty.scene_to_view(3.0, 4.0);
    assert!(x.is_finite() && y.is_finite());
    let negative = Scene2d::new(-5.0, 10.0);
    assert_eq!(negative.size(), (0.0, 10.0), "a negative size counts as 0");
}

#[test]
fn headless_the_frame_clock_steps_by_the_time_since_the_last_frame() {
    let mut clock = FrameClock::default();
    assert_eq!(clock.begin_frame(10.0), Some(0.0), "the first frame has no step");
    assert_eq!(clock.begin_frame(10.25), Some(0.25));
    assert_eq!(clock.begin_frame(9.0), Some(0.0), "a clock that goes back never steps negative");
}

#[test]
fn headless_a_paused_clock_skips_the_time_it_was_paused() {
    let mut clock = FrameClock::default();
    clock.begin_frame(0.0);
    clock.set_paused(true);
    assert!(clock.is_paused());
    assert_eq!(clock.begin_frame(5.0), None);
    clock.set_paused(false);
    assert_eq!(clock.begin_frame(6.0), Some(1.0), "the paused span is not simulated");
}

#[test]
fn headless_the_frame_phases_run_in_the_reference_order() {
    assert_eq!(
        FRAME_PHASES,
        [
            FramePhase::Update,
            FramePhase::EvaluateActions,
            FramePhase::SimulatePhysics,
            FramePhase::ApplyConstraints,
            FramePhase::FinishUpdate,
        ]
    );
}

#[test]
fn headless_a_positional_node_is_full_inside_its_reference_distance() {
    let on_edge = positional_mix((6.0, 8.0), (0.0, 0.0), true, 10.0, 110.0);
    assert_eq!(on_edge.volume, 1.0, "exactly the reference distance of 10");
    let near = positional_mix((105.0, 0.0), (100.0, 0.0), true, 10.0, 110.0);
    assert_eq!(near.volume, 1.0, "distance is measured from the listener");
    assert!((near.pan - 5.0 / 110.0).abs() < 1e-5);
}

#[test]
fn headless_a_positional_node_fades_linearly_between_reference_and_max() {
    let half = positional_mix((60.0, 0.0), (0.0, 0.0), true, 10.0, 110.0);
    assert!((half.volume - 0.5).abs() < 1e-5);
    let quiet = positional_mix((100.0, 0.0), (0.0, 0.0), true, 10.0, 110.0);
    assert!((quiet.volume - 0.1).abs() < 1e-5);
    assert!((half.pan - 60.0 / 110.0).abs() < 1e-5, "pan is the offset over max");
}

#[test]
fn headless_a_positional_node_beyond_max_is_silent_and_its_pan_clamps() {
    let far = positional_mix((-200.0, 0.0), (0.0, 0.0), true, 10.0, 110.0);
    assert_eq!((far.volume, far.pan), (0.0, -1.0));
    let past_reference = positional_mix((30.0, 0.0), (0.0, 0.0), true, 10.0, 10.0);
    assert_eq!((past_reference.volume, past_reference.pan), (0.0, 1.0), "max at the reference is silent beyond it");
    let no_range = positional_mix((5.0, 0.0), (0.0, 0.0), true, 0.0, 0.0);
    assert_eq!((no_range.volume, no_range.pan), (0.0, 0.0));
}

#[test]
fn headless_a_node_that_is_not_positional_plays_full_and_centred() {
    let flat = positional_mix((-200.0, 0.0), (0.0, 0.0), false, 10.0, 110.0);
    assert_eq!(flat, Mix { volume: 1.0, pan: 0.0 });
}
