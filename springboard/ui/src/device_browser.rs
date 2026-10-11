//! Device browser: the cameras and scanners attached to the machine, and the add and remove events a
//! browser reports as they come and go. This is the pure model. The system layer enumerates USB devices
//! and watches for hotplug; it hands each snapshot to `DeviceBrowser::update`, and the browser reports
//! only what changed since the previous snapshot. Devices are identified by a stable id, so a device that
//! is unplugged and plugged back in is the same device to the caller.

use std::collections::BTreeMap;

/// The kinds of device a browser can look for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Camera,
    Scanner,
}

impl DeviceKind {
    fn bit(self) -> u8 {
        match self {
            DeviceKind::Camera => 1,
            DeviceKind::Scanner => 2,
        }
    }
}

/// A set of device kinds to browse for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DeviceMask(u8);

impl DeviceMask {
    pub const NONE: DeviceMask = DeviceMask(0);
    pub const CAMERA: DeviceMask = DeviceMask(1);
    pub const SCANNER: DeviceMask = DeviceMask(2);
    pub const ALL: DeviceMask = DeviceMask(3);

    pub fn contains(self, kind: DeviceKind) -> bool {
        self.0 & kind.bit() != 0
    }

    pub fn union(self, other: DeviceMask) -> DeviceMask {
        DeviceMask(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// How the device is connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Network,
    Other,
}

/// One attached device, as the system layer describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Stable identity across reconnects, for example a serial number or a persistent id.
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub transport: Transport,
    pub serial: Option<String>,
}

/// A change the browser reports to its caller, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceEvent {
    /// A device appeared.
    Added(Device),
    /// A device went away. Carries the last description the browser had of it.
    Removed(Device),
}

/// Browses for devices of the kinds in its mask. Holds the devices seen in the latest snapshot.
#[derive(Debug, Default)]
pub struct DeviceBrowser {
    mask: DeviceMask,
    browsing: bool,
    devices: BTreeMap<String, Device>,
    preferred: Option<String>,
}

impl DeviceBrowser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts browsing for the kinds in `mask`. A browser asked for nothing does not start, and the
    /// call returns false. A browser that is already running narrows to the new mask: devices of a kind
    /// it no longer browses are forgotten without events, since the caller asked for that.
    pub fn start(&mut self, mask: DeviceMask) -> bool {
        if mask.is_empty() {
            return false;
        }
        self.mask = mask;
        self.browsing = true;
        self.devices.retain(|_, d| mask.contains(d.kind));
        true
    }

    /// Stops browsing. Known devices stay listed, and `update` changes nothing until the browser starts again.
    pub fn stop(&mut self) {
        self.browsing = false;
    }

    pub fn is_browsing(&self) -> bool {
        self.browsing
    }

    /// The kinds the browser is looking for.
    pub fn browsed_mask(&self) -> DeviceMask {
        self.mask
    }

    /// Takes a new snapshot of attached devices and returns what changed since the last one.
    ///
    /// Only devices of the browsed kinds count. A snapshot listing the same id twice keeps the first.
    /// Removals come first, then additions, each in id order. A device whose id is unchanged but whose
    /// name or serial changed is updated in place with no event. While the browser is stopped, the
    /// snapshot is ignored and no event is returned.
    pub fn update(&mut self, seen: Vec<Device>) -> Vec<DeviceEvent> {
        if !self.browsing {
            return Vec::new();
        }
        let mut next: BTreeMap<String, Device> = BTreeMap::new();
        for d in seen {
            if self.mask.contains(d.kind) && !next.contains_key(&d.id) {
                next.insert(d.id.clone(), d);
            }
        }
        let mut events = Vec::new();
        for (id, old) in &self.devices {
            if !next.contains_key(id) {
                events.push(DeviceEvent::Removed(old.clone()));
            }
        }
        for (id, new) in &next {
            if !self.devices.contains_key(id) {
                events.push(DeviceEvent::Added(new.clone()));
            }
        }
        self.devices = next;
        events
    }

    /// Every device in the latest snapshot, in id order.
    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.values()
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.get(id)
    }

    /// Remembers `id` as the device to select when the app next opens. Returns false when no device with
    /// that id is attached now.
    pub fn set_preferred(&mut self, id: &str) -> bool {
        if self.devices.contains_key(id) {
            self.preferred = Some(id.to_string());
            true
        } else {
            false
        }
    }

    /// The device to select: the remembered one when it is attached, otherwise the first attached device.
    /// The remembered id is kept while the device is away, so it is selected again when it returns.
    pub fn preferred_device(&self) -> Option<&Device> {
        if let Some(id) = &self.preferred {
            if let Some(d) = self.devices.get(id) {
                return Some(d);
            }
        }
        self.devices.values().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(id: &str) -> Device {
        Device { id: id.into(), name: format!("Camera {id}"), kind: DeviceKind::Camera, transport: Transport::Usb, serial: None }
    }

    fn scanner(id: &str) -> Device {
        Device { id: id.into(), name: format!("Scanner {id}"), kind: DeviceKind::Scanner, transport: Transport::Usb, serial: None }
    }

    #[test]
    fn a_browser_asked_for_nothing_does_not_start() {
        let mut b = DeviceBrowser::new();
        assert!(!b.start(DeviceMask::NONE));
        assert!(!b.is_browsing());
        assert!(b.start(DeviceMask::CAMERA));
        assert!(b.is_browsing());
    }

    #[test]
    fn only_the_browsed_kinds_are_reported() {
        let mut b = DeviceBrowser::new();
        b.start(DeviceMask::CAMERA);
        let events = b.update(vec![cam("c1"), scanner("s1")]);
        assert_eq!(events, vec![DeviceEvent::Added(cam("c1"))]);
        assert!(b.device("s1").is_none(), "a scanner is not browsed for");
        assert!(b.browsed_mask().contains(DeviceKind::Camera));
        assert!(!b.browsed_mask().contains(DeviceKind::Scanner));
    }

    #[test]
    fn hotplug_reports_each_change_once_removals_first() {
        let mut b = DeviceBrowser::new();
        b.start(DeviceMask::ALL);
        assert_eq!(b.update(vec![cam("c1"), scanner("s1")]).len(), 2);
        // c1 is unplugged, s1 stays, c2 is plugged in.
        let events = b.update(vec![scanner("s1"), cam("c2")]);
        assert_eq!(events, vec![DeviceEvent::Removed(cam("c1")), DeviceEvent::Added(cam("c2"))]);
        // The same snapshot again changes nothing.
        assert!(b.update(vec![scanner("s1"), cam("c2")]).is_empty());
        let ids: Vec<&str> = b.devices().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["c2", "s1"]);
    }

    #[test]
    fn a_stopped_browser_ignores_snapshots() {
        let mut b = DeviceBrowser::new();
        b.start(DeviceMask::ALL);
        b.update(vec![cam("c1")]);
        b.stop();
        assert!(!b.is_browsing());
        assert!(b.update(vec![]).is_empty(), "the removal is not reported while stopped");
        assert!(b.device("c1").is_some(), "known devices stay listed after stop");
    }

    #[test]
    fn a_duplicate_id_in_one_snapshot_keeps_the_first_entry() {
        let mut b = DeviceBrowser::new();
        b.start(DeviceMask::ALL);
        let mut second = cam("c1");
        second.name = "Impostor".into();
        let events = b.update(vec![cam("c1"), second]);
        assert_eq!(events, vec![DeviceEvent::Added(cam("c1"))]);
        assert_eq!(b.device("c1").unwrap().name, "Camera c1");
    }

    #[test]
    fn a_renamed_device_is_updated_without_an_event() {
        let mut b = DeviceBrowser::new();
        b.start(DeviceMask::ALL);
        b.update(vec![cam("c1")]);
        let mut renamed = cam("c1");
        renamed.serial = Some("SN-42".into());
        assert!(b.update(vec![renamed]).is_empty());
        assert_eq!(b.device("c1").unwrap().serial.as_deref(), Some("SN-42"));
    }

    #[test]
    fn the_preferred_device_survives_an_unplug_and_falls_back_meanwhile() {
        let mut b = DeviceBrowser::new();
        b.start(DeviceMask::ALL);
        b.update(vec![cam("c1"), cam("c2")]);
        assert!(!b.set_preferred("c9"), "an unknown device cannot be preferred");
        assert!(b.set_preferred("c2"));
        assert_eq!(b.preferred_device().unwrap().id, "c2");
        // c2 is unplugged: the first attached device is selected instead.
        b.update(vec![cam("c1")]);
        assert_eq!(b.preferred_device().unwrap().id, "c1");
        // c2 comes back and is selected again.
        b.update(vec![cam("c1"), cam("c2")]);
        assert_eq!(b.preferred_device().unwrap().id, "c2");
    }

    #[test]
    fn narrowing_the_mask_forgets_devices_of_the_dropped_kind() {
        let mut b = DeviceBrowser::new();
        b.start(DeviceMask::ALL);
        b.update(vec![cam("c1"), scanner("s1")]);
        assert!(b.start(DeviceMask::SCANNER));
        assert!(b.device("c1").is_none());
        assert!(b.device("s1").is_some());
    }
}
