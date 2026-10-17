//! The Locate window's model: the sidebar sections, the device list the cloud returned, the first-run onboarding and the
//! Share My Location switch. It draws nothing and sends nothing: the window reads it, and the cloud client fills it
//! through `apply_list`. Position and battery come from the device model (`devices`), so a stale fix never replaces a
//! newer one.

use crate::devices::{Device, Devices, Fix};
use bifrost_app::cloud_findmy::{CloudFix, DeviceList};

/// The sidebar sections, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    People,
    Devices,
    Items,
    Me,
}

impl Section {
    pub const ALL: [Section; 4] = [Section::People, Section::Devices, Section::Items, Section::Me];

    pub fn title(self) -> &'static str {
        match self {
            Section::People => "People",
            Section::Devices => "Devices",
            Section::Items => "Items",
            Section::Me => "Me",
        }
    }
}

/// What the first-run sheet says. It names what leaves this machine: the position and the battery level.
pub const ONBOARDING_LINES: [&str; 3] = [
    "Locate shows where your BifrOSt devices are. Each device sends its last position and battery level to your BifrOSt Account.",
    "Your position is seen only by your own devices and by the contacts you choose under Share My Location. Nothing is shared until you turn sharing on.",
    "Play Sound and Locate are sent only when you press their buttons. Erase asks you to confirm first.",
];

/// The location state shown on the Me page when the shell has no location service.
pub const LOCATION_UNAVAILABLE: &str = "Location services unavailable";

/// One line of the device list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub battery: Option<String>,
    pub located: bool,
    pub is_this_device: bool,
}

#[derive(Debug)]
pub struct Panel {
    pub section: Section,
    onboarded: bool,
    share_on: bool,
    this_device: Option<String>,
    devices: Devices,
    /// The shell's location authorization, as text. None when the shell has no location service.
    pub location_status: Option<String>,
}

impl Panel {
    pub fn new(onboarded: bool) -> Panel {
        Panel {
            section: Section::Devices,
            onboarded,
            share_on: false,
            this_device: None,
            devices: Devices::new(Vec::new()),
            location_status: None,
        }
    }

    pub fn needs_onboarding(&self) -> bool {
        !self.onboarded
    }

    /// The first-run sheet was dismissed: it is not shown again.
    pub fn finish_onboarding(&mut self) {
        self.onboarded = true;
    }

    pub fn select(&mut self, section: Section) {
        self.section = section;
    }

    pub fn sharing(&self) -> bool {
        self.share_on
    }

    /// Turns Share My Location on or off. Returns whether the state changed.
    pub fn set_sharing(&mut self, on: bool) -> bool {
        let changed = self.share_on != on;
        self.share_on = on;
        changed
    }

    /// The location text for the Me page.
    pub fn location_text(&self) -> &str {
        self.location_status.as_deref().unwrap_or(LOCATION_UNAVAILABLE)
    }

    /// Replaces the device list with the cloud's answer. Each device keeps its newest fix and battery level.
    pub fn apply_list(&mut self, list: &DeviceList) {
        let devices = list.devices.iter().map(|c| {
            let mut d = Device::new(&c.id, &c.name);
            if let Some(p) = c.battery_percent {
                d.set_battery(p);
            }
            if let Some(f) = c.fix {
                d.report(fix_of(f));
            }
            d
        });
        self.devices = Devices::new(devices.collect());
        self.this_device = list.this_device.clone();
    }

    /// The device list rows, this machine first when the cloud named it.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .devices
            .list()
            .iter()
            .map(|d| Row {
                id: d.id.clone(),
                name: d.name.clone(),
                battery: d.battery_label(),
                located: d.position.is_some(),
                is_this_device: self.this_device.as_deref() == Some(d.id.as_str()),
            })
            .collect();
        rows.sort_by_key(|r| !r.is_this_device);
        rows
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.get(id)
    }
}

fn fix_of(f: CloudFix) -> Fix {
    Fix {
        lat: f.latitude,
        lon: f.longitude,
        // An unknown accuracy is infinitely loose, so the fix is still kept but never called precise.
        accuracy_m: f.accuracy_m.unwrap_or(f64::INFINITY),
        at_unix: f.recorded_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::cloud_findmy::CloudDevice;

    fn cloud(id: &str, name: &str, battery: Option<u8>, fix: Option<CloudFix>) -> CloudDevice {
        CloudDevice { id: id.into(), name: name.into(), model: String::new(), last_seen: 0, battery_percent: battery, is_charging: None, fix }
    }

    fn fix(at: i64) -> CloudFix {
        CloudFix { latitude: 64.1, longitude: -21.9, accuracy_m: Some(10.0), recorded_at: at }
    }

    #[test]
    fn device_list_shows_this_machine_first_with_battery_and_position() {
        let mut p = Panel::new(true);
        p.apply_list(&DeviceList {
            devices: vec![cloud("d2", "Tab", None, None), cloud("d1", "Midget", Some(82), Some(fix(5)))],
            this_device: Some("d1".into()),
        });
        let rows = p.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "d1");
        assert!(rows[0].is_this_device);
        assert_eq!(rows[0].battery.as_deref(), Some("Battery 82%"));
        assert!(rows[0].located);
        assert_eq!(rows[1].battery, None);
        assert!(!rows[1].located);
    }

    #[test]
    fn the_answers_fix_becomes_the_device_position() {
        let mut p = Panel::new(true);
        p.apply_list(&DeviceList { devices: vec![cloud("d1", "Midget", None, Some(fix(10)))], this_device: None });
        let held = p.device("d1").unwrap().position.unwrap();
        assert_eq!(held.at_unix, 10);
        assert_eq!(held.accuracy_m, 10.0);
    }

    #[test]
    fn an_unknown_accuracy_is_kept_as_infinitely_loose() {
        let mut f = fix(7);
        f.accuracy_m = None;
        assert!(fix_of(f).accuracy_m.is_infinite());
    }

    #[test]
    fn onboarding_is_shown_once_until_dismissed() {
        let mut p = Panel::new(false);
        assert!(p.needs_onboarding());
        p.finish_onboarding();
        assert!(!p.needs_onboarding());
        assert!(!Panel::new(true).needs_onboarding());
        assert_eq!(ONBOARDING_LINES.len(), 3);
    }

    #[test]
    fn share_toggle_reports_only_real_changes() {
        let mut p = Panel::new(true);
        assert!(!p.sharing());
        assert!(p.set_sharing(true));
        assert!(!p.set_sharing(true), "turning on twice is no change");
        assert!(p.sharing());
        assert!(p.set_sharing(false));
    }

    #[test]
    fn location_text_says_unavailable_without_a_location_service() {
        let mut p = Panel::new(true);
        assert_eq!(p.location_text(), "Location services unavailable");
        p.location_status = Some("Allowed when in use".into());
        assert_eq!(p.location_text(), "Allowed when in use");
    }

    #[test]
    fn sections_are_in_sidebar_order() {
        let titles: Vec<&str> = Section::ALL.iter().map(|s| s.title()).collect();
        assert_eq!(titles, ["People", "Devices", "Items", "Me"]);
    }
}
