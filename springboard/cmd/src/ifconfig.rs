//! `ifconfig`: lists the network interfaces with their flags, MTU, hardware address and IPv6 addresses. Read-only.
//! The interfaces and their flags come from `/sys/class/net`, the IPv6 addresses from `/proc/net/if_inet6`.
//! See `src/bin/ifconfig.rs` for the command.
//! The primary IPv4 address of an interface, with its netmask and broadcast address, comes from the interface ioctls
//! (`SIOCGIFADDR`, `SIOCGIFNETMASK`, `SIOCGIFBRDADDR`), the same calls `springboard::net::wifi::iface_ipv4` uses; the
//! netmask prints in hex as the reference tool does. An interface with several IPv4 addresses shows the primary one.
//! Changing an interface (`ifconfig en0 up`, addresses, MTU) is refused: that needs netlink.

use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

pub const USAGE: &str = "usage: ifconfig [-a | interface]";
pub const SYS_NET: &str = "/sys/class/net";
pub const INET6_PATH: &str = "/proc/net/if_inet6";

/// Linux interface flag bits and the names `ifconfig` prints for them, in the order it prints them.
const FLAG_NAMES: [(u32, &str); 11] = [
    (0x1, "UP"),
    (0x2, "BROADCAST"),
    (0x8, "LOOPBACK"),
    (0x10, "POINTOPOINT"),
    (0x40, "RUNNING"),
    (0x80, "NOARP"),
    (0x100, "PROMISC"),
    (0x200, "ALLMULTI"),
    (0x1000, "MULTICAST"),
    (0x8000, "DEBUG"),
    (0x10000, "NOTRAILERS"),
];

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    /// No interface named: every interface.
    List,
    /// One interface by name.
    Show(String),
}

/// Reads the arguments: nothing or `-a` lists every interface, a name shows one.
pub fn parse(args: &[String]) -> Result<Action, String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["-a"] => Ok(Action::List),
        ["-h" | "-help" | "--help"] => Ok(Action::Help),
        [name] if name.starts_with('-') => Err(format!("option {name} is not supported yet; ifconfig here only lists interfaces")),
        [name] => Ok(Action::Show(name.to_string())),
        _ => Err("ifconfig takes one interface name, or none".to_string()),
    }
}

/// One interface, as read from `/sys/class/net/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub flags: u32,
    pub mtu: u32,
    pub mac: String,
}

/// One IPv6 address from `/proc/net/if_inet6`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inet6 {
    pub iface: String,
    pub addr: Ipv6Addr,
    pub prefix: u8,
    pub scope: u8,
}

/// The IPv4 configuration of one interface: address, netmask and (when the link has one) broadcast address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inet4 {
    pub addr: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub broadcast: Option<Ipv4Addr>,
}

const SIOCGIFADDR: std::ffi::c_ulong = 0x8915;
const SIOCGIFBRDADDR: std::ffi::c_ulong = 0x8919;
const SIOCGIFNETMASK: std::ffi::c_ulong = 0x891b;

/// One interface ioctl that answers with an IPv4 `sockaddr` in a 40-byte `ifreq` (16 bytes of name, then the
/// family, port and address). `None` when the interface has no such address.
fn ioctl_addr(sock: &std::net::UdpSocket, name: &str, request: std::ffi::c_ulong) -> Option<Ipv4Addr> {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn ioctl(fd: i32, req: std::ffi::c_ulong, ...) -> i32;
    }
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() >= 16 {
        return None;
    }
    let mut req = [0u8; 40];
    req[..bytes.len()].copy_from_slice(bytes);
    // SAFETY: the request reads the interface name and fills the 40-byte `ifreq` this function owns.
    let r = unsafe { ioctl(sock.as_raw_fd(), request, req.as_mut_ptr()) };
    (r == 0).then(|| Ipv4Addr::new(req[20], req[21], req[22], req[23]))
}

/// The primary IPv4 configuration of `name`, or `None` when it has no IPv4 address.
pub fn read_inet4(name: &str) -> Option<Inet4> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    let addr = ioctl_addr(&sock, name, SIOCGIFADDR).filter(|a| !a.is_unspecified())?;
    let netmask = ioctl_addr(&sock, name, SIOCGIFNETMASK).unwrap_or(Ipv4Addr::new(255, 255, 255, 255));
    let broadcast = ioctl_addr(&sock, name, SIOCGIFBRDADDR).filter(|b| !b.is_unspecified());
    Some(Inet4 { addr, netmask, broadcast })
}

/// The names of the set flag bits, joined with commas: `UP,LOOPBACK,RUNNING`.
pub fn flags_text(flags: u32) -> String {
    FLAG_NAMES.iter().filter(|(bit, _)| flags & bit != 0).map(|(_, name)| *name).collect::<Vec<_>>().join(",")
}

/// The IPv6 addresses in `/proc/net/if_inet6`: a 32-digit hex address, index, prefix length, scope, flags, name.
pub fn parse_inet6(text: &str) -> Vec<Inet6> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 6 || f[0].len() != 32 {
                return None;
            }
            let bits = u128::from_str_radix(f[0], 16).ok()?;
            Some(Inet6 {
                iface: f[5].to_string(),
                addr: Ipv6Addr::from(bits),
                prefix: u8::from_str_radix(f[2], 16).ok()?,
                scope: u8::from_str_radix(f[3], 16).ok()?,
            })
        })
        .collect()
}

/// The scope `ifconfig` names after the scope id.
fn scope_name(scope: u8) -> Option<&'static str> {
    match scope {
        0x00 => Some("global"),
        0x10 => Some("host"),
        0x20 => Some("link"),
        0x40 => Some("site"),
        _ => None,
    }
}

/// Reads every interface under `dir` (the `/sys/class/net` layout), sorted by name. Unreadable fields read as zero or empty.
pub fn read_interfaces(dir: &Path) -> Vec<Iface> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let base = dir.join(&name);
            let read = |file: &str| std::fs::read_to_string(base.join(file)).ok().map(|s| s.trim().to_string()).unwrap_or_default();
            let flags = u32::from_str_radix(read("flags").trim_start_matches("0x"), 16).unwrap_or(0);
            let mtu = read("mtu").parse().unwrap_or(0);
            Iface { name, flags, mtu, mac: read("address") }
        })
        .collect()
}

/// The block `ifconfig` prints for one interface, with the IPv6 addresses that belong to it.
pub fn render(iface: &Iface, inet6: &[Inet6]) -> String {
    render_with(iface, None, inet6)
}

/// Like `render`, with the interface's IPv4 line (`inet ADDR netmask 0xMASK broadcast ADDR`) before its IPv6 lines.
pub fn render_with(iface: &Iface, inet4: Option<&Inet4>, inet6: &[Inet6]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}: flags={}<{}>  mtu {}", iface.name, iface.flags & 0xffff, flags_text(iface.flags), iface.mtu);
    if iface.mac != "00:00:00:00:00:00" && !iface.mac.is_empty() {
        let _ = writeln!(out, "        ether {}", iface.mac);
    }
    if let Some(v4) = inet4 {
        let _ = write!(out, "        inet {} netmask 0x{:08x}", v4.addr, u32::from(v4.netmask));
        if let Some(b) = v4.broadcast {
            let _ = write!(out, " broadcast {b}");
        }
        out.push('\n');
    }
    for a in inet6.iter().filter(|a| a.iface == iface.name) {
        let scope = match scope_name(a.scope) {
            Some(name) => format!("0x{:x}<{name}>", a.scope),
            None => format!("0x{:x}", a.scope),
        };
        let _ = writeln!(out, "        inet6 {}  prefixlen {}  scopeid {}", a.addr, a.prefix, scope);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const INET6: &str = "00000000000000000000000000000001 01 80 10 80       lo\n\
fe80000000000000021122fffe334455 02 40 20 80 eth0\n";

    #[test]
    fn flags_are_named_in_ifconfig_order() {
        assert_eq!(flags_text(0x49), "UP,LOOPBACK,RUNNING");
        assert_eq!(flags_text(0x1003), "UP,BROADCAST,MULTICAST");
        assert_eq!(flags_text(0), "");
    }

    #[test]
    fn inet6_lines_are_parsed_with_prefix_and_scope() {
        let addrs = parse_inet6(INET6);
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].addr, Ipv6Addr::LOCALHOST);
        assert_eq!(addrs[0].prefix, 128);
        assert_eq!(addrs[0].scope, 0x10);
        assert_eq!(addrs[0].iface, "lo");
        assert_eq!(addrs[1].iface, "eth0");
        assert_eq!(addrs[1].prefix, 64);
        assert_eq!(addrs[1].scope, 0x20);
        assert_eq!(parse_inet6("short line\n").len(), 0);
    }

    #[test]
    fn a_block_shows_flags_mtu_hardware_address_and_its_own_addresses() {
        let lo = Iface { name: "lo".into(), flags: 0x49, mtu: 65536, mac: "00:00:00:00:00:00".into() };
        let eth = Iface { name: "eth0".into(), flags: 0x1043, mtu: 1500, mac: "02:11:22:33:44:55".into() };
        let addrs = parse_inet6(INET6);
        let lo_text = render(&lo, &addrs);
        assert!(lo_text.starts_with("lo: flags=73<UP,LOOPBACK,RUNNING>  mtu 65536\n"));
        assert!(!lo_text.contains("ether"));
        assert!(lo_text.contains("inet6 ::1  prefixlen 128  scopeid 0x10<host>"));
        assert!(!lo_text.contains("eth0"));
        let eth_text = render(&eth, &addrs);
        assert!(eth_text.contains("ether 02:11:22:33:44:55"));
        assert!(eth_text.contains("scopeid 0x20<link>"));
    }

    #[test]
    fn the_ipv4_line_has_a_hex_netmask_and_the_broadcast_address() {
        let eth = Iface { name: "eth0".into(), flags: 0x1043, mtu: 1500, mac: "02:11:22:33:44:55".into() };
        let v4 = Inet4 { addr: Ipv4Addr::new(192, 168, 1, 7), netmask: Ipv4Addr::new(255, 255, 255, 0), broadcast: Some(Ipv4Addr::new(192, 168, 1, 255)) };
        let text = render_with(&eth, Some(&v4), &parse_inet6(INET6));
        assert!(text.contains("        inet 192.168.1.7 netmask 0xffffff00 broadcast 192.168.1.255\n"), "{text}");
        assert!(text.find("inet 192").unwrap() < text.find("inet6").unwrap(), "IPv4 comes before IPv6");
        let lo = Inet4 { addr: Ipv4Addr::LOCALHOST, netmask: Ipv4Addr::new(255, 0, 0, 0), broadcast: None };
        let lo_text = render_with(&Iface { name: "lo".into(), flags: 0x49, mtu: 65536, mac: String::new() }, Some(&lo), &[]);
        assert!(lo_text.contains("inet 127.0.0.1 netmask 0xff000000\n"), "{lo_text}");
    }

    #[test]
    fn ipv4_is_read_from_the_kernel_and_an_unknown_interface_has_none() {
        // The loopback interface carries 127.0.0.1 on any host that has one; a bare namespace reads as None.
        if let Some(v4) = read_inet4("lo") {
            assert_eq!(v4.addr, Ipv4Addr::LOCALHOST);
        }
        assert_eq!(read_inet4("no-such-if0"), None);
        assert_eq!(read_inet4(""), None);
    }

    #[test]
    fn interfaces_are_read_from_a_sysfs_style_folder() {
        let dir = std::env::temp_dir().join(format!("bifrost-ifconfig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, flags, mtu, mac) in [("wlan0", "0x1003\n", "1500\n", "02:aa:bb:cc:dd:ee\n"), ("lo", "0x49\n", "65536\n", "00:00:00:00:00:00\n")] {
            let base = dir.join(name);
            std::fs::create_dir_all(&base).unwrap();
            std::fs::write(base.join("flags"), flags).unwrap();
            std::fs::write(base.join("mtu"), mtu).unwrap();
            std::fs::write(base.join("address"), mac).unwrap();
        }
        let ifaces = read_interfaces(&dir);
        assert_eq!(ifaces.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["lo", "wlan0"]);
        assert_eq!(ifaces[0].flags, 0x49);
        assert_eq!(ifaces[1].mtu, 1500);
        assert_eq!(ifaces[1].mac, "02:aa:bb:cc:dd:ee");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_show_and_help_parse_and_options_are_refused() {
        assert_eq!(parse(&args(&[])), Ok(Action::List));
        assert_eq!(parse(&args(&["-a"])), Ok(Action::List));
        assert_eq!(parse(&args(&["-h"])), Ok(Action::Help));
        assert_eq!(parse(&args(&["eth0"])), Ok(Action::Show("eth0".into())));
        assert!(parse(&args(&["-m"])).unwrap_err().contains("option -m"));
        assert!(parse(&args(&["eth0", "up"])).unwrap_err().contains("one interface name"));
    }
}
