//! Parity sdk-os-10: headless tests for the system-level rows that gained an app-facing model. No test opens a device,
//! reads the real card list, starts playback or touches a sound card: each listing, report and policy is plain data.

use std::path::{Path, PathBuf};

use bifrost_app::audio_hal::{ObjectId, ObjectKind, Scope, System, SYSTEM};
use bifrost_app::audio_session::{Category, InterruptionType, Mode, Options, Route, SessionError, SessionEvent, SessionHub};
use bifrost_app::fswatch::{EventJournal, WatchEvent, WatchEventFlags, SINCE_NOW};
use bifrost_app::hid_virtual::{modifier, validate_name, KeyboardReport, ReportError, VirtualKeyboard, BOOT_KEYBOARD_DESCRIPTOR, REPORT_LEN};
use bifrost_app::input_grab::{may_grab, Refusal, Requester};
use bifrost_app::media_extension::{register_reader, Declaration, Entitlement, ExtensionError};
use bifrost_app::sign_in::{find_code, scheme_of};
use bifrost_app::video_decode::{choose, Backend, Codec, DecodeError, HardwareSupport, Preference};
use bifrost_ui::format_readers::{FormatReader, ReaderRegistry, RegistryError};
use bifrost_ui::hid_report::{ReportDescriptor, ReportKind};
use springboard::bare::input::Roles;
use springboard::media::capture::{pcm16, wav_bytes, wav_duration_ms};
use springboard::media::codec::{from_extension, sniff, Kind};

const LISTING: &str = "00-00: ALC892 Analog : ALC892 Analog : playback 1 : capture 1\n\
01-00: HDMI 0 : HDMI 0 : playback 1\n\
02-00: USB Mic : USB Mic : capture 1\n";

// ---- audio_hal: the audio hardware object model -------------------------------------------------------------------

#[test]
fn a_listing_gives_each_device_an_id_after_the_system_and_its_streams_after_the_device() {
    let system = System::from_listing(LISTING);
    let ids: Vec<(u32, Option<u32>)> = system
        .devices()
        .iter()
        .map(|d| (d.id.0, d.streams.first().map(|s| s.id.0)))
        .collect();
    // Device 0 is object 2 with output stream 3 and input stream 4; device 1 is 5 with output 6; device 2 is 7 with input 8.
    assert_eq!(ids, vec![(2, Some(3)), (5, Some(6)), (7, Some(8))]);
    assert_eq!(SYSTEM, ObjectId(1));
}

#[test]
fn a_device_has_one_output_and_one_input_stream_only_when_the_listing_names_them() {
    let system = System::from_listing(LISTING);
    let both = &system.devices()[0];
    assert_eq!(both.streams.len(), 2);
    assert_eq!(both.stream(Scope::Output).map(|s| s.substreams), Some(1));
    assert_eq!(both.stream(Scope::Input).map(|s| s.id), Some(ObjectId(4)));
    let hdmi = &system.devices()[1];
    assert!(hdmi.stream(Scope::Input).is_none());
    let mic = &system.devices()[2];
    assert!(mic.stream(Scope::Output).is_none());
}

#[test]
fn a_device_is_addressed_by_its_card_and_device_numbers() {
    let system = System::from_listing(LISTING);
    assert_eq!(system.devices()[0].uid(), "hw:0,0");
    assert_eq!(system.devices()[2].uid(), "hw:2,0");
    assert_eq!(system.devices()[2].name, "USB Mic");
    assert_eq!(system.devices()[0].identifier, "ALC892 Analog");
}

#[test]
fn the_default_device_for_a_direction_is_the_first_device_with_a_stream_in_it() {
    let system = System::from_listing(LISTING);
    assert_eq!(system.default_device(Scope::Output).map(|d| d.id), Some(ObjectId(2)));
    assert_eq!(system.default_device(Scope::Input).map(|d| d.id), Some(ObjectId(7)));
}

#[test]
fn the_kind_of_an_object_id_names_the_system_a_device_or_a_stream() {
    let system = System::from_listing(LISTING);
    assert_eq!(system.kind_of(SYSTEM), Some(ObjectKind::System));
    assert_eq!(system.kind_of(ObjectId(2)), Some(ObjectKind::Device));
    assert_eq!(system.kind_of(ObjectId(3)), Some(ObjectKind::Stream));
    assert_eq!(system.kind_of(ObjectId(99)), None);
}

#[test]
fn a_stream_is_owned_by_the_device_it_was_listed_under() {
    let system = System::from_listing(LISTING);
    assert_eq!(system.owner_of(ObjectId(8)), Some(ObjectId(7)));
    assert_eq!(system.owner_of(ObjectId(2)), None, "a device is not a stream");
}

#[test]
fn malformed_lines_are_skipped_and_an_empty_listing_has_no_devices() {
    let text = "not a device line\n00-00: Broken : Broken : playback many\n02-00: USB Mic : USB Mic : capture 1\n";
    let system = System::from_listing(text);
    assert_eq!(system.devices().len(), 1);
    assert_eq!(system.devices()[0].name, "USB Mic");
    assert_eq!(system.devices()[0].id, ObjectId(2), "a skipped line takes no id");
    assert!(System::from_listing("").devices().is_empty());
    assert_eq!(System::from_listing("").default_device(Scope::Output), None);
}

#[test]
fn a_device_with_no_substreams_is_kept_and_has_no_streams() {
    let system = System::from_listing("03-00: Modem : Modem : playback 0 : capture 0\n");
    assert_eq!(system.devices().len(), 1);
    assert!(system.devices()[0].streams.is_empty());
    assert_eq!(system.default_device(Scope::Output), None);
}

// ---- audio_session: sessions, interruptions and the output route ----------------------------------------------------

#[test]
fn a_session_that_mixes_with_others_is_never_interrupted_by_them() {
    let mut hub = SessionHub::new();
    hub.register("music");
    hub.register("game");
    hub.set_category("music", Category::Ambient, Mode::Default, Options::default()).unwrap();
    hub.set_category("game", Category::Playback, Mode::Default, Options::default()).unwrap();
    hub.set_active("music", true).unwrap();
    hub.set_active("game", true).unwrap();
    assert!(!hub.is_interrupted("music"));
}

#[test]
fn activating_a_playback_session_interrupts_another_playback_session() {
    let mut hub = SessionHub::new();
    hub.register("player");
    hub.register("alarm");
    hub.set_category("player", Category::Playback, Mode::Default, Options::default()).unwrap();
    hub.set_category("alarm", Category::Playback, Mode::Default, Options::default()).unwrap();
    hub.set_active("player", true).unwrap();
    hub.set_active("alarm", true).unwrap();
    assert!(hub.is_interrupted("player"));
    let events = hub.take_events("player");
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Interruption { kind: InterruptionType::Began, .. })));
}

#[test]
fn an_unknown_client_cannot_activate_a_session() {
    let mut hub = SessionHub::new();
    assert_eq!(hub.set_active("nobody", true), Err(SessionError::UnknownClient));
}

#[test]
fn a_route_change_is_posted_to_every_client_with_the_route_it_replaced() {
    let mut hub = SessionHub::new();
    hub.register("a");
    hub.register("b");
    hub.set_route(Route::Headphones);
    assert_eq!(hub.route(), Route::Headphones);
    for client in ["a", "b"] {
        let events = hub.take_events(client);
        assert!(events.iter().any(|e| matches!(e, SessionEvent::RouteChange { previous: Route::Speaker, .. })));
    }
}

#[test]
fn setting_the_route_that_is_already_in_use_posts_nothing() {
    let mut hub = SessionHub::new();
    hub.register("a");
    hub.set_route(Route::Speaker);
    assert!(hub.take_events("a").is_empty());
}

// ---- fswatch: the change feed of file items --------------------------------------------------------------------------

fn event(path: &str, flags: WatchEventFlags) -> WatchEvent {
    WatchEvent { path: PathBuf::from(path), flags }
}

#[test]
fn the_journal_numbers_events_from_one_and_reports_the_latest() {
    let mut journal = EventJournal::new(8);
    assert_eq!(journal.latest_id(), 0);
    assert_eq!(journal.record(event("/a", WatchEventFlags::CREATED)), 1);
    assert_eq!(journal.record(event("/a", WatchEventFlags::MODIFIED)), 2);
    assert_eq!(journal.latest_id(), 2);
}

#[test]
fn the_journal_keeps_only_the_newest_events() {
    let mut journal = EventJournal::new(2);
    for _ in 0..3 {
        journal.record(event("/a", WatchEventFlags::MODIFIED));
    }
    assert_eq!(journal.oldest_id(), Some(2));
    assert_eq!(journal.latest_id(), 3);
}

#[test]
fn a_replay_says_when_events_the_caller_has_not_seen_were_dropped() {
    let mut journal = EventJournal::new(2);
    journal.record(event("/a", WatchEventFlags::CREATED));
    journal.record(event("/b", WatchEventFlags::REMOVED));
    journal.record(event("/c", WatchEventFlags::RENAMED));
    let lost = journal.replay_since(0);
    assert!(!lost.complete);
    let caught_up = journal.replay_since(1);
    assert!(caught_up.complete);
    let ids: Vec<u64> = caught_up.events.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![2, 3]);
}

#[test]
fn a_replay_from_now_on_returns_nothing() {
    let mut journal = EventJournal::new(4);
    journal.record(event("/a", WatchEventFlags::CREATED));
    let replay = journal.replay_since(SINCE_NOW);
    assert!(replay.events.is_empty());
    assert!(replay.complete);
}

#[test]
fn a_journal_with_no_capacity_still_keeps_one_event() {
    let mut journal = EventJournal::new(0);
    journal.record(event("/a", WatchEventFlags::CREATED));
    journal.record(event("/b", WatchEventFlags::CREATED));
    assert_eq!(journal.oldest_id(), Some(2));
}

#[test]
fn event_flags_combine_and_test_by_bit() {
    let flags = WatchEventFlags::CREATED.union(WatchEventFlags::ITEM_IS_DIR);
    assert!(flags.contains(WatchEventFlags::CREATED));
    assert!(flags.contains(WatchEventFlags::ITEM_IS_DIR));
    assert!(!flags.contains(WatchEventFlags::MODIFIED));
}

// ---- hid_virtual: keyboard reports of virtual devices ----------------------------------------------------------------

#[test]
fn a_report_encodes_the_modifiers_then_a_reserved_byte_then_the_keys() {
    let report = KeyboardReport::new(modifier::LEFT_SHIFT | modifier::RIGHT_GUI, &[0x04, 0x05]).unwrap();
    assert_eq!(report.encode(), [0x82, 0, 0x04, 0x05, 0, 0, 0, 0]);
}

#[test]
fn a_report_holds_at_most_six_keys() {
    assert_eq!(KeyboardReport::new(0, &[4, 5, 6, 7, 8, 9]).unwrap().keys().len(), 6);
    assert_eq!(KeyboardReport::new(0, &[4, 5, 6, 7, 8, 9, 10]), Err(ReportError::TooManyKeys(7)));
}

#[test]
fn a_key_outside_the_keyboard_key_range_is_refused() {
    assert_eq!(KeyboardReport::new(0, &[0x00]), Err(ReportError::InvalidKey(0)));
    assert_eq!(KeyboardReport::new(0, &[0xe0]), Err(ReportError::InvalidKey(0xe0)), "modifier usages are not keys");
    assert!(KeyboardReport::new(0, &[0xa4]).is_ok());
    assert_eq!(KeyboardReport::new(0, &[0xa5]), Err(ReportError::InvalidKey(0xa5)));
}

#[test]
fn a_key_listed_twice_in_one_report_is_refused() {
    assert_eq!(KeyboardReport::new(0, &[0x04, 0x04]), Err(ReportError::DuplicateKey(0x04)));
}

#[test]
fn decoding_an_encoded_report_gives_back_the_same_report() {
    let report = KeyboardReport::new(modifier::LEFT_CONTROL, &[0x04, 0x28]).unwrap();
    let decoded = KeyboardReport::decode(&report.encode()).unwrap();
    assert_eq!(decoded, report);
}

#[test]
fn decoding_drops_the_empty_key_slots() {
    let bytes: [u8; REPORT_LEN] = [0, 0, 0x04, 0, 0x05, 0, 0, 0];
    assert_eq!(KeyboardReport::decode(&bytes).unwrap().keys(), &[0x04, 0x05]);
}

#[test]
fn a_device_name_must_not_be_blank_or_longer_than_64_bytes() {
    assert_eq!(validate_name("   "), Err(ReportError::InvalidName));
    assert_eq!(validate_name(&"k".repeat(65)), Err(ReportError::InvalidName));
    assert!(validate_name(&"k".repeat(64)).is_ok());
}

#[test]
fn a_virtual_keyboard_keeps_its_ids_and_builds_its_reports() {
    let keyboard = VirtualKeyboard::new("Test Keyboard", 0x1234, 0x5678).unwrap();
    assert_eq!(keyboard.name(), "Test Keyboard");
    assert_eq!((keyboard.vendor_id(), keyboard.product_id()), (0x1234, 0x5678));
    assert_eq!(keyboard.report(0, &[0x04]).unwrap(), [0, 0, 0x04, 0, 0, 0, 0, 0]);
    assert!(VirtualKeyboard::new("", 1, 2).is_err());
}

#[test]
fn the_boot_keyboard_descriptor_describes_the_eight_byte_report_that_the_keyboard_sends() {
    let descriptor = ReportDescriptor::parse(&BOOT_KEYBOARD_DESCRIPTOR).unwrap();
    assert_eq!(descriptor.payload_len(ReportKind::Input, 0), REPORT_LEN);
    let report = KeyboardReport::new(modifier::LEFT_CONTROL, &[0x04]).unwrap().encode();
    let fields = descriptor.fields(ReportKind::Input, 0);
    assert_eq!(fields[0].usage, 0xe0, "the first modifier is left control");
    assert_eq!(fields[0].value(&report), Some(1));
    assert_eq!(fields[7].usage, 0xe7, "the last modifier is right GUI");
    assert_eq!(fields[7].value(&report), Some(0));
}

// ---- input_grab: exclusive ownership of input devices ------------------------------------------------------------------

#[test]
fn an_app_is_refused_a_grab_of_any_input_device() {
    let keyboard = Roles { keyboard: true, ..Roles::default() };
    assert_eq!(may_grab(keyboard, Requester::App), Err(Refusal::AppRequester));
    assert_eq!(may_grab(Roles::default(), Requester::App), Err(Refusal::AppRequester));
}

#[test]
fn the_shell_may_grab_its_keyboards_pointers_and_touchscreens() {
    for roles in [
        Roles { keyboard: true, ..Roles::default() },
        Roles { pointer: true, ..Roles::default() },
        Roles { touchscreen: true, ..Roles::default() },
    ] {
        assert_eq!(may_grab(roles, Requester::Shell), Ok(()));
    }
}

#[test]
fn the_shell_never_grabs_a_power_button_or_a_lid_even_with_a_keyboard() {
    let power = Roles { power: true, keyboard: true, ..Roles::default() };
    assert_eq!(may_grab(power, Requester::Shell), Err(Refusal::SystemDevice));
    let lid = Roles { lid: true, ..Roles::default() };
    assert_eq!(may_grab(lid, Requester::Shell), Err(Refusal::SystemDevice));
}

// ---- video_decode: which decoder a codec runs on ----------------------------------------------------------------------

fn hardware_h264_only() -> HardwareSupport {
    HardwareSupport { h264: true, ..HardwareSupport::default() }
}

#[test]
fn a_codec_the_hardware_decodes_runs_in_hardware() {
    assert_eq!(choose(Codec::H264, hardware_h264_only(), Preference::Automatic), Ok(Backend::Hardware));
}

#[test]
fn a_codec_the_hardware_lacks_falls_back_to_software_when_automatic() {
    assert_eq!(choose(Codec::Hevc, hardware_h264_only(), Preference::Automatic), Ok(Backend::Software));
}

#[test]
fn a_required_hardware_path_refuses_a_codec_the_hardware_lacks() {
    assert_eq!(
        choose(Codec::Av1, hardware_h264_only(), Preference::RequireHardware),
        Err(DecodeError::NoHardwarePath(Codec::Av1))
    );
    assert_eq!(choose(Codec::H264, hardware_h264_only(), Preference::RequireHardware), Ok(Backend::Hardware));
}

#[test]
fn support_is_read_per_codec_and_automatic_is_the_default_preference() {
    let all = HardwareSupport { h264: true, hevc: true, vp9: true, av1: true };
    for codec in [Codec::H264, Codec::Hevc, Codec::Vp9, Codec::Av1] {
        assert!(all.supports(codec));
        assert!(!HardwareSupport::default().supports(codec));
    }
    assert_eq!(Preference::default(), Preference::Automatic);
}

// ---- media_extension: entitlements for format readers ------------------------------------------------------------------

#[test]
fn a_declaration_grants_only_the_entitlements_it_lists() {
    let declaration = Declaration::new("com.example.reader", &[Entitlement::FormatReader]);
    assert!(declaration.grants(Entitlement::FormatReader));
    assert!(!declaration.grants(Entitlement::VideoDecoder));
}

#[test]
fn a_reader_is_not_registered_without_the_format_reader_entitlement() {
    let mut registry = ReaderRegistry::new();
    let declaration = Declaration::new("com.example.decoder", &[Entitlement::VideoDecoder]);
    let reader = FormatReader::new("raw-reader", &["rawx"], &[]);
    assert_eq!(
        register_reader(&mut registry, &declaration, reader),
        Err(ExtensionError::MissingEntitlement(Entitlement::FormatReader))
    );
    assert!(registry.names().is_empty());
}

#[test]
fn a_reader_with_the_entitlement_is_registered_and_found_by_its_extension() {
    let mut registry = ReaderRegistry::new();
    let declaration = Declaration::new("com.example.reader", &[Entitlement::FormatReader]);
    let reader = FormatReader::new("raw-reader", &["rawx"], &[]);
    assert_eq!(register_reader(&mut registry, &declaration, reader), Ok(()));
    assert_eq!(registry.names(), vec!["raw-reader"]);
    assert!(registry.find_by_extension("rawx").is_some());
}

#[test]
fn the_registry_refuses_a_second_reader_with_the_same_name() {
    let mut registry = ReaderRegistry::new();
    let declaration = Declaration::new("com.example.reader", &[Entitlement::FormatReader]);
    register_reader(&mut registry, &declaration, FormatReader::new("raw-reader", &["rawx"], &[])).unwrap();
    let again = register_reader(&mut registry, &declaration, FormatReader::new("raw-reader", &["rawy"], &[]));
    assert_eq!(again, Err(ExtensionError::Registry(RegistryError::Duplicate("raw-reader".to_string()))));
}

#[test]
fn a_professional_reader_is_found_only_after_the_professional_workflow_is_enabled() {
    let mut registry = ReaderRegistry::new();
    let declaration = Declaration::new("com.example.reader", &[Entitlement::FormatReader]);
    let reader = FormatReader::new("pro-reader", &["mxf"], &[]).professional();
    register_reader(&mut registry, &declaration, reader).unwrap();
    assert!(registry.find_by_extension("mxf").is_none());
    registry.enable_professional_workflow(true);
    assert!(registry.find_by_extension("mxf").is_some());
}

// ---- sign_in: the sign-in helpers apps reach through the SDK ----------------------------------------------------------

#[test]
fn the_scheme_of_a_callback_url_names_the_app_to_return_to() {
    assert_eq!(scheme_of("myapp://callback?code=1"), Some("myapp"));
    assert_eq!(scheme_of("no colon here"), None);
}

#[test]
fn a_one_time_code_is_found_in_a_message_that_names_it() {
    let found = find_code("Your verification code is 482913. Do not share it.").expect("a code");
    assert_eq!(found.code, "482913");
}

// ---- audio file kinds: naming and sniffing the bytes of a file ---------------------------------------------------------

#[test]
fn a_file_extension_names_its_audio_kind_in_any_case() {
    assert_eq!(from_extension(Path::new("track.flac")), Some(Kind::Flac));
    assert_eq!(from_extension(Path::new("TRACK.OGG")), Some(Kind::Ogg));
    assert_eq!(from_extension(Path::new("notes.txt")), None);
}

#[test]
fn the_magic_bytes_name_flac_ogg_and_wav_files() {
    assert_eq!(sniff(None, b"fLaC\x00\x00\x00\x22"), Some(Kind::Flac));
    assert_eq!(sniff(None, b"OggS\x00\x02"), Some(Kind::Ogg));
    assert_eq!(sniff(None, b"RIFF\x24\x00\x00\x00WAVEfmt "), Some(Kind::Wav));
}

#[test]
fn an_mp4_container_is_named_m4a_by_its_file_type_box() {
    assert_eq!(sniff(None, b"\x00\x00\x00\x18ftypM4A "), Some(Kind::M4a));
}

#[test]
fn a_wav_file_reports_its_duration_from_its_header() {
    let format = pcm16(8000, 1);
    let samples = vec![0i16; 8000];
    let bytes = wav_bytes(&format, &samples);
    assert_eq!(wav_duration_ms(&bytes, bytes.len() as u64), Some(1000));
}
