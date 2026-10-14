//! Device inquiry (scan) as an app drives it: start a search for a length of time, receive a `Found` event the first
//! time each device answers, read the results, and end the search early with `stop`. Every device found in one search
//! is kept in the order it was first heard, and a later search adds to that list until `clear_found` empties it.
//!
//! `Inquiry` is the model: the caller feeds it the radio's reports and the elapsed seconds, and it returns the events
//! an app's delegate would receive. It opens no radio and holds no timer. The reports arrive as the controller's
//! `Device Found` event payload, which [`parse_device_found`] reads into a [`Report`]; [`Inquiry::feed`] takes such a
//! payload directly. The socket that delivers the payload is the shell's job, so the matrix row that uses this model
//! is partial. Pure.

use bifrost_core::bt_address::{BdAddr, ClassOfDevice};

use crate::bluetooth_devices::RemoteDevice;

/// Shortest inquiry the model accepts, in seconds.
pub const MIN_LENGTH_SECS: u32 = 1;
/// Longest inquiry the model accepts, in seconds.
pub const MAX_LENGTH_SECS: u32 = 61;

/// Bytes of the `Device Found` payload before the extended inquiry data: address (6), address type (1), RSSI (1),
/// flags (4) and the extended data length (2).
const DEVICE_FOUND_HEADER: usize = 14;
/// RSSI value the controller sends when it has no measurement.
const RSSI_UNAVAILABLE: i8 = 127;
/// Extended inquiry data field types used here: shortened and complete local name, and class of device.
const EIR_NAME_SHORT: u8 = 0x08;
const EIR_NAME_COMPLETE: u8 = 0x09;
const EIR_CLASS_OF_DEVICE: u8 = 0x0D;

/// One report from the radio: what a single `Device Found` event says about one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub address: BdAddr,
    pub class: Option<ClassOfDevice>,
    pub name: Option<String>,
    pub rssi: Option<i8>,
}

/// Why a `Device Found` payload could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The payload is shorter than its header, or its extended data is shorter than the length it declares.
    Truncated,
}

/// Reads a `Device Found` event payload (the controller's management event, without its own header).
///
/// The address arrives least significant byte first and is returned in printing order. The extended inquiry data is
/// a list of fields, each one a length byte followed by a type byte and its value. A zero length ends the list early.
/// The name is the complete local name when the device sent one, and the shortened name otherwise. An RSSI of 127
/// means the controller had no measurement, and the report then carries no RSSI.
pub fn parse_device_found(payload: &[u8]) -> Result<Report, ParseError> {
    if payload.len() < DEVICE_FOUND_HEADER {
        return Err(ParseError::Truncated);
    }
    let mut address = [0u8; 6];
    for (i, byte) in payload[..6].iter().enumerate() {
        address[5 - i] = *byte;
    }
    let rssi = payload[7] as i8;
    let eir_len = u16::from_le_bytes([payload[12], payload[13]]) as usize;
    let eir = payload.get(DEVICE_FOUND_HEADER..DEVICE_FOUND_HEADER + eir_len).ok_or(ParseError::Truncated)?;

    let mut class = None;
    let mut complete_name = None;
    let mut short_name = None;
    let mut at = 0;
    while at < eir.len() && eir[at] != 0 {
        let field_len = eir[at] as usize;
        let field = eir.get(at + 1..at + 1 + field_len).ok_or(ParseError::Truncated)?;
        if let Some((&kind, value)) = field.split_first() {
            match kind {
                EIR_NAME_COMPLETE => complete_name = Some(name_text(value)),
                EIR_NAME_SHORT => short_name = Some(name_text(value)),
                EIR_CLASS_OF_DEVICE if value.len() == 3 => {
                    let raw = u32::from_le_bytes([value[0], value[1], value[2], 0]);
                    class = ClassOfDevice::new(raw);
                }
                _ => {}
            }
        }
        at += field_len + 1;
    }
    let name = complete_name.or(short_name).filter(|n| !n.is_empty());
    let rssi = (rssi != RSSI_UNAVAILABLE).then_some(rssi);
    Ok(Report { address: BdAddr(address), class, name, rssi })
}

/// Text of a name field: UTF-8 with the trailing NUL padding removed.
fn name_text(value: &[u8]) -> String {
    String::from_utf8_lossy(value).trim_end_matches('\0').to_string()
}

/// What the delegate of an inquiry receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The search began.
    Started,
    /// A device answered for the first time in this search.
    Found(BdAddr),
    /// The search ended. `aborted` is true when `stop` ended it before its length ran out.
    Complete { aborted: bool, found: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Running,
}

/// One inquiry object: its length, its state and the devices found so far.
#[derive(Debug)]
pub struct Inquiry {
    length_secs: u32,
    state: State,
    elapsed_secs: u32,
    found: Vec<RemoteDevice>,
}

impl Inquiry {
    /// A new, idle inquiry. The length is clamped to `MIN_LENGTH_SECS..=MAX_LENGTH_SECS`.
    pub fn new(length_secs: u32) -> Self {
        Inquiry {
            length_secs: length_secs.clamp(MIN_LENGTH_SECS, MAX_LENGTH_SECS),
            state: State::Idle,
            elapsed_secs: 0,
            found: Vec::new(),
        }
    }

    pub fn length_secs(&self) -> u32 {
        self.length_secs
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }

    /// Begins a search. A search that is already running is not restarted, and the call returns no events.
    pub fn start(&mut self) -> Vec<Event> {
        if self.is_running() {
            return Vec::new();
        }
        self.state = State::Running;
        self.elapsed_secs = 0;
        vec![Event::Started]
    }

    /// Takes one report from the radio. Returns `Found` when the device is new to this inquiry. A repeat report merges
    /// into the stored device and returns nothing. Reports that arrive while the inquiry is idle are ignored.
    pub fn report(&mut self, address: BdAddr, class: Option<ClassOfDevice>, name: Option<&str>, rssi: Option<i8>) -> Option<Event> {
        if !self.is_running() {
            return None;
        }
        if let Some(dev) = self.found.iter_mut().find(|d| d.address == address) {
            dev.merge(class, name, rssi);
            return None;
        }
        let mut dev = RemoteDevice::new(address);
        dev.merge(class, name, rssi);
        self.found.push(dev);
        Some(Event::Found(address))
    }

    /// Takes one `Device Found` payload from the controller, as [`parse_device_found`] reads it, and returns the same
    /// event that [`Inquiry::report`] would. A payload that cannot be read is an error and changes nothing.
    pub fn feed(&mut self, payload: &[u8]) -> Result<Option<Event>, ParseError> {
        let report = parse_device_found(payload)?;
        Ok(self.report(report.address, report.class, report.name.as_deref(), report.rssi))
    }

    /// Ends a running search before its length. Returns `Complete` with `aborted` set, or nothing when idle.
    pub fn stop(&mut self) -> Vec<Event> {
        if !self.is_running() {
            return Vec::new();
        }
        self.state = State::Idle;
        vec![Event::Complete { aborted: true, found: self.found.len() }]
    }

    /// Adds `secs` of elapsed time to a running search. When the total reaches the length, the search ends and
    /// `Complete` is returned. Time passed while idle is ignored.
    pub fn elapse(&mut self, secs: u32) -> Vec<Event> {
        if !self.is_running() {
            return Vec::new();
        }
        self.elapsed_secs = self.elapsed_secs.saturating_add(secs);
        if self.elapsed_secs < self.length_secs {
            return Vec::new();
        }
        self.state = State::Idle;
        vec![Event::Complete { aborted: false, found: self.found.len() }]
    }

    /// The devices found so far, in the order they were first heard.
    pub fn found(&self) -> &[RemoteDevice] {
        &self.found
    }

    /// Empties the list of found devices.
    pub fn clear_found(&mut self) {
        self.found.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> BdAddr {
        BdAddr([0, 0, 0, 0, 0, n])
    }

    #[test]
    fn the_length_is_clamped_to_the_accepted_range() {
        assert_eq!(Inquiry::new(0).length_secs(), 1);
        assert_eq!(Inquiry::new(10).length_secs(), 10);
        assert_eq!(Inquiry::new(500).length_secs(), 61);
    }

    #[test]
    fn start_emits_started_once_and_a_second_start_is_refused() {
        let mut inq = Inquiry::new(10);
        assert_eq!(inq.start(), vec![Event::Started]);
        assert!(inq.is_running());
        assert!(inq.start().is_empty());
    }

    #[test]
    fn reports_while_idle_are_ignored() {
        let mut inq = Inquiry::new(10);
        assert_eq!(inq.report(addr(1), None, Some("Mouse"), Some(-60)), None);
        assert!(inq.found().is_empty());
    }

    #[test]
    fn a_device_is_found_once_and_its_later_reports_merge() {
        let mut inq = Inquiry::new(10);
        inq.start();
        let class = ClassOfDevice::new(0x002580);
        assert_eq!(inq.report(addr(1), class, Some("Mouse"), Some(-60)), Some(Event::Found(addr(1))));
        assert_eq!(inq.report(addr(1), None, None, Some(-50)), None);
        assert_eq!(inq.found().len(), 1);
        let d = &inq.found()[0];
        assert_eq!(d.name.as_deref(), Some("Mouse"));
        assert_eq!(d.rssi, Some(-50));
        assert_eq!(d.class, class);
    }

    #[test]
    fn the_search_ends_when_the_elapsed_time_reaches_the_length() {
        let mut inq = Inquiry::new(10);
        inq.start();
        inq.report(addr(2), None, None, None);
        assert!(inq.elapse(4).is_empty());
        assert!(inq.is_running());
        assert_eq!(inq.elapse(6), vec![Event::Complete { aborted: false, found: 1 }]);
        assert_eq!(inq.state(), State::Idle);
    }

    #[test]
    fn stop_ends_early_and_keeps_the_results_until_they_are_cleared() {
        let mut inq = Inquiry::new(30);
        inq.start();
        inq.report(addr(3), None, None, None);
        inq.report(addr(4), None, None, None);
        assert_eq!(inq.stop(), vec![Event::Complete { aborted: true, found: 2 }]);
        assert_eq!(inq.found().len(), 2);
        assert!(inq.stop().is_empty());
        inq.clear_found();
        assert!(inq.found().is_empty());
    }

    #[test]
    fn a_huge_elapsed_value_ends_the_search_without_overflow() {
        let mut inq = Inquiry::new(10);
        inq.start();
        assert_eq!(inq.elapse(u32::MAX), vec![Event::Complete { aborted: false, found: 0 }]);
        assert!(inq.elapse(1).is_empty());
    }
}
