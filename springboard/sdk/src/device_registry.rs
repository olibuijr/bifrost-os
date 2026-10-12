//! Device registry for apps: list the devices of a class with their properties, match them by property values,
//! and watch for devices that arrive or go away. The model and the sysfs scan live in
//! `bifrost_core::device_registry`. Apps get read-only data: nothing here opens a device or calls a driver.
//! Change events come from polling (`DeviceWatch::observe`), not from a kernel notification.
pub use bifrost_core::device_registry::{changes, find, matching, scan_in, Change, Device, Matching};
use std::path::Path;

/// The kernel's device class directory.
pub const CLASS_ROOT: &str = "/sys/class";

/// The devices of `class` on this system, with the listed property keys read from the device directories.
pub fn devices(class: &str, keys: &[&str]) -> Vec<Device> {
    scan_in(Path::new(CLASS_ROOT), class, keys)
}

/// Remembers the last snapshot and reports what changed since the previous `observe`.
/// A new watch starts from an empty snapshot, so its first `observe` reports every device as arrived.
#[derive(Clone, Debug, Default)]
pub struct DeviceWatch {
    last: Vec<Device>,
}

impl DeviceWatch {
    pub fn new() -> DeviceWatch {
        DeviceWatch { last: Vec::new() }
    }

    /// A watch that already knows `snapshot`, so the next `observe` reports only what differs from it.
    pub fn starting_from(snapshot: Vec<Device>) -> DeviceWatch {
        DeviceWatch { last: snapshot }
    }

    /// Stores `now` as the new snapshot and returns the arrivals and removals since the last one.
    pub fn observe(&mut self, now: Vec<Device>) -> Vec<Change> {
        let out = changes(&self.last, &now);
        self.last = now;
        out
    }

    /// The snapshot the next `observe` will compare against.
    pub fn last(&self) -> &[Device] {
        &self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_watch_reports_everything_on_the_first_observe() {
        let mut watch = DeviceWatch::new();
        let now = vec![Device::new("input", "event0"), Device::new("input", "event1")];
        let out = watch.observe(now.clone());
        assert_eq!(out, vec![Change::Arrived(now[0].clone()), Change::Arrived(now[1].clone())]);
        assert_eq!(watch.last().len(), 2);
    }

    #[test]
    fn observe_reports_only_the_difference_from_the_previous_snapshot() {
        let mut watch = DeviceWatch::starting_from(vec![Device::new("input", "event0"), Device::new("input", "event1")]);
        let out = watch.observe(vec![Device::new("input", "event1")]);
        assert_eq!(out, vec![Change::Removed(Device::new("input", "event0"))]);
        // The same snapshot again is quiet.
        assert!(watch.observe(vec![Device::new("input", "event1")]).is_empty());
    }

    #[test]
    fn a_watch_started_from_a_snapshot_reports_only_new_devices() {
        let base = vec![Device::new("drm", "card0")];
        let mut watch = DeviceWatch::starting_from(base.clone());
        let out = watch.observe(vec![Device::new("drm", "card0"), Device::new("drm", "card1")]);
        assert_eq!(out, vec![Change::Arrived(Device::new("drm", "card1"))]);
    }

    #[test]
    fn devices_of_a_class_that_does_not_exist_is_empty() {
        // The class root is the real kernel directory; a class name that cannot exist yields nothing.
        assert!(devices("bifrost-no-such-class", &["vendor"]).is_empty());
    }

    #[test]
    fn matching_is_reexported_for_apps() {
        let all = vec![
            Device::new("drm", "card0").with_property("vendor", "0x8086"),
            Device::new("drm", "card1").with_property("vendor", "0x1002"),
        ];
        let found = matching(&all, &Matching::class("drm").with_property("vendor", "0x1002"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "card1");
    }
}
