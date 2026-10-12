//! Wi-Fi change events for apps: the typed changes between two snapshots of the link, in the order a delegate
//! would hear them (network name, link, signal, scan results).
//!
//! The shell owns the supplicant and sends one status snapshot on every change. [`diff`] turns two snapshots into
//! events and [`Watch`] keeps the last snapshot so an app can feed each new one in. Pure: no service call is made
//! here. Signal changes smaller than [`RSSI_STEP`] dBm are not reported, so a noisy reading does not fire on every
//! poll.

/// The smallest signal change, in dBm, that is reported as [`Event::RssiChanged`].
pub const RSSI_STEP: u32 = 2;

/// The Wi-Fi state an app can observe. `scan` holds the network names of the last scan, in scan order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// the link is associated and has an address
    pub link_up: bool,
    /// the joined network's name
    pub ssid: Option<String>,
    /// signal of the joined network, dBm
    pub rssi: Option<i32>,
    /// the names of the networks in the last scan, in scan order
    pub scan: Vec<String>,
}

/// One change between two snapshots.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// the joined network's name changed; `None` when nothing is joined
    SsidChanged { from: Option<String>, to: Option<String> },
    /// the link came up (`true`) or went down (`false`)
    LinkChanged(bool),
    /// the signal moved by at least [`RSSI_STEP`] dBm, or appeared or disappeared
    RssiChanged(Option<i32>),
    /// the scan results differ from the previous scan
    ScanCacheUpdated,
}

/// The events between `prev` and `next`, in the order name, link, signal, scan. Empty when nothing changed.
pub fn diff(prev: &Snapshot, next: &Snapshot) -> Vec<Event> {
    let mut out = Vec::new();
    if prev.ssid != next.ssid {
        out.push(Event::SsidChanged { from: prev.ssid.clone(), to: next.ssid.clone() });
    }
    if prev.link_up != next.link_up {
        out.push(Event::LinkChanged(next.link_up));
    }
    if rssi_moved(prev.rssi, next.rssi) {
        out.push(Event::RssiChanged(next.rssi));
    }
    if prev.scan != next.scan {
        out.push(Event::ScanCacheUpdated);
    }
    out
}

fn rssi_moved(a: Option<i32>, b: Option<i32>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x.abs_diff(y) >= RSSI_STEP,
        (None, None) => false,
        _ => true,
    }
}

/// Keeps the last snapshot and reports what changed on each new one. A new watch starts from an empty snapshot,
/// so the first update reports everything that is set.
#[derive(Debug, Default)]
pub struct Watch {
    last: Snapshot,
}

impl Watch {
    pub fn new() -> Watch {
        Watch::default()
    }

    /// The snapshot the next update is compared with.
    pub fn last(&self) -> &Snapshot {
        &self.last
    }

    /// Compares `next` with the last snapshot, keeps it, and returns the events.
    pub fn update(&mut self, next: Snapshot) -> Vec<Event> {
        let events = diff(&self.last, &next);
        self.last = next;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(link_up: bool, ssid: Option<&str>, rssi: Option<i32>, scan: &[&str]) -> Snapshot {
        Snapshot {
            link_up,
            ssid: ssid.map(String::from),
            rssi,
            scan: scan.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn first_join_reports_name_link_signal_and_scan_in_order() {
        let next = snap(true, Some("Home"), Some(-61), &["Home"]);
        assert_eq!(
            diff(&Snapshot::default(), &next),
            vec![
                Event::SsidChanged { from: None, to: Some("Home".into()) },
                Event::LinkChanged(true),
                Event::RssiChanged(Some(-61)),
                Event::ScanCacheUpdated,
            ]
        );
    }

    #[test]
    fn no_change_is_no_event() {
        let s = snap(true, Some("Home"), Some(-61), &["Home"]);
        assert!(diff(&s, &s.clone()).is_empty());
    }

    #[test]
    fn signal_moves_only_in_steps_of_two_dbm() {
        let a = snap(true, Some("Home"), Some(-60), &[]);
        assert!(diff(&a, &snap(true, Some("Home"), Some(-61), &[])).is_empty());
        assert_eq!(
            diff(&a, &snap(true, Some("Home"), Some(-62), &[])),
            vec![Event::RssiChanged(Some(-62))]
        );
        assert_eq!(
            diff(&snap(true, Some("Home"), Some(-62), &[]), &a),
            vec![Event::RssiChanged(Some(-60))]
        );
    }

    #[test]
    fn signal_appearing_or_vanishing_is_reported() {
        let joined = snap(true, Some("Home"), Some(-50), &[]);
        let gone = snap(true, Some("Home"), None, &[]);
        assert_eq!(diff(&joined, &gone), vec![Event::RssiChanged(None)]);
        assert_eq!(diff(&gone, &joined), vec![Event::RssiChanged(Some(-50))]);
        assert!(diff(&gone, &gone.clone()).is_empty());
    }

    #[test]
    fn switching_networks_and_dropping_the_link() {
        let home = snap(true, Some("Home"), Some(-55), &["Home"]);
        let work = snap(false, Some("Work"), Some(-55), &["Home"]);
        assert_eq!(
            diff(&home, &work),
            vec![
                Event::SsidChanged { from: Some("Home".into()), to: Some("Work".into()) },
                Event::LinkChanged(false),
            ]
        );
    }

    #[test]
    fn scan_results_compare_in_scan_order() {
        let a = snap(true, Some("Home"), Some(-55), &["Home", "Cafe"]);
        let reordered = snap(true, Some("Home"), Some(-55), &["Cafe", "Home"]);
        assert_eq!(diff(&a, &reordered), vec![Event::ScanCacheUpdated]);
        let same = snap(true, Some("Home"), Some(-55), &["Home", "Cafe"]);
        assert!(diff(&a, &same).is_empty());
    }

    #[test]
    fn watch_keeps_the_last_snapshot_between_updates() {
        let mut w = Watch::new();
        let first = snap(true, Some("Home"), Some(-60), &[]);
        assert_eq!(w.update(first.clone()).len(), 3);
        assert_eq!(w.last(), &first);
        assert!(w.update(first).is_empty());
        let moved = snap(true, Some("Home"), Some(-63), &[]);
        assert_eq!(w.update(moved.clone()), vec![Event::RssiChanged(Some(-63))]);
        assert_eq!(w.last(), &moved);
    }
}
