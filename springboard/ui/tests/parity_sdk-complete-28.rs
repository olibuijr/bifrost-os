//! Parity sdk-complete-28: headless tests for popover tip rules, tooltip timing, the preview panel's transition
//! geometry, spoken replies to voice commands and the media play queue. Pure models: no clock is read, no speech
//! engine runs, no window is drawn.

use bifrost_ui::media_queue::PlaybackQueue;
use bifrost_ui::media_request::{PlaybackRequest, PlaybackAction};
use bifrost_ui::media_search::{MediaItem, MediaKind};
use bifrost_ui::preview_transition::{lerp_rect, PanelFrame};
use bifrost_ui::speech::Synthesizer;
use bifrost_ui::tip::{Invalidation, Rules, Tip, TipStore};
use bifrost_ui::tooltip::{Timing, Tooltips};
use bifrost_ui::voice_commands::{Command, Resolution, Target};
use bifrost_ui::voice_reply::{speak_reply, spoken_reply};
use bifrost_ui::Rect;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

// ---- popover tips: content and display rules (bifrost_ui::tip) ----

#[test]
fn a_tip_with_an_action_keeps_its_title_message_and_action_id() {
    let tip = Tip::new("share", "Share faster", "Use the share button").with_action("open-share", "Show Me");
    assert_eq!(tip.title, "Share faster");
    assert_eq!(tip.message, "Use the share button");
    let action = tip.action.expect("the tip offers an action");
    assert_eq!(action.id, "open-share");
    assert_eq!(action.title, "Show Me");
}

#[test]
fn a_tip_without_rules_stays_eligible_however_often_it_is_shown() {
    let tips = [Tip::new("hint", "Hint", "Try it")];
    let mut store = TipStore::new();
    for _ in 0..5 {
        assert_eq!(store.present(&tips).map(|t| t.id.as_str()), Some("hint"));
    }
    assert_eq!(store.display_count("hint"), 5);
}

#[test]
fn a_tip_stops_after_its_maximum_number_of_displays() {
    let rules = Rules { max_displays: Some(2), required_events: vec![] };
    let tips = [Tip::new("twice", "Twice", "Only two times").with_rules(rules)];
    let mut store = TipStore::new();
    assert!(store.present(&tips).is_some());
    assert!(store.present(&tips).is_some());
    assert!(store.present(&tips).is_none(), "the third offer is refused");
    assert_eq!(store.display_count("twice"), 2);
}

#[test]
fn a_tip_waits_until_its_required_event_has_been_donated_enough_times() {
    let rules = Rules { max_displays: None, required_events: vec![("editor-opened".to_string(), 3)] };
    let tips = [Tip::new("editor", "Editor", "Try the editor").with_rules(rules)];
    let mut store = TipStore::new();
    store.donate("editor-opened");
    store.donate("editor-opened");
    assert!(store.present(&tips).is_none(), "two of three events are not enough");
    store.donate("editor-opened");
    assert_eq!(store.event_count("editor-opened"), 3);
    assert!(store.present(&tips).is_some());
}

#[test]
fn an_invalidated_tip_is_never_presented_again() {
    let tips = [Tip::new("done", "Done", "Already learned")];
    let mut store = TipStore::new();
    store.invalidate("done", Invalidation::ActionPerformed);
    assert_eq!(store.invalidation("done"), Some(Invalidation::ActionPerformed));
    assert!(store.present(&tips).is_none());
}

#[test]
fn the_first_invalidation_reason_is_kept() {
    let mut store = TipStore::new();
    store.invalidate("tip", Invalidation::Dismissed);
    store.invalidate("tip", Invalidation::ActionPerformed);
    assert_eq!(store.invalidation("tip"), Some(Invalidation::Dismissed), "a dismissed tip stays dismissed");
}

#[test]
fn presenting_a_list_shows_only_the_first_eligible_tip() {
    let capped = Rules { max_displays: Some(1), required_events: vec![] };
    let tips = [
        Tip::new("first", "First", "One").with_rules(capped.clone()),
        Tip::new("second", "Second", "Two").with_rules(capped),
    ];
    let mut store = TipStore::new();
    assert_eq!(store.present(&tips).map(|t| t.id.as_str()), Some("first"));
    assert_eq!(store.present(&tips).map(|t| t.id.as_str()), Some("second"), "the first tip is used up");
    assert!(store.present(&tips).is_none());
}

#[test]
fn an_invalidated_tip_gives_way_to_the_next_tip_in_the_list() {
    let tips = [Tip::new("a", "A", "first"), Tip::new("b", "B", "second")];
    let mut store = TipStore::new();
    store.invalidate("a", Invalidation::Dismissed);
    assert_eq!(store.present(&tips).map(|t| t.id.as_str()), Some("b"));
}

// ---- tooltips: hover timing (bifrost_ui::tooltip) ----

fn tooltips() -> Tooltips {
    Tooltips::new(Timing { delay: 1.0, grace: 0.3 })
}

#[test]
fn a_tooltip_shows_only_after_the_delay() {
    let mut tip = tooltips();
    tip.enter(1, 0.0);
    assert_eq!(tip.tick(0.5), None);
    assert_eq!(tip.tick(1.0), Some(1));
    assert_eq!(tip.visible(), Some(1));
}

#[test]
fn leaving_before_the_delay_cancels_the_tooltip() {
    let mut tip = tooltips();
    tip.enter(1, 0.0);
    tip.leave(1, 0.4);
    assert_eq!(tip.tick(2.0), None);
}

#[test]
fn moving_to_a_neighbour_while_a_tooltip_shows_shows_the_next_one_at_once() {
    let mut tip = tooltips();
    tip.enter(1, 0.0);
    assert_eq!(tip.tick(1.0), Some(1));
    tip.leave(1, 1.2);
    tip.enter(2, 1.2);
    assert_eq!(tip.visible(), Some(2), "no second delay while a tooltip is open");
}

#[test]
fn entering_a_control_within_the_grace_period_after_a_tooltip_closed_shows_at_once() {
    let mut tip = tooltips();
    tip.enter(1, 0.0);
    tip.tick(1.0);
    tip.leave(1, 2.0);
    tip.enter(2, 2.2);
    assert_eq!(tip.visible(), Some(2));
}

#[test]
fn entering_a_control_after_the_grace_period_waits_for_the_delay_again() {
    let mut tip = tooltips();
    tip.enter(1, 0.0);
    tip.tick(1.0);
    tip.leave(1, 2.0);
    tip.enter(2, 2.5);
    assert_eq!(tip.visible(), None);
    assert_eq!(tip.tick(3.5), Some(2));
}

#[test]
fn leaving_a_control_that_is_not_the_current_one_changes_nothing() {
    let mut tip = tooltips();
    tip.enter(1, 0.0);
    tip.leave(2, 0.5);
    assert_eq!(tip.tick(1.0), Some(1));
}

#[test]
fn the_default_timing_waits_one_second_and_allows_a_three_tenths_grace() {
    let default = Timing::default();
    assert_eq!(default.delay, 1.0);
    assert_eq!(default.grace, 0.3);
}

// ---- preview panel: transition and full-screen geometry (bifrost_ui::preview_transition) ----

#[test]
fn the_panel_opens_from_the_thumbnail_frame_to_its_resting_frame() {
    let frame = PanelFrame::new(rect(100.0, 100.0, 800.0, 600.0));
    let source = rect(40.0, 60.0, 80.0, 60.0);
    let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
    assert_eq!(frame.transition(source, bounds, 20.0, 0.0), source);
    assert_eq!(frame.transition(source, bounds, 20.0, 1.0), rect(100.0, 100.0, 800.0, 600.0));
}

#[test]
fn the_transition_is_halfway_at_progress_one_half() {
    assert_eq!(lerp_rect(rect(0.0, 0.0, 100.0, 50.0), rect(200.0, 100.0, 300.0, 150.0), 0.5), rect(100.0, 50.0, 200.0, 100.0));
}

#[test]
fn an_overshooting_progress_is_clamped_to_the_end_frames() {
    let from = rect(0.0, 0.0, 10.0, 10.0);
    let to = rect(50.0, 50.0, 20.0, 20.0);
    assert_eq!(lerp_rect(from, to, -0.5), from);
    assert_eq!(lerp_rect(from, to, 1.5), to);
}

#[test]
fn full_screen_mode_fills_the_bounds_inset_by_the_margin_and_toggles_back() {
    let mut frame = PanelFrame::new(rect(10.0, 20.0, 300.0, 200.0));
    let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
    assert!(frame.toggle_full_screen());
    assert!(frame.is_full_screen());
    assert_eq!(frame.target(bounds, 40.0), rect(40.0, 40.0, 1840.0, 1000.0));
    assert!(!frame.toggle_full_screen());
    assert_eq!(frame.target(bounds, 40.0), rect(10.0, 20.0, 300.0, 200.0));
}

#[test]
fn a_margin_wider_than_the_bounds_gives_an_empty_full_screen_frame() {
    let mut frame = PanelFrame::new(rect(0.0, 0.0, 1.0, 1.0));
    frame.toggle_full_screen();
    let target = frame.target(rect(0.0, 0.0, 100.0, 50.0), 80.0);
    assert_eq!(target.w, 0.0);
    assert_eq!(target.h, 0.0);
}

// ---- voice commands: spoken replies (bifrost_ui::voice_reply) ----

fn targets() -> Vec<Target> {
    vec![
        Target { id: 1, name: "Save".to_string(), actionable: true },
        Target { id: 2, name: "Cancel".to_string(), actionable: true },
    ]
}

#[test]
fn a_click_that_found_its_control_is_confirmed_by_name() {
    let reply = spoken_reply(&Command::Click("save".to_string()), &Resolution::Matched(1), &targets());
    assert_eq!(reply.as_deref(), Some("Clicked Save."));
}

#[test]
fn an_ambiguous_click_asks_for_a_number() {
    let reply = spoken_reply(&Command::Click("save".to_string()), &Resolution::Ambiguous(vec![1, 3]), &targets());
    assert_eq!(reply.as_deref(), Some("More than one item is called save. Say its number."));
}

#[test]
fn a_click_on_an_unknown_name_says_it_cannot_be_found() {
    let reply = spoken_reply(&Command::Click("share".to_string()), &Resolution::NotFound, &targets());
    assert_eq!(reply.as_deref(), Some("I cannot find share."));
}

#[test]
fn a_phrase_that_is_not_a_command_gets_an_apology() {
    let reply = spoken_reply(&Command::Unknown, &Resolution::NotFound, &targets());
    assert_eq!(reply.as_deref(), Some("Sorry, I did not understand that."));
}

#[test]
fn silent_commands_get_no_spoken_reply() {
    use bifrost_ui::voice_commands::{Direction, Overlay};
    let silent = [
        Command::Dictate("hello".to_string()),
        Command::Key("return".to_string()),
        Command::Scroll(Direction::Down),
        Command::Back,
        Command::Show(Overlay::Numbers),
        Command::Hide(Overlay::Grid),
    ];
    for command in &silent {
        assert_eq!(spoken_reply(command, &Resolution::NotFound, &targets()), None, "{command:?} is silent");
    }
}

#[test]
fn a_reply_is_queued_on_the_synthesizer_in_the_chosen_language() {
    let mut synth = Synthesizer::new();
    let queued = speak_reply(&mut synth, &Command::Click("save".to_string()), &Resolution::Matched(1), &targets(), "en-GB");
    assert_eq!(queued.as_deref(), Some("Clicked Save."));
    assert!(synth.is_speaking());
    let utterance = synth.current().expect("the reply is the current utterance");
    assert_eq!(utterance.text, "Clicked Save.");
    assert_eq!(utterance.language.as_deref(), Some("en-GB"));
}

#[test]
fn a_silent_command_queues_nothing() {
    let mut synth = Synthesizer::new();
    let queued = speak_reply(&mut synth, &Command::Back, &Resolution::NotFound, &targets(), "en-GB");
    assert_eq!(queued, None);
    assert!(!synth.is_speaking());
}

// ---- playback requests: the play queue (bifrost_ui::media_queue) ----

fn song(id: &str, title: &str) -> MediaItem {
    MediaItem::new(id, MediaKind::Song, title)
}

fn catalogue() -> Vec<MediaItem> {
    let mut blue = song("s1", "Blue Hour");
    blue.artist = Some("Ann".to_string());
    vec![blue, song("s2", "Blue Tide"), song("s3", "Red Sand")]
}

#[test]
fn a_play_request_replaces_the_queue_and_starts_at_its_first_item() {
    let catalogue = catalogue();
    let request = PlaybackRequest::parse("play blue");
    let mut queue = PlaybackQueue::new();
    queue.apply(&PlaybackAction::Enqueue(vec![&catalogue[2]]));
    let action = request.plan(&catalogue, 10);
    let now = queue.apply(&action).map(|item| item.id.clone());
    assert_eq!(now.as_deref(), Some("s1"));
    assert_eq!(queue.len(), 2, "the old queued item is gone");
    assert_eq!(queue.upcoming().len(), 1);
    assert_eq!(queue.upcoming()[0].id, "s2");
}

#[test]
fn an_add_to_queue_request_keeps_the_playing_item() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    queue.apply(&PlaybackAction::Play(vec![&catalogue[0]]));
    let action = PlaybackRequest::parse("add red sand to queue").plan(&catalogue, 10);
    queue.apply(&action);
    assert_eq!(queue.current().map(|item| item.id.as_str()), Some("s1"), "queueing does not interrupt playback");
    assert_eq!(queue.upcoming().iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec!["s3"]);
}

#[test]
fn an_enqueue_into_an_empty_queue_starts_the_first_added_item() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    let action = PlaybackRequest::parse("queue red sand").plan(&catalogue, 10);
    assert_eq!(queue.apply(&action).map(|item| item.id.as_str()), Some("s3"));
}

#[test]
fn a_search_request_leaves_the_queue_alone() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    queue.apply(&PlaybackAction::Play(vec![&catalogue[0]]));
    let action = PlaybackRequest::parse("find blue").plan(&catalogue, 10);
    assert!(matches!(action, PlaybackAction::Show(_)));
    assert!(queue.apply(&action).is_none(), "results are shown, nothing is played");
    assert_eq!(queue.current().map(|item| item.id.as_str()), Some("s1"), "the playing item is still the current one");
    assert_eq!(queue.len(), 1);
}

#[test]
fn advancing_moves_forward_and_stops_at_the_end_without_wrapping() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    queue.apply(&PlaybackAction::Play(catalogue.iter().collect()));
    assert_eq!(queue.advance().map(|item| item.id.as_str()), Some("s2"));
    assert_eq!(queue.advance().map(|item| item.id.as_str()), Some("s3"));
    assert!(queue.advance().is_none(), "the queue ended");
    assert_eq!(queue.current().map(|item| item.id.as_str()), Some("s3"), "the last item stays current");
}

#[test]
fn a_no_match_request_changes_nothing_and_clear_empties_the_queue() {
    let catalogue = catalogue();
    let mut queue = PlaybackQueue::new();
    queue.apply(&PlaybackAction::Play(vec![&catalogue[1]]));
    assert!(queue.apply(&PlaybackAction::NoMatch).is_none());
    assert_eq!(queue.current().map(|item| item.id.as_str()), Some("s2"));
    queue.clear();
    assert!(queue.is_empty());
    assert!(queue.current().is_none());
}
