//! `ipconfig`: asks for the IP configuration of an interface. Read-only.
//!
//! Subcommands: `getifaddr IF` (the IPv4 address), `getv6address IF` (the first global IPv6 address), `ifcount` (how
//! many interfaces there are), and `getoption IF NAME` for `subnet_mask`, `router` (the default gateway, from
//! `/proc/net/route`) and `domain_name_server` (the resolver addresses in `/etc/resolv.conf`). The values come from
//! the same readers `ifconfig` uses (`bifrost_cmd::ifconfig`). `set`, `waitall`, `getpacket` and the rest need the
//! DHCP client's state and are refused with a message.

use crate::ifconfig::{Inet4, Inet6};
use std::net::Ipv4Addr;

pub const USAGE: &str = "usage: ipconfig getifaddr IF | getv6address IF | ifcount | getoption IF subnet_mask|router|domain_name_server";
pub const ROUTE_PATH: &str = "/proc/net/route";
pub const RESOLV_PATH: &str = "/etc/resolv.conf";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    GetIfAddr(String),
    GetV6Address(String),
    IfCount,
    GetOption(String, String),
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Action, String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["-h" | "-help" | "--help" | "help"] => Ok(Action::Help),
        ["getifaddr", iface] => Ok(Action::GetIfAddr(iface.to_string())),
        ["getv6address", iface] => Ok(Action::GetV6Address(iface.to_string())),
        ["ifcount"] => Ok(Action::IfCount),
        ["getoption", iface, name] => Ok(Action::GetOption(iface.to_string(), name.to_string())),
        ["getifaddr" | "getv6address" | "getoption", ..] => Err("missing or extra arguments".to_string()),
        [other, ..] => Err(format!("{other} is not supported: this ipconfig only reads the configuration")),
    }
}

/// The default gateway of `iface` in `/proc/net/route` text: a row with destination 0 and the gateway flag, the
/// gateway a little-endian hex address.
pub fn parse_router(route: &str, iface: &str) -> Option<Ipv4Addr> {
    route.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 || f[0] != iface || f[1] != "00000000" {
            return None;
        }
        let flags = u32::from_str_radix(f[3], 16).ok()?;
        if flags & 0x2 == 0 {
            return None;
        }
        let gw = u32::from_str_radix(f[2], 16).ok()?;
        (gw != 0).then(|| Ipv4Addr::from(gw.swap_bytes()))
    })
}

/// The `nameserver` addresses of `resolv.conf` text, in order.
pub fn parse_nameservers(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            if w.next() == Some("nameserver") {
                w.next().map(str::to_string)
            } else {
                None
            }
        })
        .collect()
}

/// The first global-scope IPv6 address of `iface`.
pub fn global_v6(iface: &str, inet6: &[Inet6]) -> Option<String> {
    inet6.iter().find(|a| a.iface == iface && a.scope == 0).map(|a| a.addr.to_string())
}

/// The value of one `getoption` name for an interface. `Ok(None)` when the interface has none.
pub fn option_value(name: &str, inet4: Option<&Inet4>, route: &str, iface: &str, resolv: &str) -> Result<Option<String>, String> {
    match name {
        "subnet_mask" => Ok(inet4.map(|v| v.netmask.to_string())),
        "router" => Ok(parse_router(route, iface).map(|a| a.to_string())),
        "domain_name_server" => {
            let servers = parse_nameservers(resolv);
            Ok((!servers.is_empty()).then(|| servers.join(" ")))
        }
        other => Err(format!("option {other} is not supported (subnet_mask, router, domain_name_server)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const ROUTE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
wlan0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n";

    #[test]
    fn subcommands_parse_and_unsupported_ones_are_refused() {
        assert_eq!(parse(&args(&["getifaddr", "en0"])), Ok(Action::GetIfAddr("en0".into())));
        assert_eq!(parse(&args(&["ifcount"])), Ok(Action::IfCount));
        assert_eq!(parse(&args(&["getoption", "en0", "router"])), Ok(Action::GetOption("en0".into(), "router".into())));
        assert_eq!(parse(&args(&[])), Ok(Action::Help));
        assert!(parse(&args(&["waitall"])).unwrap_err().contains("waitall is not supported"));
        assert!(parse(&args(&["getifaddr"])).is_err());
    }

    #[test]
    fn the_router_is_the_default_route_gateway_of_the_interface() {
        assert_eq!(parse_router(ROUTE, "wlan0"), Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(parse_router(ROUTE, "eth0"), None);
        assert_eq!(parse_router("Iface\n", "wlan0"), None);
    }

    #[test]
    fn nameservers_come_from_resolv_conf_in_order() {
        let text = "# generated\nsearch lan\nnameserver 192.168.1.1\nnameserver 9.9.9.9\noptions edns0\n";
        assert_eq!(parse_nameservers(text), ["192.168.1.1", "9.9.9.9"]);
        assert!(parse_nameservers("search lan\n").is_empty());
    }

    #[test]
    fn options_answer_from_the_interface_state_and_unknown_names_are_refused() {
        let v4 = Inet4 { addr: Ipv4Addr::new(192, 168, 1, 7), netmask: Ipv4Addr::new(255, 255, 255, 0), broadcast: None };
        assert_eq!(option_value("subnet_mask", Some(&v4), ROUTE, "wlan0", "").unwrap(), Some("255.255.255.0".into()));
        assert_eq!(option_value("subnet_mask", None, ROUTE, "wlan0", "").unwrap(), None);
        assert_eq!(option_value("router", Some(&v4), ROUTE, "wlan0", "").unwrap(), Some("192.168.1.1".into()));
        assert_eq!(option_value("domain_name_server", None, ROUTE, "wlan0", "nameserver 1.1.1.1\n").unwrap(), Some("1.1.1.1".into()));
        assert!(option_value("lease_time", None, ROUTE, "wlan0", "").is_err());
    }

    #[test]
    fn the_global_v6_address_skips_link_local_ones() {
        let inet6 = crate::ifconfig::parse_inet6(
            "fe80000000000000021122fffe334455 02 40 20 80 eth0\n20010db8000000000000000000000007 02 40 00 80 eth0\n",
        );
        assert_eq!(global_v6("eth0", &inet6), Some("2001:db8::7".into()));
        assert_eq!(global_v6("wlan0", &inet6), None);
    }
}
