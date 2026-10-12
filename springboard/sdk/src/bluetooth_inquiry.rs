//! Device inquiry (scan) as an app drives it: start a search for a length of time, receive a `Found` event the first
//! time each device answers, read the results, and end the search early with `stop`. Every device found in one search
//! is kept in the order it was first heard, and a later search adds to that list until `clear_found` empties it.
//!
//! `Inquiry` is the model: the caller feeds it the radio's reports and the elapsed seconds, and it returns the events
//! an app's delegate would receive. It opens no radio and holds no timer. Nothing feeds it yet, so the matrix row that
//! uses it is partial. Pure.

use bifrost_core::bt_address::{BdAddr, ClassOfDevice};

use crate::bluetooth_devices::RemoteDevice;

/// Shortest inquiry the model accepts, in seconds.
pub const MIN_LENGTH_SECS: u32 = 1;
/// Longest inquiry the model accepts, in seconds.
pub const MAX_LENGTH_SECS: u32 = 61;

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
