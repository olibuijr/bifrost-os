//! Route addresses on the local network: the full or partial IP addresses of receivers an app already knows, so a route
//! can be reached without discovery. A partial address is a network in CIDR form (`192.168.1.0/24`) and matches every
//! address inside it. Pure std: the shell supplies the addresses and the network stack answers for them.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A full address (`192.168.1.12`, `fe80::1`) or a partial one (`192.168.1.0/24`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteAddress {
    Full(IpAddr),
    /// `network` has its host bits cleared, so two spellings of one network compare equal.
    Partial { network: IpAddr, prefix_len: u8 },
}

impl RouteAddress {
    /// Parses a full address, or a partial address written as `address/prefix`. Host bits in a partial address are
    /// cleared, so `192.168.1.77/24` names the `192.168.1.0/24` network. Returns `None` for other text and for a prefix
    /// longer than the address (32 bits for IPv4, 128 for IPv6).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        match text.split_once('/') {
            None => text.parse::<IpAddr>().ok().map(RouteAddress::Full),
            Some((address, len)) => {
                let address: IpAddr = address.parse().ok()?;
                let prefix_len: u8 = len.parse().ok()?;
                if prefix_len > max_prefix(address) {
                    return None;
                }
                Some(RouteAddress::Partial { network: mask(address, prefix_len), prefix_len })
            }
        }
    }

    /// True when `ip` is this address or lies inside this network. An IPv4 address never matches an IPv6 route, and
    /// the reverse.
    pub fn matches(&self, ip: IpAddr) -> bool {
        match *self {
            RouteAddress::Full(address) => address == ip,
            RouteAddress::Partial { network, prefix_len } => same_family(network, ip) && mask(ip, prefix_len) == network,
        }
    }
}

/// The route addresses an app knows. Each address is kept once, in the order it was added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownRoutes {
    addresses: Vec<RouteAddress>,
}

impl KnownRoutes {
    /// Builds the list from text entries. Returns the first entry that is not an address or a network.
    pub fn from_texts<'a>(entries: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let mut routes = KnownRoutes::default();
        for entry in entries {
            let address = RouteAddress::parse(entry).ok_or_else(|| entry.to_string())?;
            routes.add(address);
        }
        Ok(routes)
    }

    /// Adds an address. Returns false when the same address or network is already known.
    pub fn add(&mut self, address: RouteAddress) -> bool {
        if self.addresses.contains(&address) {
            return false;
        }
        self.addresses.push(address);
        true
    }

    /// Removes an address. Returns false when it was not known.
    pub fn remove(&mut self, address: RouteAddress) -> bool {
        let before = self.addresses.len();
        self.addresses.retain(|a| *a != address);
        self.addresses.len() != before
    }

    /// True when some known address or network holds `ip`.
    pub fn contains(&self, ip: IpAddr) -> bool {
        self.addresses.iter().any(|a| a.matches(ip))
    }

    pub fn addresses(&self) -> &[RouteAddress] {
        &self.addresses
    }

    pub fn len(&self) -> usize {
        self.addresses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }
}

fn max_prefix(address: IpAddr) -> u8 {
    match address {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}

fn same_family(a: IpAddr, b: IpAddr) -> bool {
    matches!((a, b), (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)))
}

/// Keeps the first `prefix_len` bits of `address` and clears the rest.
fn mask(address: IpAddr, prefix_len: u8) -> IpAddr {
    match address {
        IpAddr::V4(a) => {
            let keep = if prefix_len == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix_len)) };
            IpAddr::V4(Ipv4Addr::from(u32::from(a) & keep))
        }
        IpAddr::V6(a) => {
            let keep = if prefix_len == 0 { 0 } else { u128::MAX << (128 - u32::from(prefix_len)) };
            IpAddr::V6(Ipv6Addr::from(u128::from(a) & keep))
        }
    }
}
