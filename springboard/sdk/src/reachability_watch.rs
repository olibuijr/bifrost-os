//! Reachability callbacks (`SCNetworkReachabilitySetCallback`) for a named host or an address.
//!
//! A watch remembers the flags it last reported for its target. Each time the network path changes, the app feeds
//! the new path to [`ReachabilityWatch::update`], and the watch answers with the flags only when they differ from the
//! last report, so a callback fires on a real change (the link went down, the route became cellular), not on every
//! path report. A host watch keeps the addresses it was given; when the name is resolved again, the app passes the
//! new addresses to [`ReachabilityWatch::set_resolved`].
//!
//! ```ignore
//! let mut watch = ReachabilityWatch::for_host("updates.example.org", resolved);
//! if let Some(flags) = watch.update(&path) { /* reachability changed: flags.is_reachable() */ }
//! ```
//!
//! Gaps: the watch is polled, not scheduled. The app must call `update` from the same place it sees network path
//! changes (the `network_path` service). Name resolution is the caller's job, as in `reachability_flags`.

use super::reachability_flags::{self, Flags};
use bifrost_core::net_path::Path;
use std::net::IpAddr;

/// What a watch follows: a host name with the addresses it resolved to, or one address.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Host { name: String, resolved: Vec<IpAddr> },
    Address(IpAddr),
}

/// One reachability callback: the target and the flags it last reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReachabilityWatch {
    target: Target,
    last: Option<Flags>,
}

impl ReachabilityWatch {
    /// A watch on a named host, with the addresses its name resolved to.
    pub fn for_host(name: &str, resolved: Vec<IpAddr>) -> ReachabilityWatch {
        ReachabilityWatch { target: Target::Host { name: name.to_string(), resolved }, last: None }
    }

    /// A watch on one address.
    pub fn for_address(addr: IpAddr) -> ReachabilityWatch {
        ReachabilityWatch { target: Target::Address(addr), last: None }
    }

    /// The flags the watch last reported, `None` before the first `update`.
    pub fn last(&self) -> Option<Flags> {
        self.last
    }

    /// Replaces the addresses of a host watch after the name was resolved again. Does nothing for an address watch.
    /// The next `update` reports if the new addresses change the answer.
    pub fn set_resolved(&mut self, addresses: Vec<IpAddr>) {
        if let Target::Host { resolved, .. } = &mut self.target {
            *resolved = addresses;
        }
    }

    /// Checks the target against `path`. Returns the flags when they differ from the last report (the first call
    /// always reports), and `None` when nothing changed.
    pub fn update(&mut self, path: &Path) -> Option<Flags> {
        let now = self.flags(path);
        if self.last == Some(now) {
            return None;
        }
        self.last = Some(now);
        Some(now)
    }

    fn flags(&self, path: &Path) -> Flags {
        match &self.target {
            Target::Host { name, resolved } => reachability_flags::for_host(path, name, resolved),
            Target::Address(addr) => reachability_flags::for_address(path, *addr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::net_path::{IfaceKind, Interface};
    use std::net::Ipv4Addr;

    fn iface(kind: IfaceKind) -> Interface {
        Interface { name: "test0".to_string(), kind }
    }

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn wired() -> Path {
        Path::new(vec![iface(IfaceKind::Wired)], true, false)
    }

    #[test]
    fn the_first_update_reports_and_an_unchanged_path_does_not() {
        let mut watch = ReachabilityWatch::for_host("example.test", vec![v4(192, 0, 2, 1)]);
        assert_eq!(watch.last(), None);
        let first = watch.update(&wired()).expect("first report");
        assert!(first.is_reachable());
        assert_eq!(watch.update(&wired()), None, "same flags are not reported again");
        assert_eq!(watch.last(), Some(first));
    }

    #[test]
    fn losing_the_link_reports_the_new_flags_once() {
        let mut watch = ReachabilityWatch::for_address(v4(192, 0, 2, 1));
        watch.update(&wired());
        let down = watch.update(&Path::offline()).expect("the link went down");
        assert!(!down.is_reachable());
        assert_eq!(watch.update(&Path::offline()), None);
    }

    #[test]
    fn a_route_change_to_cellular_is_reported() {
        let mut watch = ReachabilityWatch::for_address(v4(192, 0, 2, 1));
        watch.update(&wired());
        let cellular = Path::new(vec![iface(IfaceKind::Cellular)], true, false);
        let flags = watch.update(&cellular).expect("the route became metered");
        assert!(flags.is_reachable());
        assert!(flags.contains(Flags::IS_WWAN));
    }

    #[test]
    fn a_host_watch_reports_when_the_name_resolves_again() {
        let mut watch = ReachabilityWatch::for_host("example.test", vec![]);
        let unresolved = watch.update(&wired()).expect("first report");
        assert_eq!(unresolved, Flags::default(), "a name with no address has no flags");
        watch.set_resolved(vec![v4(192, 0, 2, 1)]);
        let resolved = watch.update(&wired()).expect("the new address answers");
        assert!(resolved.is_reachable());
    }

    #[test]
    fn set_resolved_does_nothing_to_an_address_watch() {
        let mut watch = ReachabilityWatch::for_address(v4(192, 0, 2, 1));
        watch.update(&wired());
        watch.set_resolved(vec![]);
        assert_eq!(watch.update(&wired()), None);
    }
}
