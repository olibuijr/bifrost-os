//! Parity sdk-tests-04: headless tests for the notification store behind the Notification Center column, the sheet's
//! state machine, the shell-side user notification registry, and the switch, slider and blend helpers of the Settings
//! controls. Every test runs on the host with plain data: no shell is started, nothing is drawn to a screen and no
//! network or clock is read.

use bifrost_proto::{NotifAction, NotifCategory, NotifRequest, NotifSettings, NotifStatus, TextInput, ToApp, NOTIF_DEFAULT_ACTION};
use springboard::shell::controls::{blend, notch, slider_value, switch_progress, SWITCH_MS};
use springboard::shell::notify::center::{Center, LIST_TOP_PT};
use springboard::shell::notify::store::{age_text, Notification, Store, KEEP};
use springboard::shell::usernotif::{display, settings_for, Added, Registry, MAX_PENDING};
use springboard::shell::AppId;

// --- notification store (the rows behind the Notification Center column) ----------------------------------------------

#[test]
fn headless_a_new_store_is_empty_and_ids_count_up_from_one() {
    let mut store = Store::memory();
    assert!(store.is_empty());
    assert_eq!((store.len(), store.count(AppId::Terminal)), (0, 0));
    let first = store.push(Notification::new(AppId::Terminal, "one", "a", 100));
    let second = store.push(Notification::new(AppId::Terminal, "two", "b", 101));
    assert_eq!((first, second), (1, 2));
    assert_eq!(store.get(second).map(|n| n.title.as_str()), Some("two"));
}

#[test]
fn headless_the_groups_list_each_app_once_with_its_newest_first() {
    let mut store = Store::memory();
    store.push(Notification::new(AppId::Terminal, "one", "", 1));
    store.push(Notification::new(AppId::Settings, "two", "", 2));
    store.push(Notification::new(AppId::Terminal, "three", "", 3));
    let groups = store.groups();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].app, AppId::Terminal, "the group with the newest notification comes first");
    let titles: Vec<&str> = groups[0].items.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["three", "one"]);
    assert_eq!(groups[0].latest().title, "three");
    assert_eq!(groups[1].app, AppId::Settings);
    assert_eq!(groups[1].items.len(), 1);
}

#[test]
fn headless_clearing_one_app_keeps_the_others_and_clearing_all_empties_the_store() {
    let mut store = Store::memory();
    store.push(Notification::new(AppId::Terminal, "a", "", 1));
    store.push(Notification::new(AppId::Terminal, "b", "", 2));
    store.push(Notification::new(AppId::Settings, "c", "", 3));
    assert!(store.clear_app(AppId::Terminal), "clearing an app with entries reports a change");
    assert_eq!((store.count(AppId::Terminal), store.count(AppId::Settings)), (0, 1));
    assert!(!store.clear_app(AppId::Terminal), "clearing it again changes nothing");
    store.clear();
    assert!(store.is_empty());
}

#[test]
fn headless_removing_one_notification_by_id_leaves_the_rest() {
    let mut store = Store::memory();
    let keep = store.push(Notification::new(AppId::Terminal, "keep", "", 1));
    let drop = store.push(Notification::new(AppId::Terminal, "drop", "", 2));
    assert!(store.remove(drop));
    assert!(store.get(drop).is_none());
    assert!(store.get(keep).is_some());
    assert!(!store.remove(drop), "a missing id is not removed twice");
}

#[test]
fn headless_the_store_keeps_only_the_newest_entries_up_to_its_limit() {
    let mut store = Store::memory();
    let total = KEEP as u64 + 5;
    for i in 1..=total {
        store.push(Notification::new(AppId::Terminal, format!("n{i}"), "", i as i64));
    }
    assert_eq!(store.len(), KEEP);
    assert!(store.get(1).is_none(), "the oldest entries are evicted first");
    assert!(store.get(5).is_none());
    assert_eq!(store.get(6).map(|n| n.title.as_str()), Some("n6"));
    assert!(store.get(total).is_some());
}

#[test]
fn headless_mute_focus_and_permission_prompt_state_is_kept_per_app() {
    let mut store = Store::memory();
    assert!(!store.is_muted(AppId::Settings) && !store.is_focus_allowed(AppId::Settings) && !store.is_asked(AppId::Settings));
    store.set_muted(AppId::Settings, true);
    assert!(store.is_muted(AppId::Settings));
    assert!(!store.is_muted(AppId::Terminal), "muting one app does not mute another");
    store.set_muted(AppId::Settings, false);
    assert!(!store.is_muted(AppId::Settings));
    store.set_focus_allowed(AppId::Terminal, true);
    assert!(store.is_focus_allowed(AppId::Terminal) && !store.is_focus_allowed(AppId::Settings));
    store.set_asked(AppId::Deildu);
    assert!(store.is_asked(AppId::Deildu) && !store.is_asked(AppId::Terminal));
}

#[test]
fn headless_the_age_text_reads_minutes_hours_and_days_and_never_goes_negative() {
    assert_eq!(age_text(1000, 1000), "now");
    assert_eq!(age_text(1000, 1059), "now");
    assert_eq!(age_text(1000, 1060), "1m ago");
    assert_eq!(age_text(1000, 1000 + 3600), "1h ago");
    assert_eq!(age_text(1000, 1000 + 2 * 86_400), "2d ago");
    assert_eq!(age_text(2000, 1000), "now", "a time in the future reads as now");
}

// --- Notification Center sheet state -------------------------------------------------------------------------------

#[test]
fn headless_the_notification_center_starts_hidden_and_stays_hidden_when_hidden() {
    let mut center = Center::default();
    assert!(center.is_hidden());
    assert!(!center.is_shown());
    assert_eq!(center.describe(), "hidden");
    assert_eq!(center.progress(0.0), 0.0);
    assert_eq!(center.progress(5.0), 0.0, "a hidden sheet has no progress at any time");
    center.hide();
    assert!(center.is_hidden());
    assert_eq!(center.describe(), "hidden");
}

#[test]
fn headless_the_expanded_list_starts_below_the_clock_at_a_fixed_point() {
    assert_eq!(LIST_TOP_PT, 275.0);
}

// --- shell-side user notification registry ---------------------------------------------------------------------------

fn request(id: &str) -> NotifRequest {
    NotifRequest { id: id.into(), title: "Title".into(), ..Default::default() }
}

fn timed(id: &str, delay_ms: u32, repeats: bool) -> NotifRequest {
    NotifRequest { delay_ms, repeats, ..request(id) }
}

fn chat_categories() -> Vec<NotifCategory> {
    vec![NotifCategory {
        id: "reply".into(),
        actions: vec![
            NotifAction::button("ok", "OK"),
            NotifAction { text_input: Some(TextInput { button_title: "Send".into(), placeholder: "Reply".into() }), ..NotifAction::button("reply", "Reply") },
        ],
    }]
}

fn answer(action: &str) -> ToApp {
    ToApp::NotificationAction { id: "m".into(), action: action.into(), text: String::new(), thread: String::new(), category: String::new() }
}

#[test]
fn headless_an_immediate_request_is_delivered_now_and_a_scheduled_one_waits() {
    let mut reg = Registry::default();
    assert_eq!(reg.add("app.one", request("now"), 0.0), Added::Deliver { replaced: vec![] });
    assert_eq!(reg.add("app.one", timed("later", 1000, false), 0.0), Added::Scheduled);
    assert_eq!(reg.list("app.one", false), ["later"]);
    assert_eq!(reg.list("app.one", true), Vec::<String>::new(), "nothing is delivered before its trigger");
}

#[test]
fn headless_a_request_without_an_identifier_or_any_content_is_refused_as_invalid() {
    let mut reg = Registry::default();
    assert_eq!(reg.add("app.one", NotifRequest { id: String::new(), title: "T".into(), ..Default::default() }, 0.0), Added::Refused(NotifStatus::Param));
    assert_eq!(reg.add("app.one", NotifRequest { id: "x".into(), ..Default::default() }, 0.0), Added::Refused(NotifStatus::Param));
    let subtitle_only = NotifRequest { id: "x".into(), subtitle: "S".into(), ..Default::default() };
    assert!(matches!(reg.add("app.one", subtitle_only, 0.0), Added::Deliver { .. }), "a subtitle alone is content");
}

#[test]
fn headless_a_category_must_be_registered_before_a_request_names_it() {
    let mut reg = Registry::default();
    let named = NotifRequest { category: "reply".into(), ..request("m") };
    assert_eq!(reg.add("app.one", named.clone(), 0.0), Added::Refused(NotifStatus::Param));
    reg.set_categories("app.one", chat_categories());
    assert!(reg.category("app.one", "reply").is_some());
    assert!(reg.category("app.two", "reply").is_none(), "categories belong to the app that registered them");
    assert_eq!(reg.add("app.one", named, 0.0), Added::Deliver { replaced: vec![] });
}

#[test]
fn headless_a_delayed_request_fires_once_at_its_trigger_time() {
    let mut reg = Registry::default();
    reg.add("app.one", timed("later", 1000, false), 0.0);
    assert_eq!(reg.next_due(), Some(1000.0));
    assert!(reg.due(999.0).is_empty());
    let fired = reg.due(1000.0);
    assert_eq!(fired.len(), 1);
    assert_eq!((fired[0].0.as_str(), fired[0].1.id.as_str()), ("app.one", "later"));
    assert!(reg.due(5000.0).is_empty(), "a one-shot request does not fire twice");
    assert_eq!(reg.next_due(), None);
}

#[test]
fn headless_a_repeating_request_stays_pending_on_its_grid() {
    let mut reg = Registry::default();
    reg.add("app.one", timed("tick", 500, true), 0.0);
    assert_eq!(reg.due(500.0).len(), 1);
    assert_eq!(reg.next_due(), Some(1000.0), "the next firing is one interval later");
    assert_eq!(reg.list("app.one", false), ["tick"]);
}

#[test]
fn headless_a_pending_request_with_the_same_identifier_replaces_the_earlier_one() {
    let mut reg = Registry::default();
    reg.add("app.one", timed("p", 1000, false), 0.0);
    reg.add("app.one", timed("p", 5000, false), 0.0);
    assert_eq!(reg.list("app.one", false), ["p"]);
    assert_eq!(reg.next_due(), Some(5000.0));
}

#[test]
fn headless_pending_requests_are_capped_per_app_and_a_replacement_still_fits() {
    let mut reg = Registry::default();
    for i in 0..MAX_PENDING {
        assert_eq!(reg.add("app.one", timed(&format!("p{i}"), 1000, false), 0.0), Added::Scheduled);
    }
    assert_eq!(reg.add("app.one", timed("extra", 1000, false), 0.0), Added::Refused(NotifStatus::Full));
    assert_eq!(reg.add("app.one", timed("p0", 2000, false), 0.0), Added::Scheduled, "replacing a pending id does not grow the list");
    assert_eq!(reg.add("app.two", timed("other", 1000, false), 0.0), Added::Scheduled, "the cap is per app");
    assert_eq!(MAX_PENDING, 64);
}

#[test]
fn headless_removing_pending_requests_by_identifier_or_all_of_them() {
    let mut reg = Registry::default();
    reg.add("app.one", timed("a", 1000, false), 0.0);
    reg.add("app.one", timed("b", 1000, false), 0.0);
    assert!(reg.remove("app.one", false, &["a".to_string()]).is_empty(), "pending requests have no store entry");
    assert_eq!(reg.list("app.one", false), ["b"]);
    reg.remove("app.one", false, &[]);
    assert!(reg.list("app.one", false).is_empty());
}

#[test]
fn headless_removing_delivered_requests_returns_the_store_entries_they_occupied() {
    let mut reg = Registry::default();
    reg.record_delivery("app.one", request("a"), 10);
    reg.record_delivery("app.one", request("b"), 11);
    reg.record_delivery("app.two", request("c"), 12);
    assert_eq!(reg.list("app.one", true), ["a", "b"], "oldest first");
    assert_eq!(reg.remove("app.one", true, &["a".to_string()]), vec![10]);
    assert_eq!(reg.remove("app.one", true, &[]), vec![11]);
    assert_eq!(reg.store_ids(), vec![12], "another app's delivery is untouched");
}

#[test]
fn headless_a_new_delivery_replaces_the_one_with_the_same_identifier() {
    let mut reg = Registry::default();
    reg.record_delivery("app.one", request("m"), 7);
    assert_eq!(reg.add("app.one", request("m"), 0.0), Added::Deliver { replaced: vec![7] });
    assert!(reg.store_ids().is_empty());
}

#[test]
fn headless_pruning_forgets_deliveries_whose_store_entry_is_gone() {
    let mut reg = Registry::default();
    reg.record_delivery("app.one", request("a"), 1);
    reg.record_delivery("app.one", request("b"), 2);
    reg.prune(|id| id == 2);
    assert_eq!(reg.store_ids(), vec![2]);
}

#[test]
fn headless_the_newest_delivery_of_an_app_is_the_last_one_recorded() {
    let mut reg = Registry::default();
    reg.record_delivery("app.one", request("a"), 1);
    reg.record_delivery("app.one", request("b"), 2);
    reg.record_delivery("app.two", request("c"), 3);
    assert_eq!(reg.newest_for("app.one").map(|d| d.store_id), Some(2));
    assert!(reg.newest_for("app.none").is_none());
}

#[test]
fn headless_the_actions_of_a_delivery_come_from_its_category_in_order() {
    let mut reg = Registry::default();
    reg.set_categories("app.one", chat_categories());
    reg.record_delivery("app.one", NotifRequest { category: "reply".into(), ..request("m") }, 5);
    let ids: Vec<String> = reg.actions(5).into_iter().map(|a| a.id).collect();
    assert_eq!(ids, ["ok", "reply"]);
    assert!(reg.actions(99).is_empty());
}

#[test]
fn headless_responding_sends_the_action_to_its_app_and_text_only_for_a_text_action() {
    let mut reg = Registry::default();
    reg.set_categories("app.one", chat_categories());
    reg.record_delivery("app.one", NotifRequest { category: "reply".into(), thread: "t".into(), ..request("m") }, 5);

    let (app, msg) = reg.respond(5, "ok", "ignored").expect("the action exists");
    assert_eq!(app, "app.one");
    let ToApp::NotificationAction { id, action, text, thread, category } = msg else { panic!("an action message") };
    assert_eq!((id.as_str(), action.as_str(), text.as_str(), thread.as_str(), category.as_str()), ("m", "ok", "", "t", "reply"));

    let (_, typed) = reg.respond(5, "reply", "on my way").expect("the text action exists");
    assert!(matches!(typed, ToApp::NotificationAction { ref text, .. } if text == "on my way"));

    assert!(reg.respond(5, "missing", "").is_none(), "an action outside the category is refused");
    assert!(reg.respond(99, "ok", "").is_none(), "an unknown notification has no answer");
    let opened = reg.respond(5, NOTIF_DEFAULT_ACTION, "x").expect("the default action is always allowed");
    assert!(matches!(opened.1, ToApp::NotificationAction { ref text, .. } if text.is_empty()));
}

#[test]
fn headless_a_held_request_is_released_to_its_own_app_only_once() {
    let mut reg = Registry::default();
    reg.hold("app.one", 1, 42, request("h"));
    reg.hold("app.two", 2, 43, request("i"));
    let mine = reg.take_held("app.one");
    assert_eq!(mine.len(), 1);
    assert_eq!((mine[0].0, mine[0].1, mine[0].2.id.as_str()), (1, 42, "h"));
    assert!(reg.take_held("app.one").is_empty());
    assert_eq!(reg.take_held("app.two").len(), 1);
}

#[test]
fn headless_answers_for_a_closed_app_wait_for_it_and_only_the_newest_thirty_two_are_kept() {
    let mut reg = Registry::default();
    reg.queue("app.one", answer("a0"));
    assert!(reg.take_queued("app.two").is_empty());
    assert_eq!(reg.take_queued("app.one"), vec![answer("a0")]);
    assert!(reg.take_queued("app.one").is_empty());

    for i in 0..33 {
        reg.queue("app.one", answer(&format!("a{i}")));
    }
    let kept = reg.take_queued("app.one");
    assert_eq!(kept.len(), 32);
    assert_eq!(kept.first(), Some(&answer("a1")), "the oldest answer is dropped first");
    assert_eq!(kept.last(), Some(&answer("a32")));
}

#[test]
fn headless_forgetting_an_app_drops_everything_of_it_and_nothing_of_another() {
    let mut reg = Registry::default();
    reg.set_categories("app.one", chat_categories());
    reg.record_delivery("app.one", request("a"), 1);
    reg.hold("app.one", 1, 2, request("h"));
    reg.queue("app.one", answer("ok"));
    reg.record_delivery("app.two", request("b"), 3);
    reg.forget_app("app.one");
    assert!(reg.category("app.one", "reply").is_none());
    assert_eq!(reg.store_ids(), vec![3]);
    assert!(reg.take_held("app.one").is_empty());
    assert!(reg.take_queued("app.one").is_empty());
}

#[test]
fn headless_the_settings_answer_follows_permission_mute_and_focus() {
    let off = NotifSettings { authorized: false, alert: false, sound: false, badge: false, focus: false };
    assert_eq!(settings_for(false, false, false), off);
    assert_eq!(settings_for(true, true, false), NotifSettings { authorized: true, ..off }, "a muted app is authorized but silent");
    assert_eq!(settings_for(true, false, true), NotifSettings { authorized: true, alert: true, sound: true, badge: true, focus: true });
    assert_eq!(settings_for(false, false, true), NotifSettings { focus: true, ..off }, "Focus is reported even without permission");
}

#[test]
fn headless_a_delivered_request_shows_its_subtitle_between_title_and_body() {
    let full = NotifRequest { title: "T".into(), subtitle: "S".into(), body: "B".into(), ..Default::default() };
    assert_eq!(display(&full), ("T".to_string(), "S\nB".to_string()));
    let no_subtitle = NotifRequest { subtitle: String::new(), ..full.clone() };
    assert_eq!(display(&no_subtitle), ("T".to_string(), "B".to_string()));
    let no_body = NotifRequest { body: String::new(), ..full.clone() };
    assert_eq!(display(&no_body), ("T".to_string(), "S".to_string()));
    assert_eq!(display(&NotifRequest { title: "T".into(), ..Default::default() }), ("T".to_string(), String::new()));
}

// --- Settings controls: switch, slider and the blend of a source over a destination --------------------------------

/// The four 8-bit channels of a packed 0xAARRGGBB pixel, alpha first.
fn channels(px: u32) -> [u32; 4] {
    [px >> 24, (px >> 16) & 255, (px >> 8) & 255, px & 255]
}

/// Whether two pixels agree within one level on every channel (fixed point rounding).
fn near(a: u32, b: u32) -> bool {
    channels(a).iter().zip(channels(b)).all(|(x, y)| x.abs_diff(y) <= 1)
}

#[test]
fn headless_a_transparent_source_leaves_the_destination_as_it_was() {
    let dst = 0xFF33_6699;
    assert!(near(blend(dst, 0xFFFF_0000, 0), dst));
}

#[test]
fn headless_a_fully_opaque_source_at_full_alpha_replaces_the_destination() {
    assert!(near(blend(0xFF33_6699, 0xFFFF_0000, 255), 0xFFFF_0000));
}

#[test]
fn headless_half_alpha_red_over_nothing_is_half_red_and_half_transparent() {
    let [a, r, g, b] = channels(blend(0x0000_0000, 0xFFFF_0000, 128));
    assert!((127..=129).contains(&a) && (127..=129).contains(&r), "alpha {a} red {r}");
    assert_eq!((g, b), (0, 0));
}

#[test]
fn headless_half_alpha_red_over_opaque_black_stays_opaque_and_half_red() {
    let [a, r, g, b] = channels(blend(0xFF00_0000, 0xFFFF_0000, 128));
    assert!(a >= 254, "source over an opaque destination stays opaque");
    assert!((127..=129).contains(&r));
    assert_eq!((g, b), (0, 0));
}

#[test]
fn headless_the_red_of_a_source_over_black_grows_with_its_alpha() {
    let reds: Vec<u32> = [0, 64, 128, 192, 255].iter().map(|&alpha| channels(blend(0xFF00_0000, 0xFFFF_0000, alpha))[1]).collect();
    assert!(reds.windows(2).all(|w| w[0] < w[1]), "{reds:?}");
    assert!(reds[0] <= 1 && reds[4] >= 254);
}

#[test]
fn headless_the_switch_knob_eases_out_over_its_duration() {
    assert_eq!(switch_progress(true, 0.0, -1.0), 0.0, "before the flip the knob is at rest");
    assert_eq!(switch_progress(true, 0.0, SWITCH_MS), 1.0);
    let mid = switch_progress(true, 0.0, SWITCH_MS / 2.0);
    assert!(mid > 0.5 && mid < 1.0, "ease out starts fast: {mid}");
    assert_eq!(switch_progress(false, 0.0, 0.0), 1.0, "switching off starts from on");
    assert_eq!(switch_progress(false, 0.0, SWITCH_MS), 0.0);
}

#[test]
fn headless_a_slider_touch_maps_to_a_value_clamped_to_its_track() {
    assert_eq!(slider_value(100.0, 100.0, 400.0), 0.0);
    assert_eq!(slider_value(300.0, 100.0, 400.0), 0.5);
    assert_eq!(slider_value(900.0, 100.0, 400.0), 1.0);
    assert_eq!(slider_value(0.0, 100.0, 400.0), 0.0, "left of the track is zero");
}

#[test]
fn headless_a_slider_value_snaps_to_the_nearest_notch() {
    assert_eq!(notch(0.0, 7), 0);
    assert_eq!(notch(0.5, 7), 3);
    assert_eq!(notch(0.99, 7), 6);
    assert_eq!(notch(0.6, 2), 1);
    assert_eq!(notch(1.0, 1), 0, "a single notch is always the first");
}
