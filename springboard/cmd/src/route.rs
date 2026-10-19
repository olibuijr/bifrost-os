//! `route`: prints the IPv4 routing table in the numeric `route -n` layout, read from the kernel's `/proc/net/route`.
//! See `src/bin/route.rs` for the command.
//! Supported: `route` and `route -n` (the table; this command always prints addresses, never host names).
//! Refused with a message: `get`, `add`, `delete`, `change`, `flush` and every other option. Changing the table needs the
//! netlink routing service, which the desktop does not run; `ip route` covers those jobs.

use std::fmt::Write as _;
use std::net::Ipv4Addr;

pub const USAGE: &str = "usage: route [-n]";
/// The kernel's table, one line per route after a header line.
pub const PROC_PATH: &str = "/proc/net/route";

const RTF_UP: u32 = 0x1;
const RTF_GATEWAY: u32 = 0x2;
const RTF_HOST: u32 = 0x4;

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    Show,
}

/// Reads the arguments: nothing or `-n` shows the table.
pub fn parse(args: &[String]) -> Result<Action, String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["-n"] => Ok(Action::Show),
        ["-h" | "-help" | "--help"] => Ok(Action::Help),
        [first, ..] if matches!(*first, "get" | "add" | "delete" | "change" | "flush") => {
            Err(format!("route {first} is not supported yet; route here lists the table with -n, and ip route changes it"))
        }
        [other, ..] => Err(format!("option {other} is not supported yet; route here lists the table with -n")),
    }
}

/// One line of the kernel's routing table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub iface: String,
    pub destination: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub flags: u32,
    pub metric: u32,
    pub refcnt: u32,
    pub use_count: u32,
}

/// The kernel writes addresses as the native `u32` of their bytes, so on a little-endian machine the hex value's
/// little-endian bytes are the address's octets in order.
fn address(hex: &str) -> Option<Ipv4Addr> {
    u32::from_str_radix(hex, 16).ok().map(|v| Ipv4Addr::from(v.to_le_bytes()))
}

/// Parses the text of `/proc/net/route`. The header and any malformed line are skipped.
pub fn parse_table(text: &str) -> Vec<Route> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 8 {
                return None;
            }
            Some(Route {
                iface: f[0].to_string(),
                destination: address(f[1])?,
                gateway: address(f[2])?,
                flags: u32::from_str_radix(f[3], 16).ok()?,
                refcnt: f[4].parse().ok()?,
                use_count: f[5].parse().ok()?,
                metric: f[6].parse().ok()?,
                mask: address(f[7])?,
            })
        })
        .collect()
}

/// The flag letters `route -n` prints: U (up), G (gateway) and H (host route), in that order.
pub fn flags_text(flags: u32) -> String {
    let mut out = String::new();
    if flags & RTF_UP != 0 {
        out.push('U');
    }
    if flags & RTF_GATEWAY != 0 {
        out.push('G');
    }
    if flags & RTF_HOST != 0 {
        out.push('H');
    }
    out
}

/// The table in the `route -n` layout.
pub fn table_text(routes: &[Route]) -> String {
    let mut out = String::from("Kernel IP routing table\nDestination     Gateway         Genmask         Flags Metric Ref    Use Iface\n");
    for r in routes {
        let _ = writeln!(
            out,
            "{:<15} {:<15} {:<15} {:<5} {:<6} {:<4} {:>3} {}",
            r.destination.to_string(),
            r.gateway.to_string(),
            r.mask.to_string(),
            flags_text(r.flags),
            r.metric,
            r.refcnt,
            r.use_count,
            r.iface
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const SAMPLE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
eth0\t00000000\t0201A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
lo\t0100007F\t00000000\t0001\t0\t0\t0\t000000FF\t0\t0\t0\n";

    #[test]
    fn parses_the_table_into_routes() {
        let routes = parse_table(SAMPLE);
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].iface, "eth0");
        assert_eq!(routes[0].destination, Ipv4Addr::new(0, 0, 0, 0));
        assert_eq!(routes[0].gateway, Ipv4Addr::new(192, 168, 1, 2));
        assert_eq!(routes[0].metric, 100);
        assert_eq!(routes[1].mask, Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(routes[2].destination, Ipv4Addr::new(127, 0, 0, 1));
    }

    #[test]
    fn flags_are_u_g_h_in_order() {
        assert_eq!(flags_text(0x3), "UG");
        assert_eq!(flags_text(0x1), "U");
        assert_eq!(flags_text(0x7), "UGH");
        assert_eq!(flags_text(0), "");
    }

    #[test]
    fn the_table_prints_in_route_n_layout() {
        let text = table_text(&parse_table(SAMPLE));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Kernel IP routing table");
        assert!(lines[2].starts_with("0.0.0.0"));
        assert!(lines[2].contains("192.168.1.2"));
        assert!(lines[2].ends_with("eth0"));
        assert!(lines[2].contains(" UG "));
        assert!(lines[4].starts_with("127.0.0.1"));
    }

    #[test]
    fn a_malformed_line_is_skipped() {
        assert_eq!(parse_table("header\neth0\tzz\t0\t0\t0\t0\t0\t0\n").len(), 0);
        assert_eq!(parse_table("").len(), 0);
    }

    #[test]
    fn show_and_help_parse_and_changes_are_refused() {
        assert_eq!(parse(&args(&[])), Ok(Action::Show));
        assert_eq!(parse(&args(&["-n"])), Ok(Action::Show));
        assert_eq!(parse(&args(&["-h"])), Ok(Action::Help));
        assert!(parse(&args(&["get", "default"])).unwrap_err().contains("route get is not supported"));
        assert!(parse(&args(&["add", "default"])).unwrap_err().contains("route add"));
        assert!(parse(&args(&["-f"])).unwrap_err().contains("option -f"));
    }
}
