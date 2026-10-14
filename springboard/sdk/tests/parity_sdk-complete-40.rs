//! Parity sdk-complete-40: headless tests for the remote device model (name, class, paired, connected) and the
//! Bluetooth audio and link rows. They use the model only: no radio, no socket, no device.

use bifrost_app::bluetooth_devices::{DeviceList, RemoteDevice};
use bifrost_core::bt_address::{BdAddr, ClassOfDevice};

fn addr(n: u8) -> BdAddr {
    BdAddr([0, 0, 0, 0, 0, n])
}

fn class(raw: u32) -> Option<ClassOfDevice> {
    ClassOfDevice::new(raw)
}

#[test]
fn a_device_without_a_class_reads_unknown() {
    let d = RemoteDevice::new(addr(1));
    assert_eq!(d.type_label(), "Unknown");
}

#[test]
fn an_audio_video_headset_class_reads_headset() {
    let mut list = DeviceList::new();
    list.report(addr(2), class(0x240404), Some("Buds"), None);
    assert_eq!(list.get(addr(2)).unwrap().type_label(), "Headset");
}

#[test]
fn an_audio_video_loudspeaker_class_reads_speaker() {
    let mut list = DeviceList::new();
    list.report(addr(3), class(0x240414), Some("Kitchen"), None);
    assert_eq!(list.get(addr(3)).unwrap().type_label(), "Speaker");
}

#[test]
fn peripheral_classes_say_keyboard_or_pointing_device() {
    let mut list = DeviceList::new();
    list.report(addr(4), class(0x000540), None, None);
    list.report(addr(5), class(0x000580), None, None);
    assert_eq!(list.get(addr(4)).unwrap().type_label(), "Keyboard");
    assert_eq!(list.get(addr(5)).unwrap().type_label(), "Pointing Device");
}

#[test]
fn a_computer_class_reads_computer_and_an_uncategorised_class_reads_device() {
    let mut list = DeviceList::new();
    list.report(addr(6), class(0x000100), None, None);
    list.report(addr(7), class(0x001F00), None, None);
    assert_eq!(list.get(addr(6)).unwrap().type_label(), "Computer");
    assert_eq!(list.get(addr(7)).unwrap().type_label(), "Device");
}

#[test]
fn the_label_is_the_name_and_falls_back_to_the_address() {
    let mut list = DeviceList::new();
    list.report(addr(8), None, None, None);
    assert_eq!(list.get(addr(8)).unwrap().label(), "00:00:00:00:00:08");
    list.report(addr(8), None, Some("Desk speaker"), None);
    assert_eq!(list.get(addr(8)).unwrap().label(), "Desk speaker");
}

#[test]
fn a_paired_device_that_is_also_connected_appears_in_both_filters() {
    let mut list = DeviceList::new();
    list.set_paired(addr(9), true);
    list.set_connected(addr(9), true);
    list.report(addr(10), None, Some("Stranger"), None);
    assert_eq!(list.paired().len(), 1);
    assert_eq!(list.connected().len(), 1);
    assert_eq!(list.all().len(), 2);
    assert!(list.all()[0].paired, "paired devices come first");
}

#[test]
fn a_device_list_forgets_a_device_and_its_state() {
    let mut list = DeviceList::new();
    list.set_paired(addr(11), true);
    list.set_connected(addr(11), true);
    assert!(list.forget(addr(11)));
    assert!(list.is_empty());
    assert!(!list.forget(addr(11)), "a second forget finds nothing");
}
