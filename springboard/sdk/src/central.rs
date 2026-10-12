//! Central role for apps: the manager state an app reads, scans with their service filter and duplicate rule, and the
//! connection state of one peripheral. Pure model: the shell reports adapter power through `ManagerState::from_adapter`
//! and `Scanner::set_state`, a transport reports advertisements to `Scanner::offer` and link results to `Link`, and no
//! radio is touched here. The matrix row that uses it is partial: nothing feeds these types yet.

use std::collections::BTreeSet;

use bifrost_core::bt_address::BdAddr;
use bifrost_ui::advertising;
use bifrost_ui::gatt::Uuid;

use crate::ble_error::BleError;

/// Adapter power as the system layer reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterPower {
    Unavailable,
    Off,
    On,
}

/// The state of the central manager, as an app reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ManagerState {
    Unknown,
    Resetting,
    Unsupported,
    Unauthorized,
    PoweredOff,
    PoweredOn,
}

impl ManagerState {
    /// The state for an adapter report. A missing adapter is unsupported, and the other two map one to one.
    pub fn from_adapter(power: AdapterPower) -> ManagerState {
        match power {
            AdapterPower::Unavailable => ManagerState::Unsupported,
            AdapterPower::Off => ManagerState::PoweredOff,
            AdapterPower::On => ManagerState::PoweredOn,
        }
    }

    pub fn is_powered_on(self) -> bool {
        self == ManagerState::PoweredOn
    }
}

/// What a scan asks for: the services an advertisement must list, and whether repeated advertisements are reported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// An advertisement passes when it lists at least one of these. Empty matches every advertisement.
    pub services: Vec<Uuid>,
    /// Report every advertisement, not one per device. Off by default.
    pub allow_duplicates: bool,
}

/// One advertisement that passed the scan filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    pub address: BdAddr,
    pub rssi: i8,
    pub name: Option<String>,
    pub services: Vec<Uuid>,
}

/// A scan for peripherals. It only reports while it is running and the manager is powered on.
#[derive(Debug)]
pub struct Scanner {
    options: ScanOptions,
    state: ManagerState,
    scanning: bool,
    seen: BTreeSet<BdAddr>,
}

impl Scanner {
    /// A scanner that is not running. The manager state starts unknown until the shell reports it.
    pub fn new(options: ScanOptions) -> Scanner {
        Scanner { options, state: ManagerState::Unknown, scanning: false, seen: BTreeSet::new() }
    }

    pub fn state(&self) -> ManagerState {
        self.state
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    /// Records a manager state. Leaving the powered-on state stops a running scan.
    pub fn set_state(&mut self, state: ManagerState) {
        self.state = state;
        if !state.is_powered_on() {
            self.stop();
        }
    }

    /// Starts a scan and forgets earlier devices. Returns false, and does not start, unless the manager is powered on.
    pub fn start(&mut self) -> bool {
        if !self.state.is_powered_on() {
            return false;
        }
        self.scanning = true;
        self.seen.clear();
        true
    }

    /// Stops the scan. Its duplicate record is dropped with it.
    pub fn stop(&mut self) {
        self.scanning = false;
        self.seen.clear();
    }

    /// Offers one advertisement to the scan. Returns the discovery when the scan is running, the advertisement lists
    /// one of the requested services (or none were requested), and the device has not been reported in this scan
    /// (unless duplicates are allowed).
    pub fn offer(&mut self, address: BdAddr, rssi: i8, advertisement: &[u8]) -> Option<Discovery> {
        if !self.scanning {
            return None;
        }
        let services = advertising::service_uuids(advertisement);
        if !self.options.services.is_empty() && !services.iter().any(|s| self.options.services.contains(s)) {
            return None;
        }
        if !self.options.allow_duplicates && !self.seen.insert(address) {
            return None;
        }
        Some(Discovery { address, rssi, name: advertising::local_name(advertisement), services })
    }
}

/// Connection state of one peripheral.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

/// The connection to one peripheral and the reason for its last failure. Each method returns false when the call
/// does not apply in the current state, and then changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    state: LinkState,
    last_error: Option<BleError>,
}

impl Default for Link {
    fn default() -> Self {
        Link::new()
    }
}

impl Link {
    pub fn new() -> Link {
        Link { state: LinkState::Disconnected, last_error: None }
    }

    pub fn state(&self) -> LinkState {
        self.state
    }

    /// The reason the link last went down, or the last attempt failed. Starting a new attempt clears it.
    pub fn last_error(&self) -> Option<&BleError> {
        self.last_error.as_ref()
    }

    /// Starts a connection. Only a disconnected link can start one.
    pub fn connect(&mut self) -> bool {
        if self.state != LinkState::Disconnected {
            return false;
        }
        self.state = LinkState::Connecting;
        self.last_error = None;
        true
    }

    pub fn on_connected(&mut self) -> bool {
        if self.state != LinkState::Connecting {
            return false;
        }
        self.state = LinkState::Connected;
        true
    }

    /// The attempt failed: the link is disconnected and the reason is kept.
    pub fn on_failed(&mut self, error: BleError) -> bool {
        if self.state != LinkState::Connecting {
            return false;
        }
        self.state = LinkState::Disconnected;
        self.last_error = Some(error);
        true
    }

    /// Cancels a pending attempt, which records `OperationCancelled`, or starts closing a live connection.
    pub fn cancel(&mut self) -> bool {
        match self.state {
            LinkState::Connecting => {
                self.state = LinkState::Disconnected;
                self.last_error = Some(BleError::OperationCancelled);
                true
            }
            LinkState::Connected => {
                self.state = LinkState::Disconnecting;
                true
            }
            _ => false,
        }
    }

    /// The link went down while connected or closing. `reason` is `None` for a clean disconnect.
    pub fn on_disconnected(&mut self, reason: Option<BleError>) -> bool {
        if !matches!(self.state, LinkState::Connected | LinkState::Disconnecting) {
            return false;
        }
        self.state = LinkState::Disconnected;
        self.last_error = reason;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(last: u8) -> BdAddr {
        BdAddr([0, 0, 0, 0, 0, last])
    }

    /// A complete local name AD structure: length 4, type 0x09, "Box".
    const NAMED: [u8; 5] = [4, 0x09, b'B', b'o', b'x'];

    fn heart_rate_advert() -> Vec<u8> {
        // Complete list of 16-bit UUIDs: length 3, type 0x03, 0x180D (little endian).
        vec![3, 0x03, 0x0D, 0x18]
    }

    fn battery_advert() -> Vec<u8> {
        // Complete list of 16-bit UUIDs: length 3, type 0x03, 0x180F (little endian).
        vec![3, 0x03, 0x0F, 0x18]
    }

    #[test]
    fn adapter_power_maps_to_the_manager_state() {
        assert_eq!(ManagerState::from_adapter(AdapterPower::Unavailable), ManagerState::Unsupported);
        assert_eq!(ManagerState::from_adapter(AdapterPower::Off), ManagerState::PoweredOff);
        assert_eq!(ManagerState::from_adapter(AdapterPower::On), ManagerState::PoweredOn);
        assert!(ManagerState::PoweredOn.is_powered_on());
        assert!(!ManagerState::Resetting.is_powered_on());
    }

    #[test]
    fn a_scan_does_not_start_until_the_manager_is_powered_on() {
        let mut s = Scanner::new(ScanOptions::default());
        assert_eq!(s.state(), ManagerState::Unknown);
        assert!(!s.start());
        assert!(!s.is_scanning());
        s.set_state(ManagerState::PoweredOn);
        assert!(s.start());
        assert!(s.is_scanning());
    }

    #[test]
    fn a_device_is_reported_once_per_scan_unless_duplicates_are_allowed() {
        let mut s = Scanner::new(ScanOptions::default());
        s.set_state(ManagerState::PoweredOn);
        s.start();
        let first = s.offer(addr(1), -60, &NAMED).expect("first advertisement");
        assert_eq!(first.name.as_deref(), Some("Box"));
        assert_eq!(first.rssi, -60);
        assert!(s.offer(addr(1), -58, &NAMED).is_none());
        assert!(s.offer(addr(2), -70, &NAMED).is_some());
        // A new scan forgets the devices it has seen.
        s.stop();
        s.start();
        assert!(s.offer(addr(1), -60, &NAMED).is_some());
    }

    #[test]
    fn duplicates_are_reported_when_the_scan_allows_them() {
        let mut s = Scanner::new(ScanOptions { services: Vec::new(), allow_duplicates: true });
        s.set_state(ManagerState::PoweredOn);
        s.start();
        assert!(s.offer(addr(7), -50, &NAMED).is_some());
        assert!(s.offer(addr(7), -51, &NAMED).is_some());
    }

    #[test]
    fn a_service_filter_keeps_only_advertisements_that_list_a_requested_service() {
        let hr = Uuid::from_16(0x180D);
        let mut s = Scanner::new(ScanOptions { services: vec![hr], allow_duplicates: false });
        s.set_state(ManagerState::PoweredOn);
        s.start();
        assert!(s.offer(addr(3), -40, &battery_advert()).is_none());
        let hit = s.offer(addr(4), -41, &heart_rate_advert()).expect("heart rate device");
        assert_eq!(hit.services, vec![hr]);
        assert_eq!(hit.name, None);
    }

    #[test]
    fn leaving_the_powered_on_state_stops_the_scan() {
        let mut s = Scanner::new(ScanOptions::default());
        s.set_state(ManagerState::PoweredOn);
        s.start();
        s.set_state(ManagerState::PoweredOff);
        assert!(!s.is_scanning());
        assert!(s.offer(addr(9), -60, &NAMED).is_none());
        assert!(!s.start());
    }

    #[test]
    fn a_link_connects_and_disconnects_in_order() {
        let mut l = Link::new();
        assert!(l.connect());
        assert_eq!(l.state(), LinkState::Connecting);
        assert!(!l.connect(), "a pending link cannot start a second attempt");
        assert!(l.on_connected());
        assert!(!l.connect(), "a connected link cannot connect again");
        assert!(l.cancel());
        assert_eq!(l.state(), LinkState::Disconnecting);
        assert!(l.on_disconnected(None));
        assert_eq!(l.state(), LinkState::Disconnected);
        assert_eq!(l.last_error(), None);
    }

    #[test]
    fn a_failed_attempt_keeps_its_reason_until_the_next_attempt() {
        let mut l = Link::new();
        assert!(l.connect());
        assert!(l.on_failed(BleError::ConnectionTimeout));
        assert_eq!(l.state(), LinkState::Disconnected);
        assert_eq!(l.last_error(), Some(&BleError::ConnectionTimeout));
        assert!(l.connect());
        assert_eq!(l.last_error(), None);
    }

    #[test]
    fn cancelling_a_pending_attempt_records_an_operation_cancelled_error() {
        let mut l = Link::new();
        assert!(!l.cancel(), "nothing to cancel while disconnected");
        assert!(l.connect());
        assert!(l.cancel());
        assert_eq!(l.state(), LinkState::Disconnected);
        assert_eq!(l.last_error(), Some(&BleError::OperationCancelled));
        assert!(!l.on_connected(), "a cancelled attempt cannot connect");
    }
}
