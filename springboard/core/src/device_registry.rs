//! Device registry model: a flat list of devices, each with a class, a name and string properties.
//!
//! Pure: a snapshot is a `Vec<Device>`. `matching` and `find` query a snapshot, and `changes` diffs two snapshots
//! by (class, name) into arrivals and removals. `scan_in` builds a snapshot from a sysfs-style tree (one directory
//! per class, one child directory per device, one file per property). It only reads files and writes nothing.
//! Change events are found by comparing snapshots; the caller decides how often to take one.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// One device: its class (such as `drm` or `input`), its name within the class, and its properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub class: String,
    pub name: String,
    pub properties: BTreeMap<String, String>,
}

impl Device {
    pub fn new(class: &str, name: &str) -> Device {
        Device { class: class.to_string(), name: name.to_string(), properties: BTreeMap::new() }
    }

    /// Sets one property, replacing an earlier value under the same key.
    pub fn with_property(mut self, key: &str, value: &str) -> Device {
        self.properties.insert(key.to_string(), value.to_string());
        self
    }

    /// The value of `key`, if the device has that property.
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(|v| v.as_str())
    }
}

/// Query criteria: an optional class, and property values that must all be equal.
/// `Matching::any()` matches every device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matching {
    pub class: Option<String>,
    pub properties: Vec<(String, String)>,
}

impl Matching {
    pub fn any() -> Matching {
        Matching::default()
    }

    /// Matches devices of `class` only.
    pub fn class(class: &str) -> Matching {
        Matching { class: Some(class.to_string()), properties: Vec::new() }
    }

    /// Adds a required property value. A device without that key does not match.
    pub fn with_property(mut self, key: &str, value: &str) -> Matching {
        self.properties.push((key.to_string(), value.to_string()));
        self
    }

    pub fn matches(&self, device: &Device) -> bool {
        if self.class.as_deref().is_some_and( |c| c != device.class.as_str()) {
            return false;
        }
        self.properties.iter().all(|(k, v)| device.property(k) == Some(v.as_str()))
    }
}

/// The devices in `devices` that match `criteria`, in the order given.
pub fn matching<'a>(devices: &'a [Device], criteria: &Matching) -> Vec<&'a Device> {
    devices.iter().filter(|d| criteria.matches(d)).collect()
}

/// The device of `class` named `name`, if the snapshot has one.
pub fn find<'a>(devices: &'a [Device], class: &str, name: &str) -> Option<&'a Device> {
    devices.iter().find(|d| d.class == class && d.name == name)
}

/// A device that appeared or disappeared between two snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Arrived(Device),
    Removed(Device),
}

/// Arrivals and removals from `before` to `after`, matched by (class, name).
/// Removals come first, each in `before` order, then arrivals, each in `after` order.
/// A device that is in both snapshots is not reported, even when its properties differ.
pub fn changes(before: &[Device], after: &[Device]) -> Vec<Change> {
    let mut out = Vec::new();
    for old in before {
        if !after.iter().any(|d| same_device(d, old)) {
            out.push(Change::Removed(old.clone()));
        }
    }
    for new in after {
        if !before.iter().any(|d| same_device(d, new)) {
            out.push(Change::Arrived(new.clone()));
        }
    }
    out
}

fn same_device(a: &Device, b: &Device) -> bool {
    a.class == b.class && a.name == b.name
}

/// Reads the devices of `class` under `root`: each child directory of `root/class` is one device, and each key in
/// `keys` is read from a file of that name inside it (whitespace trimmed). A key with no readable file is left out
/// of that device. Devices come back sorted by name. A missing class directory gives an empty list.
pub fn scan_in(root: &Path, class: &str, keys: &[&str]) -> Vec<Device> {
    let dir = root.join(class);
    let mut names: Vec<String> = match fs::read_dir(&dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => return Vec::new(),
    };
    names.sort();
    names
        .iter()
        .map(|name| {
            let mut device = Device::new(class, name);
            for key in keys {
                if let Ok(text) = fs::read_to_string(dir.join(name).join(*key)) {
                    device = device.with_property(key, text.trim());
                }
            }
            device
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Device> {
        vec![
            Device::new("drm", "card0").with_property("vendor", "0x8086").with_property("driver", "i915"),
            Device::new("drm", "card1").with_property("vendor", "0x1002").with_property("driver", "amdgpu"),
            Device::new("input", "event0").with_property("vendor", "0x8086"),
        ]
    }

    #[test]
    fn matching_by_class_and_property_keeps_order() {
        let all = sample();
        let criteria = Matching::class("drm").with_property("vendor", "0x8086");
        let found: Vec<&str> = matching(&all, &criteria).iter().map(|d| d.name.as_str()).collect();
        assert_eq!(found, vec!["card0"]);
    }

    #[test]
    fn any_matches_every_device_in_order() {
        let all = sample();
        let found: Vec<&str> = matching(&all, &Matching::any()).iter().map(|d| d.name.as_str()).collect();
        assert_eq!(found, vec!["card0", "card1", "event0"]);
    }

    #[test]
    fn a_device_without_the_required_key_does_not_match() {
        let all = sample();
        // event0 has no driver key, so only the two drm cards can match a driver requirement.
        let criteria = Matching::any().with_property("driver", "i915");
        assert_eq!(matching(&all, &criteria).len(), 1);
        let none = Matching::class("input").with_property("driver", "i915");
        assert!(matching(&all, &none).is_empty());
    }

    #[test]
    fn find_needs_both_class_and_name() {
        let all = sample();
        assert_eq!(find(&all, "drm", "card1").and_then(|d| d.property("driver")), Some("amdgpu"));
        assert!(find(&all, "input", "card1").is_none());
    }

    #[test]
    fn changes_report_removals_then_arrivals() {
        let before = vec![Device::new("drm", "card0"), Device::new("drm", "card1")];
        let after = vec![Device::new("drm", "card1"), Device::new("drm", "card2")];
        let out = changes(&before, &after);
        assert_eq!(out, vec![Change::Removed(Device::new("drm", "card0")), Change::Arrived(Device::new("drm", "card2"))]);
    }

    #[test]
    fn a_property_change_alone_is_not_reported() {
        let before = vec![Device::new("power_supply", "BAT0").with_property("status", "Charging")];
        let after = vec![Device::new("power_supply", "BAT0").with_property("status", "Full")];
        assert!(changes(&before, &after).is_empty());
    }

    #[test]
    fn first_snapshot_against_nothing_is_all_arrivals() {
        let after = vec![Device::new("input", "event3"), Device::new("input", "event4")];
        let out = changes(&[], &after);
        assert_eq!(out, vec![Change::Arrived(after[0].clone()), Change::Arrived(after[1].clone())]);
    }

    #[test]
    fn scan_of_a_missing_class_is_empty() {
        let found = scan_in(Path::new("/nonexistent/bifrost-sys"), "drm", &["vendor"]);
        assert!(found.is_empty());
    }

    #[test]
    fn scan_reads_trimmed_properties_and_skips_missing_keys() {
        let base = std::env::temp_dir().join(format!("bifrost-devreg-{}-{}", std::process::id(), line!()));
        let _ = fs::remove_dir_all(&base);
        let card1 = base.join("drm").join("card1");
        let card0 = base.join("drm").join("card0");
        fs::create_dir_all(&card1).unwrap();
        fs::create_dir_all(&card0).unwrap();
        fs::write(card0.join("vendor"), "0x8086\n").unwrap();
        fs::write(card0.join("driver"), "i915").unwrap();
        fs::write(card1.join("vendor"), "0x1002").unwrap();

        let found = scan_in(&base, "drm", &["vendor", "driver"]);
        let _ = fs::remove_dir_all(&base);

        let names: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["card0", "card1"]);
        assert_eq!(found[0].property("vendor"), Some("0x8086"));
        assert_eq!(found[0].property("driver"), Some("i915"));
        assert_eq!(found[1].property("vendor"), Some("0x1002"));
        assert_eq!(found[1].property("driver"), None);
    }
}
