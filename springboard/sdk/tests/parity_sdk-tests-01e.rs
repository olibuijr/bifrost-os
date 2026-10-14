//! Parity tests for the SDK models behind user notifications, scheduled notifications, Transferable and App Shortcuts.
//! Each test drives the public API of one module with concrete values. No window, device, network or wall clock is used:
//! times are passed in as Unix seconds and zone offsets as plain numbers.

use bifrost_app::dnd::{DragItem, Payload};
use bifrost_app::intents::{Intent, IntentMode};
use bifrost_app::notification_schedule::{self, Reply, Schedule, Trigger, TriggerError};
use bifrost_app::pasteboard_types::Item as PasteItem;
use bifrost_app::transfer::{
    drag_item, drop_values, from_pasteboard_item, pasteboard_item, Kind, ReceivedTransferredFile, Representations,
    SentTransferredFile, Transferable, Viewer, Visibility,
};
use bifrost_app::usernotif_model::{
    foreground_presentation, Attachment, AuthorizationOptions as Opts, AuthorizationStatus, AuthorizationStore, CalendarTrigger,
    DateComponents, Decision, IntervalTrigger, PresentationOptions, Setting, Sound,
};
use bifrost_core::json::Json;
use std::path::PathBuf;

// ---------------------------------------------------------------------------------------------------------------------
// usernotif_model: authorization, presentation, sounds, attachments, calendar and interval triggers
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn authorization_option_sets_combine_by_bit() {
    let both = Opts::BADGE.union(Opts::CRITICAL_ALERT);
    assert!(both.contains(Opts::BADGE) && both.contains(Opts::CRITICAL_ALERT));
    assert!(!both.contains(Opts::SOUND));
    assert_eq!(both, Opts(17));
}

#[test]
fn only_granted_statuses_may_post() {
    assert!(AuthorizationStatus::Authorized.allows_posting());
    assert!(AuthorizationStatus::Provisional.allows_posting());
    assert!(AuthorizationStatus::Ephemeral.allows_posting());
    assert!(!AuthorizationStatus::Denied.allows_posting());
    assert!(!AuthorizationStatus::NotDetermined.allows_posting());
}

#[test]
fn a_denied_app_gets_no_presentation_and_no_sound() {
    let mut store = AuthorizationStore::default();
    assert!(!store.request("quiet", Opts::ALERT.union(Opts::SOUND), Decision::Deny));
    let s = store.settings("quiet");
    assert_eq!(s.authorization_status, AuthorizationStatus::Denied);
    assert_eq!((s.alert, s.sound, s.badge), (Setting::Disabled, Setting::Disabled, Setting::Disabled));
    let asked = PresentationOptions::BANNER.union(PresentationOptions::SOUND);
    assert_eq!(foreground_presentation(Some(asked), &s), PresentationOptions::NONE);
    assert!(!Sound::Default.allowed_by(&s));
}

#[test]
fn a_banner_needs_the_alert_setting_not_only_the_sound() {
    let mut store = AuthorizationStore::default();
    store.request("music", Opts::SOUND, Decision::Allow);
    let s = store.settings("music");
    let asked = PresentationOptions::BANNER.union(PresentationOptions::LIST).union(PresentationOptions::SOUND);
    assert_eq!(foreground_presentation(Some(asked), &s), PresentationOptions::SOUND);
}

#[test]
fn provisional_authorization_is_granted_without_a_prompt() {
    let mut store = AuthorizationStore::default();
    assert!(store.request("news", Opts::ALERT.union(Opts::PROVISIONAL), Decision::Deny));
    assert_eq!(store.status("news"), AuthorizationStatus::Provisional);
    assert_eq!(store.settings("news").alert, Setting::Enabled);
}

#[test]
fn the_authorization_text_skips_unknown_codes_and_empty_names() {
    let text = "\t2\t4\nbad\t9\t4\nok\t2\t4\nextra\t2\t4\textra\n";
    let store = AuthorizationStore::from_text(text);
    assert_eq!(store.status("ok"), AuthorizationStatus::Authorized);
    assert_eq!(store.settings("ok").alert, Setting::Enabled);
    assert_eq!(store.status("bad"), AuthorizationStatus::NotDetermined);
    assert_eq!(store.status(""), AuthorizationStatus::NotDetermined);
    assert_eq!(store.status("extra"), AuthorizationStatus::NotDetermined);
}

#[test]
fn critical_sounds_need_the_critical_alert_grant() {
    let mut store = AuthorizationStore::default();
    store.request("game", Opts::ALERT.union(Opts::SOUND), Decision::Allow);
    let plain = store.settings("game");
    assert_eq!(plain.critical_alert, Setting::NotSupported);
    assert!(!Sound::DefaultCritical.allowed_by(&plain));
    assert!(Sound::Default.allowed_by(&plain));

    store.request("alarm", Opts::ALERT.union(Opts::CRITICAL_ALERT), Decision::Allow);
    let alarm = store.settings("alarm");
    assert_eq!(alarm.critical_alert, Setting::Enabled);
    assert!(Sound::critical_with_volume(0.25).unwrap().allowed_by(&alarm));
    assert!(!Sound::Default.allowed_by(&alarm), "no sound grant: a default sound is held back");
}

#[test]
fn sound_names_are_capped_at_255_bytes() {
    assert_eq!(Sound::named(&"a".repeat(255)), Ok(Sound::Named("a".repeat(255))));
    assert!(Sound::named(&"a".repeat(256)).is_err());
    assert!(Sound::named("sub/ding.caf").is_err(), "a sound name is a bare file name");
}

#[test]
fn critical_volume_must_be_between_zero_and_one() {
    assert_eq!(Sound::critical_with_volume(0.0), Ok(Sound::DefaultCriticalVolume(0.0)));
    assert_eq!(Sound::critical_with_volume(1.0), Ok(Sound::DefaultCriticalVolume(1.0)));
    assert!(Sound::critical_with_volume(-0.1).is_err());
    assert!(Sound::critical_with_volume(1.01).is_err());
}

#[test]
fn attachment_types_follow_the_extension() {
    let wav = Attachment::new("s", "chime.WAV", 5 << 20).unwrap();
    assert_eq!(wav.type_identifier, "com.microsoft.waveform-audio");
    assert_eq!((wav.identifier.as_str(), wav.file_name.as_str()), ("s", "chime.WAV"));
    assert_eq!(Attachment::new("a", "clip.aiff", 1).unwrap().type_identifier, "public.aiff-audio");
    assert_eq!(Attachment::new("j", "p.jpeg", 10 << 20).unwrap().type_identifier, "public.jpeg");
    assert_eq!(Attachment::MAX_PER_REQUEST, 10);
}

#[test]
fn attachment_sizes_are_capped_per_type() {
    assert!(Attachment::new("j", "p.jpeg", (10 << 20) + 1).is_err(), "over the image limit");
    assert!(Attachment::new("m", "a.m4a", (5 << 20) + 1).is_err(), "over the audio limit");
    assert!(Attachment::new("m", "a.mp4", 50 << 20).is_ok());
    assert!(Attachment::new("m", "a.mp4", (50 << 20) + 1).is_err());
}

#[test]
fn date_components_reject_out_of_range_fields() {
    assert!(DateComponents::default().is_valid(), "unset fields match any value");
    assert!(DateComponents { hour: Some(23), minute: Some(59), second: Some(59), ..Default::default() }.is_valid());
    assert!(!DateComponents { hour: Some(24), ..Default::default() }.is_valid());
    assert!(!DateComponents { minute: Some(60), ..Default::default() }.is_valid());
    assert!(!DateComponents { second: Some(60), ..Default::default() }.is_valid());
    assert!(!DateComponents { month: Some(0), ..Default::default() }.is_valid());
    assert!(!DateComponents { day: Some(32), ..Default::default() }.is_valid());
    assert!(!DateComponents { weekday: Some(0), ..Default::default() }.is_valid());
    assert!(!DateComponents { weekday: Some(8), ..Default::default() }.is_valid());
}

#[test]
fn a_weekly_sunday_trigger_fires_on_sundays() {
    let utc = |_: i64| 0i64;
    // 1970-01-01 was a Thursday; Sunday 1970-01-04 is day 3 and weekday 1
    let sun = CalendarTrigger { date: DateComponents { weekday: Some(1), hour: Some(0), minute: Some(0), ..Default::default() }, repeats: true };
    assert_eq!(sun.next_trigger_date(0, &utc), Some(3 * 86_400));
    assert_eq!(sun.next_trigger_date(3 * 86_400, &utc), Some(10 * 86_400));
}

#[test]
fn an_interval_trigger_rejects_zero_and_non_finite_intervals() {
    assert!(IntervalTrigger::new(-1.0, false).is_err());
    assert!(IntervalTrigger::new(f64::INFINITY, false).is_err());
    assert_eq!(IntervalTrigger::MIN_REPEAT_SECS, 60.0);
    assert_eq!(IntervalTrigger::new(90.0, false).unwrap(), IntervalTrigger { seconds: 90.0, repeats: false });
}

// ---------------------------------------------------------------------------------------------------------------------
// notification_schedule: triggers, validation, the request builder and service replies
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_repeating_interval_under_a_minute_is_refused() {
    assert_eq!(Schedule::checked_interval("tick", "Tick", 59, true), Err(TriggerError::RepeatBelowMinimum));
    assert!(Schedule::checked_interval("tick", "Tick", 60, true).is_ok());
    assert_eq!(Schedule::checked_interval("once", "Once", 0, false), Err(TriggerError::ZeroInterval));
    assert!(Schedule::checked_interval("once", "Once", 1, false).is_ok());
}

#[test]
fn a_calendar_schedule_checks_its_time_and_weekdays() {
    assert_eq!(Schedule::checked_calendar("c", "C", 24, 0, &[], false), Err(TriggerError::InvalidTime));
    assert_eq!(Schedule::checked_calendar("c", "C", 9, 60, &[], false), Err(TriggerError::InvalidTime));
    assert_eq!(Schedule::checked_calendar("c", "C", 9, 0, &[2, 8], false), Err(TriggerError::InvalidWeekday(8)));
    assert_eq!(Schedule::checked_calendar("c", "C", 9, 0, &[0], false), Err(TriggerError::InvalidWeekday(0)));
    assert!(Schedule::checked_calendar("c", "C", 23, 59, &[1, 7], true).is_ok());
}

#[test]
fn an_interval_trigger_fires_after_its_delay_whatever_the_zone() {
    let t = Trigger::Interval { seconds: 90, repeats: true };
    assert_eq!(t.next_trigger_date(1_000, 3_600), Some(1_090));
    assert_eq!(t.next_trigger_date(1_000, -18_000), Some(1_090));
}

#[test]
fn a_daily_calendar_trigger_is_the_next_local_time_after_now() {
    let t = Trigger::Calendar { hour: 9, minute: 15, weekdays: vec![], repeats: true };
    assert_eq!(t.next_trigger_date(0, 0), Some(9 * 3_600 + 15 * 60));
    // a time exactly at the fire moment counts as passed
    assert_eq!(t.next_trigger_date(33_300, 0), Some(33_300 + 86_400));
    // a zone one hour east: 09:15 local is 08:15 UTC
    assert_eq!(t.next_trigger_date(0, 3_600), Some(8 * 3_600 + 15 * 60));
}

#[test]
fn a_weekday_calendar_trigger_skips_other_days() {
    // 1970-01-01 was a Thursday (5); Monday 1970-01-05 is day 4 (weekday 2)
    let mon = Trigger::Calendar { hour: 0, minute: 0, weekdays: vec![2], repeats: false };
    assert_eq!(mon.next_trigger_date(0, 0), Some(4 * 86_400));
    let mon_or_thu = Trigger::Calendar { hour: 0, minute: 0, weekdays: vec![2, 5], repeats: false };
    assert_eq!(mon_or_thu.next_trigger_date(0, 0), Some(4 * 86_400), "Monday comes before the next Thursday");
    assert_eq!(mon_or_thu.next_trigger_date(-1, 0), Some(0), "Thursday 00:00 is still ahead of -1");
}

#[test]
fn an_invalid_trigger_has_no_next_date() {
    let bad_hour = Trigger::Calendar { hour: 25, minute: 0, weekdays: vec![], repeats: false };
    assert_eq!(bad_hour.next_trigger_date(0, 0), None);
    let zero = Trigger::Interval { seconds: 0, repeats: false };
    assert_eq!(zero.next_trigger_date(0, 0), None);
}

#[test]
fn a_schedule_keeps_its_content_fields() {
    let s = Schedule::calendar("standup", "Standup", 9, 15, &[2, 3], true)
        .subtitle("Team")
        .body("Daily sync")
        .thread("work")
        .category("meeting")
        .badge(2);
    assert_eq!((s.id.as_str(), s.title.as_str(), s.subtitle.as_str(), s.body.as_str()), ("standup", "Standup", "Team", "Daily sync"));
    assert_eq!((s.thread.as_str(), s.category.as_str(), s.badge), ("work", "meeting", Some(2)));
    assert_eq!(s.trigger, Trigger::Calendar { hour: 9, minute: 15, weekdays: vec![2, 3], repeats: true });
    assert_eq!(s.validate(), Ok(()));
}

#[test]
fn service_replies_are_read_by_their_field() {
    let added = Json::Obj(vec![("added".into(), Json::Str("standup".into()))]);
    assert_eq!(notification_schedule::reply(&Ok(added)), Reply::Added("standup".into()));
    let removed = Json::Obj(vec![("removed".into(), Json::Num(2.0))]);
    assert_eq!(notification_schedule::reply(&Ok(removed)), Reply::Removed(2));
    let pending = Json::Obj(vec![("pending".into(), Json::Arr(vec![Json::Str("a".into())]))]);
    assert_eq!(notification_schedule::reply(&Ok(pending)), Reply::Pending(vec!["a".into()]));
    assert_eq!(notification_schedule::reply(&Err("refused".into())), Reply::Failed("refused".into()));
    assert!(matches!(notification_schedule::reply(&Ok(Json::Obj(vec![]))), Reply::Failed(_)));
}

// ---------------------------------------------------------------------------------------------------------------------
// transfer: Transferable representations, visibility, pasteboard items, drag and drop, files
// ---------------------------------------------------------------------------------------------------------------------

fn note_bytes(n: &Note) -> Vec<u8> {
    n.0.clone().into_bytes()
}

fn note_from(b: &[u8]) -> Option<Note> {
    String::from_utf8(b.to_vec()).ok().map(Note)
}

#[derive(Clone, Debug, PartialEq)]
struct Note(String);

impl Transferable for Note {
    fn transfer_representations() -> Representations<Note> {
        Representations::new()
            .codable("com.example.note", Visibility::All, note_bytes, note_from)
            .data("public.utf8-plain-text", Visibility::All, note_bytes, note_from)
            .data("com.example.raw", Visibility::OwnProcess, note_bytes, note_from)
    }
}

fn label_bytes(l: &Label) -> Vec<u8> {
    l.0.clone().into_bytes()
}

fn label_from(b: &[u8]) -> Option<Label> {
    String::from_utf8(b.to_vec()).ok().map(Label)
}

#[derive(Clone, Debug, PartialEq)]
struct Label(String);

impl Transferable for Label {
    fn transfer_representations() -> Representations<Label> {
        Representations::new().data("com.example.label", Visibility::All, label_bytes, label_from)
    }
}

/// Forwards a `Label` through a proxy that is only visible to the team.
#[derive(Clone, Debug, PartialEq)]
struct Pin(String);

impl Transferable for Pin {
    fn transfer_representations() -> Representations<Pin> {
        Representations::new().proxy_of::<Label>(Visibility::Team, |p: &Pin| Label(p.0.clone()), Some(|l: Label| Pin(l.0)))
    }
}

/// Forwards a `Label` without a way back: export only.
#[derive(Clone, Debug, PartialEq)]
struct Sealed(String);

impl Transferable for Sealed {
    fn transfer_representations() -> Representations<Sealed> {
        Representations::new().proxy_of::<Label>(Visibility::All, |s: &Sealed| Label(s.0.clone()), None)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Doc(PathBuf);

fn doc_out(d: &Doc) -> SentTransferredFile {
    SentTransferredFile::new(d.0.clone())
}

fn doc_in(r: &ReceivedTransferredFile) -> Option<Doc> {
    Some(Doc(r.file.clone()))
}

impl Transferable for Doc {
    fn transfer_representations() -> Representations<Doc> {
        Representations::new().file("com.example.document", Visibility::All, doc_out, doc_in)
    }
}

#[test]
fn visibility_gates_each_viewer() {
    assert!(Visibility::All.is_visible_to(Viewer::Other));
    assert!(Visibility::Team.is_visible_to(Viewer::Team));
    assert!(!Visibility::Team.is_visible_to(Viewer::Other));
    assert!(Visibility::OwnProcess.is_visible_to(Viewer::OwnProcess));
    assert!(!Visibility::OwnProcess.is_visible_to(Viewer::Team));
}

#[test]
fn exported_and_imported_types_follow_declaration_order_and_visibility() {
    let reps = Note::transfer_representations();
    assert_eq!(reps.exported_content_type(Viewer::Other), Some("com.example.note"));
    assert_eq!(reps.exported_content_type(Viewer::OwnProcess), Some("com.example.note"));
    assert_eq!(reps.types_for(Viewer::Other), vec!["com.example.note", "public.utf8-plain-text"]);
    assert_eq!(reps.types_for(Viewer::OwnProcess), vec!["com.example.note", "public.utf8-plain-text", "com.example.raw"]);
    assert_eq!(reps.imported_content_types(), vec!["com.example.note", "public.utf8-plain-text", "com.example.raw"]);
    assert_eq!(reps.kind_of("com.example.note"), Some(Kind::Codable));
    assert_eq!(reps.kind_of("public.utf8-plain-text"), Some(Kind::Data));
    assert_eq!(reps.kind_of("missing"), None);
}

#[test]
fn export_hides_owner_types_from_other_apps_and_import_rejects_bad_bytes() {
    let reps = Note::transfer_representations();
    let n = Note("abc".into());
    assert_eq!(reps.export(&n, "com.example.raw", Viewer::Other), None);
    assert_eq!(reps.export(&n, "com.example.raw", Viewer::OwnProcess), Some(b"abc".to_vec()));
    assert_eq!(reps.import("com.example.note", b"abc"), Some(n.clone()));
    assert_eq!(reps.import("com.example.note", &[0xff, 0xfe]), None, "bytes the importer rejects");
    assert_eq!(reps.import("com.example.unknown", b"abc"), None);
}

#[test]
fn a_proxy_is_offered_only_as_visible_as_its_stricter_side() {
    let reps = Pin::transfer_representations();
    assert_eq!(reps.types_for(Viewer::Other), Vec::<&str>::new(), "a team-only proxy is hidden from other apps");
    assert_eq!(reps.types_for(Viewer::Team), vec!["com.example.label"]);
    assert_eq!(reps.export(&Pin("x".into()), "com.example.label", Viewer::Other), None);
    assert_eq!(reps.export(&Pin("x".into()), "com.example.label", Viewer::Team), Some(b"x".to_vec()));
    assert_eq!(reps.import("com.example.label", b"y"), Some(Pin("y".into())));
    assert_eq!(reps.imported_content_types(), vec!["com.example.label"]);
    assert_eq!(reps.kind_of("com.example.label"), Some(Kind::Proxy));
}

#[test]
fn a_proxy_without_a_way_back_is_export_only() {
    let reps = Sealed::transfer_representations();
    assert_eq!(reps.export(&Sealed("s".into()), "com.example.label", Viewer::Other), Some(b"s".to_vec()));
    assert_eq!(reps.import("com.example.label", b"s"), None);
    assert!(reps.imported_content_types().is_empty());
}

#[test]
fn a_file_representation_hands_over_and_rebuilds_its_file() {
    let reps = Doc::transfer_representations();
    let path = PathBuf::from("/notes/plan.txt");
    let doc = Doc(path.clone());
    assert_eq!(reps.kind_of("com.example.document"), Some(Kind::File));
    assert_eq!(reps.export_file(&doc, "com.example.document", Viewer::Other), Some(SentTransferredFile::new(path.clone())));
    let received = ReceivedTransferredFile { file: path.clone(), is_original_file: false };
    assert_eq!(reps.import_file("com.example.document", &received), Some(doc));
    assert_eq!(reps.imported_content_types(), vec!["com.example.document"]);
}

#[test]
fn a_sent_file_opened_in_place_hands_over_the_original() {
    let sent = SentTransferredFile::new("/notes/plan.txt").allow_open_in_place(true);
    let got = sent.receive(std::path::Path::new("/unused")).unwrap();
    assert_eq!(got, ReceivedTransferredFile { file: PathBuf::from("/notes/plan.txt"), is_original_file: true });
}

#[test]
fn a_sent_file_without_open_in_place_is_copied_to_the_receiver() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("parity_sdk-tests-01e-transfer");
    let dest = dir.join("received");
    std::fs::create_dir_all(&dest).unwrap();
    let src = dir.join("sent.txt");
    std::fs::write(&src, b"payload").unwrap();
    let got = SentTransferredFile::new(src.clone()).receive(&dest).unwrap();
    assert!(!got.is_original_file);
    assert_eq!(got.file, dest.join("sent.txt"));
    assert_eq!(std::fs::read(&got.file).unwrap(), b"payload");
}

#[test]
fn a_pasteboard_item_carries_each_visible_type_and_reads_back() {
    let reps = Note::transfer_representations();
    let item = pasteboard_item(&reps, &Note("hi".into()));
    assert_eq!(item.data("com.example.note"), Some(&b"hi"[..]));
    assert_eq!(item.data("public.utf8-plain-text"), Some(&b"hi"[..]));
    assert_eq!(item.data("com.example.raw"), None, "owner-only types stay off the board");
    assert_eq!(from_pasteboard_item(&reps, &item), Some(Note("hi".into())));
}

#[test]
fn a_pasteboard_item_with_only_an_owner_type_still_imports() {
    let reps = Note::transfer_representations();
    let item = PasteItem::default().with("com.example.raw", b"z");
    assert_eq!(from_pasteboard_item(&reps, &item), Some(Note("z".into())));
    let unknown = PasteItem::default().with("com.example.label", b"z");
    assert_eq!(from_pasteboard_item(&reps, &unknown), None, "no offered type this model reads");
}

#[test]
fn a_drag_carries_the_text_payload_and_its_types() {
    let item = drag_item(&Note::transfer_representations(), &Note("hi".into())).unwrap();
    assert_eq!(item.types, vec!["com.example.note".to_string(), "public.utf8-plain-text".to_string()]);
    assert!(matches!(&item.payload, Payload::Text(t) if t == "hi"));
}

#[test]
fn a_drag_of_hidden_types_carries_nothing() {
    assert!(drag_item(&Pin::transfer_representations(), &Pin("x".into())).is_none());
}

#[test]
fn a_drop_rebuilds_text_values_and_skips_links_the_model_cannot_read() {
    let reps = Note::transfer_representations();
    let items = vec![
        DragItem { types: vec![], payload: Payload::Text("one".into()) },
        DragItem { types: vec![], payload: Payload::Url("https://example.test".into()) },
    ];
    assert_eq!(drop_values(&reps, &items), vec![Note("one".into())]);
}

// ---------------------------------------------------------------------------------------------------------------------
// intents: App Shortcuts flag in the manifest table and in the intent JSON
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_shortcut_intent_is_written_to_the_manifest_with_its_flag() {
    let open = Intent::new("open-notes", "Open Notes", "Shows the notes").shortcut();
    let toml = open.manifest_toml();
    assert!(toml.starts_with("[[intent]]\nid = \"open-notes\"\n"), "got {toml}");
    assert!(toml.contains("shortcut = true\n"));
    assert!(toml.contains("params = []"));
    let plain = Intent::new("refresh", "Refresh", "");
    assert!(plain.manifest_toml().contains("shortcut = false\n"));
}

#[test]
fn a_shortcut_flag_is_listed_in_the_intent_json() {
    let open = Intent::new("open-notes", "Open Notes", "").shortcut();
    assert_eq!(open.to_json().get("shortcut"), Some(&Json::Bool(true)));
    assert_eq!(Intent::new("refresh", "Refresh", "").to_json().get("shortcut"), Some(&Json::Bool(false)));
    assert!(!open.supports(IntentMode::Foreground), "a shortcut runs in the background unless it asks for foreground");
    assert!(open.foreground().supports(IntentMode::Foreground));
}
