//! Parity tests for the capture, photo and control rows (sdk-os-09).
//!
//! These are headless tests of pure models: no camera is opened, no frame is captured, no display is drawn and no clock
//! is read. Each test drives the public API of one module and asserts concrete values.

use bifrost_ui::capture_session::{CaptureError, CaptureSession, SessionState};
use bifrost_ui::capture_shot::ImageFormat;
use bifrost_ui::control_widget::{ControlError, ControlKind, ControlRegistry, StaticControlConfiguration, MAX_IDENTIFIER_LEN};
use bifrost_ui::photo_capture::{Crop, PhotoError, PhotoSettings};
use bifrost_ui::video_device::{PixelFormat, StreamFormat, VideoDevice, BUFFER_COUNT, CAP_STREAMING, CAP_VIDEO_CAPTURE};

fn yuyv(width: u32, height: u32) -> StreamFormat {
    StreamFormat { format: PixelFormat::Yuyv, width, height }
}

fn camera(path: &str, formats: Vec<StreamFormat>) -> VideoDevice {
    VideoDevice {
        path: path.to_string(),
        card: "test camera".to_string(),
        capabilities: CAP_VIDEO_CAPTURE | CAP_STREAMING,
        device_caps: 0,
        formats,
    }
}

fn metadata_node(path: &str) -> VideoDevice {
    VideoDevice {
        path: path.to_string(),
        card: "metadata".to_string(),
        capabilities: CAP_STREAMING,
        device_caps: 0,
        formats: vec![yuyv(640, 480)],
    }
}

fn toggle(identifier: &str, initial_value: bool) -> StaticControlConfiguration {
    StaticControlConfiguration {
        identifier: identifier.to_string(),
        display_name: "Toggle".to_string(),
        kind: ControlKind::Toggle,
        initial_value,
    }
}

fn button(identifier: &str) -> StaticControlConfiguration {
    StaticControlConfiguration {
        identifier: identifier.to_string(),
        display_name: "Button".to_string(),
        kind: ControlKind::Button,
        initial_value: false,
    }
}

// Capture sessions.

#[test]
fn a_new_capture_session_is_stopped_with_no_input() {
    let session = CaptureSession::new(640, 480).unwrap();
    assert_eq!(session.state(), SessionState::Stopped);
    assert!(session.input().is_none());
    assert_eq!(session.size(), (640, 480));
}

#[test]
fn a_capture_session_refuses_a_zero_size() {
    assert_eq!(CaptureSession::new(0, 480), Err(CaptureError::InvalidSize));
    let mut session = CaptureSession::new(640, 480).unwrap();
    assert_eq!(session.set_size(640, 0), Err(CaptureError::InvalidSize));
    assert_eq!(session.size(), (640, 480), "a refused size leaves the session as it was");
}

#[test]
fn a_capture_session_cannot_start_without_an_input() {
    let mut session = CaptureSession::new(640, 480).unwrap();
    assert_eq!(session.start(), Err(CaptureError::NoInput));
    assert_eq!(session.state(), SessionState::Stopped);
}

#[test]
fn a_device_that_cannot_stream_is_refused_as_an_input() {
    let mut session = CaptureSession::new(640, 480).unwrap();
    let error = session.set_input(metadata_node("/dev/video1")).unwrap_err();
    assert_eq!(error, CaptureError::UnusableInput { path: "/dev/video1".to_string() });
    assert!(session.input().is_none());
}

#[test]
fn starting_a_session_plans_its_input_at_the_session_size() {
    let mut session = CaptureSession::new(640, 480).unwrap();
    session.set_input(camera("/dev/video0", vec![yuyv(640, 480)])).unwrap();
    let plan = session.start().unwrap();
    assert_eq!(plan.path, "/dev/video0");
    assert_eq!(plan.format, yuyv(640, 480));
    assert_eq!(plan.buffers, BUFFER_COUNT);
    assert_eq!(plan.buffer_bytes, Some(640 * 480 * 2), "YUYV carries two bytes per pixel");
    assert_eq!(session.state(), SessionState::Running);
}

#[test]
fn a_running_session_picks_the_format_nearest_its_size() {
    let mut session = CaptureSession::new(1280, 720).unwrap();
    session.set_input(camera("/dev/video0", vec![yuyv(640, 480), yuyv(1280, 720)])).unwrap();
    let plan = session.start().unwrap();
    assert_eq!((plan.format.width, plan.format.height), (1280, 720));
}

#[test]
fn a_running_session_locks_its_input_and_its_size() {
    let mut session = CaptureSession::new(640, 480).unwrap();
    session.set_input(camera("/dev/video0", vec![yuyv(640, 480)])).unwrap();
    session.start().unwrap();
    assert_eq!(session.set_input(camera("/dev/video2", vec![yuyv(640, 480)])), Err(CaptureError::Running));
    assert_eq!(session.remove_input(), Err(CaptureError::Running));
    assert_eq!(session.set_size(320, 240), Err(CaptureError::Running));
    assert_eq!(session.input().unwrap().path, "/dev/video0", "the locked input is still the first camera");
}

#[test]
fn a_running_session_cannot_start_again() {
    let mut session = CaptureSession::new(640, 480).unwrap();
    session.set_input(camera("/dev/video0", vec![yuyv(640, 480)])).unwrap();
    session.start().unwrap();
    assert_eq!(session.start(), Err(CaptureError::AlreadyRunning));
}

#[test]
fn stopping_a_session_unlocks_its_configuration() {
    let mut session = CaptureSession::new(640, 480).unwrap();
    session.set_input(camera("/dev/video0", vec![yuyv(640, 480)])).unwrap();
    session.start().unwrap();
    session.stop();
    assert_eq!(session.state(), SessionState::Stopped);
    session.set_size(320, 240).unwrap();
    assert_eq!(session.size(), (320, 240));
    session.remove_input().unwrap();
    assert!(session.input().is_none());
}

// Photo settings.

#[test]
fn default_photo_settings_are_a_jpeg_with_no_timer_and_no_crop() {
    let settings = PhotoSettings::default();
    assert_eq!(settings.format, ImageFormat::Jpeg);
    assert_eq!(settings.self_timer_seconds, 0);
    assert!(!settings.square);
    assert_eq!(settings.validate(), Ok(()));
    assert_eq!(settings.plan(640, 480), Ok(Crop { x: 0, y: 0, width: 640, height: 480 }));
}

#[test]
fn a_jpeg_quality_outside_zero_to_one_is_refused() {
    let mut settings = PhotoSettings::default();
    settings.quality = 1.5;
    assert_eq!(settings.validate(), Err(PhotoError::InvalidQuality));
    settings.quality = f32::NAN;
    assert_eq!(settings.validate(), Err(PhotoError::InvalidQuality));
}

#[test]
fn a_png_photo_ignores_the_quality_setting() {
    let settings = PhotoSettings { format: ImageFormat::Png, quality: 1.5, ..PhotoSettings::default() };
    assert_eq!(settings.validate(), Ok(()));
}

#[test]
fn only_the_supported_self_timer_delays_are_accepted() {
    let mut settings = PhotoSettings::default();
    settings.self_timer_seconds = 5;
    assert_eq!(settings.validate(), Err(PhotoError::UnsupportedTimer { seconds: 5 }));
    settings.self_timer_seconds = 10;
    assert_eq!(settings.validate(), Ok(()));
    settings.self_timer_seconds = 3;
    assert_eq!(settings.validate(), Ok(()));
}

#[test]
fn a_square_photo_of_a_landscape_frame_keeps_the_centred_square() {
    let settings = PhotoSettings { square: true, ..PhotoSettings::default() };
    assert_eq!(settings.plan(1920, 1080), Ok(Crop { x: 420, y: 0, width: 1080, height: 1080 }));
}

#[test]
fn a_square_photo_of_a_portrait_frame_keeps_the_centred_square() {
    let settings = PhotoSettings { square: true, ..PhotoSettings::default() };
    assert_eq!(settings.plan(1000, 1500), Ok(Crop { x: 0, y: 250, width: 1000, height: 1000 }));
}

#[test]
fn a_photo_of_an_empty_frame_is_refused() {
    let settings = PhotoSettings::default();
    assert_eq!(settings.plan(0, 480), Err(PhotoError::EmptyFrame));
}

// Control widgets.

#[test]
fn controls_are_listed_in_registration_order() {
    let mut registry = ControlRegistry::new();
    registry.register(toggle("wifi.toggle", true)).unwrap();
    registry.register(button("camera.open")).unwrap();
    let identifiers: Vec<&str> = registry.configurations().iter().map(|c| c.identifier.as_str()).collect();
    assert_eq!(identifiers, vec!["wifi.toggle", "camera.open"]);
}

#[test]
fn a_duplicate_control_identifier_is_refused() {
    let mut registry = ControlRegistry::new();
    registry.register(button("camera.open")).unwrap();
    assert_eq!(
        registry.register(toggle("camera.open", false)),
        Err(ControlError::Duplicate { identifier: "camera.open".to_string() })
    );
    assert_eq!(registry.configurations().len(), 1);
}

#[test]
fn a_control_identifier_must_be_a_short_plain_name() {
    let mut registry = ControlRegistry::new();
    assert_eq!(registry.register(button("")), Err(ControlError::EmptyIdentifier));
    assert_eq!(
        registry.register(button("has space")),
        Err(ControlError::InvalidIdentifier { identifier: "has space".to_string() })
    );
    let long = "a".repeat(MAX_IDENTIFIER_LEN + 1);
    assert_eq!(registry.register(button(&long)), Err(ControlError::IdentifierTooLong { length: MAX_IDENTIFIER_LEN + 1 }));
    assert!(registry.register(button(&"a".repeat(MAX_IDENTIFIER_LEN))).is_ok());
}

#[test]
fn a_control_needs_a_display_name() {
    let mut registry = ControlRegistry::new();
    let mut configuration = button("camera.open");
    configuration.display_name = "   ".to_string();
    assert_eq!(registry.register(configuration), Err(ControlError::EmptyDisplayName));
}

#[test]
fn a_toggle_starts_at_its_initial_value_and_flips() {
    let mut registry = ControlRegistry::new();
    registry.register(toggle("wifi.toggle", true)).unwrap();
    assert_eq!(registry.value("wifi.toggle"), Ok(true));
    assert_eq!(registry.toggle("wifi.toggle"), Ok(false));
    assert_eq!(registry.value("wifi.toggle"), Ok(false));
    registry.set_value("wifi.toggle", true).unwrap();
    assert_eq!(registry.value("wifi.toggle"), Ok(true));
}

#[test]
fn a_button_has_no_value_and_a_toggle_is_not_pressed() {
    let mut registry = ControlRegistry::new();
    registry.register(button("camera.open")).unwrap();
    registry.register(toggle("wifi.toggle", false)).unwrap();
    assert_eq!(
        registry.value("camera.open"),
        Err(ControlError::WrongKind { identifier: "camera.open".to_string(), kind: ControlKind::Button })
    );
    assert_eq!(
        registry.press("wifi.toggle"),
        Err(ControlError::WrongKind { identifier: "wifi.toggle".to_string(), kind: ControlKind::Toggle })
    );
}

#[test]
fn a_press_of_a_button_is_accepted() {
    let mut registry = ControlRegistry::new();
    registry.register(button("camera.open")).unwrap();
    assert_eq!(registry.press("camera.open"), Ok(()));
}

#[test]
fn an_unknown_control_is_reported_by_its_identifier() {
    let mut registry = ControlRegistry::new();
    assert_eq!(registry.value("missing"), Err(ControlError::Unknown { identifier: "missing".to_string() }));
    assert_eq!(registry.press("missing"), Err(ControlError::Unknown { identifier: "missing".to_string() }));
}

#[test]
fn an_unregistered_control_is_gone() {
    let mut registry = ControlRegistry::new();
    registry.register(toggle("wifi.toggle", false)).unwrap();
    assert!(registry.unregister("wifi.toggle"));
    assert!(!registry.unregister("wifi.toggle"));
    assert_eq!(registry.value("wifi.toggle"), Err(ControlError::Unknown { identifier: "wifi.toggle".to_string() }));
}
