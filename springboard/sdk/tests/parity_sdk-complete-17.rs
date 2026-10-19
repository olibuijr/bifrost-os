//! Parity sdk-complete-17: headless tests for the app-facing font API and the process environment. Family
//! substitution maps a reference family to open equivalents, the design variants change the requested family, the
//! cascade lists script faces, and `UserFonts` reads the packed system and monospaced faces. The process tests cover
//! sudden termination, activities, thermal states and workspace notification names. The fonts come from the packed
//! asset directory; nothing draws, reads a device or depends on the host's load.

use bifrost_app::font_descriptor::UserFonts;
use bifrost_app::midi::endpoints::ObjectId;
use bifrost_app::midi::events::MidiEvent;
use bifrost_app::midi::hub::Hub;
use bifrost_app::midi::ump::{from_midi1, to_midi1, words_for_type, Ump};
use bifrost_app::font_substitution::{candidates, cascade_list, class_of_family, substitute, with_design, FamilyClass, SystemDesign};
use bifrost_app::process_info::{parse_low_power, thermal_state_for, ActivityOptions, ActivityTracker, SuddenTermination, ThermalState, TripPoint};
use bifrost_app::workspace::{icon_for_content_type, icon_for_file, notification_name};
use bifrost_proto::WorkspaceEventKind;
use bifrost_render::fontobj::FontSpec;

const ASSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets");

#[test]
fn a_reference_sans_family_substitutes_to_the_open_grotesque_faces() {
    assert_eq!(substitute("Helvetica"), vec!["Inter", "Geist", "Source Sans 3", "Noto Sans"]);
    assert_eq!(class_of_family("  Helvetica Neue "), Some(FamilyClass::GrotesqueSans), "case and spaces are ignored");
}

#[test]
fn a_reference_serif_family_substitutes_to_the_open_transitional_faces() {
    assert_eq!(substitute("Times New Roman").first().copied(), Some("Noto Serif"));
    assert_eq!(class_of_family("Times New Roman"), Some(FamilyClass::TransitionalSerif));
}

#[test]
fn a_family_the_table_does_not_know_has_no_substitute() {
    assert!(substitute("No Such Font").is_empty());
    assert_eq!(class_of_family("No Such Font"), None);
}

#[test]
fn a_reference_monospaced_family_substitutes_to_the_open_fixed_pitch_faces() {
    assert_eq!(substitute("Menlo").first().copied(), Some("DejaVu Sans Mono"), "the licensed face gets its open stand-in first");
    assert!(substitute("Menlo").contains(&"JetBrains Mono"));
}

#[test]
fn candidates_are_the_open_equivalents_of_the_requested_family_in_order() {
    let spec = FontSpec::new().family("Helvetica");
    let families: Vec<Option<String>> = candidates(&spec).into_iter().map(|s| s.family).collect();
    assert_eq!(
        families,
        vec![Some("Inter".to_string()), Some("Geist".to_string()), Some("Source Sans 3".to_string()), Some("Noto Sans".to_string())]
    );
}

#[test]
fn an_unknown_family_falls_back_to_the_generic_class_of_its_traits() {
    let sans = candidates(&FontSpec::new().family("Unknown Face"));
    assert_eq!(sans.first().and_then(|s| s.family.clone()), Some("Inter".to_string()));

    let fixed = candidates(&FontSpec::new().family("Unknown Face").mono());
    assert_eq!(fixed.first().and_then(|s| s.family.clone()), Some("JetBrains Mono".to_string()));
}

#[test]
fn a_spec_without_a_family_has_no_candidates() {
    assert!(candidates(&FontSpec::new()).is_empty());
}

#[test]
fn the_monospaced_design_requests_a_fixed_pitch_face() {
    let spec = with_design(FontSpec::new(), SystemDesign::Monospaced);
    assert_eq!(spec.family.as_deref(), Some("JetBrains Mono"));
    assert!(spec.traits.is_mono(), "the monospaced design also sets the fixed-pitch trait");
}

#[test]
fn the_rounded_and_serif_designs_request_their_first_open_equivalent() {
    assert_eq!(with_design(FontSpec::new(), SystemDesign::Rounded).family.as_deref(), Some("Nunito"));
    assert_eq!(with_design(FontSpec::new(), SystemDesign::Serif).family.as_deref(), Some("Noto Serif"));
}

#[test]
fn the_default_design_leaves_the_spec_unchanged() {
    let spec = FontSpec::new().family("Inter").size(13.0);
    assert_eq!(with_design(spec.clone(), SystemDesign::Default), spec);
}

#[test]
fn the_cascade_for_japanese_and_traditional_chinese_lists_each_script_once() {
    assert_eq!(
        cascade_list(&["ja", "ja-JP", "zh-Hant"]),
        vec!["Noto Sans CJK JP", "Noto Serif CJK JP", "Noto Sans CJK TC", "Noto Serif CJK TC"]
    );
}

#[test]
fn the_cascade_takes_the_primary_language_of_an_underscored_tag() {
    assert_eq!(cascade_list(&["ar_EG"]), vec!["Noto Naskh Arabic", "Noto Sans Arabic"]);
}

#[test]
fn an_unknown_language_tag_adds_no_cascade_faces() {
    assert!(cascade_list(&["xx"]).is_empty());
}

#[test]
fn the_system_font_object_is_the_packed_system_face_at_the_requested_size() {
    let o = UserFonts::system_object(ASSET_DIR, 16.0).expect("the packed system face loads");
    assert_eq!(o.postscript_name(), "Inter-Regular");
    assert_eq!(o.size(), 16.0);
}

#[test]
fn the_monospaced_font_object_is_the_packed_monospaced_face() {
    let o = UserFonts::mono_object(ASSET_DIR, 12.0).expect("the packed monospaced face loads");
    assert_eq!(o.postscript_name(), "JetBrainsMono-Regular");
}

#[test]
fn a_system_font_object_is_missing_when_the_font_directory_has_no_face() {
    assert!(UserFonts::system_object("/nonexistent-bifrost-font-dir", 12.0).is_none());
}

#[test]
fn the_system_path_is_the_packed_face_when_no_override_is_saved() {
    assert!(UserFonts::system_path(ASSET_DIR).ends_with("SFUI.ttf"));
}

// process information: sudden termination, activities, thermal states and low power mode

#[test]
fn sudden_termination_starts_disabled_and_one_balanced_enable_turns_it_on() {
    let mut s = SuddenTermination::default();
    assert!(!s.is_enabled(), "an app starts with termination disabled");
    assert_eq!(s.enable(), Some(true), "the first enable flips the state");
    assert!(s.is_enabled());
    assert_eq!(s.disable(), Some(false), "a disable flips it back");
    assert!(!s.is_enabled());
}

#[test]
fn nested_disables_need_matching_enables_before_termination_is_allowed() {
    let mut s = SuddenTermination::default();
    assert_eq!(s.disable(), None, "a second disable does not flip the state");
    assert_eq!(s.enable(), None, "one enable leaves one disable standing");
    assert!(!s.is_enabled());
    assert_eq!(s.enable(), Some(true));
    assert!(s.is_enabled());
}

#[test]
fn an_unbalanced_enable_is_ignored() {
    let mut s = SuddenTermination::default();
    assert_eq!(s.enable(), Some(true));
    assert_eq!(s.enable(), None, "enabling again changes nothing");
    assert!(s.is_enabled());
}

#[test]
fn an_activity_reports_the_union_only_when_it_changes() {
    let mut t = ActivityTracker::default();
    let (first, changed) = t.begin(ActivityOptions::USER_INITIATED, "saving");
    assert_eq!(changed, Some(ActivityOptions::USER_INITIATED.0));
    let (second, same) = t.begin(ActivityOptions::USER_INITIATED, "saving again");
    assert_eq!(same, None, "the same option adds nothing to the union");
    assert_eq!(t.count(), 2);
    assert_eq!(t.end(first), None, "the second activity still holds the option");
    assert_eq!(t.end(second), Some(0), "the last activity out clears the union");
    assert_eq!(t.count(), 0);
}

#[test]
fn distinct_activity_options_widen_the_union_and_combine_as_bits() {
    let mut t = ActivityTracker::default();
    let (_, _) = t.begin(ActivityOptions::USER_INITIATED, "a");
    let (_, widened) = t.begin(ActivityOptions::BACKGROUND, "b");
    let both = ActivityOptions::USER_INITIATED.union(ActivityOptions::BACKGROUND).0;
    assert_eq!(widened, Some(both));
}

#[test]
fn without_trip_points_a_zone_is_fair_at_75_degrees_and_critical_at_95() {
    assert_eq!(thermal_state_for(70_000, &[]), ThermalState::Nominal);
    assert_eq!(thermal_state_for(75_000, &[]), ThermalState::Fair);
    assert_eq!(thermal_state_for(85_000, &[]), ThermalState::Serious);
    assert_eq!(thermal_state_for(95_000, &[]), ThermalState::Critical);
}

#[test]
fn the_passive_hot_and_critical_trips_set_the_states_five_degrees_below_critical() {
    let trips = [
        TripPoint { kind: "passive".into(), temp: 80_000 },
        TripPoint { kind: "hot".into(), temp: 90_000 },
        TripPoint { kind: "critical".into(), temp: 100_000 },
    ];
    assert_eq!(thermal_state_for(79_999, &trips), ThermalState::Nominal);
    assert_eq!(thermal_state_for(80_000, &trips), ThermalState::Fair);
    assert_eq!(thermal_state_for(90_000, &trips), ThermalState::Serious);
    assert_eq!(thermal_state_for(95_000, &trips), ThermalState::Critical);
}

#[test]
fn a_zone_without_a_hot_trip_is_serious_ten_degrees_below_critical() {
    let trips = [TripPoint { kind: "critical".into(), temp: 100_000 }];
    assert_eq!(thermal_state_for(89_999, &trips), ThermalState::Nominal);
    assert_eq!(thermal_state_for(90_000, &trips), ThermalState::Serious);
}

#[test]
fn a_trip_without_a_temperature_never_triggers() {
    let trips = [TripPoint { kind: "passive".into(), temp: 0 }];
    assert_eq!(thermal_state_for(200_000, &trips), ThermalState::Nominal);
}

#[test]
fn thermal_states_order_from_nominal_to_critical_and_round_trip_their_wire_names() {
    assert!(ThermalState::Nominal < ThermalState::Fair && ThermalState::Serious < ThermalState::Critical);
    for s in [ThermalState::Nominal, ThermalState::Fair, ThermalState::Serious, ThermalState::Critical] {
        assert_eq!(ThermalState::parse(s.name()), Some(s));
    }
    assert_eq!(ThermalState::parse(" critical\n"), Some(ThermalState::Critical), "surrounding space is ignored");
    assert_eq!(ThermalState::parse("hot"), None);
}

#[test]
fn low_power_mode_is_read_from_the_power_line_only() {
    assert!(parse_low_power("low_power=1\n"));
    assert!(parse_low_power("state=on\n  low_power=1  \n"));
    assert!(!parse_low_power("low_power=0\n"));
    assert!(!parse_low_power(""));
}

// workspace: notification names and icon names for content types and file names

#[test]
fn each_workspace_event_kind_has_its_notification_name() {
    assert_eq!(notification_name(WorkspaceEventKind::WillSleep), "workspace.willSleep");
    assert_eq!(notification_name(WorkspaceEventKind::DidWake), "workspace.didWake");
    assert_eq!(notification_name(WorkspaceEventKind::ScreensDidSleep), "workspace.screensDidSleep");
    assert_eq!(notification_name(WorkspaceEventKind::DidLaunchApplication), "workspace.didLaunchApplication");
}

#[test]
fn a_content_type_gets_the_glyph_of_its_nearest_known_ancestor() {
    assert_eq!(icon_for_content_type("public.folder"), "folder");
    assert_eq!(icon_for_content_type("public.movie"), "film");
    assert_eq!(icon_for_content_type("com.example.unknown"), "doc", "an unknown type gets the plain document glyph");
}

#[test]
fn a_file_name_gets_the_icon_of_its_extension() {
    assert_eq!(icon_for_file("holiday.png"), "photo");
    assert_eq!(icon_for_file("notes.txt"), "doc.text");
    assert_eq!(icon_for_file("README"), "doc", "a name without an extension gets the plain document glyph");
}

// midi: the routing hub that apps and the MIDI server share, and the UMP packets it carries

#[test]
fn an_event_from_a_connected_source_reaches_the_listening_app() {
    let mut hub = Hub::new();
    let (client, src, _dst) = hub.create_app("Sequencer");
    assert!(hub.registry.connect_source(client, src));
    assert_eq!(hub.send_from_source(src, 100, Ump::one(0x2090_3C64)), 1);
    assert_eq!(hub.pending(client), 1);
    let got = hub.drain_inbox(client, 100);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].host_ns, 100);
    assert_eq!(got[0].source, src);
    assert_eq!(got[0].packet, Ump::one(0x2090_3C64));
}

#[test]
fn an_app_that_is_not_connected_to_a_source_queues_nothing() {
    let mut hub = Hub::new();
    let (client, src, _dst) = hub.create_app("Sequencer");
    assert_eq!(hub.send_from_source(src, 100, Ump::one(1)), 0);
    assert_eq!(hub.pending(client), 0);
}

#[test]
fn events_leave_in_timestamp_order_and_equal_stamps_keep_their_arrival_order() {
    let mut hub = Hub::new();
    let (client, src, _dst) = hub.create_app("Sequencer");
    hub.registry.connect_source(client, src);
    hub.send_from_source(src, 30, Ump::one(1));
    hub.send_from_source(src, 10, Ump::one(2));
    hub.send_from_source(src, 30, Ump::one(3));
    let packets: Vec<u32> = hub.drain_inbox(client, 30).iter().map(|t| t.packet.words[0]).collect();
    assert_eq!(packets, vec![2, 1, 3]);
}

#[test]
fn draining_stops_at_the_given_time_and_keeps_later_events_queued() {
    let mut hub = Hub::new();
    let (client, src, _dst) = hub.create_app("Sequencer");
    hub.registry.connect_source(client, src);
    hub.send_from_source(src, 10, Ump::one(1));
    hub.send_from_source(src, 50, Ump::one(2));
    assert_eq!(hub.drain_inbox(client, 20).len(), 1);
    assert_eq!(hub.pending(client), 1, "the event at 50 waits");
}

#[test]
fn a_disposed_app_has_no_inbox_and_cannot_be_disposed_twice() {
    let mut hub = Hub::new();
    let (client, src, _dst) = hub.create_app("Sequencer");
    hub.registry.connect_source(client, src);
    hub.send_from_source(src, 10, Ump::one(1));
    assert!(hub.dispose_app(client));
    assert_eq!(hub.pending(client), 0);
    assert!(!hub.dispose_app(client));
}

#[test]
fn a_destination_takes_what_is_sent_to_it_with_its_sender() {
    let mut hub = Hub::new();
    let (_client, src, dst) = hub.create_app("Sequencer");
    assert!(hub.send_to_destination(src, dst, 5, Ump::one(7)));
    let got = hub.take_destination(dst, 5);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].source, src);
    assert_eq!(got[0].packet.words[0], 7);
    assert!(hub.take_destination(dst, 5).is_empty(), "taking removes the events");
}

#[test]
fn an_untracked_destination_refuses_events() {
    let mut hub = Hub::new();
    let (_client, src, _dst) = hub.create_app("Sequencer");
    assert!(!hub.send_to_destination(src, ObjectId(9999), 1, Ump::one(1)));
}

#[test]
fn a_midi_one_channel_note_becomes_one_type_two_packet_and_back() {
    let event = MidiEvent::NoteOn { channel: 0, key: 60, velocity: 100 };
    let packet = from_midi1(0, event);
    assert_eq!(packet, Ump::one(0x2090_3C64));
    assert_eq!(packet.message_type(), 2);
    assert_eq!(to_midi1(&packet), Some(event));
}

#[test]
fn packet_sizes_follow_the_message_type() {
    assert_eq!(words_for_type(2), 1, "channel voice messages are one word");
    assert_eq!(words_for_type(4), 2, "MIDI 2.0 channel voice messages are two words");
    assert_eq!(words_for_type(0xB), 3);
    assert_eq!(words_for_type(0xF), 4);
}
