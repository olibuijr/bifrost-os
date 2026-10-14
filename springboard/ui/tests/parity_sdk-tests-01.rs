//! Parity sdk-tests-01: headless tests of the live activity life cycle (`bifrost_ui::live_activity`). Times are passed
//! in as whole seconds, so each state is derived without a clock and nothing is drawn.

use bifrost_ui::live_activity::{Content, Dismissal, Error, Registry, State, DISMISSAL_WINDOW_S};

fn live() -> Registry<u32> {
    Registry::new(true)
}

#[test]
fn headless_the_dismissal_window_is_four_hours() {
    assert_eq!(DISMISSAL_WINDOW_S, 4 * 60 * 60);
}

#[test]
fn headless_a_request_without_a_start_time_is_active_at_once() {
    let mut r = live();
    let id = r.request("notes", Content::new(1), None, 100).unwrap();
    assert_eq!(id, 1, "ids start at one");
    assert_eq!(r.state(id, 100), Some(State::Active));
    assert_eq!(r.content(id, 100).map(|c| c.state), Some(1));
}

#[test]
fn headless_a_scheduled_activity_is_pending_until_its_start_time() {
    let mut r = live();
    let id = r.request("timer", Content::new(7), Some(200), 100).unwrap();
    assert_eq!(r.state(id, 150), Some(State::Pending));
    assert_eq!(r.state(id, 199), Some(State::Pending));
    assert_eq!(r.state(id, 200), Some(State::Active));
}

#[test]
fn headless_ids_are_unique_and_unknown_ids_have_no_state() {
    let mut r = live();
    let a = r.request("a", Content::new(0), None, 0).unwrap();
    let b = r.request("b", Content::new(0), None, 0).unwrap();
    assert_ne!(a, b);
    assert_eq!(r.state(999, 0), None);
    assert!(r.content(999, 0).is_none());
    assert_eq!(r.update(999, Content::new(1), 0), Err(Error::NotFound));
    assert_eq!(r.end(999, None, Dismissal::Default, 0), Err(Error::NotFound));
}

#[test]
fn headless_the_switch_for_live_activities_refuses_new_requests_only() {
    let mut r = live();
    let id = r.request("notes", Content::new(1), None, 0).unwrap();
    r.set_enabled(false);
    assert!(!r.enabled());
    assert_eq!(r.request("notes", Content::new(2), None, 0), Err(Error::Disabled));
    assert_eq!(r.state(id, 10), Some(State::Active), "a running activity keeps its state");
    r.set_enabled(true);
    assert!(r.request("notes", Content::new(3), None, 10).is_ok());
}

#[test]
fn headless_content_goes_stale_at_its_stale_date_and_an_update_makes_it_fresh() {
    let mut r = live();
    let id = r.request("delivery", Content::new(1).stale_at(150), None, 100).unwrap();
    assert_eq!(r.state(id, 149), Some(State::Active));
    assert_eq!(r.state(id, 150), Some(State::Stale));
    r.update(id, Content::new(2), 150).unwrap();
    assert_eq!(r.state(id, 150), Some(State::Active));
    assert_eq!(r.content(id, 150).map(|c| c.state), Some(2));
}

#[test]
fn headless_only_active_or_stale_activities_accept_updates() {
    let mut r = live();
    let later = r.request("timer", Content::new(1), Some(500), 100).unwrap();
    assert_eq!(r.update(later, Content::new(2), 100), Err(Error::NotLive), "a pending activity is not live yet");
    let stale = r.request("delivery", Content::new(1).stale_at(120), None, 100).unwrap();
    assert!(r.update(stale, Content::new(2), 130).is_ok(), "a stale activity still takes an update");
}

#[test]
fn headless_an_ended_activity_keeps_its_final_content_until_it_is_dismissed() {
    let mut r = live();
    let id = r.request("timer", Content::new(1), None, 100).unwrap();
    r.end(id, Some(Content::new(9)), Dismissal::Default, 200).unwrap();
    assert_eq!(r.state(id, 199), Some(State::Active));
    assert_eq!(r.state(id, 200), Some(State::Ended));
    assert_eq!(r.content(id, 200).map(|c| c.state), Some(9), "the final content replaces the old one");
    assert_eq!(r.state(id, 200 + DISMISSAL_WINDOW_S - 1), Some(State::Ended));
    assert_eq!(r.state(id, 200 + DISMISSAL_WINDOW_S), Some(State::Dismissed));
    assert!(r.content(id, 200 + DISMISSAL_WINDOW_S).is_none());
}

#[test]
fn headless_an_immediate_dismissal_removes_the_activity_when_it_ends() {
    let mut r = live();
    let id = r.request("timer", Content::new(1), None, 0).unwrap();
    r.end(id, None, Dismissal::Immediate, 300).unwrap();
    assert_eq!(r.state(id, 300), Some(State::Dismissed));
    assert!(r.visible("timer", 300).is_empty());
}

#[test]
fn headless_a_timed_dismissal_is_cut_back_to_the_window() {
    let mut r = live();
    let timed = r.request("a", Content::new(1), None, 0).unwrap();
    r.end(timed, None, Dismissal::After(250), 200).unwrap();
    assert_eq!(r.state(timed, 249), Some(State::Ended));
    assert_eq!(r.state(timed, 250), Some(State::Dismissed));

    let far = r.request("a", Content::new(1), None, 0).unwrap();
    r.end(far, None, Dismissal::After(200 + 10 * DISMISSAL_WINDOW_S), 200).unwrap();
    assert_eq!(r.state(far, 200 + DISMISSAL_WINDOW_S), Some(State::Dismissed), "a later time is the window at most");

    let past = r.request("a", Content::new(1), None, 0).unwrap();
    r.end(past, None, Dismissal::After(10), 200).unwrap();
    assert_eq!(r.state(past, 200), Some(State::Dismissed), "a time already past means at once");
}

#[test]
fn headless_an_ended_activity_takes_no_more_updates_or_ends() {
    let mut r = live();
    let id = r.request("timer", Content::new(1), None, 0).unwrap();
    r.end(id, None, Dismissal::Default, 10).unwrap();
    assert_eq!(r.update(id, Content::new(2), 20), Err(Error::NotLive));
    assert_eq!(r.end(id, None, Dismissal::Default, 20), Err(Error::NotLive));
}

#[test]
fn headless_an_app_sees_its_own_visible_activities_in_request_order() {
    let mut r = live();
    let maps = r.request("maps", Content::new(1), None, 0).unwrap();
    let _mail = r.request("mail", Content::new(2), None, 0).unwrap();
    let later = r.request("maps", Content::new(3), Some(500), 0).unwrap();
    assert_eq!(r.visible("maps", 100), vec![maps, later], "a pending activity counts as on screen");
    assert_eq!(r.visible("mail", 100).len(), 1);
    r.end(maps, None, Dismissal::Immediate, 100).unwrap();
    assert_eq!(r.visible("maps", 100), vec![later]);
    assert!(r.visible("unknown", 100).is_empty());
}
