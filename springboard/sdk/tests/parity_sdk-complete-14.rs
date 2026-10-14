//! Parity sdk-complete-14: headless tests of the app-facing accessory modules. The system layer is simulated by
//! calling attach, detach and receive directly; no bus, device or thread is used.

use bifrost_app::accessory_access::{
    AccessoryManager as AccessAccessoryManager, Accessory as AccessAccessory, DeviceMatch, Error as AccessError,
    Event as AccessEvent, InterfaceMatch, MatchingCriteria,
};
use bifrost_app::external_accessory::{Accessory, AccessoryError, AccessoryEvent, AccessoryManager};
use bifrost_app::stream::StreamStatus;
use bifrost_core::usb_descriptors::DeviceDescriptor;

fn meter() -> Accessory {
    Accessory::new(7, "Meter", "Acme", "M-100", "SN-1", &["com.acme.meter"])
}

fn scale() -> Accessory {
    Accessory::new(9, "Scale", "Acme", "S-2", "SN-2", &["com.acme.log", "com.acme.meter"])
}

fn usb_device(vendor: u16) -> DeviceDescriptor {
    DeviceDescriptor {
        usb_version: 0x0200,
        class: 0,
        subclass: 0,
        protocol: 0,
        max_packet_size0: 64,
        vendor_id: vendor,
        product_id: 0x0001,
        device_version: 0x0100,
        configurations: 1,
    }
}

#[test]
fn a_registered_app_gets_one_connect_and_one_disconnect_event_per_accessory() {
    let mut manager = AccessoryManager::new();
    manager.register_for_notifications();
    assert!(manager.attach(meter()));
    assert!(manager.detach(7));
    let events = manager.take_events();
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], AccessoryEvent::Connected(a) if a.connection_id() == 7 && a.is_connected()));
    assert!(matches!(&events[1], AccessoryEvent::Disconnected(a) if !a.is_connected()));
}

#[test]
fn unregistering_drops_the_events_that_were_not_taken() {
    let mut manager = AccessoryManager::new();
    manager.register_for_notifications();
    manager.attach(meter());
    manager.unregister_for_notifications();
    assert!(manager.take_events().is_empty());
    assert!(!manager.is_registered());
}

#[test]
fn protocol_matching_returns_each_accessory_that_declares_the_protocol_in_attach_order() {
    let mut manager = AccessoryManager::new();
    manager.attach(meter());
    manager.attach(scale());
    let logs: Vec<u64> = manager.accessories_for_protocol("com.acme.log").iter().map(|a| a.connection_id()).collect();
    assert_eq!(logs, vec![9]);
    let meters: Vec<u64> = manager.accessories_for_protocol("com.acme.meter").iter().map(|a| a.connection_id()).collect();
    assert_eq!(meters, vec![7, 9]);
}

#[test]
fn a_session_carries_bytes_in_both_directions_and_closes_its_output() {
    let mut manager = AccessoryManager::new();
    manager.attach(scale());
    let mut session = manager.session(9, "com.acme.log").expect("declared protocol opens");
    assert_eq!(session.protocol_string(), "com.acme.log");
    assert_eq!(session.output_status(), StreamStatus::NotOpen);
    session.receive(b"ok");
    let mut buf = [0u8; 4];
    session.input_stream().open();
    assert_eq!(session.input_stream().read(&mut buf), Ok(2));
    assert_eq!(&buf[..2], b"ok");
    session.output_stream().open();
    assert_eq!(session.output_stream().write(b"ping"), Ok(4));
    assert_eq!(session.take_output(), b"ping".to_vec());
    session.close();
    assert_eq!(session.output_status(), StreamStatus::Closed);
}

#[test]
fn a_session_names_the_protocol_error_that_fits_the_accessory_state() {
    let mut manager = AccessoryManager::new();
    manager.attach(meter());
    assert_eq!(manager.session(7, "com.acme.log").err(), Some(AccessoryError::ProtocolNotSupported));
    assert_eq!(manager.session(8, "com.acme.meter").err(), Some(AccessoryError::NotConnected));
}

#[test]
fn accessory_errors_are_readable_text() {
    assert_eq!(AccessoryError::NotConnected.to_string(), "The accessory is not connected");
    assert_eq!(AccessoryError::ProtocolNotSupported.to_string(), "The accessory does not support this protocol");
}

fn access_criteria(vendor: u16) -> MatchingCriteria {
    MatchingCriteria { device: DeviceMatch { vendor_id: Some(vendor), ..Default::default() }, ..Default::default() }
}

#[test]
fn an_access_listener_hears_only_the_vendor_it_matches() {
    let mut manager = AccessAccessoryManager::new();
    assert_eq!(manager.register(1, access_criteria(0x1234)), Ok(Vec::new()));
    manager.attach(AccessAccessory::new(1, usb_device(0x9999)));
    assert!(manager.take_events(1).is_empty());
    let id = manager.attach(AccessAccessory::new(2, usb_device(0x1234)));
    assert_eq!(manager.take_events(1), vec![AccessEvent::Connected(id)]);
}

#[test]
fn an_access_listener_can_be_told_about_matches_that_were_already_connected() {
    let mut manager = AccessAccessoryManager::new();
    let id = manager.attach(AccessAccessory::new(0, usb_device(0x1234)));
    assert_eq!(manager.register(2, access_criteria(0x1234)), Ok(vec![id]));
}

#[test]
fn an_interface_criterion_names_the_interface_class_it_needs() {
    let criteria = MatchingCriteria {
        device: DeviceMatch::default(),
        interfaces: vec![InterfaceMatch { class: Some(0x03), subclass: None, protocol: None }],
        option: None,
    };
    let mut manager = AccessAccessoryManager::new();
    manager.register(4, criteria).unwrap();
    manager.attach(AccessAccessory::new(0, usb_device(0x1234)));
    assert!(manager.take_events(4).is_empty(), "a device with no read configuration does not match");
}

#[test]
fn an_access_open_is_exclusive_and_its_errors_read_as_text() {
    let mut manager = AccessAccessoryManager::new();
    let id = manager.attach(AccessAccessory::new(0, usb_device(0x1234)));
    assert_eq!(manager.open(id, 10), Ok(()));
    assert_eq!(manager.open(id, 11), Err(AccessError::NotAccessible));
    assert_eq!(AccessError::InvalidState.to_string(), "The accessory is not in the state this call needs");
    assert_eq!(manager.close(id, 10), Ok(()));
}
