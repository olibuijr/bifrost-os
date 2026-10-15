//! Parity sdk-complete-21: headless tests of the Bluetooth error values, the Bluetooth device list model and the
//! image save options. Each model is pure data, so every test runs without an adapter, a display or a file system.
use bifrost_ui::bluetooth_devices::{device_list, DeviceRow, LinkState};
use bifrost_ui::bluetooth_errors::{AttError, MgmtStatus};
use bifrost_ui::save_options::{ImageFormat, SaveOptions};

fn device(address: &str, name: &str, paired: bool) -> DeviceRow {
    DeviceRow { address: address.into(), name: name.into(), paired, connected: false, connecting: false }
}

#[test]
fn a_refused_pairing_reports_text_and_retry_advice_from_one_code() {
    let status = MgmtStatus::from_code(0x0a);
    assert_eq!(status, MgmtStatus::Busy);
    assert_eq!(status.text(), "Device Busy");
    assert!(status.is_retryable(), "a busy device may accept the same request later");
    assert!(!MgmtStatus::from_code(0x0b).is_retryable(), "a rejected request is not retried");
}

#[test]
fn an_att_error_from_a_device_names_the_refusal_and_keeps_its_code() {
    let error = AttError::from_code(0x03).expect("write not permitted is specified");
    assert_eq!(error, AttError::WriteNotPermitted);
    assert_eq!(error.code(), 0x03);
    assert_eq!(error.text(), "The device does not allow writing this value");
}

#[test]
fn a_vendor_att_code_is_an_application_error_and_zero_is_no_error() {
    assert_eq!(AttError::from_code(0x00), None);
    assert_eq!(AttError::from_code(0x8c), Some(AttError::Application(0x8c)));
    assert_eq!(AttError::from_code(0xf0).map(AttError::text), Some("The device reported a service error"));
}

#[test]
fn the_device_list_shows_paired_devices_first_with_their_link_state() {
    let mut speaker = device("02", "Speaker", true);
    speaker.connected = true;
    let mut keyboard = device("01", "Keyboard", true);
    keyboard.connecting = true;
    let list = device_list(&[device("03", "Phone", false), keyboard, speaker]);
    let titles: Vec<&str> = list.paired.iter().map(|d| d.title()).collect();
    assert_eq!(titles, vec!["Keyboard", "Speaker"]);
    assert_eq!(list.paired[0].link_state(), LinkState::Connecting);
    assert_eq!(list.paired[1].link_state().label(), "Connected");
    assert_eq!(list.found[0].link_state().label(), "Not Connected");
}

#[test]
fn a_nameless_device_in_the_list_is_shown_by_its_address() {
    let list = device_list(&[device("AA:BB:CC:DD:EE:09", "", false)]);
    assert_eq!(list.found[0].title(), "AA:BB:CC:DD:EE:09");
}

#[test]
fn the_default_save_is_a_png_named_after_the_picture() {
    let options = SaveOptions::default();
    assert_eq!(options.format, ImageFormat::Png);
    assert_eq!(options.file_name("Harbour"), "Harbour edited.png");
}

#[test]
fn a_jpeg_save_carries_its_quality_to_the_encoder() {
    let options = SaveOptions { format: ImageFormat::Jpeg, ..SaveOptions::default() }.with_quality(0.75);
    assert_eq!(options.encoder_quality(), Some(75));
    assert_eq!(options.file_name("Harbour"), "Harbour edited.jpg");
}

#[test]
fn a_save_into_a_folder_with_the_name_taken_numbers_the_new_file() {
    let taken = ["Harbour edited.png"];
    let options = SaveOptions::default();
    assert_eq!(options.unique_file_name("Harbour", &taken), "Harbour edited 2.png");
    let replacing = SaveOptions { replace_existing: true, ..SaveOptions::default() };
    assert_eq!(replacing.unique_file_name("Harbour", &taken), "Harbour edited.png");
}
