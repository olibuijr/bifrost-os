//! Parity sdk-os-01: headless tests for the device, display and media models an app reads. No test opens a device,
//! a display, a sound card or a radio.

use bifrost_ui::default_device::{Defaults, Device as AudioDevice, DeviceError, Role};
use bifrost_ui::device_match::{Capabilities, Criteria, ABS_MT_POSITION_X, ABS_MT_POSITION_Y, BTN_LEFT, EV_REL, KEY_A, KEY_ENTER};
use bifrost_ui::display_info::{self, ChangeFlags, DisplayInfo, DisplayMode};
use bifrost_ui::media_content::{MediaContent, MediaDuration, MediaType};
use bifrost_ui::media_session::{CommandState, MediaSession, MediaSessionError, PlaybackState, RemoteCommand};
use bifrost_ui::video_device::{
    choose_format, device_index, plan_stream, PixelFormat, StreamError, StreamFormat, VideoDevice, BUFFER_COUNT, CAP_META_CAPTURE,
    CAP_STREAMING, CAP_VIDEO_CAPTURE, fourcc,
};
use bifrost_ui::Rect;

// ---- audio: default devices ----

fn speakers() -> AudioDevice {
    AudioDevice { can_input: false, can_sound_effects: false, ..AudioDevice::new("speakers", "Speakers") }
}

fn microphone() -> AudioDevice {
    AudioDevice { can_output: false, can_sound_effects: false, ..AudioDevice::new("mic", "Microphone") }
}

#[test]
fn the_first_device_that_takes_a_role_is_the_automatic_default_for_it() {
    let mut defaults = Defaults::new();
    defaults.add(speakers()).expect("added");
    defaults.add(microphone()).expect("added");
    assert_eq!(defaults.default_for(Role::Output).map(|d| d.id.as_str()), Some("speakers"));
    assert_eq!(defaults.default_for(Role::Input).map(|d| d.id.as_str()), Some("mic"));
    assert!(defaults.default_for(Role::SoundEffects).is_some());
}

#[test]
fn a_chosen_default_returns_after_its_device_is_unplugged_and_plugged_back_in() {
    let mut defaults = Defaults::new();
    defaults.add(speakers()).expect("added");
    defaults.add(AudioDevice { can_input: false, can_sound_effects: false, ..AudioDevice::new("headphones", "Headphones") }).expect("added");
    defaults.set_default(Role::Output, "headphones").expect("headphones can output");
    assert_eq!(defaults.default_for(Role::Output).map(|d| d.id.as_str()), Some("headphones"));

    defaults.remove("headphones").expect("listed");
    assert_eq!(defaults.default_for(Role::Output).map(|d| d.id.as_str()), Some("speakers"), "the fallback is the first eligible device");

    defaults.add(AudioDevice { can_input: false, can_sound_effects: false, ..AudioDevice::new("headphones", "Headphones") }).expect("added again");
    assert_eq!(defaults.default_for(Role::Output).map(|d| d.id.as_str()), Some("headphones"));
}

#[test]
fn a_device_cannot_be_chosen_for_a_role_it_does_not_take() {
    let mut defaults = Defaults::new();
    defaults.add(speakers()).expect("added");
    assert_eq!(defaults.set_default(Role::Input, "speakers"), Err(DeviceError::NotEligible));
    assert_eq!(defaults.set_default(Role::Output, "missing"), Err(DeviceError::NotFound));
}

#[test]
fn a_hidden_device_never_becomes_a_default() {
    let mut defaults = Defaults::new();
    defaults.add(AudioDevice { hidden: true, ..AudioDevice::new("loopback", "Loopback") }).expect("added");
    assert!(defaults.default_for(Role::Output).is_none());
    assert_eq!(defaults.set_default(Role::Output, "loopback"), Err(DeviceError::NotEligible));
}

#[test]
fn an_empty_or_repeated_device_identifier_is_refused() {
    let mut defaults = Defaults::new();
    assert_eq!(defaults.add(AudioDevice::new("", "Nothing")), Err(DeviceError::Empty));
    defaults.add(speakers()).expect("added");
    assert_eq!(defaults.add(speakers()), Err(DeviceError::Duplicate));
    assert_eq!(defaults.devices().len(), 1);
}

#[test]
fn clearing_a_chosen_default_returns_the_role_to_automatic_choice() {
    let mut defaults = Defaults::new();
    defaults.add(speakers()).expect("added");
    defaults.add(AudioDevice { can_input: false, can_sound_effects: false, ..AudioDevice::new("headphones", "Headphones") }).expect("added");
    defaults.set_default(Role::Output, "headphones").expect("set");
    defaults.clear_default(Role::Output);
    assert_eq!(defaults.default_for(Role::Output).map(|d| d.id.as_str()), Some("speakers"));
}

// ---- video: capture nodes and stream formats ----

const CAPTURE: u32 = CAP_VIDEO_CAPTURE | CAP_STREAMING;

fn format(format: PixelFormat, width: u32, height: u32) -> StreamFormat {
    StreamFormat { format, width, height }
}

fn node(path: &str, capabilities: u32, formats: Vec<StreamFormat>) -> VideoDevice {
    VideoDevice { path: path.to_owned(), card: "camera".to_owned(), capabilities, device_caps: 0, formats }
}

#[test]
fn a_metadata_node_is_never_chosen_for_a_stream() {
    let devices = vec![
        node("/dev/video0", CAPTURE | CAP_META_CAPTURE, vec![format(PixelFormat::Yuyv, 640, 480)]),
        node("/dev/video1", CAPTURE, vec![format(PixelFormat::Yuyv, 640, 480)]),
    ];
    assert!(!devices[0].is_usable());
    let plan = plan_stream(&devices, 640, 480).expect("a usable node");
    assert_eq!(plan.path, "/dev/video1");
}

#[test]
fn the_lowest_numbered_node_with_a_valid_format_is_planned() {
    let devices = vec![
        node("/dev/video7", CAPTURE, vec![format(PixelFormat::Yuyv, 320, 240)]),
        node("/dev/video2", CAPTURE, vec![format(PixelFormat::Yuyv, 320, 240)]),
    ];
    assert_eq!(plan_stream(&devices, 320, 240).expect("planned").path, "/dev/video2");
}

#[test]
fn a_raw_format_is_planned_with_one_full_frame_per_buffer_and_four_buffers() {
    let devices = vec![node("/dev/video0", CAPTURE, vec![format(PixelFormat::Yuyv, 640, 480)])];
    let plan = plan_stream(&devices, 640, 480).expect("planned");
    assert_eq!(plan.buffer_bytes, Some(640 * 2 * 480), "YUYV is two bytes a pixel");
    assert_eq!(plan.buffers, BUFFER_COUNT);
    assert_eq!(BUFFER_COUNT, 4);
}

#[test]
fn a_compressed_format_has_no_fixed_buffer_size() {
    let devices = vec![node("/dev/video0", CAPTURE, vec![format(PixelFormat::Mjpeg, 1280, 720)])];
    let plan = plan_stream(&devices, 1280, 720).expect("planned");
    assert_eq!(plan.buffer_bytes, None, "the driver sets each compressed frame's size");
}

#[test]
fn no_stream_is_planned_without_a_usable_node() {
    let devices = vec![node("/dev/video0", CAP_META_CAPTURE | CAP_STREAMING, vec![format(PixelFormat::Yuyv, 640, 480)])];
    assert_eq!(plan_stream(&devices, 640, 480), Err(StreamError::NoUsableDevice));
    assert_eq!(plan_stream(&[], 640, 480), Err(StreamError::NoUsableDevice));
}

#[test]
fn the_raw_format_wins_over_a_compressed_one_of_the_same_size() {
    let offered = [format(PixelFormat::Mjpeg, 640, 480), format(PixelFormat::Yuyv, 640, 480)];
    assert_eq!(choose_format(&offered, 640, 480).map(|f| f.format), Some(PixelFormat::Yuyv));
}

#[test]
fn the_size_nearest_the_wanted_size_wins_within_a_format() {
    let offered = [format(PixelFormat::Yuyv, 320, 240), format(PixelFormat::Yuyv, 640, 480)];
    let chosen = choose_format(&offered, 600, 450).expect("a format");
    assert_eq!((chosen.width, chosen.height), (640, 480));
}

#[test]
fn an_odd_sized_planar_format_is_not_valid() {
    assert!(!format(PixelFormat::Nv12, 640, 481).is_valid(), "4:2:0 needs an even height");
    assert!(format(PixelFormat::Nv12, 640, 480).is_valid());
    assert_eq!(PixelFormat::from_fourcc(fourcc(b"NV12")), PixelFormat::Nv12);
    assert_eq!(PixelFormat::from_fourcc(fourcc(b"GREY")), PixelFormat::Other(fourcc(b"GREY")));
}

#[test]
fn a_device_index_is_read_only_from_a_video_node_path() {
    assert_eq!(device_index("/dev/video12"), Some(12));
    assert_eq!(device_index("/dev/videox"), None);
    assert_eq!(device_index("/dev/video"), None);
    assert_eq!(device_index("/dev/input/event3"), None);
}

// ---- displays ----

fn display(id: u32, x: f32, w: f32, main: bool) -> DisplayInfo {
    DisplayInfo {
        id,
        name: format!("Display {id}"),
        is_main: main,
        is_builtin: false,
        is_active: true,
        is_online: true,
        is_asleep: false,
        bounds: Rect { x, y: 0.0, w, h: 900.0 },
        pixel_width: (w * 2.0) as u32,
        pixel_height: 1800,
        mode: DisplayMode { width: w as u32, height: 900, pixel_width: (w * 2.0) as u32, pixel_height: 1800, refresh_hz: 60.0, usable: true },
        modes: Vec::new(),
        rotation: display_info::Rotation::Deg0,
        mirror_of: None,
        vendor: 0,
        model: 0,
        serial: 0,
        size_mm: (0.0, 0.0),
    }
}

#[test]
fn the_main_display_is_the_one_flagged_main_and_online() {
    let list = vec![display(1, 0.0, 1920.0, false), display(2, 1920.0, 1920.0, true)];
    assert_eq!(display_info::main_display(&list).map(|d| d.id), Some(2));
}

#[test]
fn a_point_on_the_desktop_resolves_to_the_display_that_holds_it() {
    let list = vec![display(1, 0.0, 1920.0, true), display(2, 1920.0, 1280.0, false)];
    assert_eq!(display_info::at_point(&list, 2000.0, 100.0).map(|d| d.id), Some(2));
    assert_eq!(display_info::at_point(&list, 5000.0, 100.0).map(|d| d.id), None);
}

#[test]
fn a_display_that_is_not_part_of_the_desktop_is_left_out_of_the_active_list() {
    let mut hidden = display(2, 1920.0, 1280.0, false);
    hidden.is_active = false;
    let list = vec![display(1, 0.0, 1920.0, true), hidden];
    assert_eq!(display_info::active(&list).len(), 1);
    assert_eq!(display_info::online(&list).len(), 2);
}

#[test]
fn a_moved_display_reports_the_moved_flag_and_nothing_else() {
    let before = vec![display(1, 0.0, 1920.0, true), display(2, 1920.0, 1280.0, false)];
    let after = vec![display(1, 0.0, 1920.0, true), display(2, 2560.0, 1280.0, false)];
    let changes = display_info::diff(&before, &after);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, 2);
    assert!(changes[0].1.contains(ChangeFlags::MOVED));
    assert!(!changes[0].1.contains(ChangeFlags::SET_MODE));
}

#[test]
fn a_display_that_disappears_reports_removed_and_a_new_one_reports_added() {
    let before = vec![display(1, 0.0, 1920.0, true), display(2, 1920.0, 1280.0, false)];
    let after = vec![display(1, 0.0, 1920.0, true), display(3, 1920.0, 1280.0, false)];
    let changes = display_info::diff(&before, &after);
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0], (2, ChangeFlags(ChangeFlags::REMOVED)));
    assert_eq!(changes[1], (3, ChangeFlags(ChangeFlags::ADDED)));
}

#[test]
fn an_unchanged_list_reports_no_change() {
    let list = vec![display(1, 0.0, 1920.0, true)];
    assert!(display_info::diff(&list, &list).is_empty());
}

// ---- input: capability criteria ----

fn keyboard_device() -> Capabilities {
    Capabilities::new().with_key(KEY_ENTER).with_key(KEY_A)
}

fn pointer_device() -> Capabilities {
    Capabilities::new().with_type(EV_REL).with_key(BTN_LEFT)
}

fn touch_device() -> Capabilities {
    Capabilities::new().with_axis(ABS_MT_POSITION_X).with_axis(ABS_MT_POSITION_Y)
}

#[test]
fn a_pointer_criteria_selects_a_relative_device_with_a_left_button() {
    let devices = vec![keyboard_device(), pointer_device(), touch_device()];
    assert_eq!(Criteria::pointer().select(&devices), vec![1]);
}

#[test]
fn a_keyboard_criteria_needs_the_enter_and_a_keys() {
    let devices = vec![keyboard_device(), pointer_device(), touch_device()];
    assert_eq!(Criteria::keyboard().select(&devices), vec![0]);
}

#[test]
fn a_touch_criteria_needs_both_multitouch_position_axes() {
    let one_axis = Capabilities::new().with_axis(ABS_MT_POSITION_X);
    let devices = vec![one_axis, touch_device()];
    assert_eq!(Criteria::touch().select(&devices), vec![1]);
}

// ---- media: a Now Playing session ----

fn song(seconds: f64) -> MediaContent {
    MediaContent::new("track-1", "Song", MediaType::Audio, MediaDuration::Finite(seconds)).expect("valid content")
}

#[test]
fn a_session_with_an_empty_identifier_is_refused() {
    assert_eq!(MediaSession::new("   ").unwrap_err(), MediaSessionError::EmptyId);
}

#[test]
fn an_undeclared_command_is_unsupported_and_refused() {
    let session = MediaSession::new("main").expect("a session");
    assert_eq!(session.command_state(RemoteCommand::Play), CommandState::Unsupported);
    assert_eq!(session.dispatch(RemoteCommand::Play), Err(MediaSessionError::CommandUnsupported));
}

#[test]
fn a_declared_command_runs_only_when_enabled() {
    let mut session = MediaSession::new("main").expect("a session");
    session.set_command(RemoteCommand::Next, CommandState::Disabled);
    assert_eq!(session.dispatch(RemoteCommand::Next), Err(MediaSessionError::CommandDisabled));
    session.set_command(RemoteCommand::Next, CommandState::Enabled);
    assert_eq!(session.dispatch(RemoteCommand::Next), Ok(()));
}

#[test]
fn the_published_position_is_capped_at_a_finite_duration() {
    let mut session = MediaSession::new("main").expect("a session");
    session.set_content(Some(song(120.0)));
    session.set_playback(PlaybackState::Playing, 500_000);
    assert_eq!(session.snapshot().position_ms, 120_000);
    assert_eq!(session.snapshot().state, PlaybackState::Playing);
}

#[test]
fn a_live_position_is_kept_as_given() {
    let mut session = MediaSession::new("main").expect("a session");
    session.set_content(Some(MediaContent::new("radio", "Stream", MediaType::Audio, MediaDuration::Live).expect("valid")));
    session.set_playback(PlaybackState::Paused, 500_000);
    assert_eq!(session.snapshot().position_ms, 500_000);
}
