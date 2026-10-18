//! Reachability flags (`SCNetworkReachabilityFlags`): what the system's network path says about a host or an address,
//! without sending anything. The answer comes from the path the shell's network service reports ([`Path`], see
//! `network_path`), so an app learns whether traffic can leave the machine for that destination, whether a connection
//! must be set up first, and whether the route is cellular. The model opens no socket: for a named host the caller
//! resolves the name first and passes the addresses in, and a TCP probe is `reachability::probe`.
//!
//! ```ignore
//! let flags = reachability_flags::for_host(&path, "updates.example.org", &resolved_addresses);
//! if flags.is_reachable() { /* traffic can leave for that host */ }
//! ```

use bifrost_core::net_path::{Path, Status};
use std::net::{IpAddr, Ipv4Addr};

/// A set of reachability flags, with the bit values the Security and SystemConfiguration headers define.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(u32);

impl Flags {
    /// The destination can be reached on the current path.
    pub const REACHABLE: Flags = Flags(1 << 1);
    /// A connection must be set up before traffic can flow (the link is up but has no address yet).
    pub const CONNECTION_REQUIRED: Flags = Flags(1 << 2);
    /// The destination is this machine (loopback).
    pub const IS_LOCAL_ADDRESS: Flags = Flags(1 << 16);
    /// The route leaves through a cellular link, so the traffic is metered.
    pub const IS_WWAN: Flags = Flags(1 << 18);

    /// The raw flag bits.
    pub fn bits(self) -> u32 {
        self.0
    }

    /// Whether every flag in `other` is set here.
    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether the destination is reachable on the current path.
    pub fn is_reachable(self) -> bool {
        self.contains(Flags::REACHABLE)
    }

    fn with(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }
}

/// The flags for one address. The unspecified address (0.0.0.0 or ::) asks about the default route: it is reachable
/// when either address family has an address. Loopback is always reachable and marked local, even offline.
pub fn for_address(path: &Path, addr: IpAddr) -> Flags {
    if addr.is_loopback() {
        return Flags::REACHABLE.with(Flags::IS_LOCAL_ADDRESS);
    }
    let family_up = if addr.is_unspecified() {
        path.ipv4 || path.ipv6
    } else {
        match addr {
            IpAddr::V4(_) => path.ipv4,
            IpAddr::V6(_) => path.ipv6,
        }
    };
    match path.status {
        Status::Satisfied if family_up => {
            if path.expensive {
                Flags::REACHABLE.with(Flags::IS_WWAN)
            } else {
                Flags::REACHABLE
            }
        }
        Status::RequiresConnection => Flags::CONNECTION_REQUIRED,
        _ => Flags::default(),
    }
}

/// The flags for a named host. `localhost` is this machine. Otherwise the answer is the first reachable address in
/// `resolved`, or the first address's flags when none is reachable, or nothing when the name resolved to no address.
pub fn for_host(path: &Path, name: &str, resolved: &[IpAddr]) -> Flags {
    if name.is_empty() {
        return Flags::default();
    }
    if name.eq_ignore_ascii_case("localhost") {
        return for_address(path, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let flags: Vec<Flags> = resolved.iter().map(|&addr| for_address(path, addr)).collect();
    flags
        .iter()
        .copied()
        .find(|f| f.is_reachable())
        .or_else(|| flags.first().copied())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::net_path::{IfaceKind, Interface};
    use std::net::Ipv6Addr;

    fn iface(kind: IfaceKind) -> Interface {
        Interface { name: "test0".to_string(), kind }
    }

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn a_wired_path_with_ipv4_reaches_an_ipv4_address() {
        let path = Path::new(vec![iface(IfaceKind::Wired)], true, false);
        let flags = for_address(&path, v4(192, 0, 2, 1));
        assert!(flags.is_reachable());
        assert!(!flags.contains(Flags::CONNECTION_REQUIRED));
        assert!(!flags.contains(Flags::IS_WWAN));
    }

    #[test]
    fn no_link_reaches_nothing_but_loopback() {
        let path = Path::offline();
        assert_eq!(for_address(&path, v4(192, 0, 2, 1)), Flags::default());
        let local = for_address(&path, v4(127, 0, 0, 1));
        assert!(local.is_reachable());
        assert!(local.contains(Flags::IS_LOCAL_ADDRESS));
    }

    #[test]
    fn a_link_without_an_address_requires_a_connection() {
        let path = Path::new(vec![iface(IfaceKind::Wifi)], false, false);
        let flags = for_address(&path, v4(192, 0, 2, 1));
        assert!(!flags.is_reachable());
        assert!(flags.contains(Flags::CONNECTION_REQUIRED));
    }

    #[test]
    fn an_ipv6_address_is_not_reachable_on_an_ipv4_only_path() {
        let path = Path::new(vec![iface(IfaceKind::Wired)], true, false);
        let v6 = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert!(!for_address(&path, v6).is_reachable());
    }

    #[test]
    fn a_cellular_route_is_marked_as_wwan() {
        let path = Path::new(vec![iface(IfaceKind::Cellular)], true, false);
        let flags = for_address(&path, v4(192, 0, 2, 1));
        assert!(flags.is_reachable());
        assert!(flags.contains(Flags::IS_WWAN));
    }

    #[test]
    fn the_unspecified_address_asks_about_the_default_route() {
        let path = Path::new(vec![iface(IfaceKind::Wired)], true, false);
        assert!(for_address(&path, v4(0, 0, 0, 0)).is_reachable());
        assert!(!for_address(&Path::offline(), v4(0, 0, 0, 0)).is_reachable());
    }

    #[test]
    fn a_host_is_reachable_through_any_of_its_addresses() {
        let path = Path::new(vec![iface(IfaceKind::Wired)], true, false);
        let v6 = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert!(for_host(&path, "example.test", &[v6, v4(192, 0, 2, 1)]).is_reachable());
    }

    #[test]
    fn a_host_with_no_reachable_address_reports_the_first_address_flags() {
        let path = Path::new(vec![iface(IfaceKind::Wifi)], false, false);
        let flags = for_host(&path, "example.test", &[v4(192, 0, 2, 1)]);
        assert!(flags.contains(Flags::CONNECTION_REQUIRED));
        assert!(!flags.is_reachable());
    }

    #[test]
    fn localhost_is_local_even_offline_and_an_unresolved_name_has_no_flags() {
        let path = Path::offline();
        assert!(for_host(&path, "LocalHost", &[]).contains(Flags::IS_LOCAL_ADDRESS));
        assert_eq!(for_host(&path, "example.test", &[]), Flags::default());
        assert_eq!(for_host(&path, "", &[v4(192, 0, 2, 1)]), Flags::default());
    }

    #[test]
    fn flag_bits_match_the_system_values() {
        assert_eq!(Flags::REACHABLE.bits(), 1 << 1);
        assert_eq!(Flags::CONNECTION_REQUIRED.bits(), 1 << 2);
        assert_eq!(Flags::IS_LOCAL_ADDRESS.bits(), 1 << 16);
        assert_eq!(Flags::IS_WWAN.bits(), 1 << 18);
    }
}
