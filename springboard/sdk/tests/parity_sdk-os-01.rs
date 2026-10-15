//! Parity sdk-os-01: headless tests for the device models an app reads (HID devices, remote Bluetooth devices).
//! No test opens a device, a radio or a socket.

use bifrost_app::bluetooth_devices::DeviceList;
use bifrost_app::hid_devices::{changes, matching, Criteria, Device, DeviceManager, Transport, Usage};
use bifrost_core::bt_address::BdAddr;

fn hid(id: &str, usage: Usage, transport: Transport) -> Device {
    Device {
        unique_id: id.to_owned(),
        product: format!("product {id}"),
        manufacturer: "maker".to_owned(),
        serial: String::new(),
        vendor_id: 0x046d,
        product_id: 0xc077,
        version: 1,
        transport,
        location_id: 0x1400_0000,
        built_in: false,
        primary_usage: usage,
        device_usages: vec![usage],
    }
}

fn ids(devices: &[&Device]) -> Vec<String> {
    devices.iter().map(|d| d.unique_id.clone()).collect()
}

#[test]
fn a_mouse_criteria_selects_only_the_mouse() {
    let devices = vec![
        hid("kbd", Usage::KEYBOARD, Transport::Usb),
        hid("mouse", Usage::MOUSE, Transport::Usb),
    ];
    let mouse = [Criteria { primary_usage: Some(Usage::MOUSE), ..Default::default() }];
    assert_eq!(ids(&matching(&mouse, &devices)), vec!["mouse"]);
}

#[test]
fn a_bluetooth_transport_criteria_selects_wireless_devices_by_transport_name() {
    let devices = vec![
        hid("wired", Usage::KEYBOARD, Transport::Usb),
        hid("wireless", Usage::KEYBOARD, Transport::from_name("bluetoothLowEnergy")),
    ];
    let wireless = [Criteria { transport: Some(Transport::BluetoothLowEnergy), ..Default::default() }];
    assert_eq!(ids(&matching(&wireless, &devices)), vec!["wireless"]);
    assert_eq!(Transport::BluetoothLowEnergy.name(), "bluetoothLowEnergy");
}

#[test]
fn an_unknown_transport_name_is_kept_as_written() {
    assert_eq!(Transport::from_name("vendorBus").name(), "vendorBus");
    assert_eq!(Transport::from_name("usb"), Transport::Usb);
}

#[test]
fn an_empty_criteria_list_matches_no_device() {
    let devices = vec![hid("kbd", Usage::KEYBOARD, Transport::Usb)];
    assert!(matching(&[], &devices).is_empty());
}

#[test]
fn the_change_set_reports_a_newly_plugged_mouse_and_a_removed_one() {
    let criteria = [Criteria { primary_usage: Some(Usage::MOUSE), ..Default::default() }];
    let before = vec![hid("m1", Usage::MOUSE, Transport::Usb)];
    let after = vec![hid("m2", Usage::MOUSE, Transport::Usb)];
    let diff = changes(&criteria, &before, &after);
    assert_eq!(ids(&diff.matched), vec!["m2"]);
    assert_eq!(ids(&diff.removed), vec!["m1"]);
}

#[test]
fn the_device_manager_reports_a_keyboard_that_was_unplugged_as_removed() {
    let mut manager = DeviceManager::new(vec![Criteria { primary_usage: Some(Usage::KEYBOARD), ..Default::default() }]);
    let (first_matched, first_removed) = manager.update(vec![hid("k1", Usage::KEYBOARD, Transport::Usb), hid("m1", Usage::MOUSE, Transport::Usb)]);
    assert_eq!(first_matched.len(), 1);
    assert!(first_removed.is_empty());

    let (matched, removed) = manager.update(vec![hid("m1", Usage::MOUSE, Transport::Usb)]);
    assert!(matched.is_empty());
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].unique_id, "k1");
    assert!(manager.devices().is_empty(), "no keyboard is left in the last snapshot");
}

fn address(last: u8) -> BdAddr {
    BdAddr([0x00, 0x1A, 0x7D, 0xDA, 0x71, last])
}

#[test]
fn a_device_list_keeps_one_entry_per_address_and_merges_reports() {
    let mut list = DeviceList::new();
    assert!(list.report(address(1), None, Some("Headset"), Some(-40)), "the first report adds the device");
    assert!(!list.report(address(1), None, None, Some(-55)), "a second report for the same address does not");
    assert_eq!(list.len(), 1);
    let device = list.get(address(1)).expect("the device is listed");
    assert_eq!(device.name.as_deref(), Some("Headset"), "an absent name keeps the earlier name");
    assert_eq!(device.rssi, Some(-55), "the newest signal strength wins");
}

#[test]
fn a_device_without_a_name_is_labelled_by_its_address() {
    let mut list = DeviceList::new();
    list.report(address(2), None, None, None);
    assert_eq!(list.get(address(2)).expect("listed").label(), address(2).to_string());
}

#[test]
fn paired_devices_are_listed_before_the_rest_and_each_group_by_label() {
    let mut list = DeviceList::new();
    list.report(address(1), None, Some("Zebra speaker"), None);
    list.report(address(2), None, Some("alpha keyboard"), None);
    list.report(address(3), None, Some("Beta mouse"), None);
    list.set_paired(address(1), true);
    list.set_paired(address(3), true);
    let labels: Vec<String> = list.all().iter().map(|d| d.label()).collect();
    assert_eq!(labels, vec!["Beta mouse", "Zebra speaker", "alpha keyboard"]);
    assert_eq!(list.paired().len(), 2);
}

#[test]
fn a_connection_state_survives_a_later_inquiry_report() {
    let mut list = DeviceList::new();
    list.report(address(4), None, Some("Pad"), None);
    list.set_connected(address(4), true);
    list.report(address(4), None, Some("Pad"), Some(-30));
    assert_eq!(list.connected().len(), 1, "an inquiry report does not disconnect a device");
}

#[test]
fn forgetting_a_device_removes_its_pairing_and_connection() {
    let mut list = DeviceList::new();
    list.set_paired(address(5), true);
    list.set_connected(address(5), true);
    assert!(list.forget(address(5)));
    assert!(list.is_empty());
    assert!(!list.forget(address(5)), "a second forget finds nothing");
}
