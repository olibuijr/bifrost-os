//! Per-app network transfer totals as a pure model (docs/parity-matrix, metrickit): how many bytes each app uploaded
//! and downloaded, split by link (Wi-Fi or cellular). The network daemon counts bytes per interface and does not
//! attribute them to apps; a caller that knows which app owns a transfer records it here.
//!
//! The model reads no socket and no clock. Totals saturate at the maximum instead of wrapping, and a zero byte count
//! creates no entry, so an app that never transferred anything is not listed.

use std::collections::BTreeMap;

/// The network link a transfer used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Link {
    Wifi,
    Cellular,
}

/// The direction of a transfer, from the app's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    Upload,
    Download,
}

/// The four totals an app keeps, as (link, direction) pairs in the order a report lists them.
pub const ALL_METRICS: [(Link, Direction); 4] = [
    (Link::Wifi, Direction::Upload),
    (Link::Wifi, Direction::Download),
    (Link::Cellular, Direction::Upload),
    (Link::Cellular, Direction::Download),
];

impl Link {
    fn index(self) -> usize {
        match self {
            Link::Wifi => 0,
            Link::Cellular => 1,
        }
    }
}

impl Direction {
    fn index(self) -> usize {
        match self {
            Direction::Upload => 0,
            Direction::Download => 1,
        }
    }
}

fn slot(link: Link, dir: Direction) -> usize {
    link.index() * 2 + dir.index()
}

/// The byte totals of one app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    bytes: [u64; 4],
}

impl Totals {
    /// The bytes one link carried in one direction.
    pub fn get(&self, link: Link, dir: Direction) -> u64 {
        self.bytes[slot(link, dir)]
    }

    /// Both directions of one link added together, saturating at the maximum.
    pub fn link_sum(&self, link: Link) -> u64 {
        self.get(link, Direction::Upload).saturating_add(self.get(link, Direction::Download))
    }
}

/// The transfer totals of every app that has recorded a transfer.
#[derive(Debug, Default)]
pub struct Ledger {
    apps: BTreeMap<String, Totals>,
}

impl Ledger {
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// Adds `bytes` to the app's total for one link and direction. Zero bytes change nothing and create no entry.
    pub fn record(&mut self, app: &str, link: Link, dir: Direction, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let totals = self.apps.entry(app.to_string()).or_default();
        let cell = &mut totals.bytes[slot(link, dir)];
        *cell = cell.saturating_add(bytes);
    }

    /// The totals of one app. An app with no recorded transfer has all zeros.
    pub fn totals(&self, app: &str) -> Totals {
        self.apps.get(app).copied().unwrap_or_default()
    }

    /// The app identifiers with a recorded transfer, in sorted order.
    pub fn apps(&self) -> Vec<&str> {
        self.apps.keys().map(|name| name.as_str()).collect()
    }

    /// The bytes one link carried for every app together, both directions, saturating at the maximum.
    pub fn link_sum(&self, link: Link) -> u64 {
        self.apps.values().fold(0u64, |sum, t| sum.saturating_add(t.link_sum(link)))
    }

    /// Removes an app's totals and returns them. An unknown app returns all zeros.
    pub fn take(&mut self, app: &str) -> Totals {
        self.apps.remove(app).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_link_and_direction_keeps_its_own_total() {
        let mut ledger = Ledger::new();
        ledger.record("notes", Link::Wifi, Direction::Upload, 100);
        ledger.record("notes", Link::Wifi, Direction::Download, 250);
        ledger.record("notes", Link::Cellular, Direction::Download, 7);
        let t = ledger.totals("notes");
        assert_eq!(t.get(Link::Wifi, Direction::Upload), 100);
        assert_eq!(t.get(Link::Wifi, Direction::Download), 250);
        assert_eq!(t.get(Link::Cellular, Direction::Upload), 0);
        assert_eq!(t.get(Link::Cellular, Direction::Download), 7);
    }

    #[test]
    fn repeated_records_add_up() {
        let mut ledger = Ledger::new();
        ledger.record("mail", Link::Cellular, Direction::Upload, 40);
        ledger.record("mail", Link::Cellular, Direction::Upload, 60);
        assert_eq!(ledger.totals("mail").get(Link::Cellular, Direction::Upload), 100);
    }

    #[test]
    fn zero_bytes_create_no_entry() {
        let mut ledger = Ledger::new();
        ledger.record("idle", Link::Wifi, Direction::Download, 0);
        assert!(ledger.apps().is_empty());
        assert_eq!(ledger.totals("idle"), Totals::default());
    }

    #[test]
    fn totals_saturate_at_the_maximum() {
        let mut ledger = Ledger::new();
        ledger.record("big", Link::Wifi, Direction::Download, u64::MAX - 5);
        ledger.record("big", Link::Wifi, Direction::Download, 10);
        assert_eq!(ledger.totals("big").get(Link::Wifi, Direction::Download), u64::MAX);
    }

    #[test]
    fn apps_are_kept_apart_and_listed_in_order() {
        let mut ledger = Ledger::new();
        ledger.record("zeta", Link::Wifi, Direction::Upload, 1);
        ledger.record("alpha", Link::Wifi, Direction::Upload, 2);
        assert_eq!(ledger.apps(), vec!["alpha", "zeta"]);
        assert_eq!(ledger.totals("alpha").get(Link::Wifi, Direction::Upload), 2);
        assert_eq!(ledger.totals("zeta").get(Link::Wifi, Direction::Upload), 1);
    }

    #[test]
    fn link_sum_adds_both_directions_and_all_apps() {
        let mut ledger = Ledger::new();
        ledger.record("a", Link::Wifi, Direction::Upload, 1);
        ledger.record("a", Link::Wifi, Direction::Download, 2);
        ledger.record("b", Link::Wifi, Direction::Download, 10);
        ledger.record("b", Link::Cellular, Direction::Upload, 4);
        assert_eq!(ledger.link_sum(Link::Wifi), 13);
        assert_eq!(ledger.link_sum(Link::Cellular), 4);
    }

    #[test]
    fn take_removes_the_app_and_returns_its_totals() {
        let mut ledger = Ledger::new();
        ledger.record("maps", Link::Cellular, Direction::Download, 512);
        let taken = ledger.take("maps");
        assert_eq!(taken.get(Link::Cellular, Direction::Download), 512);
        assert!(ledger.apps().is_empty());
        assert_eq!(ledger.take("maps"), Totals::default());
    }

    #[test]
    fn the_four_metrics_are_distinct_pairs() {
        let mut slots: Vec<usize> = ALL_METRICS.iter().map(|&(l, d)| slot(l, d)).collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![0, 1, 2, 3]);
    }
}
