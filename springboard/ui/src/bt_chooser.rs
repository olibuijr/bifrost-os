//! Device chooser: the model behind a panel that lists nearby and paired Bluetooth devices and returns the
//! ones the user picked. It follows the shape of the reference device-selector controller: an allowed-UUID
//! filter (`addAllowedUUID`, `clearAllowedUUIDs`), search text, two sections ("My Devices" and "Other
//! Devices"), and results that only exist once the user has confirmed. Renderer-independent: the shell
//! draws the panel and feeds it the device list.
//!
//! Selection is single by default. With `multiple` on, a second pick adds to the selection and a pick of an
//! already selected device removes it; the results are then a list, as the reference returns an array.

/// One device as the chooser shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooserDevice {
    /// "AA:BB:CC:DD:EE:FF", the identity of the device.
    pub addr: String,
    pub name: String,
    /// Bonded with this machine: listed under "My Devices".
    pub paired: bool,
    /// Something that can carry sound (headphones, speaker, car kit).
    pub audio: bool,
    /// Service UUIDs the device advertises, 16-bit form.
    pub uuids: Vec<u16>,
}

/// Which devices the chooser accepts. Empty allowed UUIDs accept every device; otherwise a device must
/// advertise at least one of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceFilter {
    allowed: Vec<u16>,
    /// Hide devices that cannot carry sound.
    pub audio_only: bool,
    /// Case-insensitive text matched against the device name. Blank matches everything.
    pub query: String,
}

impl DeviceFilter {
    /// Adds a UUID to the allowed list. A UUID already in the list is not added twice.
    pub fn add_allowed_uuid(&mut self, uuid: u16) {
        if !self.allowed.contains(&uuid) {
            self.allowed.push(uuid);
        }
    }

    pub fn add_allowed_uuids(&mut self, uuids: &[u16]) {
        for uuid in uuids {
            self.add_allowed_uuid(*uuid);
        }
    }

    /// Back to accepting any device.
    pub fn clear_allowed_uuids(&mut self) {
        self.allowed.clear();
    }

    pub fn allowed_uuids(&self) -> &[u16] {
        &self.allowed
    }

    pub fn accepts(&self, device: &ChooserDevice) -> bool {
        if self.audio_only && !device.audio {
            return false;
        }
        if !self.allowed.is_empty() && !device.uuids.iter().any(|u| self.allowed.contains(u)) {
            return false;
        }
        let query = self.query.trim().to_lowercase();
        query.is_empty() || device.name.to_lowercase().contains(&query)
    }
}

/// The visible devices, split into the two sections. Each section is sorted by name, ignoring case, and
/// then by address, so the order does not depend on discovery order.
#[derive(Debug, PartialEq, Eq)]
pub struct Sections<'a> {
    pub my_devices: Vec<&'a ChooserDevice>,
    pub other_devices: Vec<&'a ChooserDevice>,
}

pub fn sections<'a>(devices: &'a [ChooserDevice], filter: &DeviceFilter) -> Sections<'a> {
    let mut my_devices = Vec::new();
    let mut other_devices = Vec::new();
    for device in devices.iter().filter(|d| filter.accepts(d)) {
        if device.paired {
            my_devices.push(device);
        } else {
            other_devices.push(device);
        }
    }
    let order = |a: &&ChooserDevice, b: &&ChooserDevice| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.addr.cmp(&b.addr))
    };
    my_devices.sort_by(order);
    other_devices.sort_by(order);
    Sections { my_devices, other_devices }
}

/// The panel's state: the filter, the labels, the selection and whether the user confirmed it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chooser {
    pub filter: DeviceFilter,
    /// Allow more than one device to be picked.
    pub multiple: bool,
    /// Window title (shown when the panel is not a sheet).
    pub title: String,
    /// Text above the list.
    pub header: String,
    /// Text under the header.
    pub description: String,
    /// Title of the default button (the confirm action).
    pub prompt: String,
    /// Title of the cancel button.
    pub cancel: String,
    selected: Vec<String>,
    confirmed: bool,
}

impl Chooser {
    pub fn new(multiple: bool) -> Self {
        Chooser { multiple, ..Chooser::default() }
    }

    /// The user picked a device. Single mode replaces the selection. Multiple mode toggles the device.
    pub fn select(&mut self, addr: &str) {
        if !self.multiple {
            self.selected = vec![addr.to_string()];
        } else if let Some(at) = self.selected.iter().position(|a| a == addr) {
            self.selected.remove(at);
        } else {
            self.selected.push(addr.to_string());
        }
    }

    pub fn selected(&self) -> &[String] {
        &self.selected
    }

    /// The default button is enabled only with something picked.
    pub fn can_confirm(&self) -> bool {
        !self.selected.is_empty()
    }

    /// Confirms the selection. Returns false, and changes nothing, when nothing is picked.
    pub fn confirm(&mut self) -> bool {
        if self.can_confirm() {
            self.confirmed = true;
        }
        self.confirmed
    }

    /// Closes without a result: the selection is cleared and `results` returns nothing.
    pub fn cancel(&mut self) {
        self.selected.clear();
        self.confirmed = false;
    }

    /// The picked addresses once the user has confirmed, otherwise nothing. This is the value the
    /// caller reads after the panel closes.
    pub fn results(&self) -> Option<&[String]> {
        if self.confirmed {
            Some(&self.selected)
        } else {
            None
        }
    }

    /// Drops picked devices that are gone or filtered out since the last list update.
    pub fn retain_visible(&mut self, devices: &[ChooserDevice]) {
        let visible = |addr: &String| devices.iter().any(|d| &d.addr == addr && self.filter.accepts(d));
        let keep: Vec<String> = self.selected.iter().filter(|a| visible(*a)).cloned().collect();
        self.selected = keep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(addr: &str, name: &str, paired: bool, audio: bool, uuids: &[u16]) -> ChooserDevice {
        ChooserDevice { addr: addr.into(), name: name.into(), paired, audio, uuids: uuids.to_vec() }
    }

    #[test]
    fn an_advertised_uuid_must_be_in_the_allowed_list() {
        let headset = dev("A", "Buds", false, true, &[0x110B]);
        let mut f = DeviceFilter::default();
        assert!(f.accepts(&headset), "no allowed UUIDs accepts every device");
        f.add_allowed_uuid(0x1124);
        assert!(!f.accepts(&headset), "0x1124 is not advertised by the headset");
        f.add_allowed_uuid(0x110B);
        assert!(f.accepts(&headset));
    }

    #[test]
    fn adding_a_uuid_twice_keeps_one_entry_and_clear_resets() {
        let mut f = DeviceFilter::default();
        f.add_allowed_uuids(&[0x110B, 0x110B, 0x1124]);
        assert_eq!(f.allowed_uuids(), &[0x110B, 0x1124]);
        f.clear_allowed_uuids();
        assert!(f.allowed_uuids().is_empty());
    }

    #[test]
    fn audio_only_and_the_name_query_narrow_the_list() {
        let buds = dev("A", "Buds Pro", true, true, &[]);
        let keyboard = dev("B", "Keyboard", true, false, &[]);
        let mut f = DeviceFilter { audio_only: true, ..DeviceFilter::default() };
        assert!(f.accepts(&buds));
        assert!(!f.accepts(&keyboard), "a keyboard cannot carry sound");
        f.audio_only = false;
        f.query = "  BUDS ".into();
        assert!(f.accepts(&buds), "the query ignores case and surrounding spaces");
        assert!(!f.accepts(&keyboard));
    }

    #[test]
    fn paired_devices_come_first_in_their_own_section_sorted_by_name() {
        let devices = vec![
            dev("1", "speaker", true, true, &[]),
            dev("2", "Keyboard", false, false, &[]),
            dev("3", "Buds", true, true, &[]),
            dev("4", "alpha", false, true, &[]),
        ];
        let s = sections(&devices, &DeviceFilter::default());
        let mine: Vec<&str> = s.my_devices.iter().map(|d| d.name.as_str()).collect();
        let other: Vec<&str> = s.other_devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(mine, ["Buds", "speaker"]);
        assert_eq!(other, ["alpha", "Keyboard"]);
    }

    #[test]
    fn single_selection_replaces_and_confirm_yields_the_result() {
        let mut c = Chooser::new(false);
        c.select("A");
        c.select("B");
        assert_eq!(c.selected(), ["B"]);
        assert_eq!(c.results(), None, "no result before the user confirms");
        assert!(c.confirm());
        assert_eq!(c.results(), Some(&["B".to_string()][..]));
    }

    #[test]
    fn multiple_selection_toggles_and_confirm_needs_a_pick() {
        let mut c = Chooser::new(true);
        assert!(!c.confirm(), "nothing picked, nothing to confirm");
        assert_eq!(c.results(), None);
        c.select("A");
        c.select("B");
        c.select("A");
        assert_eq!(c.selected(), ["B"]);
        c.cancel();
        assert!(c.selected().is_empty());
        assert_eq!(c.results(), None, "cancel gives no result");
    }

    #[test]
    fn retain_visible_drops_devices_that_disappeared_or_were_filtered_out() {
        let devices = vec![dev("A", "Buds", false, true, &[0x110B]), dev("B", "Speaker", false, true, &[0x1124])];
        let mut c = Chooser::new(true);
        c.select("A");
        c.select("B");
        c.filter.add_allowed_uuid(0x110B);
        c.retain_visible(&devices);
        assert_eq!(c.selected(), ["A"], "B no longer passes the UUID filter");
        c.retain_visible(&[]);
        assert!(c.selected().is_empty(), "A is gone from the list");
    }
}
