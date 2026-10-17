//! Remote Bluetooth devices as an app sees them: a name, a class of device, whether the device is paired and whether
//! it is connected. `DeviceList` keeps one entry per address, merges each report from an inquiry into its entry
//! without losing the paired and connected state, and orders the list the way Settings shows it: paired devices first,
//! then the rest, each group by label.
//!
//! This is the model only. The shell keeps its own device table in `springboard/src/bare/bluetooth.rs`, and no app can
//! reach this list yet, so the matrix row that uses it is partial. Pure: no radio and no socket.

use std::collections::BTreeMap;

use bifrost_core::bt_address::{BdAddr, ClassOfDevice, MajorClass};

/// One remote device. The address is its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDevice {
    pub address: BdAddr,
    /// The class of device from the last report that carried one.
    pub class: Option<ClassOfDevice>,
    /// The remote name, when the device has reported one.
    pub name: Option<String>,
    pub paired: bool,
    pub connected: bool,
    /// Signal strength in dBm from the last report that carried one.
    pub rssi: Option<i8>,
}

impl RemoteDevice {
    /// A device with nothing known about it yet.
    pub fn new(address: BdAddr) -> Self {
        RemoteDevice { address, class: None, name: None, paired: false, connected: false, rssi: None }
    }

    /// Merges one report into this device. A value that is absent in the report leaves the old value in place, and an
    /// empty name counts as absent.
    pub fn merge(&mut self, class: Option<ClassOfDevice>, name: Option<&str>, rssi: Option<i8>) {
        if class.is_some() {
            self.class = class;
        }
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            self.name = Some(n.to_string());
        }
        if rssi.is_some() {
            self.rssi = rssi;
        }
    }

    /// The name, or the address when the device never gave a name.
    pub fn label(&self) -> String {
        match self.name.as_deref() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => self.address.to_string(),
        }
    }

    /// The major device class, when the class of device is known.
    pub fn major_class(&self) -> Option<MajorClass> {
        self.class.map(|c| c.major())
    }

    /// The rows of the device's properties sheet, in display order: name, address, type, paired, connected and signal.
    /// A value the device never reported reads "Not reported".
    pub fn properties(&self) -> Vec<(&'static str, String)> {
        let yes_no = |on: bool| (if on { "Yes" } else { "No" }).to_string();
        vec![
            ("Name", self.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "Not reported".to_string())),
            ("Address", self.address.to_string()),
            ("Type", self.major_class().map_or("Not reported".to_string(), |m| major_class_title(m).to_string())),
            ("Paired", yes_no(self.paired)),
            ("Connected", yes_no(self.connected)),
            ("Signal", self.rssi.map_or("Not reported".to_string(), |r| format!("{r} dBm"))),
        ]
    }
}

/// The name of a major device class as the properties sheet shows it.
pub fn major_class_title(class: MajorClass) -> &'static str {
    match class {
        MajorClass::Miscellaneous => "Miscellaneous",
        MajorClass::Computer => "Computer",
        MajorClass::Phone => "Phone",
        MajorClass::NetworkAccess => "Network access",
        MajorClass::AudioVideo => "Audio and video",
        MajorClass::Peripheral => "Peripheral",
        MajorClass::Imaging => "Imaging",
        MajorClass::Wearable => "Wearable",
        MajorClass::Toy => "Toy",
        MajorClass::Health => "Health",
        MajorClass::Uncategorized => "Uncategorized",
        MajorClass::Reserved(_) => "Other",
    }
}

/// The remote devices this app knows about, one entry per address.
#[derive(Debug, Default)]
pub struct DeviceList {
    devices: BTreeMap<BdAddr, RemoteDevice>,
}

impl DeviceList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of devices in the list.
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// The entry for `address`, when the list has one.
    pub fn get(&self, address: BdAddr) -> Option<&RemoteDevice> {
        self.devices.get(&address)
    }

    /// Merges one inquiry report. Returns true when the address was not in the list before.
    pub fn report(&mut self, address: BdAddr, class: Option<ClassOfDevice>, name: Option<&str>, rssi: Option<i8>) -> bool {
        let new = !self.devices.contains_key(&address);
        self.devices.entry(address).or_insert_with(|| RemoteDevice::new(address)).merge(class, name, rssi);
        new
    }

    /// Sets the paired state. A device that was never reported is added with nothing else known about it.
    pub fn set_paired(&mut self, address: BdAddr, paired: bool) {
        self.devices.entry(address).or_insert_with(|| RemoteDevice::new(address)).paired = paired;
    }

    /// Sets the connected state. A device that was never reported is added with nothing else known about it.
    pub fn set_connected(&mut self, address: BdAddr, connected: bool) {
        self.devices.entry(address).or_insert_with(|| RemoteDevice::new(address)).connected = connected;
    }

    /// Removes a device, its pairing and its connection state included. Returns false when it was not in the list.
    pub fn forget(&mut self, address: BdAddr) -> bool {
        self.devices.remove(&address).is_some()
    }

    /// Every device: paired ones first, then the rest, each group by label (ignoring case), then by address.
    pub fn all(&self) -> Vec<&RemoteDevice> {
        let mut v: Vec<&RemoteDevice> = self.devices.values().collect();
        v.sort_by(|a, b| {
            (!a.paired)
                .cmp(&!b.paired)
                .then_with(|| a.label().to_lowercase().cmp(&b.label().to_lowercase()))
                .then_with(|| a.address.cmp(&b.address))
        });
        v
    }

    /// The paired devices, in the order of `all`.
    pub fn paired(&self) -> Vec<&RemoteDevice> {
        self.all().into_iter().filter(|d| d.paired).collect()
    }

    /// The connected devices, in the order of `all`.
    pub fn connected(&self) -> Vec<&RemoteDevice> {
        self.all().into_iter().filter(|d| d.connected).collect()
    }
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

    #[test]
    fn properties_sheet_lists_what_the_device_reported() {
        let mut d = RemoteDevice::new(addr(7));
        d.merge(class(0x240404), Some("Speaker"), Some(-55));
        d.paired = true;
        let rows = d.properties();
        let get = |k: &str| rows.iter().find(|(label, _)| *label == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(rows.iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["Name", "Address", "Type", "Paired", "Connected", "Signal"]);
        assert_eq!(get("Name"), "Speaker");
        assert_eq!(get("Address"), addr(7).to_string());
        assert_eq!(get("Type"), "Audio and video");
        assert_eq!(get("Paired"), "Yes");
        assert_eq!(get("Connected"), "No");
        assert_eq!(get("Signal"), "-55 dBm");
    }

    #[test]
    fn properties_sheet_says_not_reported_for_unknown_values() {
        let d = RemoteDevice::new(addr(8));
        let rows = d.properties();
        for k in ["Name", "Type", "Signal"] {
            let v = rows.iter().find(|(label, _)| *label == k).map(|(_, v)| v.as_str()).unwrap();
            assert_eq!(v, "Not reported", "{k}");
        }
    }

    #[test]
    fn every_major_class_has_a_title() {
        assert_eq!(major_class_title(MajorClass::Reserved(9)), "Other");
        assert_eq!(major_class_title(MajorClass::Phone), "Phone");
    }

    #[test]
    fn a_new_address_is_added_once_and_later_reports_merge() {
        let mut list = DeviceList::new();
        assert!(list.report(addr(1), class(0x240404), None, Some(-70)));
        assert!(!list.report(addr(1), None, Some("Speaker"), Some(-55)));
        assert_eq!(list.len(), 1);
        let d = list.get(addr(1)).unwrap();
        assert_eq!(d.name.as_deref(), Some("Speaker"));
        assert_eq!(d.class, class(0x240404));
        assert_eq!(d.rssi, Some(-55));
    }

    #[test]
    fn an_absent_or_empty_name_keeps_the_old_name_and_label_falls_back_to_the_address() {
        let mut list = DeviceList::new();
        list.report(addr(2), None, Some(""), None);
        assert_eq!(list.get(addr(2)).unwrap().label(), "00:00:00:00:00:02");
        list.report(addr(2), None, Some("Keys"), None);
        list.report(addr(2), None, None, None);
        assert_eq!(list.get(addr(2)).unwrap().label(), "Keys");
    }

    #[test]
    fn a_report_does_not_clear_paired_or_connected_state() {
        let mut list = DeviceList::new();
        list.set_paired(addr(3), true);
        list.set_connected(addr(3), true);
        list.report(addr(3), None, Some("Headset"), Some(-40));
        let d = list.get(addr(3)).unwrap();
        assert!(d.paired && d.connected);
        assert_eq!(d.label(), "Headset");
    }

    #[test]
    fn paired_devices_come_first_then_each_group_by_label_ignoring_case() {
        let mut list = DeviceList::new();
        list.report(addr(10), None, Some("alpha"), None);
        list.report(addr(11), None, Some("Zeta"), None);
        list.report(addr(12), None, Some("Beta"), None);
        list.set_paired(addr(11), true);
        list.set_paired(addr(12), true);
        let labels: Vec<String> = list.all().iter().map(|d| d.label()).collect();
        assert_eq!(labels, vec!["Beta", "Zeta", "alpha"]);
        let paired: Vec<BdAddr> = list.paired().iter().map(|d| d.address).collect();
        assert_eq!(paired, vec![addr(12), addr(11)]);
    }

    #[test]
    fn connected_filters_and_forget_removes_the_device_once() {
        let mut list = DeviceList::new();
        list.set_paired(addr(20), true);
        list.set_connected(addr(20), true);
        list.report(addr(21), None, Some("Idle"), None);
        assert_eq!(list.connected().len(), 1);
        assert!(list.forget(addr(20)));
        assert!(!list.forget(addr(20)));
        assert!(list.connected().is_empty());
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn major_class_is_read_from_the_class_of_device() {
        let mut list = DeviceList::new();
        list.report(addr(30), class(0x240414), None, None);
        let d = list.get(addr(30)).unwrap();
        assert_eq!(d.major_class(), Some(MajorClass::AudioVideo));
        list.report(addr(31), None, None, None);
        assert_eq!(list.get(addr(31)).unwrap().major_class(), None);
    }
}
