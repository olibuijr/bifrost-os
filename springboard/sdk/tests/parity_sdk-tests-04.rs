//! Parity sdk-tests-04: headless tests for the app-side user notification API and its model. Every call runs against an
//! in-memory context (`Ctx::testing`): what the app sends to the shell is collected by `take_sent`, and the trigger,
//! presentation, authorization, sound and attachment rules are plain functions. No shell, no display, no network.

use bifrost_app::proto::{Configure, NotifAction, NotifCategory, NotifRequest, NotifSettings, NotifStatus, TextInput, ToShell, NOTIF_DEFAULT_ACTION};
use bifrost_app::usernotif::{
    self, Attachment, AuthorizationOptions, AuthorizationStatus, AuthorizationStore, CalendarTrigger, DateComponents, Decision, Event, InterruptionLevel,
    IntervalTrigger, PresentationOptions, Request, Response, Setting, Settings, Sound,
};
use bifrost_app::usernotif_model::foreground_presentation;
use bifrost_app::Ctx;

/// An in-memory context: what the app sends to the shell is collected by `take_sent`.
fn testing_ctx() -> Ctx {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(bifrost_render::ui::Ui::new(bifrost_render::font::Font::parse(data).unwrap(), 1.0), cfg)
}

/// Settings of an app the person allowed for alerts, sounds and badges, with no critical-alert grant.
fn granted() -> Settings {
    Settings {
        authorization_status: AuthorizationStatus::Authorized,
        alert: Setting::Enabled,
        sound: Setting::Enabled,
        badge: Setting::Enabled,
        critical_alert: Setting::NotSupported,
    }
}

#[test]
fn headless_add_sends_every_content_field_to_the_shell() {
    let ctx = testing_ctx();
    let req = Request::new("ride", "Bus at 8")
        .subtitle("Platform 2")
        .body("Leaves in 3 minutes")
        .thread("commute")
        .category("transit")
        .badge(2)
        .sound(&Sound::named("chime.wav").unwrap())
        .interruption_level(InterruptionLevel::TimeSensitive)
        .relevance_score(0.5);
    let id = usernotif::add(&ctx, req);
    let expected = NotifRequest {
        id: "ride".into(),
        title: "Bus at 8".into(),
        subtitle: "Platform 2".into(),
        body: "Leaves in 3 minutes".into(),
        thread: "commute".into(),
        category: "transit".into(),
        badge: Some(2),
        sound_kind: 2,
        sound_name: "chime.wav".into(),
        interruption_level: 2,
        relevance_milli: 500,
        ..Default::default()
    };
    assert_eq!(ctx.take_sent(), vec![ToShell::NotifyRequest { request: id, req: expected }]);
}

#[test]
fn headless_a_plain_request_sends_no_sound_and_no_trigger() {
    let ctx = testing_ctx();
    usernotif::add(&ctx, Request::new("hello", "Hi"));
    let sent = ctx.take_sent();
    let [ToShell::NotifyRequest { req, .. }] = sent.as_slice() else { panic!("one request is sent") };
    assert_eq!(req.sound_kind, 0, "no content.sound means no sound");
    assert_eq!((req.delay_ms, req.repeats), (0, false), "without after_ms the request is delivered now, once");
    assert_eq!(req.badge, None, "without badge the Dock badge is left alone");
}

#[test]
fn headless_each_add_gets_its_own_request_id() {
    let ctx = testing_ctx();
    let first = usernotif::add(&ctx, Request::new("a", "One"));
    let second = usernotif::add(&ctx, Request::new("a", "Two"));
    assert_ne!(first, second, "the request id is the answer's key, so it is unique per call");
}

#[test]
fn headless_user_info_and_critical_volume_reach_the_wire() {
    let ctx = testing_ctx();
    let req = Request::new("gas", "Gas low").sound(&Sound::critical_with_volume(0.5).unwrap()).user_info(&[("station", "7")]).unwrap();
    let request = usernotif::add(&ctx, req);
    let expected = NotifRequest {
        id: "gas".into(),
        title: "Gas low".into(),
        sound_kind: 4,
        sound_volume: 50,
        user_info: vec![("station".into(), "7".into())],
        ..Default::default()
    };
    assert_eq!(ctx.take_sent(), vec![ToShell::NotifyRequest { request, req: expected }]);
}

#[test]
fn headless_a_repeating_interval_keeps_its_delay_on_the_wire() {
    let ctx = testing_ctx();
    usernotif::add(&ctx, Request::new("tick", "Tick").after_ms(60_000).repeats(true));
    let sent = ctx.take_sent();
    let [ToShell::NotifyRequest { req, .. }] = sent.as_slice() else { panic!("one request is sent") };
    assert_eq!((req.delay_ms, req.repeats), (60_000, true));
}

#[test]
fn headless_user_info_refuses_more_than_sixteen_entries_or_long_values() {
    let many: Vec<(&str, &str)> = (0..17).map(|_| ("k", "v")).collect();
    assert!(Request::new("a", "T").user_info(&many).is_err());
    assert!(Request::new("a", "T").user_info(&[("", "v")]).is_err(), "an empty key is refused");
    assert!(Request::new("a", "T").user_info(&[("k", &"x".repeat(513))]).is_err());
    assert!(Request::new("a", "T").user_info(&[("k", &"x".repeat(512))]).is_ok());
}

#[test]
fn headless_set_categories_sends_the_actions_in_their_order() {
    let ctx = testing_ctx();
    let categories = vec![NotifCategory {
        id: "reply".into(),
        actions: vec![
            NotifAction::button("ok", "OK"),
            NotifAction { destructive: true, ..NotifAction::button("delete", "Delete") },
            NotifAction { text_input: Some(TextInput { button_title: "Send".into(), placeholder: "Reply".into() }), ..NotifAction::button("reply", "Reply") },
        ],
    }];
    usernotif::set_categories(&ctx, &categories);
    assert_eq!(ctx.take_sent(), vec![ToShell::NotifyCategories(categories)]);
}

#[test]
fn headless_remove_delivered_sends_the_identifiers_it_was_given() {
    let ctx = testing_ctx();
    usernotif::remove_delivered(&ctx, &["a", "b"]);
    assert_eq!(ctx.take_sent(), vec![ToShell::NotifyRemove { delivered: true, ids: vec!["a".into(), "b".into()] }]);
}

#[test]
fn headless_remove_pending_without_identifiers_asks_for_all_of_them() {
    let ctx = testing_ctx();
    usernotif::remove_pending(&ctx, &[]);
    assert_eq!(ctx.take_sent(), vec![ToShell::NotifyRemove { delivered: false, ids: vec![] }]);
}

#[test]
fn headless_the_delivered_and_pending_queries_carry_their_own_request_ids() {
    let ctx = testing_ctx();
    let delivered = usernotif::delivered(&ctx);
    let pending = usernotif::pending(&ctx);
    assert_ne!(delivered, pending);
    assert_eq!(
        ctx.take_sent(),
        vec![ToShell::NotifyQuery { request: delivered, delivered: true }, ToShell::NotifyQuery { request: pending, delivered: false }]
    );
}

#[test]
fn headless_the_settings_query_is_sent_with_its_request_id() {
    let ctx = testing_ctx();
    let request = usernotif::settings(&ctx);
    assert_eq!(ctx.take_sent(), vec![ToShell::NotifySettings { request }]);
}

#[test]
fn headless_request_authorization_sends_the_known_option_bits_and_drops_the_rest() {
    let ctx = testing_ctx();
    let request = usernotif::request_authorization(&ctx, AuthorizationOptions(2 | 4 | 0x80));
    assert_eq!(ctx.take_sent(), vec![ToShell::NotifyAuthorize { request, options: 2 | 4 }]);
}

#[test]
fn headless_settings_flags_pack_into_one_byte_with_focus_on_bit_four() {
    let flags = NotifSettings { authorized: true, alert: true, sound: true, badge: true, focus: false };
    assert_eq!(flags.bits(), 15);
    let focused = NotifSettings { focus: true, ..flags };
    assert_eq!(focused.bits(), 31);
    assert_eq!(NotifSettings::from_bits(focused.bits()), Some(focused));
    assert_eq!(NotifSettings::from_bits(0x20), None, "an unknown bit makes the packet invalid");
}

#[test]
fn headless_status_and_list_events_keep_their_request_ids() {
    let status = Event::Status { request: 4, status: NotifStatus::Full };
    assert_ne!(status, Event::Status { request: 4, status: NotifStatus::Ok });
    assert_ne!(status, Event::Status { request: 5, status: NotifStatus::Full });
    let list = Event::List { request: 2, delivered: true, ids: vec!["x".into()] };
    assert_eq!(list.clone(), list);
    assert_ne!(list, Event::List { request: 2, delivered: false, ids: vec!["x".into()] });
}

#[test]
fn headless_a_text_reply_response_carries_the_typed_text() {
    let e = Event::Response(Response { id: "msg".into(), action: "reply".into(), text: "on my way".into(), thread: "t".into(), category: "reply".into() });
    assert!(matches!(&e, Event::Response(r) if r.text == "on my way" && r.action == "reply"));
}

#[test]
fn headless_the_default_action_response_has_no_text_and_differs_from_dismiss() {
    let e = Event::Response(Response { id: "msg".into(), action: NOTIF_DEFAULT_ACTION.into(), text: String::new(), thread: String::new(), category: String::new() });
    assert!(matches!(&e, Event::Response(r) if r.action == NOTIF_DEFAULT_ACTION && r.text.is_empty()));
    assert_ne!(NOTIF_DEFAULT_ACTION, bifrost_app::proto::NOTIF_DISMISS_ACTION);
}

#[test]
fn headless_foreground_presentation_shows_only_what_the_delegate_asked_and_the_user_allowed() {
    let asked = PresentationOptions::BANNER.union(PresentationOptions::SOUND).union(PresentationOptions::BADGE);
    assert_eq!(foreground_presentation(Some(asked), &granted()), asked);

    let no_sound = Settings { sound: Setting::Disabled, ..granted() };
    assert_eq!(foreground_presentation(Some(asked), &no_sound), PresentationOptions::BANNER.union(PresentationOptions::BADGE));

    let list_only = foreground_presentation(Some(PresentationOptions::LIST), &granted());
    assert!(list_only.contains(PresentationOptions::LIST) && !list_only.contains(PresentationOptions::BANNER));
}

#[test]
fn headless_a_missing_delegate_or_a_denied_app_presents_nothing() {
    assert_eq!(foreground_presentation(None, &granted()), PresentationOptions::NONE, "no delegate, nothing is shown");
    let denied = Settings { authorization_status: AuthorizationStatus::Denied, alert: Setting::Disabled, ..granted() };
    assert_eq!(foreground_presentation(Some(PresentationOptions::BANNER), &denied), PresentationOptions::NONE);
}

#[test]
fn headless_presentation_options_compose_by_union() {
    let both = PresentationOptions::BANNER.union(PresentationOptions::LIST);
    assert!(both.contains(PresentationOptions::BANNER) && both.contains(PresentationOptions::LIST));
    assert!(!PresentationOptions::BANNER.contains(PresentationOptions::SOUND));
    assert!(both.contains(PresentationOptions::NONE), "every set contains the empty set");
}

#[test]
fn headless_the_first_authorization_answer_stands_for_the_app() {
    let mut store = AuthorizationStore::default();
    assert_eq!(store.status("app.one"), AuthorizationStatus::NotDetermined);
    assert!(store.request("app.one", AuthorizationOptions::ALERT.union(AuthorizationOptions::SOUND), Decision::Allow));
    assert_eq!(store.status("app.one"), AuthorizationStatus::Authorized);
    assert!(store.request("app.one", AuthorizationOptions::ALERT, Decision::Deny), "a later request is not asked again");
    assert_eq!(store.status("app.one"), AuthorizationStatus::Authorized);
}

#[test]
fn headless_a_denied_app_gets_no_settings_and_an_undecided_app_reports_not_supported() {
    let mut store = AuthorizationStore::default();
    assert!(!store.request("app.two", AuthorizationOptions::ALERT, Decision::Deny));
    let denied = store.settings("app.two");
    assert_eq!(denied.authorization_status, AuthorizationStatus::Denied);
    assert_eq!((denied.alert, denied.sound, denied.badge), (Setting::Disabled, Setting::Disabled, Setting::Disabled));
    let undecided = store.settings("app.three");
    assert_eq!((undecided.alert, undecided.sound, undecided.badge), (Setting::NotSupported, Setting::NotSupported, Setting::NotSupported));
}

#[test]
fn headless_the_granted_options_decide_each_setting() {
    let mut store = AuthorizationStore::default();
    store.request("app.four", AuthorizationOptions::ALERT.union(AuthorizationOptions::BADGE), Decision::Allow);
    let s = store.settings("app.four");
    assert_eq!((s.alert, s.sound, s.badge), (Setting::Enabled, Setting::Disabled, Setting::Enabled));
    assert_eq!(s.critical_alert, Setting::NotSupported, "critical alerts are not supported unless asked for");
}

#[test]
fn headless_a_provisional_grant_needs_no_prompt() {
    let mut store = AuthorizationStore::default();
    assert!(store.request("app.five", AuthorizationOptions::ALERT.union(AuthorizationOptions::PROVISIONAL), Decision::Deny));
    assert_eq!(store.status("app.five"), AuthorizationStatus::Provisional);
    assert_eq!(store.settings("app.five").alert, Setting::Enabled);
}

#[test]
fn headless_a_critical_alert_grant_enables_the_critical_setting() {
    let mut store = AuthorizationStore::default();
    store.request("app.six", AuthorizationOptions::ALERT.union(AuthorizationOptions::CRITICAL_ALERT), Decision::Allow);
    assert_eq!(store.settings("app.six").critical_alert, Setting::Enabled);
}

#[test]
fn headless_the_authorization_store_round_trips_through_text_and_skips_bad_lines() {
    let mut store = AuthorizationStore::default();
    store.request("app.one", AuthorizationOptions::ALERT, Decision::Allow);
    store.request("app.two", AuthorizationOptions::default(), Decision::Deny);
    let text = store.to_text();
    assert_eq!(AuthorizationStore::from_text(&text), store);
    let damaged = "\t1\t2\na\t9\t2\nb\tx\t1\nc\t2\n";
    assert_eq!(AuthorizationStore::from_text(damaged), AuthorizationStore::default(), "every line is malformed");
}

#[test]
fn headless_an_authorization_status_allows_posting_only_once_granted() {
    assert!(!AuthorizationStatus::NotDetermined.allows_posting());
    assert!(!AuthorizationStatus::Denied.allows_posting());
    assert!(AuthorizationStatus::Authorized.allows_posting());
    assert!(AuthorizationStatus::Provisional.allows_posting());
    assert!(AuthorizationStatus::Ephemeral.allows_posting());
}

#[test]
fn headless_authorization_options_combine_and_test_membership() {
    let both = AuthorizationOptions::ALERT.union(AuthorizationOptions::SOUND);
    assert!(both.contains(AuthorizationOptions::SOUND));
    assert!(!both.contains(AuthorizationOptions::BADGE));
    assert!(both.contains(both));
}

#[test]
fn headless_a_sound_name_is_a_bare_file_name_of_at_most_255_bytes() {
    assert_eq!(Sound::named("chime.wav"), Ok(Sound::Named("chime.wav".into())));
    assert!(Sound::named("").is_err());
    assert!(Sound::named("../chime.wav").is_err(), "a path is refused");
    assert!(Sound::named("sounds/chime.wav").is_err(), "a sub directory is refused");
    assert!(Sound::named(&"a".repeat(255)).is_ok());
    assert!(Sound::named(&"a".repeat(256)).is_err());
}

#[test]
fn headless_a_critical_volume_is_from_zero_to_one() {
    assert!(Sound::critical_with_volume(0.0).is_ok());
    assert!(Sound::critical_with_volume(1.0).is_ok());
    assert!(Sound::critical_with_volume(1.5).is_err());
    assert!(Sound::critical_with_volume(-0.1).is_err());
}

#[test]
fn headless_only_critical_sounds_need_the_critical_grant() {
    assert!(!Sound::Default.is_critical());
    assert!(Sound::DefaultCritical.is_critical());
    assert!(Sound::critical_with_volume(0.3).unwrap().is_critical());
    assert!(Sound::Default.allowed_by(&granted()));
    assert!(!Sound::DefaultCritical.allowed_by(&granted()), "a critical sound needs the critical-alert grant");
    let critical = Settings { critical_alert: Setting::Enabled, ..granted() };
    assert!(Sound::DefaultCritical.allowed_by(&critical));
    let muted = Settings { sound: Setting::Disabled, ..granted() };
    assert!(!Sound::Default.allowed_by(&muted));
}

#[test]
fn headless_an_attachment_takes_its_type_from_the_extension_and_its_size_limit_from_the_type() {
    let png = Attachment::new("pic", "photo.png", 10 << 20).unwrap();
    assert_eq!((png.identifier.as_str(), png.type_identifier.as_str()), ("pic", "public.png"));
    assert!(Attachment::new("pic", "photo.png", (10 << 20) + 1).is_err());
    assert_eq!(Attachment::new("pic", "PHOTO.JPG", 1).unwrap().type_identifier, "public.jpeg", "the extension is case blind");
    assert!(Attachment::new("clip", "clip.wav", 5 << 20).is_ok());
    assert!(Attachment::new("clip", "clip.wav", (5 << 20) + 1).is_err(), "audio is limited to 5 MiB");
    assert_eq!(Attachment::new("movie", "movie.mp4", 50 << 20).unwrap().type_identifier, "public.mpeg-4");
}

#[test]
fn headless_an_attachment_of_an_unknown_type_is_refused() {
    assert!(Attachment::new("doc", "notes.txt", 1).is_err());
    assert!(Attachment::new("doc", "readme", 1).is_err(), "a file name without an extension has no type");
}

#[test]
fn headless_interruption_levels_order_from_passive_to_critical_and_round_trip_their_codes() {
    assert!(InterruptionLevel::Passive < InterruptionLevel::Active);
    assert!(InterruptionLevel::TimeSensitive < InterruptionLevel::Critical);
    for level in [InterruptionLevel::Passive, InterruptionLevel::Active, InterruptionLevel::TimeSensitive, InterruptionLevel::Critical] {
        assert_eq!(InterruptionLevel::from_code(level.code()), Some(level));
    }
    assert_eq!(InterruptionLevel::from_code(4), None);
}

#[test]
fn headless_date_components_refuse_out_of_range_fields() {
    assert!(DateComponents::default().is_valid(), "no field means every value");
    assert!(DateComponents { month: Some(12), day: Some(31), hour: Some(23), minute: Some(59), second: Some(59), ..Default::default() }.is_valid());
    assert!(!DateComponents { month: Some(13), ..Default::default() }.is_valid());
    assert!(!DateComponents { hour: Some(24), ..Default::default() }.is_valid());
    assert!(!DateComponents { weekday: Some(0), ..Default::default() }.is_valid(), "weekdays run from 1 for Sunday");
}

#[test]
fn headless_a_daily_calendar_trigger_fires_at_its_next_local_time() {
    let utc = |_t: i64| 0i64;
    let daily = CalendarTrigger { date: DateComponents { hour: Some(8), minute: Some(30), ..Default::default() }, repeats: true };
    assert_eq!(daily.next_trigger_date(0, &utc), Some(30_600), "1970-01-01 08:30 UTC");
    assert_eq!(daily.next_trigger_date(30_600, &utc), Some(117_000), "strictly after the given time");
}

#[test]
fn headless_a_weekday_calendar_trigger_skips_to_the_next_matching_day() {
    let utc = |_t: i64| 0i64;
    // 1970-01-01 was a Thursday, so the next Sunday is 1970-01-04.
    let sunday = CalendarTrigger { date: DateComponents { weekday: Some(1), hour: Some(0), minute: Some(0), ..Default::default() }, repeats: true };
    assert_eq!(sunday.next_trigger_date(0, &utc), Some(3 * 86_400));
}

#[test]
fn headless_a_calendar_trigger_follows_the_zone_offset() {
    let plus_one = |_t: i64| 3600i64;
    let daily_eight = CalendarTrigger { date: DateComponents { hour: Some(8), minute: Some(0), ..Default::default() }, repeats: true };
    assert_eq!(daily_eight.next_trigger_date(0, &plus_one), Some(25_200), "08:00 at UTC+1 is 07:00 UTC");
}

#[test]
fn headless_a_fixed_year_calendar_trigger_never_fires_again_after_its_date() {
    let utc = |_t: i64| 0i64;
    let once = CalendarTrigger { date: DateComponents { year: Some(1970), month: Some(1), day: Some(1), hour: Some(12), minute: Some(0), ..Default::default() }, repeats: false };
    assert_eq!(once.next_trigger_date(0, &utc), Some(43_200));
    assert_eq!(once.next_trigger_date(43_200, &utc), None);
}

#[test]
fn headless_an_interval_trigger_needs_a_positive_interval_and_a_minute_when_repeating() {
    assert!(IntervalTrigger::new(0.0, false).is_err());
    assert!(IntervalTrigger::new(-1.0, false).is_err());
    assert!(IntervalTrigger::new(f64::NAN, false).is_err());
    assert!(IntervalTrigger::new(f64::INFINITY, false).is_err());
    assert!(IntervalTrigger::new(59.0, true).is_err(), "a repeating interval is at least 60 seconds");
    assert!(IntervalTrigger::new(60.0, true).is_ok());
    assert!(IntervalTrigger::new(1.0, false).is_ok());
}

#[test]
fn headless_a_one_shot_interval_fires_once_and_then_is_past() {
    let t = IntervalTrigger::new(30.0, false).unwrap();
    assert_eq!(t.next_trigger_date(1000.0, 1000.0), Some(1030.0));
    assert_eq!(t.next_trigger_date(1000.0, 1031.0), None);
}

#[test]
fn headless_a_repeating_interval_stays_on_its_grid_of_seconds() {
    let t = IntervalTrigger::new(60.0, true).unwrap();
    assert_eq!(t.next_trigger_date(1000.0, 1000.0), Some(1060.0));
    assert_eq!(t.next_trigger_date(1000.0, 1130.0), Some(1180.0), "the grid is 1060, 1120, 1180");
    assert_eq!(t.next_trigger_date(1000.0, 1120.0), Some(1180.0), "the 1120 firing has passed");
}
