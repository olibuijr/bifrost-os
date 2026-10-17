//! The virtual network interface a packet tunnel provider reads and writes: a Linux `tun` device, and the address and
//! route setup that makes the system send the right traffic into it.
//!
//! A custom VPN is a provider (a program that speaks its own protocol to a server) plus a host that gives it a virtual
//! interface and routes. `tunnel` is the model: the session life cycle, the profile, and the include and exclude rules.
//! This module is the host side that was missing: [`TunDevice`] opens `/dev/net/tun` and moves whole IP packets, and
//! [`interface_commands`] and [`route_commands`] turn a [`RoutePolicy`] into the `ip` invocations that bring the
//! interface up and install the routes. A full tunnel adds two half-routes (`0.0.0.0/1` and `128.0.0.0/1`), which win over
//! the default route without removing it, and keeps the VPN server and the excluded networks on the physical uplink.
//!
//! Opening the device and running `ip` need the privilege the shell has. The command lists are pure and tested; the
//! device ioctl structures are checked against the kernel ABI.

use crate::tunnel::{Ipv4Route, RouteAction, RoutePolicy};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_ulong};
use std::process::Command;

const IFNAMSIZ: usize = 16;
/// A layer-3 (IP packet) device, without the packet-information header.
const IFF_TUN: i16 = 0x0001;
const IFF_NO_PI: i16 = 0x1000;
/// `_IOW('T', 202, int)`.
const TUNSETIFF: c_ulong = 0x4004_54ca;

/// `struct ifreq` as the tun ioctl reads it: the interface name, then the 24-byte union, of which the flags are the
/// first two bytes.
#[repr(C)]
struct IfReq {
    name: [u8; IFNAMSIZ],
    flags: i16,
    pad: [u8; 22],
}

extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

/// A name the kernel accepts for an interface: 1 to 15 bytes, no slash, no space, no NUL. A name may contain one `%d`,
/// which the kernel replaces with the first free number (`bftun%d` becomes `bftun0`).
pub fn valid_interface_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < IFNAMSIZ
        && name != "."
        && name != ".."
        && !name.bytes().any(|b| b == 0 || b == b'/' || b.is_ascii_whitespace())
}

/// An open tunnel interface. Dropping it closes the descriptor, which removes the interface and the routes through it.
pub struct TunDevice {
    file: File,
    name: String,
}

impl TunDevice {
    /// Creates a tun interface called `name` (or a numbered one, with `%d`) and returns it. The interface is down
    /// until [`interface_commands`] brings it up.
    pub fn open(name: &str) -> io::Result<TunDevice> {
        if !valid_interface_name(name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a valid interface name"));
        }
        let file = OpenOptions::new().read(true).write(true).open("/dev/net/tun")?;
        let mut request = IfReq { name: [0; IFNAMSIZ], flags: IFF_TUN | IFF_NO_PI, pad: [0; 22] };
        request.name[..name.len()].copy_from_slice(name.as_bytes());
        // SAFETY: `file` is an open descriptor, and TUNSETIFF reads and writes one `ifreq`, which `request` is.
        let rc = unsafe { ioctl(file.as_raw_fd(), TUNSETIFF, &mut request as *mut IfReq as *mut c_void) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        let end = request.name.iter().position(|&b| b == 0).unwrap_or(IFNAMSIZ);
        Ok(TunDevice { file, name: String::from_utf8_lossy(&request.name[..end]).into_owned() })
    }

    /// The name the kernel gave the interface.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Reads one IP packet from the interface (a packet the system routed into the tunnel). `buf` should hold the
    /// interface MTU; a packet longer than `buf` is cut.
    pub fn read_packet(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }

    /// Writes one IP packet to the interface, as if it had arrived from the network.
    pub fn write_packet(&mut self, packet: &[u8]) -> io::Result<()> {
        let n = self.file.write(packet)?;
        if n == packet.len() {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::WriteZero, "the interface took part of a packet"))
        }
    }
}

/// The physical network the tunnel's own traffic and the excluded networks keep using.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uplink {
    pub gateway: Ipv4Addr,
    pub interface: String,
}

/// Why the routes could not be planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// A full tunnel (or any exclude rule) needs the uplink to route the exempt traffic.
    NeedsUplink,
    /// The interface name is not usable.
    BadInterface,
    /// The MTU is outside 576 to 65535.
    BadMtu,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::NeedsUplink => f.write_str("the routes need the gateway and interface of the physical network"),
            PlanError::BadInterface => f.write_str("the interface name is not valid"),
            PlanError::BadMtu => f.write_str("the MTU must be between 576 and 65535"),
        }
    }
}

impl std::error::Error for PlanError {}

fn route_text(route: &Ipv4Route) -> String {
    format!("{}/{}", route.network(), route.prefix())
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// The `ip` invocations that give the interface its address and MTU and bring it up.
pub fn interface_commands(interface: &str, address: Ipv4Addr, prefix: u8, mtu: u16) -> Result<Vec<Vec<String>>, PlanError> {
    if !valid_interface_name(interface) || interface.contains('%') {
        return Err(PlanError::BadInterface);
    }
    if mtu < 576 {
        return Err(PlanError::BadMtu);
    }
    let cidr = format!("{address}/{}", prefix.min(32));
    Ok(vec![
        argv(&["addr", "add", &cidr, "dev", interface]),
        argv(&["link", "set", "dev", interface, "mtu", &mtu.to_string(), "up"]),
    ])
}

/// The `ip route` invocations for a policy. `verb` is `add` to install and `del` to remove them again. Traffic the
/// policy sends into the tunnel goes `dev <interface>`; traffic it keeps direct (the excluded networks of a full tunnel,
/// and the VPN server itself) goes `via <gateway> dev <uplink>`.
pub fn route_commands(
    verb: &str,
    interface: &str,
    policy: &RoutePolicy,
    uplink: Option<&Uplink>,
    server: Option<Ipv4Addr>,
) -> Result<Vec<Vec<String>>, PlanError> {
    if !valid_interface_name(interface) || interface.contains('%') {
        return Err(PlanError::BadInterface);
    }
    let full = policy.default_action() == RouteAction::Tunnel;
    let excluded: Vec<&Ipv4Route> = policy.rules().iter().filter(|(_, a)| *a == RouteAction::Direct).map(|(r, _)| r).collect();
    if full && uplink.is_none() && (server.is_some() || !excluded.is_empty()) {
        return Err(PlanError::NeedsUplink);
    }
    let mut out = Vec::new();
    let via = |target: &str, up: &Uplink| argv(&["route", verb, target, "via", &up.gateway.to_string(), "dev", &up.interface]);
    // the physical uplink first, so the tunnel's own packets never loop back into it
    if let (Some(up), true) = (uplink, full) {
        if let Some(server) = server {
            out.push(via(&format!("{server}/32"), up));
        }
        for route in &excluded {
            out.push(via(&route_text(route), up));
        }
    }
    if full {
        out.push(argv(&["route", verb, "0.0.0.0/1", "dev", interface]));
        out.push(argv(&["route", verb, "128.0.0.0/1", "dev", interface]));
    }
    for (route, action) in policy.rules() {
        if *action == RouteAction::Tunnel && !(full && route.prefix() == 0) {
            out.push(argv(&["route", verb, &route_text(route), "dev", interface]));
        }
    }
    Ok(out)
}

/// Runs `ip` once per argument list and stops at the first failure.
pub fn run_commands(commands: &[Vec<String>]) -> io::Result<()> {
    for args in commands {
        let output = Command::new("ip").args(args).output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!("ip {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim())));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(text: &str) -> Ipv4Route {
        Ipv4Route::parse(text).unwrap()
    }

    fn uplink() -> Uplink {
        Uplink { gateway: Ipv4Addr::new(192, 168, 1, 1), interface: "wlan0".into() }
    }

    fn text(commands: &[Vec<String>]) -> Vec<String> {
        commands.iter().map(|c| c.join(" ")).collect()
    }

    #[test]
    fn the_ioctl_request_and_structure_match_the_kernel_abi() {
        assert_eq!(std::mem::size_of::<IfReq>(), 40);
        assert_eq!(TUNSETIFF, (1 << 30) | (4 << 16) | ((b'T' as c_ulong) << 8) | 202);
        assert_eq!(IFF_TUN | IFF_NO_PI, 0x1001);
    }

    #[test]
    fn interface_names_follow_the_kernel_rules() {
        assert!(valid_interface_name("bftun0"));
        assert!(valid_interface_name("bftun%d"));
        assert!(valid_interface_name("123456789012345"));
        assert!(!valid_interface_name("1234567890123456"));
        assert!(!valid_interface_name(""));
        assert!(!valid_interface_name("a/b"));
        assert!(!valid_interface_name("a b"));
        assert!(!valid_interface_name(".."));
        assert!(TunDevice::open("bad name").is_err());
    }

    #[test]
    fn the_interface_gets_its_address_mtu_and_is_brought_up() {
        let c = interface_commands("bftun0", Ipv4Addr::new(10, 8, 0, 2), 24, 1400).unwrap();
        assert_eq!(text(&c), vec!["addr add 10.8.0.2/24 dev bftun0", "link set dev bftun0 mtu 1400 up"]);
        assert_eq!(interface_commands("bftun%d", Ipv4Addr::LOCALHOST, 8, 1400), Err(PlanError::BadInterface));
        assert_eq!(interface_commands("bftun0", Ipv4Addr::LOCALHOST, 8, 100), Err(PlanError::BadMtu));
    }

    #[test]
    fn a_full_tunnel_adds_half_routes_and_keeps_the_server_and_local_network_direct() {
        let mut policy = RoutePolicy::new(RouteAction::Tunnel);
        policy.exclude(route("192.168.1.0/24"));
        let c = route_commands("add", "bftun0", &policy, Some(&uplink()), Some(Ipv4Addr::new(203, 0, 113, 9))).unwrap();
        assert_eq!(
            text(&c),
            vec![
                "route add 203.0.113.9/32 via 192.168.1.1 dev wlan0",
                "route add 192.168.1.0/24 via 192.168.1.1 dev wlan0",
                "route add 0.0.0.0/1 dev bftun0",
                "route add 128.0.0.0/1 dev bftun0",
            ]
        );
    }

    #[test]
    fn a_split_tunnel_routes_only_the_included_networks() {
        let mut policy = RoutePolicy::new(RouteAction::Direct);
        policy.include(route("10.0.0.0/8"));
        policy.include(route("172.16.5.4"));
        policy.exclude(route("10.9.0.0/16"));
        let c = route_commands("add", "bftun0", &policy, None, None).unwrap();
        assert_eq!(text(&c), vec!["route add 10.0.0.0/8 dev bftun0", "route add 172.16.5.4/32 dev bftun0"]);
    }

    #[test]
    fn a_full_tunnel_without_an_uplink_cannot_keep_anything_direct() {
        let mut policy = RoutePolicy::new(RouteAction::Tunnel);
        policy.exclude(route("192.168.1.0/24"));
        assert_eq!(route_commands("add", "bftun0", &policy, None, None), Err(PlanError::NeedsUplink));
        // with nothing to keep direct it needs none
        let plain = RoutePolicy::new(RouteAction::Tunnel);
        assert_eq!(route_commands("add", "bftun0", &plain, None, None).unwrap().len(), 2);
    }

    #[test]
    fn removing_uses_the_same_routes_with_del() {
        let mut policy = RoutePolicy::new(RouteAction::Tunnel);
        policy.exclude(route("192.168.1.0/24"));
        let add = route_commands("add", "bftun0", &policy, Some(&uplink()), None).unwrap();
        let del = route_commands("del", "bftun0", &policy, Some(&uplink()), None).unwrap();
        assert_eq!(add.len(), del.len());
        assert!(del.iter().all(|c| c[1] == "del"));
        assert_eq!(add[0][2], del[0][2]);
    }

    #[test]
    fn a_default_route_rule_under_a_full_tunnel_is_not_added_twice() {
        let mut policy = RoutePolicy::new(RouteAction::Tunnel);
        policy.include(route("0.0.0.0/0"));
        let c = route_commands("add", "bftun0", &policy, None, None).unwrap();
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn a_bad_interface_is_refused_for_routes() {
        let policy = RoutePolicy::new(RouteAction::Direct);
        assert_eq!(route_commands("add", "bf tun", &policy, None, None), Err(PlanError::BadInterface));
        assert_eq!(route_commands("add", "bftun%d", &policy, None, None), Err(PlanError::BadInterface));
    }
}
