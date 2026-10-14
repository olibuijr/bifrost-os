//! Parity sdk-complete-14: headless tests of the accessory manager's bus snapshot sync, listener events and the
//! protocol and interface matching rules. No bus, device or thread is touched.

use bifrost_core::accessory_manager::{Accessory, AccessoryManager, Event, SyncReport};
use bifrost_core::usb_descriptors::{
    ConfigurationDescriptor, DeviceDescriptor, DeviceMatch, InterfaceDescriptor, InterfaceMatch, MatchingCriteria,
};

fn device(vendor: u16, product: u16) -> DeviceDescriptor {
    DeviceDescriptor {
        usb_version: 0x0200,
        class: 0,
        subclass: 0,
        protocol: 0,
        max_packet_size0: 64,
        vendor_id: vendor,
        product_id: product,
        device_version: 0x0100,
        configurations: 1,
    }
}

fn accessory(registry_id: u64, vendor: u16) -> Accessory {
    Accessory::new(registry_id, device(vendor, 0x0001))
}

fn vendor_criteria(vendor: u16) -> MatchingCriteria {
    MatchingCriteria { device: DeviceMatch { vendor_id: Some(vendor), ..Default::default() }, ..Default::default() }
}

fn config_with_interface(class: u8) -> ConfigurationDescriptor {
    let interface =
        InterfaceDescriptor { number: 0, alternate: 0, class, subclass: 0, protocol: 0, endpoints: Vec::new() };
    ConfigurationDescriptor { value: 1, attributes: 0x80, max_power_ma: 500, interfaces: vec![interface] }
}

#[test]
fn a_first_snapshot_attaches_every_entry_in_snapshot_order() {
    let mut manager = AccessoryManager::new();
    let report = manager.sync(vec![accessory(10, 0x1111), accessory(11, 0x2222)]);
    assert_eq!(report, SyncReport { attached: vec![1, 2], detached: Vec::new() });
    assert_eq!(manager.connected_accessories().len(), 2);
    assert_eq!(manager.accessory(2).map(|a| a.registry_id), Some(11));
}

#[test]
fn an_unchanged_snapshot_changes_nothing_and_keeps_the_ids() {
    let mut manager = AccessoryManager::new();
    manager.sync(vec![accessory(10, 0x1111)]);
    let report = manager.sync(vec![accessory(10, 0x1111)]);
    assert_eq!(report, SyncReport::default());
    assert_eq!(manager.connected_accessories()[0].connection_id(), Some(1));
}

#[test]
fn a_snapshot_without_a_connected_entry_detaches_it_and_keeps_its_record() {
    let mut manager = AccessoryManager::new();
    manager.sync(vec![accessory(10, 0x1111), accessory(11, 0x2222)]);
    let report = manager.sync(vec![accessory(11, 0x2222)]);
    assert_eq!(report, SyncReport { attached: Vec::new(), detached: vec![1] });
    let record = manager.accessory(1).expect("the detached record is kept");
    assert!(!record.is_connected());
    assert_eq!(record.connection_id(), None);
}

#[test]
fn a_device_that_returns_after_a_detach_gets_a_new_connection_id() {
    let mut manager = AccessoryManager::new();
    manager.sync(vec![accessory(10, 0x1111)]);
    manager.sync(Vec::new());
    let report = manager.sync(vec![accessory(10, 0x1111)]);
    assert_eq!(report.attached, vec![2], "ids are never reused");
    assert_eq!(manager.connected_accessories()[0].connection_id(), Some(2));
}

#[test]
fn a_snapshot_that_lists_one_device_twice_attaches_it_once() {
    let mut manager = AccessoryManager::new();
    let report = manager.sync(vec![accessory(10, 0x1111), accessory(10, 0x1111)]);
    assert_eq!(report.attached, vec![1]);
    assert_eq!(manager.connected_accessories().len(), 1);
}

#[test]
fn a_listener_hears_the_attach_and_the_detach_that_a_snapshot_makes() {
    let mut manager = AccessoryManager::new();
    manager.register(7, vendor_criteria(0x1111)).unwrap();
    manager.sync(vec![accessory(10, 0x1111), accessory(11, 0x2222)]);
    manager.sync(vec![accessory(11, 0x2222)]);
    assert_eq!(manager.take_events(7), vec![Event::Connected(1), Event::Disconnected(1)]);
}

#[test]
fn a_listener_with_interface_criteria_matches_only_a_device_whose_configuration_has_that_class() {
    let mut manager = AccessoryManager::new();
    let hid_criteria = MatchingCriteria {
        device: DeviceMatch::default(),
        interfaces: vec![InterfaceMatch { class: Some(0x03), subclass: None, protocol: None }],
        option: None,
    };
    manager.register(3, hid_criteria).unwrap();
    let mut hid = accessory(20, 0x4444);
    hid.configuration = Some(config_with_interface(0x03));
    let mut storage = accessory(21, 0x4444);
    storage.configuration = Some(config_with_interface(0x08));
    manager.sync(vec![hid, storage]);
    assert_eq!(manager.take_events(3), vec![Event::Connected(1)]);
}

#[test]
fn a_connected_accessory_with_a_protocol_declares_it_for_sessions_after_a_sync() {
    let mut manager = AccessoryManager::new();
    let mut pad = accessory(30, 0x5555);
    pad.protocol_strings = vec!["com.example.pad".to_string()];
    manager.sync(vec![pad]);
    let session = manager.session(1, "com.example.pad").expect("declared protocol opens a session");
    assert_eq!(session.accessory, 1);
    assert_eq!(session.protocol, "com.example.pad");
}
