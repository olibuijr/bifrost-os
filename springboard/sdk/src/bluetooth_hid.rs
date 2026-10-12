//! Human interface devices (keyboards, pointers and other peripherals) as an app sees them. An app can count the
//! connected and paired HID devices, and it can put a device on an ignore list. An ignored device is left out of the
//! counts; it does not change the pairing or the connection. The HID device is still in the `DeviceList` it came from.
//!
//! A device is a HID device when its class of device names the peripheral major class. Devices with no known class are
//! never counted. `IgnoreList` and `counts` are pure: they read a `DeviceList` and change nothing in the radio layer,
//! and no shell code calls them yet, so the matrix row that uses them is partial.

use std::collections::BTreeSet;

use bifrost_core::bt_address::{BdAddr, ClassOfDevice, MajorClass};

use crate::bluetooth_devices::{DeviceList, RemoteDevice};

/// True when the device's class of device names the peripheral major class.
pub fn is_hid(device: &RemoteDevice) -> bool {
    device.major_class() == Some(MajorClass::Peripheral)
}

/// The addresses of HID devices the user asked to leave out of the counts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IgnoreList {
    ignored: BTreeSet<BdAddr>,
}

impl IgnoreList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an address. Returns true when it was not on the list before.
    pub fn ignore(&mut self, address: BdAddr) -> bool {
        self.ignored.insert(address)
    }

    /// Takes an address off the list. Returns true when it was on the list.
    pub fn remove(&mut self, address: BdAddr) -> bool {
        self.ignored.remove(&address)
    }

    pub fn is_ignored(&self, address: BdAddr) -> bool {
        self.ignored.contains(&address)
    }

    /// Number of ignored addresses.
    pub fn len(&self) -> usize {
        self.ignored.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ignored.is_empty()
    }
}

/// How many HID devices an app sees: the connected ones and the paired ones, neither counting ignored devices.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HidCounts {
    pub connected: usize,
    pub paired: usize,
}

/// Counts the HID devices in `list` that are not on the ignore list.
pub fn counts(list: &DeviceList, ignore: &IgnoreList) -> HidCounts {
    let mut out = HidCounts::default();
    for d in list.all().into_iter().filter(|d| is_hid(d) && !ignore.is_ignored(d.address)) {
        if d.paired {
            out.paired += 1;
        }
        if d.connected {
            out.connected += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> BdAddr {
        BdAddr([0, 0, 0, 0, 0, n])
    }

    fn class(raw: u32) -> Option<ClassOfDevice> {
        ClassOfDevice::new(raw)
    }

    /// Keyboard (peripheral, keyboard bit), pointer (peripheral, pointer bit), headset (audio-video), and an unnamed
    /// peripheral with neither bit.
    fn sample() -> DeviceList {
        let mut list = DeviceList::new();
        list.report(addr(1), class(0x002540), Some("Keyboard"), None);
        list.report(addr(2), class(0x002580), Some("Mouse"), None);
        list.report(addr(3), class(0x240404), Some("Headset"), None);
        list.report(addr(4), class(0x000500), None, None);
        list
    }

    #[test]
    fn a_peripheral_class_is_hid_and_an_audio_class_is_not() {
        let list = sample();
        assert!(is_hid(list.get(addr(1)).unwrap()));
        assert!(is_hid(list.get(addr(4)).unwrap()));
        assert!(!is_hid(list.get(addr(3)).unwrap()));
    }

    #[test]
    fn counts_separate_connected_and_paired_hid_devices() {
        let mut list = sample();
        list.set_paired(addr(1), true);
        list.set_connected(addr(1), true);
        list.set_paired(addr(2), true);
        list.set_connected(addr(3), true);
        assert_eq!(counts(&list, &IgnoreList::new()), HidCounts { connected: 1, paired: 2 });
    }

    #[test]
    fn an_ignored_device_leaves_the_counts_and_comes_back_when_removed() {
        let mut list = sample();
        list.set_paired(addr(1), true);
        list.set_connected(addr(1), true);
        let mut ignore = IgnoreList::new();
        assert!(ignore.ignore(addr(1)));
        assert!(!ignore.ignore(addr(1)));
        assert_eq!(counts(&list, &ignore), HidCounts { connected: 0, paired: 0 });
        assert!(ignore.remove(addr(1)));
        assert!(!ignore.remove(addr(1)));
        assert_eq!(counts(&list, &ignore), HidCounts { connected: 1, paired: 1 });
    }

    #[test]
    fn ignoring_a_non_hid_device_changes_nothing() {
        let mut list = sample();
        list.set_paired(addr(3), true);
        list.set_connected(addr(3), true);
        let mut ignore = IgnoreList::new();
        ignore.ignore(addr(3));
        assert_eq!(counts(&list, &ignore), HidCounts::default());
        assert_eq!(ignore.len(), 1);
        assert!(!ignore.is_empty());
    }

    #[test]
    fn a_device_with_no_known_class_is_never_counted() {
        let mut list = DeviceList::new();
        list.set_paired(addr(9), true);
        list.set_connected(addr(9), true);
        assert_eq!(counts(&list, &IgnoreList::new()), HidCounts::default());
    }
}
