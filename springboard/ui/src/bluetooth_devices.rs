//! The Bluetooth device list as a model: which devices are paired, which were found nearby, the link state shown on
//! each row and the order of the rows. Settings and apps build their lists from this one function, so the order and
//! the labels are the same everywhere and can be tested without an adapter.

/// The link state shown beside a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    Connected,
    NotConnected,
}

impl LinkState {
    /// The row value text.
    pub fn label(self) -> &'static str {
        match self {
            LinkState::Connecting => "Connecting…",
            LinkState::Connected => "Connected",
            LinkState::NotConnected => "Not Connected",
        }
    }
}

/// One device as the adapter reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRow {
    /// the address, "AA:BB:CC:DD:EE:FF"; it identifies the device
    pub address: String,
    /// the advertised or stored name; empty when the device has none
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    /// a connection attempt is in flight
    pub connecting: bool,
}

impl DeviceRow {
    /// The name to show: the device name, or the address when the device has none.
    pub fn title(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.address
        } else {
            &self.name
        }
    }

    /// The link state. An attempt in flight wins over a stale connected flag.
    pub fn link_state(&self) -> LinkState {
        if self.connecting {
            LinkState::Connecting
        } else if self.connected {
            LinkState::Connected
        } else {
            LinkState::NotConnected
        }
    }
}

/// The two sections of the list: paired devices first, then devices found nearby that are not paired.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceList {
    pub paired: Vec<DeviceRow>,
    pub found: Vec<DeviceRow>,
}

/// Builds the sections. A device that appears twice is listed once, with the first entry kept. Paired rows sort
/// connected devices first, then by state, then by name ignoring case; found rows sort by name ignoring case.
pub fn device_list(devices: &[DeviceRow]) -> DeviceList {
    let mut seen: Vec<&str> = Vec::new();
    let mut list = DeviceList::default();
    for device in devices {
        if seen.contains(&device.address.as_str()) {
            continue;
        }
        seen.push(&device.address);
        if device.paired {
            list.paired.push(device.clone());
        } else {
            list.found.push(device.clone());
        }
    }
    list.paired.sort_by_key(|d| (rank(d.link_state()), d.title().to_lowercase(), d.address.clone()));
    list.found.sort_by_key(|d| (d.title().to_lowercase(), d.address.clone()));
    list
}

fn rank(state: LinkState) -> u8 {
    match state {
        LinkState::Connecting => 0,
        LinkState::Connected => 1,
        LinkState::NotConnected => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(address: &str, name: &str, paired: bool) -> DeviceRow {
        DeviceRow { address: address.into(), name: name.into(), paired, connected: false, connecting: false }
    }

    #[test]
    fn a_device_without_a_name_is_titled_by_its_address() {
        assert_eq!(row("AA:BB:CC:DD:EE:01", "  ", false).title(), "AA:BB:CC:DD:EE:01");
        assert_eq!(row("AA:BB:CC:DD:EE:01", "Buds", false).title(), "Buds");
    }

    #[test]
    fn an_attempt_in_flight_shows_as_connecting_over_a_connected_flag() {
        let mut d = row("AA:BB:CC:DD:EE:01", "Buds", true);
        d.connected = true;
        d.connecting = true;
        assert_eq!(d.link_state(), LinkState::Connecting);
        assert_eq!(d.link_state().label(), "Connecting…");
        d.connecting = false;
        assert_eq!(d.link_state().label(), "Connected");
        d.connected = false;
        assert_eq!(d.link_state().label(), "Not Connected");
    }

    #[test]
    fn paired_and_found_devices_land_in_their_own_sections() {
        let list = device_list(&[row("01", "Keyboard", true), row("02", "Speaker", false)]);
        assert_eq!(list.paired.len(), 1);
        assert_eq!(list.paired[0].address, "01");
        assert_eq!(list.found.len(), 1);
        assert_eq!(list.found[0].address, "02");
    }

    #[test]
    fn connected_paired_devices_come_before_idle_ones() {
        let mut idle = row("01", "Alpha", true);
        let mut live = row("02", "Zulu", true);
        live.connected = true;
        idle.connected = false;
        let list = device_list(&[idle, live]);
        assert_eq!(list.paired[0].title(), "Zulu");
        assert_eq!(list.paired[1].title(), "Alpha");
    }

    #[test]
    fn names_sort_ignoring_case_inside_each_section() {
        let list = device_list(&[row("01", "bravo", false), row("02", "Alpha", false), row("03", "charlie", true)]);
        let found: Vec<&str> = list.found.iter().map(|d| d.title()).collect();
        assert_eq!(found, vec!["Alpha", "bravo"]);
        assert_eq!(list.paired[0].title(), "charlie");
    }

    #[test]
    fn a_repeated_address_is_listed_once_with_its_first_entry() {
        let mut first = row("01", "Headset", true);
        first.connected = true;
        let second = row("01", "Headset copy", false);
        let list = device_list(&[first, second]);
        assert_eq!(list.paired.len(), 1);
        assert!(list.found.is_empty());
        assert!(list.paired[0].connected);
        assert_eq!(list.paired[0].name, "Headset");
    }

    #[test]
    fn an_empty_adapter_gives_empty_sections() {
        assert_eq!(device_list(&[]), DeviceList::default());
    }
}
