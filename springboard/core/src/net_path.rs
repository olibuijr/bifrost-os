//! Network path state and change detection (std only).
//!
//! A path is the system's view of connectivity: whether traffic can leave the machine (satisfied,
//! unsatisfied, or requires a connection that is still being set up), which interfaces carry it in
//! preference order, and whether the primary link is metered. The network daemon reports link and
//! address state; this module turns each report into a path and tells the caller when the path
//! changed. It opens no socket and reads no interface itself.

/// Whether traffic can leave the machine on this path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// A routable interface has an address.
    Satisfied,
    /// No usable interface: nothing but loopback, or no link at all.
    Unsatisfied,
    /// A link is up but has no address yet (DHCP still running).
    RequiresConnection,
}

/// The kind of link an interface is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfaceKind {
    Wired,
    Wifi,
    Cellular,
    Loopback,
    Other,
}

impl IfaceKind {
    /// Preference rank: lower is preferred. Wired beats Wi-Fi, Wi-Fi beats cellular.
    fn rank(self) -> u8 {
        match self {
            IfaceKind::Wired => 0,
            IfaceKind::Wifi => 1,
            IfaceKind::Cellular => 2,
            IfaceKind::Other => 3,
            IfaceKind::Loopback => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub kind: IfaceKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    pub status: Status,
    /// Interfaces in preference order, the first one carries new traffic.
    pub interfaces: Vec<Interface>,
    /// The primary link is cellular, so data use costs money.
    pub expensive: bool,
    pub ipv4: bool,
    pub ipv6: bool,
}

impl Path {
    /// Builds a path from the interfaces the system reports and whether each address family is up.
    pub fn new(mut interfaces: Vec<Interface>, ipv4: bool, ipv6: bool) -> Path {
        interfaces.sort_by_key(|i| i.kind.rank());
        let has_link = interfaces.iter().any(|i| i.kind != IfaceKind::Loopback);
        let has_address = ipv4 || ipv6;
        let status = if !has_link {
            Status::Unsatisfied
        } else if !has_address {
            Status::RequiresConnection
        } else {
            Status::Satisfied
        };
        let expensive = interfaces.iter().find(|i| i.kind != IfaceKind::Loopback).map_or(false, |i| i.kind == IfaceKind::Cellular);
        Path { status, interfaces, expensive, ipv4, ipv6 }
    }

    /// The path with no interfaces at all.
    pub fn offline() -> Path {
        Path::new(Vec::new(), false, false)
    }
}

/// Remembers the last path and reports only real changes.
pub struct PathMonitor {
    current: Path,
}

impl Default for PathMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl PathMonitor {
    /// Starts offline. The first report that differs from offline is a change.
    pub fn new() -> Self {
        PathMonitor { current: Path::offline() }
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    /// Takes a new report. Returns the new path when it differs from the previous one, else `None`.
    pub fn update(&mut self, next: Path) -> Option<&Path> {
        if self.current == next {
            return None;
        }
        self.current = next;
        Some(&self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, kind: IfaceKind) -> Interface {
        Interface { name: name.to_string(), kind }
    }

    #[test]
    fn no_link_is_unsatisfied() {
        let p = Path::offline();
        assert_eq!(p.status, Status::Unsatisfied);
        assert!(p.interfaces.is_empty());
        assert!(!p.expensive);
    }

    #[test]
    fn wired_with_ipv4_is_satisfied_and_sorted_first() {
        let p = Path::new(vec![iface("wlan0", IfaceKind::Wifi), iface("eth0", IfaceKind::Wired)], true, false);
        assert_eq!(p.status, Status::Satisfied);
        assert_eq!(p.interfaces[0].name, "eth0");
        assert_eq!(p.interfaces[1].name, "wlan0");
        assert!(!p.expensive);
    }

    #[test]
    fn link_up_without_address_requires_connection() {
        let p = Path::new(vec![iface("wlan0", IfaceKind::Wifi)], false, false);
        assert_eq!(p.status, Status::RequiresConnection);
    }

    #[test]
    fn loopback_alone_is_unsatisfied_even_with_addresses() {
        let p = Path::new(vec![iface("lo", IfaceKind::Loopback)], true, true);
        assert_eq!(p.status, Status::Unsatisfied);
    }

    #[test]
    fn ipv6_only_counts_as_an_address() {
        let p = Path::new(vec![iface("eth0", IfaceKind::Wired)], false, true);
        assert_eq!(p.status, Status::Satisfied);
        assert!(p.ipv6 && !p.ipv4);
    }

    #[test]
    fn cellular_primary_is_expensive_but_wifi_primary_is_not() {
        let cell_only = Path::new(vec![iface("wwan0", IfaceKind::Cellular)], true, false);
        assert!(cell_only.expensive);
        let wifi_then_cell = Path::new(vec![iface("wwan0", IfaceKind::Cellular), iface("wlan0", IfaceKind::Wifi)], true, false);
        assert!(!wifi_then_cell.expensive);
        assert_eq!(wifi_then_cell.interfaces[0].kind, IfaceKind::Wifi);
    }

    #[test]
    fn monitor_reports_only_changes() {
        let mut m = PathMonitor::new();
        assert_eq!(m.current().status, Status::Unsatisfied);
        // Offline again: no change.
        assert!(m.update(Path::offline()).is_none());
        // Link comes up with an address: change.
        let up = Path::new(vec![iface("eth0", IfaceKind::Wired)], true, false);
        let changed = m.update(up.clone()).expect("change expected");
        assert_eq!(changed.status, Status::Satisfied);
        // Same report again: no change.
        assert!(m.update(up).is_none());
        assert_eq!(m.current().status, Status::Satisfied);
    }
}
