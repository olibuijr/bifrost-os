//! The neighbour (ARP and IPv6 neighbour discovery) table, read from `ip neigh show`, shared by `arp` and `ndp`.
//! Each line of `ip` output becomes a `Neighbor`; `arp` prints the IPv4 table in the reference tool's `? (addr) at mac on
//! iface` shape, and `ndp` prints the IPv6 table as a column list. Gap: names are never resolved (the address is shown
//! as `?`), `-d` and `-s` (add or delete an entry) are refused, and the reference tools' expiry times are not shown.

use std::process::Command;

pub const ARP_USAGE: &str = "usage: arp [-n] -a | host";
pub const NDP_USAGE: &str = "usage: ndp [-n] -a | host";

/// The neighbour states `ip` reports. A line ends with one of them when it has one.
const STATES: &[&str] = &["PERMANENT", "NOARP", "REACHABLE", "STALE", "DELAY", "PROBE", "INCOMPLETE", "FAILED"];

/// One entry of the neighbour table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbor {
    pub address: String,
    /// The link-layer (MAC) address, when the entry has one.
    pub mac: Option<String>,
    pub iface: String,
    /// The state word `ip` printed (for example `REACHABLE`), or empty when it printed none.
    pub state: String,
}

/// What the command line asks for: the help text, or the table (`all`) or one host's entry.
#[derive(Debug, PartialEq, Eq)]
pub enum Query {
    Help,
    List { all: bool, host: Option<String> },
}

/// Reads `arp` or `ndp` arguments. `-a` lists every entry, `-n` is accepted (names are never resolved), and a bare
/// address shows that host's entry. Options other than these are refused.
pub fn parse_query(args: &[String], usage: &str) -> Result<Query, String> {
    let mut all = false;
    let mut host: Option<String> = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Query::Help),
            "-a" => all = true,
            "-n" => {}
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("option {other} is not supported yet\n{usage}"));
            }
            other => {
                if host.replace(other.to_string()).is_some() {
                    return Err(format!("only one host can be given\n{usage}"));
                }
            }
        }
    }
    if !all && host.is_none() {
        return Err(usage.to_string());
    }
    Ok(Query::List { all, host })
}

/// Reads the neighbour lines of one family from `ip` (`family` is `"-4"` or `"-6"`).
pub fn read_table(family: &str) -> Result<String, String> {
    let out = Command::new("ip")
        .args([family, "neigh", "show"])
        .output()
        .map_err(|e| format!("ip could not run: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Reads the table text `ip neigh show` prints. A line with no address or no device is skipped.
pub fn parse(text: &str) -> Vec<Neighbor> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Neighbor> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let address = (*tokens.first()?).to_string();
    let iface = value_after(&tokens, "dev")?.to_string();
    let mac = value_after(&tokens, "lladdr").map(str::to_string);
    let state = tokens.last().filter(|t| STATES.contains(*t)).map(|t| t.to_string()).unwrap_or_default();
    Some(Neighbor { address, mac, iface, state })
}

fn value_after<'a>(tokens: &[&'a str], key: &str) -> Option<&'a str> {
    let at = tokens.iter().position(|t| *t == key)?;
    tokens.get(at + 1).copied()
}

/// One line of the `arp -a` listing: `? (192.168.1.1) at aa:bb:cc:dd:ee:ff on wlan0 [ethernet]`, or `(incomplete)`
/// in place of the address when the entry has no link-layer address.
pub fn arp_line(n: &Neighbor) -> String {
    let mac = n.mac.as_deref().unwrap_or("(incomplete)");
    format!("? ({}) at {} on {} [ethernet]", n.address, mac, n.iface)
}

/// The column heading of the `ndp -a` listing.
pub const NDP_HEADER: &str = "Neighbor                                 Linklayer Address   Netif St";

/// One row of the `ndp -a` listing, in the columns of `NDP_HEADER`. The state is shown as its one-letter code.
pub fn ndp_line(n: &Neighbor) -> String {
    let mac = n.mac.as_deref().unwrap_or("(incomplete)");
    format!("{:<40}{:<20}{:<6}{}", n.address, mac, n.iface, state_letter(&n.state))
}

fn state_letter(state: &str) -> char {
    match state {
        "PERMANENT" | "NOARP" => 'P',
        "REACHABLE" => 'R',
        "STALE" => 'S',
        "DELAY" => 'D',
        "PROBE" => 'p',
        "INCOMPLETE" => 'I',
        "FAILED" => 'F',
        _ => '?',
    }
}

/// The entries that match the query: all of them, or only the one for `host`.
pub fn select<'a>(table: &'a [Neighbor], host: Option<&str>) -> Vec<&'a Neighbor> {
    table.iter().filter(|n| host.map_or(true, |h| n.address == h)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const V4: &str = "192.168.1.1 dev wlan0 lladdr aa:bb:cc:dd:ee:01 REACHABLE\n\
                      192.168.1.7 dev wlan0 FAILED\n\
                      \n\
                      10.0.0.2 dev eth0 lladdr aa:bb:cc:dd:ee:02 STALE\n";

    #[test]
    fn queries_read_all_or_one_host() {
        assert_eq!(parse_query(&args(&["-a"]), ARP_USAGE), Ok(Query::List { all: true, host: None }));
        assert_eq!(parse_query(&args(&["-n", "10.0.0.2"]), ARP_USAGE), Ok(Query::List { all: false, host: Some("10.0.0.2".into()) }));
        assert_eq!(parse_query(&args(&["-h"]), ARP_USAGE), Ok(Query::Help));
    }

    #[test]
    fn queries_refuse_missing_or_unsupported_arguments() {
        assert_eq!(parse_query(&args(&[]), NDP_USAGE), Err(NDP_USAGE.to_string()));
        assert!(parse_query(&args(&["-d", "10.0.0.2"]), ARP_USAGE).unwrap_err().contains("-d"));
        assert!(parse_query(&args(&["a", "b"]), ARP_USAGE).unwrap_err().contains("only one host"));
    }

    #[test]
    fn table_lines_are_read_with_device_mac_and_state() {
        let table = parse(V4);
        assert_eq!(table.len(), 3);
        assert_eq!(table[0].address, "192.168.1.1");
        assert_eq!(table[0].mac.as_deref(), Some("aa:bb:cc:dd:ee:01"));
        assert_eq!(table[0].iface, "wlan0");
        assert_eq!(table[0].state, "REACHABLE");
        assert_eq!(table[1].mac, None);
        assert_eq!(table[1].state, "FAILED");
    }

    #[test]
    fn ipv6_lines_with_router_flags_keep_their_state() {
        let line = "fe80::1 dev eth0 lladdr aa:bb:cc:dd:ee:03 router STALE";
        let n = parse(line).remove(0);
        assert_eq!(n.address, "fe80::1");
        assert_eq!(n.mac.as_deref(), Some("aa:bb:cc:dd:ee:03"));
        assert_eq!(n.state, "STALE");
    }

    #[test]
    fn arp_lines_use_the_listing_shape() {
        let table = parse(V4);
        assert_eq!(arp_line(&table[0]), "? (192.168.1.1) at aa:bb:cc:dd:ee:01 on wlan0 [ethernet]");
        assert_eq!(arp_line(&table[1]), "? (192.168.1.7) at (incomplete) on wlan0 [ethernet]");
    }

    #[test]
    fn ndp_rows_show_a_one_letter_state() {
        let n = parse("fe80::1 dev eth0 lladdr aa:bb:cc:dd:ee:03 router STALE").remove(0);
        let row = ndp_line(&n);
        assert!(row.starts_with("fe80::1"));
        assert!(row.contains("aa:bb:cc:dd:ee:03"));
        assert!(row.ends_with("eth0  S"));
    }

    #[test]
    fn a_host_selects_its_own_entry() {
        let table = parse(V4);
        assert_eq!(select(&table, None).len(), 3);
        let one = select(&table, Some("10.0.0.2"));
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].iface, "eth0");
        assert!(select(&table, Some("10.9.9.9")).is_empty());
    }
}
