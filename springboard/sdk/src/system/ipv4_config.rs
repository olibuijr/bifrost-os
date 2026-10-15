//! IPv4 settings of a wired link: automatic (DHCP) or a manual address, prefix length and router. This is the value
//! an app hands to the network service; the service applies it with netlink (link up, address, default route) and
//! runs DHCP when the method is automatic.

use std::net::Ipv4Addr;

/// How a link gets its IPv4 address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Method {
    /// A DHCP client on the link sets the address, mask and router.
    Automatic,
    /// The address, prefix length and optional router are fixed.
    Manual { address: Ipv4Addr, prefix: u8, router: Option<Ipv4Addr> },
}

/// Why a manual configuration was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// A prefix length above 32.
    PrefixOutOfRange(u8),
    /// The address is the network address of its subnet (host bits all zero).
    AddressIsNetwork,
    /// The address is the broadcast address of its subnet (host bits all one).
    AddressIsBroadcast,
    /// The router is not on the subnet of the address.
    RouterOutsideSubnet,
}

/// The network mask of a prefix length as a 32-bit value, or None above 32.
pub fn mask(prefix: u8) -> Option<u32> {
    if prefix > 32 {
        return None;
    }
    Some(u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0))
}

/// The prefix length of a dotted mask such as `255.255.255.0`, or None when the ones are not contiguous.
pub fn prefix_of_mask(mask_addr: Ipv4Addr) -> Option<u8> {
    let bits = u32::from(mask_addr);
    let ones = bits.leading_ones() as u8;
    (mask(ones) == Some(bits)).then_some(ones)
}

/// Validates a manual configuration. A /31 or /32 has no network or broadcast address, so only shorter subnets reject
/// those two host values.
pub fn manual(address: Ipv4Addr, prefix: u8, router: Option<Ipv4Addr>) -> Result<Method, ConfigError> {
    let mask = mask(prefix).ok_or(ConfigError::PrefixOutOfRange(prefix))?;
    let bits = u32::from(address);
    let host = bits & !mask;
    if prefix <= 30 {
        if host == 0 {
            return Err(ConfigError::AddressIsNetwork);
        }
        if host == !mask {
            return Err(ConfigError::AddressIsBroadcast);
        }
    }
    if let Some(router) = router {
        if u32::from(router) & mask != bits & mask {
            return Err(ConfigError::RouterOutsideSubnet);
        }
    }
    Ok(Method::Manual { address, prefix, router })
}

/// The network address of `address` in its subnet.
pub fn network_of(address: Ipv4Addr, prefix: u8) -> Option<Ipv4Addr> {
    mask(prefix).map(|m| Ipv4Addr::from(u32::from(address) & m))
}
