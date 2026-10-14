//! Parity sdk-complete-26: headless tests for the weather error kinds (bifrost_ui::weather_model), the media picker's
//! bulk choice (bifrost_ui::media_picker) and user location on a map (bifrost_ui::map_user_location). Pure models: no
//! network, no location sensor and no map drawing run here.

use std::io;

use bifrost_ui::geo::Coordinate;
use bifrost_ui::location::LocationFix;
use bifrost_ui::location_auth::AuthorizationStatus;
use bifrost_ui::map_user_location::{UserLocation, MAX_FIX_ACCURACY_M, MAX_FIX_AGE_S};
use bifrost_ui::mapview::{MapCamera, UserTracking};
use bifrost_ui::media_picker::{Error, Kind, MediaPicker, PickItem, Selection};
use bifrost_ui::scene_phase::{SceneTracker, ScenePhase};
use bifrost_ui::search_scope::{SearchBar, SearchScope, SearchToken};
use bifrost_ui::toolbar_customization::{Error as ToolbarError, ToolbarCustomization};
use bifrost_ui::weather_model::WeatherError;

// ---- weather: typed errors an app can match on (bifrost_ui::weather_model) ----

#[test]
fn headless_a_refused_permission_stays_permission_denied() {
    let err = io::Error::from(io::ErrorKind::PermissionDenied);
    assert_eq!(WeatherError::from_io(&err), WeatherError::PermissionDenied);
    assert!(!WeatherError::PermissionDenied.is_retryable());
}

#[test]
fn headless_network_failures_map_to_unreachable() {
    for kind in [
        io::ErrorKind::NotConnected,
        io::ErrorKind::ConnectionRefused,
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::TimedOut,
        io::ErrorKind::HostUnreachable,
        io::ErrorKind::NetworkDown,
    ] {
        let err = io::Error::from(kind);
        assert_eq!(WeatherError::from_io(&err), WeatherError::Unreachable, "{kind:?} is a network failure");
    }
}

#[test]
fn headless_an_unreadable_answer_maps_to_bad_answer() {
    let err = io::Error::new(io::ErrorKind::InvalidData, "unexpected forecast answer");
    assert_eq!(WeatherError::from_io(&err), WeatherError::BadAnswer);
    assert!(WeatherError::BadAnswer.is_retryable());
}

#[test]
fn headless_other_failures_stay_unknown() {
    let err = io::Error::other("something else");
    assert_eq!(WeatherError::from_io(&err), WeatherError::Unknown);
}

#[test]
fn headless_each_weather_error_has_its_own_message() {
    let messages: Vec<&str> = [
        WeatherError::PermissionDenied,
        WeatherError::Unreachable,
        WeatherError::BadAnswer,
        WeatherError::Unknown,
    ]
    .iter()
    .map(|e| e.description())
    .collect();
    assert_eq!(messages[1], "The weather service could not be reached.");
    assert_eq!(messages[2], "The weather service sent an answer that could not be read.");
    for (i, a) in messages.iter().enumerate() {
        for b in &messages[i + 1..] {
            assert_ne!(a, b, "two weather errors share a message");
        }
    }
    assert_eq!(WeatherError::Unreachable.to_string(), messages[1]);
}

// ---- media picker: bulk choice (bifrost_ui::media_picker) ----

fn song(id: u64, title: &str, playable: bool) -> PickItem {
    PickItem { id, kind: Kind::Song, title: title.to_string(), artist: String::new(), album: String::new(), playable }
}

fn songs() -> Vec<PickItem> {
    vec![song(1, "Alpha", true), song(2, "Bravo", false), song(3, "Charlie", true), song(4, "Delta", true)]
}

#[test]
fn a_limited_selection_reports_how_many_more_items_it_takes() {
    let mut p = MediaPicker::new(songs(), vec![], Selection::Many { max: Some(3) });
    assert_eq!(p.remaining(), Some(3));
    p.toggle(1).unwrap();
    assert_eq!(p.remaining(), Some(2));
}

#[test]
fn an_unlimited_selection_has_no_remaining_count() {
    let p = MediaPicker::new(songs(), vec![], Selection::Many { max: None });
    assert_eq!(p.remaining(), None);
}

#[test]
fn a_single_choice_has_one_slot_until_something_is_chosen() {
    let mut p = MediaPicker::new(songs(), vec![], Selection::One);
    assert_eq!(p.remaining(), Some(1));
    p.toggle(1).unwrap();
    assert_eq!(p.remaining(), Some(0));
}

#[test]
fn choose_all_takes_every_playable_listed_item_and_skips_the_locked_one() {
    let mut p = MediaPicker::new(songs(), vec![], Selection::Many { max: None });
    assert_eq!(p.choose_all(), 3);
    assert_eq!(p.chosen(), &[1, 3, 4], "the locked item 2 is listed but never chosen");
}

#[test]
fn choose_all_stops_at_the_limit_and_keeps_existing_choices() {
    let mut p = MediaPicker::new(songs(), vec![], Selection::Many { max: Some(2) });
    p.toggle(4).unwrap();
    assert_eq!(p.choose_all(), 1);
    assert_eq!(p.chosen(), &[4, 1]);
    assert_eq!(p.remaining(), Some(0));
}

#[test]
fn choose_all_follows_the_search_query() {
    let mut p = MediaPicker::new(songs(), vec![], Selection::Many { max: None });
    p.set_query("charlie");
    assert_eq!(p.choose_all(), 1);
    assert_eq!(p.chosen(), &[3]);
}

#[test]
fn choose_all_does_nothing_for_a_single_choice() {
    let mut p = MediaPicker::new(songs(), vec![], Selection::One);
    assert_eq!(p.choose_all(), 0);
    assert!(p.chosen().is_empty());
}

#[test]
fn clear_choice_drops_everything_chosen() {
    let mut p = MediaPicker::new(songs(), vec![], Selection::Many { max: None });
    p.choose_all();
    p.clear_choice();
    assert!(p.chosen().is_empty());
    assert_eq!(p.confirm(), Err(Error::NothingSelected));
}

// ---- map: user location on a map (bifrost_ui::map_user_location) ----

fn at(latitude: f64, longitude: f64) -> Coordinate {
    Coordinate::new(latitude, longitude).expect("a valid coordinate")
}

fn camera() -> MapCamera {
    MapCamera::new(at(0.0, 0.0), 10.0, 800.0, 600.0).expect("a valid camera")
}

fn fix(latitude: f64, longitude: f64, timestamp: f64, accuracy_m: f64) -> LocationFix {
    LocationFix { horizontal_accuracy_m: accuracy_m, ..LocationFix::new(at(latitude, longitude), timestamp) }
}

#[test]
fn headless_a_fix_is_refused_without_location_access() {
    let mut user = UserLocation::new(AuthorizationStatus::Denied);
    assert!(!user.record_fix(fix(64.1, -21.9, 100.0, 5.0)));
    assert!(user.fix().is_none());
}

#[test]
fn headless_an_invalid_fix_is_refused_even_with_access() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedWhenInUse);
    assert!(!user.record_fix(LocationFix::new(at(64.1, -21.9), 100.0)), "a fix without accuracy is not valid");
    assert!(user.record_fix(fix(64.1, -21.9, 100.0, 5.0)));
    assert_eq!(user.fix().map(|f| f.timestamp), Some(100.0));
}

#[test]
fn headless_tracking_stays_off_without_access_whatever_the_button_says() {
    let mut user = UserLocation::new(AuthorizationStatus::NotDetermined);
    assert_eq!(user.press_tracking(), UserTracking::Off);
    assert_eq!(user.tracking(), UserTracking::Off);
}

#[test]
fn headless_the_tracking_button_cycles_with_access() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedAlways);
    assert_eq!(user.press_tracking(), UserTracking::Follow);
    assert_eq!(user.press_tracking(), UserTracking::FollowWithHeading);
    assert_eq!(user.press_tracking(), UserTracking::Off);
}

#[test]
fn headless_follow_centres_the_camera_on_a_recent_fix() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedWhenInUse);
    user.record_fix(fix(64.1, -21.9, 1000.0, 5.0));
    user.press_tracking();
    let mut cam = camera();
    assert!(user.follow(&mut cam, 1010.0, None));
    assert_eq!(cam.center.latitude, 64.1);
    assert_eq!(cam.center.longitude, -21.9);
}

#[test]
fn headless_follow_with_heading_turns_the_map_to_the_user_heading() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedWhenInUse);
    user.record_fix(fix(64.1, -21.9, 1000.0, 5.0));
    user.press_tracking();
    user.press_tracking();
    let mut cam = camera();
    assert!(user.follow(&mut cam, 1000.0, Some(90.0)));
    assert_eq!(cam.heading_deg, 90.0);
}

#[test]
fn headless_follow_leaves_the_camera_alone_when_tracking_is_off() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedWhenInUse);
    user.record_fix(fix(64.1, -21.9, 1000.0, 5.0));
    let mut cam = camera();
    assert!(!user.follow(&mut cam, 1000.0, Some(90.0)));
    assert_eq!(cam.center.latitude, 0.0);
    assert_eq!(cam.heading_deg, 0.0);
}

#[test]
fn headless_a_stale_or_inaccurate_fix_is_not_followed() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedWhenInUse);
    user.record_fix(fix(64.1, -21.9, 1000.0, 5.0));
    user.press_tracking();
    let mut cam = camera();
    assert!(!user.follow(&mut cam, 1000.0 + MAX_FIX_AGE_S + 1.0, None), "the fix is older than the limit");
    assert!(user.followable_fix(1000.0 + MAX_FIX_AGE_S).is_some(), "the limit itself is still fresh");

    user.record_fix(fix(64.2, -21.8, 2000.0, MAX_FIX_ACCURACY_M + 1.0));
    assert!(!user.follow(&mut cam, 2001.0, None), "a fix worse than the accuracy limit is not followed");
    assert_eq!(cam.center.latitude, 0.0);
}

#[test]
fn headless_a_fix_from_the_future_is_not_followed() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedWhenInUse);
    user.record_fix(fix(64.1, -21.9, 1000.0, 5.0));
    assert!(user.followable_fix(999.0).is_none());
}

#[test]
fn headless_losing_access_drops_the_fix_and_turns_tracking_off() {
    let mut user = UserLocation::new(AuthorizationStatus::AuthorizedWhenInUse);
    user.record_fix(fix(64.1, -21.9, 1000.0, 5.0));
    user.press_tracking();
    user.set_authorization(AuthorizationStatus::Restricted);
    assert!(user.fix().is_none());
    assert_eq!(user.tracking(), UserTracking::Off);
    let mut cam = camera();
    assert!(!user.follow(&mut cam, 1000.0, None));
}

// ---- search: scopes, tokens and suggestions (bifrost_ui::search_scope) ----

fn scope(id: u32, title: &str) -> SearchScope {
    SearchScope { id, title: title.to_string() }
}

fn token(id: u32, title: &str) -> SearchToken {
    SearchToken { id, title: title.to_string() }
}

#[test]
fn headless_the_first_scope_is_selected_by_default() {
    let bar = SearchBar::new(vec![scope(1, "Everything"), scope(2, "Titles")]);
    assert_eq!(bar.selected_scope(), Some(1));
    assert_eq!(SearchBar::new(vec![]).selected_scope(), None, "a field without scopes has no scope choice");
}

#[test]
fn headless_selecting_a_scope_needs_a_listed_id() {
    let mut bar = SearchBar::new(vec![scope(1, "Everything"), scope(2, "Titles")]);
    assert!(bar.select_scope(2));
    assert_eq!(bar.selected_scope(), Some(2));
    assert!(!bar.select_scope(9));
    assert_eq!(bar.selected_scope(), Some(2), "an unknown id leaves the selection alone");
}

#[test]
fn headless_next_scope_wraps_after_the_last_one() {
    let mut bar = SearchBar::new(vec![scope(1, "A"), scope(2, "B")]);
    assert_eq!(bar.next_scope(), Some(2));
    assert_eq!(bar.next_scope(), Some(1));
    assert_eq!(SearchBar::new(vec![]).next_scope(), None);
}

#[test]
fn headless_a_token_is_added_once_and_removed_by_id() {
    let mut bar = SearchBar::new(vec![]);
    assert!(bar.add_token(token(7, "Albums")));
    assert!(!bar.add_token(token(7, "Albums again")), "the same id is not added twice");
    assert_eq!(bar.tokens(), &[token(7, "Albums")]);
    assert!(bar.remove_token(7));
    assert!(!bar.remove_token(7));
    assert!(bar.tokens().is_empty());
}

#[test]
fn headless_suggestions_start_with_the_text_and_ignore_case() {
    let bar = SearchBar::new(vec![]);
    let candidates = ["Rust", "rustaceans", "Ruby", "Rust", "trust"];
    assert_eq!(bar.suggestions("RUS", &candidates, 5), vec!["Rust", "rustaceans"]);
}

#[test]
fn headless_suggestions_skip_the_typed_text_and_respect_the_limit() {
    let bar = SearchBar::new(vec![]);
    let candidates = ["rust", "rusty", "rustic", "rusted"];
    assert_eq!(bar.suggestions("rust", &candidates, 2), vec!["rusty", "rustic"]);
    assert!(bar.suggestions("rust", &candidates, 0).is_empty());
}

#[test]
fn headless_blank_text_gives_no_suggestions() {
    let bar = SearchBar::new(vec![]);
    assert!(bar.suggestions("   ", &["a", "b"], 5).is_empty());
}

// ---- scene phase: active, inactive and background (bifrost_ui::scene_phase) ----

#[test]
fn headless_a_focused_visible_window_is_active() {
    assert_eq!(ScenePhase::from_facts(true, true), ScenePhase::Active);
    assert_eq!(ScenePhase::from_facts(true, false), ScenePhase::Inactive);
    assert_eq!(ScenePhase::from_facts(false, true), ScenePhase::Background);
    assert_eq!(ScenePhase::from_facts(false, false), ScenePhase::Background);
}

#[test]
fn headless_the_tracker_reports_each_change_once() {
    let mut tracker = SceneTracker::new(ScenePhase::Background);
    assert_eq!(tracker.update(true, true), Some(ScenePhase::Active));
    assert_eq!(tracker.update(true, true), None, "the same facts report nothing");
    assert_eq!(tracker.update(true, false), Some(ScenePhase::Inactive));
    assert_eq!(tracker.update(false, false), Some(ScenePhase::Background));
    assert_eq!(tracker.phase(), ScenePhase::Background);
}

// ---- toolbars: customization (bifrost_ui::toolbar_customization) ----

fn bar() -> ToolbarCustomization {
    ToolbarCustomization::new(vec![1, 2, 3, 4], vec![1])
}

#[test]
fn headless_a_new_toolbar_shows_every_item_in_the_default_order() {
    let t = bar();
    assert_eq!(t.visible(), vec![1, 2, 3, 4]);
    assert_eq!(t.order(), &[1, 2, 3, 4]);
}

#[test]
fn headless_hiding_an_item_removes_it_from_the_visible_list_only() {
    let mut t = bar();
    assert_eq!(t.hide(3), Ok(()));
    assert_eq!(t.visible(), vec![1, 2, 4]);
    assert_eq!(t.order(), &[1, 2, 3, 4], "a hidden item keeps its place in the order");
    assert!(!t.is_shown(3));
    assert_eq!(t.hide(3), Ok(()), "hiding twice changes nothing");
    assert_eq!(t.visible(), vec![1, 2, 4]);
}

#[test]
fn headless_a_required_item_cannot_be_hidden() {
    let mut t = bar();
    assert_eq!(t.hide(1), Err(ToolbarError::Required));
    assert!(t.is_shown(1));
}

#[test]
fn headless_showing_an_item_puts_it_back_in_its_place() {
    let mut t = bar();
    t.hide(2).unwrap();
    t.move_item(2, 0).unwrap();
    t.show(2).unwrap();
    assert_eq!(t.visible(), vec![2, 1, 3, 4]);
}

#[test]
fn headless_moving_an_item_reorders_the_visible_list() {
    let mut t = bar();
    assert_eq!(t.move_item(4, 1), Ok(()));
    assert_eq!(t.visible(), vec![1, 4, 2, 3]);
    assert_eq!(t.move_item(1, 99), Ok(()), "a position past the end moves the item to the end");
    assert_eq!(t.order(), &[4, 2, 3, 1]);
}

#[test]
fn headless_moving_counts_hidden_items_in_the_position() {
    let mut t = bar();
    t.hide(2).unwrap();
    t.move_item(4, 2).unwrap();
    assert_eq!(t.order(), &[1, 2, 4, 3]);
    assert_eq!(t.visible(), vec![1, 4, 3]);
}

#[test]
fn headless_an_unknown_item_is_refused_by_every_change() {
    let mut t = bar();
    assert_eq!(t.hide(9), Err(ToolbarError::UnknownItem));
    assert_eq!(t.show(9), Err(ToolbarError::UnknownItem));
    assert_eq!(t.move_item(9, 0), Err(ToolbarError::UnknownItem));
    assert_eq!(t.order(), &[1, 2, 3, 4]);
}

#[test]
fn headless_reset_restores_the_default_order_and_shows_everything() {
    let mut t = bar();
    t.hide(3).unwrap();
    t.move_item(4, 0).unwrap();
    t.reset();
    assert_eq!(t.visible(), vec![1, 2, 3, 4]);
    assert_eq!(t.order(), &[1, 2, 3, 4]);
}

#[test]
fn headless_required_ids_the_app_does_not_offer_are_ignored() {
    let t = ToolbarCustomization::new(vec![1, 2], vec![7]);
    assert_eq!(t.hide(7), Err(ToolbarError::UnknownItem));
    assert_eq!(t.visible(), vec![1, 2]);
}
