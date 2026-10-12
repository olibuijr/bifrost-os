//! HID devices for apps: the matching criteria and device model the platform's device manager uses, and a manager that
//! reports which devices started or stopped matching between two snapshots. The model lives in
//! `bifrost_ui::hid_match`. The shell supplies each snapshot from its evdev scan, so an app sees devices, their roles
//! and their transport, never raw input reports.
pub use bifrost_ui::hid_match::{changes, matching, Changes, Criteria, Device, Role, Transport, Usage};

/// Follows the devices that match a list of criteria. Each call to [`DeviceManager::update`] passes a new snapshot of
/// every connected device and gets back only what changed for the criteria.
#[derive(Debug, Default)]
pub struct DeviceManager {
    criteria: Vec<Criteria>,
    last: Vec<Device>,
}

impl DeviceManager {
    /// A manager for `criteria`. It knows no devices until the first snapshot.
    pub fn new(criteria: Vec<Criteria>) -> DeviceManager {
        DeviceManager { criteria, last: Vec::new() }
    }

    pub fn criteria(&self) -> &[Criteria] {
        &self.criteria
    }

    /// Replaces the last snapshot with `snapshot` and returns the devices that now match and the devices that no
    /// longer match, as owned copies.
    pub fn update(&mut self, snapshot: Vec<Device>) -> (Vec<Device>, Vec<Device>) {
        let (matched, removed) = {
            let diff = changes(&self.criteria, &self.last, &snapshot);
            let matched: Vec<Device> = diff.matched.into_iter().cloned().collect();
            let removed: Vec<Device> = diff.removed.into_iter().cloned().collect();
            (matched, removed)
        };
        self.last = snapshot;
        (matched, removed)
    }

    /// The devices in the last snapshot that match the criteria.
    pub fn devices(&self) -> Vec<&Device> {
        matching(&self.criteria, &self.last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyboard(id: &str) -> Device {
        Device {
            unique_id: id.to_owned(),
            product: "keys".to_owned(),
            manufacturer: "maker".to_owned(),
            serial: String::new(),
            vendor_id: 0x1234,
            product_id: 0x0001,
            version: 1,
            transport: Transport::Usb,
            location_id: 0,
            built_in: false,
            primary_usage: Usage::KEYBOARD,
            device_usages: vec![Usage::KEYBOARD],
        }
    }

    fn mouse(id: &str) -> Device {
        Device { primary_usage: Usage::MOUSE, device_usages: vec![Usage::MOUSE], ..keyboard(id) }
    }

    #[test]
    fn first_snapshot_reports_every_matching_device_as_matched() {
        let mut mgr = DeviceManager::new(vec![Criteria { primary_usage: Some(Usage::KEYBOARD), ..Default::default() }]);
        let (matched, removed) = mgr.update(vec![keyboard("k1"), mouse("m1")]);
        let ids: Vec<&str> = matched.iter().map(|d| d.unique_id.as_str()).collect();
        assert_eq!(ids, vec!["k1"]);
        assert!(removed.is_empty());
        assert_eq!(mgr.devices().len(), 1);
    }

    #[test]
    fn unplugging_a_matching_device_reports_it_removed() {
        let mut mgr = DeviceManager::new(vec![Criteria { primary_usage: Some(Usage::KEYBOARD), ..Default::default() }]);
        mgr.update(vec![keyboard("k1"), keyboard("k2")]);
        let (matched, removed) = mgr.update(vec![keyboard("k2"), keyboard("k3")]);
        let added: Vec<&str> = matched.iter().map(|d| d.unique_id.as_str()).collect();
        let gone: Vec<&str> = removed.iter().map(|d| d.unique_id.as_str()).collect();
        assert_eq!(added, vec!["k3"]);
        assert_eq!(gone, vec!["k1"]);
        let now: Vec<&str> = mgr.devices().iter().map(|d| d.unique_id.as_str()).collect();
        assert_eq!(now, vec!["k2", "k3"]);
    }

    #[test]
    fn an_unchanged_snapshot_reports_nothing() {
        let mut mgr = DeviceManager::new(vec![Criteria::default()]);
        mgr.update(vec![keyboard("k1")]);
        let (matched, removed) = mgr.update(vec![keyboard("k1")]);
        assert!(matched.is_empty());
        assert!(removed.is_empty());
        assert_eq!(mgr.criteria().len(), 1);
    }
}
