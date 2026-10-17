//! Accessory listeners for apps: listeners that match USB accessories by device and interface criteria, receive attach and
//! detach events, and open an accessory with one owner at a time.
//!
//! The rules live in `bifrost_core::accessory_manager` and `bifrost_core::usb_descriptors`. This module gives apps the
//! same names as the framework: `AccessoryManager` (the shared manager), `Accessory` (the device object with its
//! descriptors and strings), `MatchingCriteria` (what a listener selects), `Event` (attach and detach) and `Error`.
//!
//! The manager is pure. [`bus`] is the bus source that reads the kernel's USB device list and reports attach and
//! detach to it (`bus::poll`). The app sandbox does not bind USB device nodes yet (springboard/src/bare/app_policy.rs), so an app cannot reach a real
//! accessory through this module until that binding and a device transport exist.

pub use bifrost_core::accessory_manager::{Accessory, AccessoryManager, Error, Event, Session};
pub use bifrost_core::usb_bus_source as bus;
pub use bifrost_core::usb_descriptors::{
    ConfigurationDescriptor, DeviceDescriptor, DeviceMatch, InterfaceDescriptor, InterfaceMatch, InterfaceMatchOption,
    MatchingCriteria,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn device(vid: u16, pid: u16) -> DeviceDescriptor {
        DeviceDescriptor {
            usb_version: 0x0200,
            class: 0,
            subclass: 0,
            protocol: 0,
            max_packet_size0: 64,
            vendor_id: vid,
            product_id: pid,
            device_version: 0x0100,
            configurations: 1,
        }
    }

    #[test]
    fn listener_with_vendor_criteria_hears_only_its_vendor() {
        let mut manager = AccessoryManager::new();
        let criteria = MatchingCriteria {
            device: DeviceMatch { vendor_id: Some(0x1234), ..Default::default() },
            ..Default::default()
        };
        assert_eq!(manager.register(1, criteria), Ok(Vec::new()));

        let mut other = Accessory::new(1, device(0x9999, 1));
        other.name = "Other".to_string();
        manager.attach(other);
        assert!(manager.take_events(1).is_empty());

        let id = manager.attach(Accessory::new(2, device(0x1234, 7)));
        assert_eq!(manager.take_events(1), vec![Event::Connected(id)]);
    }

    #[test]
    fn open_is_exclusive_and_the_session_needs_a_declared_protocol() {
        let mut manager = AccessoryManager::new();
        let mut pad = Accessory::new(0, device(0x1234, 7));
        pad.protocol_strings = vec!["com.example.pad".to_string()];
        let id = manager.attach(pad);

        assert_eq!(manager.open(id, 10), Ok(()));
        assert_eq!(manager.open(id, 11), Err(Error::NotAccessible));
        assert_eq!(manager.session(id, "com.example.pad").map(|s| s.protocol), Ok("com.example.pad".to_string()));
        assert_eq!(manager.session(id, "com.example.other"), Err(Error::InvalidState));
        assert_eq!(manager.close(id, 10), Ok(()));
    }
}
