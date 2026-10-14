//! Network interfaces for apps: the name, kind, link state and hardware address of each network adapter, read from the
//! kernel's network class directory.
//!
//! The parsers read one attribute each and reject text they do not understand, so a malformed attribute becomes an
//! absent value instead of a guess. [`from_entries`] builds the sorted list from entries that were already read, which
//! keeps the model testable without a device. [`list`] reads a network class directory and feeds those entries in.
//! Point-to-point helper links are left out, as the shell leaves them out of its own interface picker.

use std::fs;
use std::io;
use std::path::Path;

/// The kernel's link type for Ethernet hardware.
const ARPHRD_ETHER: u32 = 1;
/// The kernel's link type for the loopback device.
const ARPHRD_LOOPBACK: u32 = 772;

/// The kind of adapter an interface is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InterfaceKind {
    Loopback,
    Wired,
    Wireless,
    Other,
}

/// One network interface as an app sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkInterface {
    pub name: String,
    pub kind: InterfaceKind,
    /// True when the kernel reports the link as operational, or does not report its state at all (loopback).
    pub up: bool,
    /// The six-octet hardware address in lowercase colon form. None when absent or all zeros.
    pub hardware_address: Option<String>,
}

/// The attributes of one interface directory, as read from sysfs. `None` marks an attribute that could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The text of the `operstate` attribute.
    pub operstate: Option<String>,
    /// The text of the `address` attribute.
    pub address: Option<String>,
    /// The text of the `type` attribute, the kernel's link type number.
    pub arp_type: Option<String>,
    /// True when the directory has a wireless or 802.11 PHY entry.
    pub wireless: bool,
}

/// Reads the link type number from the text of a `type` attribute.
pub fn parse_arp_type(text: &str) -> Option<u32> {
    text.trim().parse().ok()
}

/// Reads the link state from the text of an `operstate` attribute. `up` and `unknown` mean the link is usable; a
/// driver that does not report state (loopback) says `unknown`. `down`, `dormant`, `lowerlayerdown` and `notpresent`
/// mean it is not. Any other text is None.
pub fn parse_operstate(text: &str) -> Option<bool> {
    match text.trim() {
        "up" | "unknown" => Some(true),
        "down" | "dormant" | "lowerlayerdown" | "notpresent" => Some(false),
        _ => None,
    }
}

/// Reads a hardware address: six hexadecimal octets separated by colons. The result is lowercase. An all-zero address
/// (what a device without hardware reports) is None.
pub fn parse_hardware_address(text: &str) -> Option<String> {
    let octets: Vec<&str> = text.trim().split(':').collect();
    if octets.len() != 6 || octets.iter().any(|o| o.len() != 2 || !o.bytes().all(|b| b.is_ascii_hexdigit())) {
        return None;
    }
    let address = octets.join(":").to_ascii_lowercase();
    if address == "00:00:00:00:00:00" {
        return None;
    }
    Some(address)
}

/// The kind of an interface from its link type and whether it has a wireless PHY. A wireless PHY wins over the link
/// type, because wireless adapters report the Ethernet type.
pub fn classify(arp_type: u32, wireless: bool) -> InterfaceKind {
    match (arp_type, wireless) {
        (_, true) => InterfaceKind::Wireless,
        (ARPHRD_LOOPBACK, false) => InterfaceKind::Loopback,
        (ARPHRD_ETHER, false) => InterfaceKind::Wired,
        _ => InterfaceKind::Other,
    }
}

/// Builds the interface list from entries that were already read. Entries without a readable link type are left out,
/// since their kind is unknown. The result is sorted by name.
pub fn from_entries(entries: &[Entry]) -> Vec<NetworkInterface> {
    let mut interfaces: Vec<NetworkInterface> = entries
        .iter()
        .filter(|e| !e.name.is_empty() && !e.name.starts_with("p2p"))
        .filter_map(|e| {
            let arp_type = parse_arp_type(e.arp_type.as_deref()?)?;
            Some(NetworkInterface {
                name: e.name.clone(),
                kind: classify(arp_type, e.wireless),
                up: e.operstate.as_deref().and_then(parse_operstate).unwrap_or(false),
                hardware_address: e.address.as_deref().and_then(parse_hardware_address),
            })
        })
        .collect();
    interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    interfaces
}

/// Lists the network interfaces under `sys_net`, the kernel's network class directory (`/sys/class/net`).
pub fn list(sys_net: &Path) -> io::Result<Vec<NetworkInterface>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(sys_net)? {
        let dir = item?.path();
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        entries.push(Entry {
            name,
            operstate: read_attribute(&dir, "operstate"),
            address: read_attribute(&dir, "address"),
            arp_type: read_attribute(&dir, "type"),
            wireless: dir.join("wireless").exists() || dir.join("phy80211").exists(),
        });
    }
    Ok(from_entries(&entries))
}

fn read_attribute(dir: &Path, attribute: &str) -> Option<String> {
    fs::read_to_string(dir.join(attribute)).ok()
}
