//! Parity sdk-complete-47: headless tests for the Bluetooth service browser's allow list, choice and panel text. The
//! service list is passed in as data; no radio is queried and no device is contacted.

use bifrost_core::bt_address::BdAddr;
use bifrost_core::bt_service_browser::{BrowseError, Service, ServiceBrowser};
use bifrost_core::sdp_element::Uuid;

fn service(id: u16, name: &str) -> Service {
    Service { class_uuid: Uuid::U16(id), name: name.to_string() }
}

fn device() -> BdAddr {
    BdAddr([0x00, 0x11, 0x22, 0x33, 0x44, 0x55])
}

/// Four services on one device: a file transfer, an audio sink, a serial port and a hands-free profile.
fn offered() -> Vec<Service> {
    vec![service(0x1106, "Transfer"), service(0x110b, "Sink"), service(0x1101, "Serial"), service(0x111e, "Hands-free")]
}

#[test]
fn a_new_browser_shows_every_service_and_accepts_any_uuid() {
    let browser = ServiceBrowser::new();
    assert!(browser.allowed_uuids().is_empty());
    assert!(browser.accepts(Uuid::U16(0x1234)));
    assert_eq!(browser.visible(&offered()), offered());
}

#[test]
fn an_allow_list_hides_the_services_it_does_not_name_and_keeps_their_order() {
    let mut browser = ServiceBrowser::new();
    browser.add_allowed_uuids(&[Uuid::U16(0x111e), Uuid::U16(0x110b)]);
    let shown = browser.visible(&offered());
    let names: Vec<&str> = shown.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Sink", "Hands-free"], "the order of the device's list is kept");
    assert!(!browser.accepts(Uuid::U16(0x1101)));
}

#[test]
fn a_uuid_is_listed_once_however_often_it_is_added() {
    let mut browser = ServiceBrowser::new();
    browser.add_allowed_uuid(Uuid::U16(0x110b));
    browser.add_allowed_uuid(Uuid::U16(0x110b));
    browser.add_allowed_uuids(&[Uuid::U16(0x110b), Uuid::U16(0x1101)]);
    assert_eq!(browser.allowed_uuids(), &[Uuid::U16(0x110b), Uuid::U16(0x1101)]);
}

#[test]
fn clearing_the_allow_list_accepts_every_service_again() {
    let mut browser = ServiceBrowser::new();
    browser.add_allowed_uuid(Uuid::U16(0x110b));
    browser.clear_allowed_uuids();
    assert_eq!(browser.visible(&offered()).len(), 4);
}

#[test]
fn a_choice_indexes_the_filtered_list_not_the_device_list() {
    let mut browser = ServiceBrowser::new();
    browser.add_allowed_uuids(&[Uuid::U16(0x1101), Uuid::U16(0x111e)]);
    browser.choose(device(), &offered(), 1).unwrap();
    let selection = browser.results().expect("a choice is recorded");
    assert_eq!(selection.device, device());
    assert_eq!(selection.service, service(0x111e, "Hands-free"));
}

#[test]
fn an_index_past_the_filtered_list_is_refused_and_keeps_the_earlier_choice() {
    let mut browser = ServiceBrowser::new();
    browser.choose(device(), &offered(), 0).unwrap();
    assert_eq!(browser.choose(device(), &offered(), 4), Err(BrowseError::NoSuchService));
    assert_eq!(browser.results().map(|s| s.service.name.as_str()), Some("Transfer"));
}

#[test]
fn a_later_choice_replaces_an_earlier_one() {
    let mut browser = ServiceBrowser::new();
    browser.choose(device(), &offered(), 0).unwrap();
    browser.choose(device(), &offered(), 2).unwrap();
    assert_eq!(browser.results().map(|s| s.service.name.as_str()), Some("Serial"));
}

#[test]
fn cancelling_forgets_the_choice() {
    let mut browser = ServiceBrowser::new();
    browser.choose(device(), &offered(), 1).unwrap();
    browser.cancel();
    assert!(browser.results().is_none());
}

#[test]
fn the_panel_text_defaults_to_select_and_the_setters_replace_it() {
    let mut browser = ServiceBrowser::new();
    assert_eq!(browser.prompt(), "Select");
    assert_eq!(browser.title(), "");
    browser.set_title("Share over Bluetooth");
    browser.set_prompt("Choose a service");
    browser.set_description("The device offers these services.");
    assert_eq!(browser.title(), "Share over Bluetooth");
    assert_eq!(browser.prompt(), "Choose a service");
    assert_eq!(browser.description(), "The device offers these services.");
}

#[test]
fn the_option_bits_are_stored_as_the_caller_gives_them() {
    let mut browser = ServiceBrowser::new();
    assert_eq!(browser.options(), 0);
    browser.set_options(0b101);
    assert_eq!(browser.options(), 0b101);
}
