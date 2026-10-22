//! Application-level inbound firewall: a pure model that turns per-app inbound policies into one nftables ruleset.
//!
//! Each app has a policy: `allow` (its listening ports are accepted) or `block` (its listening ports are dropped). An
//! nftables rule can only match a port, not a process, so the policy is applied to the ports the app declares. A port
//! that two apps claim with opposite policies is a conflict and is refused rather than guessed. Default policy is
//! accept: the firewall only drops what a user blocked.
//!
//! Two machine-wide modes sit beside the per-app choices ([`Modes`]): block all incoming connections (every new inbound
//! connection is dropped whatever an app chose, while replies to what this machine started, loopback, IPv6 neighbour
//! discovery and DHCP keep working) and stealth (echo requests, "ping", get no answer). They are `@mode` lines of the
//! policy file.
//!
//! The ruleset is text for `nft -f`. It declares its own table, so applying it again replaces the previous one. App ids
//! and comments are restricted to a safe character set, so no input can break out of the rule syntax.

use std::collections::BTreeMap;

/// The nftables table the ruleset owns.
pub const TABLE: &str = "bifrost_app_firewall";

/// One port an app listens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Port {
    pub proto: Proto,
    pub number: u16,
}

impl Port {
    /// The port as the policy file and the service write it, `tcp/80`.
    pub fn text(&self) -> String {
        format!("{}/{}", self.proto.keyword(), self.number)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    fn keyword(self) -> &'static str {
        match self {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
        }
    }
}

/// What the user chose for an app's inbound traffic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Allow,
    Block,
}

/// One app's inbound policy: its id, the choice, and the ports it listens on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPolicy {
    pub app_id: String,
    pub policy: Policy,
    pub ports: Vec<Port>,
}

/// The machine-wide firewall modes. Both are off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Every new inbound connection is dropped, whatever the apps chose.
    pub block_all: bool,
    /// ICMP and ICMPv6 echo requests are dropped, so the machine does not answer a ping.
    pub stealth: bool,
}

impl Modes {
    /// The `@mode` lines that write these modes in the policy file (none when both are off).
    pub fn lines(&self) -> String {
        let mut out = String::new();
        if self.block_all {
            out.push_str("@mode block-all\n");
        }
        if self.stealth {
            out.push_str("@mode stealth\n");
        }
        out
    }
}

/// Why a set of policies cannot be turned into a ruleset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The app id is empty, too long, or has a character outside `a-z 0-9 . - _`.
    BadAppId(String),
    /// A port number is zero.
    BadPort { app_id: String, number: u16 },
    /// The same port is claimed by two apps with opposite policies.
    Conflict { number: u16, allowed_by: String, blocked_by: String },
}

const MAX_APP_ID: usize = 128;

/// True for an app id made only of `a-z 0-9 . - _`, one to 128 characters long.
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_APP_ID && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
}

/// Builds the nftables ruleset for `policies`. Blocked ports become drop rules, each commented with the app id. Allowed
/// ports need no rule (the default is accept), so they only take part in conflict checks. Output is deterministic:
/// ports are emitted in sorted order.
pub fn ruleset(policies: &[AppPolicy]) -> Result<String, Error> {
    ruleset_with(policies, Modes::default())
}

/// [`ruleset`] with the machine-wide `modes`. After the per-app drops come, in this order: loopback accept (when a mode
/// is on), the echo-request drops (stealth), then replies to what this machine started, IPv6 neighbour discovery and
/// DHCP replies, and a last drop of every other new connection (block all).
pub fn ruleset_with(policies: &[AppPolicy], modes: Modes) -> Result<String, Error> {
    // port -> (proto, owning app id) for allowed and blocked apps, so a shared port with opposite policies is reported.
    let mut allowed: BTreeMap<Port, String> = BTreeMap::new();
    let mut blocked: BTreeMap<Port, String> = BTreeMap::new();
    for p in policies {
        if !valid_app_id(&p.app_id) {
            return Err(Error::BadAppId(p.app_id.clone()));
        }
        for port in &p.ports {
            if port.number == 0 {
                return Err(Error::BadPort { app_id: p.app_id.clone(), number: 0 });
            }
            match p.policy {
                Policy::Allow => allowed.entry(*port).or_insert_with(|| p.app_id.clone()),
                Policy::Block => blocked.entry(*port).or_insert_with(|| p.app_id.clone()),
            };
        }
    }
    for (port, blocker) in &blocked {
        if let Some(allower) = allowed.iter().find(|(a, _)| a.number == port.number).map(|(_, id)| id) {
            return Err(Error::Conflict { number: port.number, allowed_by: allower.clone(), blocked_by: blocker.clone() });
        }
    }

    let mut out = String::new();
    out.push_str(&format!("add table inet {TABLE}\ndelete table inet {TABLE}\n"));
    out.push_str(&format!("table inet {TABLE} {{\n\tchain input {{\n\t\ttype filter hook input priority 0; policy accept;\n"));
    for (port, app) in &blocked {
        out.push_str(&format!("\t\t{} dport {} drop comment \"{}\"\n", port.proto.keyword(), port.number, app));
    }
    if modes.block_all || modes.stealth {
        out.push_str("\t\tiifname \"lo\" accept\n");
    }
    if modes.stealth {
        out.push_str("\t\ticmp type echo-request drop\n\t\ticmpv6 type echo-request drop\n");
    }
    if modes.block_all {
        out.push_str("\t\tct state established,related accept\n");
        out.push_str("\t\ticmpv6 type { nd-router-solicit, nd-router-advert, nd-neighbor-solicit, nd-neighbor-advert } accept\n");
        out.push_str("\t\tudp dport { 68, 546 } accept\n");
        out.push_str("\t\tct state new drop\n");
    }
    out.push_str("\t}\n}\n");
    Ok(out)
}

/// The policy file the settings page writes and the boot service reads: one app per line as
/// `<app id> <allow|block> <ports>`, where ports are `tcp/<n>` or `udp/<n>` separated by commas, or `-` for none. Blank
/// lines and lines starting with `#` are skipped.
pub fn parse_policies(text: &str) -> Result<Vec<AppPolicy>, String> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        let at = |msg: String| format!("line {}: {msg}", index + 1);
        let mut fields = line.split_whitespace();
        let app_id = fields.next().ok_or_else(|| at("no app id".into()))?.to_string();
        let policy = match fields.next() {
            Some("allow") => Policy::Allow,
            Some("block") => Policy::Block,
            other => return Err(at(format!("policy must be allow or block, not {}", other.unwrap_or("nothing")))),
        };
        let ports_field = fields.next().unwrap_or("-");
        if fields.next().is_some() {
            return Err(at("too many fields".into()));
        }
        let mut ports = Vec::new();
        if ports_field != "-" {
            for item in ports_field.split(',') {
                ports.push(parse_port(item).map_err(|e| at(e))?);
            }
        }
        out.push(AppPolicy { app_id, policy, ports });
    }
    Ok(out)
}

/// The boot step as pure logic: reads a policy file and returns the ruleset text to feed to `nft -f -`. Any error,
/// including a conflict, comes back as text so the service can log it and leave the current rules in place.
pub fn plan_from_file(text: &str) -> Result<String, String> {
    let policies = parse_policies(text)?;
    let modes = parse_modes(text)?;
    ruleset_with(&policies, modes).map_err(|e| format!("{e:?}"))
}

/// The modes a policy file names: `@mode block-all` and `@mode stealth` lines, each switching its mode on. Other lines
/// are not looked at here ([`parse_policies`] reads them). An unknown mode is an error naming its line.
pub fn parse_modes(text: &str) -> Result<Modes, String> {
    let mut modes = Modes::default();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if !line.starts_with('@') {
            continue;
        }
        let at = |msg: String| format!("line {}: {msg}", index + 1);
        let mut fields = line.split_whitespace();
        match (fields.next(), fields.next(), fields.next()) {
            (Some("@mode"), Some("block-all"), None) => modes.block_all = true,
            (Some("@mode"), Some("stealth"), None) => modes.stealth = true,
            (Some("@mode"), Some(other), None) => return Err(at(format!("unknown mode {other}"))),
            _ => return Err(at("a directive reads @mode block-all or @mode stealth".into())),
        }
    }
    Ok(modes)
}

/// The whole policy file: the `@mode` lines, then [`format_policies`].
pub fn format_file(policies: &[AppPolicy], modes: Modes) -> String {
    format!("{}{}", modes.lines(), format_policies(policies))
}

/// One port from `tcp/<n>` or `udp/<n>`.
fn parse_port(text: &str) -> Result<Port, String> {
    let (proto, number) = text.split_once('/').ok_or_else(|| format!("port {text} needs a protocol, tcp/ or udp/"))?;
    let proto = match proto {
        "tcp" => Proto::Tcp,
        "udp" => Proto::Udp,
        _ => return Err(format!("unknown protocol {proto}")),
    };
    let number: u16 = number.parse().map_err(|_| format!("port number {number} is not 0 to 65535"))?;
    Ok(Port { proto, number })
}

/// The policy file text for `policies`, in the form [`parse_policies`] reads. An app with no ports is written with `-`.
pub fn format_policies(policies: &[AppPolicy]) -> String {
    let mut out = String::new();
    for p in policies {
        let policy = match p.policy {
            Policy::Allow => "allow",
            Policy::Block => "block",
        };
        let ports = if p.ports.is_empty() {
            "-".to_string()
        } else {
            p.ports.iter().map(|port| format!("{}/{}", port.proto.keyword(), port.number)).collect::<Vec<_>>().join(",")
        };
        out.push_str(&format!("{} {policy} {ports}\n", p.app_id));
    }
    out
}

/// The script that removes the firewall's table (and so every rule it added). Safe to run when the table is absent.
pub fn clear_script() -> String {
    format!("add table inet {TABLE}\ndelete table inet {TABLE}\n")
}

/// Sets the policy of one app, replacing its earlier entry. The change is refused, and `policies` left as it was,
/// when the id or a port is invalid or a port would then be both allowed and blocked.
pub fn set_policy(policies: &mut Vec<AppPolicy>, new: AppPolicy) -> Result<(), Error> {
    let before = policies.clone();
    match policies.iter_mut().find(|p| p.app_id == new.app_id) {
        Some(existing) => *existing = new,
        None => policies.push(new),
    }
    if let Err(e) = ruleset(policies) {
        *policies = before;
        return Err(e);
    }
    Ok(())
}

/// Drops the policy of `app_id`; true when there was one.
pub fn remove_policy(policies: &mut Vec<AppPolicy>, app_id: &str) -> bool {
    let before = policies.len();
    policies.retain(|p| p.app_id != app_id);
    policies.len() != before
}

/// Ports from a comma-separated list of `tcp/<n>` or `udp/<n>`; `-` or an empty text is no ports.
pub fn parse_port_list(text: &str) -> Result<Vec<Port>, String> {
    if text.is_empty() || text == "-" {
        return Ok(Vec::new());
    }
    text.split(',').map(parse_port).collect()
}

/// The first port the kernel hands to a connecting socket (`ip_local_port_range`, 32768 by default). A UDP socket bound
/// below it chose its port; one above it is almost always a client, which waits for no one.
pub const EPHEMERAL_START: u16 = 32768;

/// One socket that waits for incoming traffic: its port and the socket inode that names its owner under `/proc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listening {
    pub port: Port,
    pub inode: u64,
}

/// True for a local address text from `/proc/net/{tcp,udp}` (8 hex digits) or `{tcp6,udp6}` (32) that only the machine
/// itself can reach: 127.0.0.0/8, `::1`, or the IPv4-mapped form of 127.0.0.0/8. The kernel prints each 32-bit word
/// in host order, so on a little-endian machine the first octet is the last two digits of the word.
fn loopback_address(hex: &str) -> bool {
    match hex.len() {
        8 => hex.ends_with("7F"),
        32 => hex == "00000000000000000000000001000000" || (hex.starts_with("0000000000000000FFFF0000") && hex.ends_with("7F")),
        _ => false,
    }
}

/// The sockets in the text of `/proc/net/tcp`, `tcp6`, `udp` or `udp6` (`proto` says which) that wait for traffic from
/// elsewhere. TCP counts the sockets in the LISTEN state; UDP the unconnected sockets bound to a port below
/// [`EPHEMERAL_START`]. A socket bound only to the loopback address is left out, as is a line that does not parse.
pub fn parse_listeners(text: &str, proto: Proto) -> Vec<Listening> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 {
            continue;
        }
        let Some((addr, port)) = f[1].split_once(':') else { continue };
        let Ok(number) = u16::from_str_radix(port, 16) else { continue };
        let Ok(inode) = f[9].parse::<u64>() else { continue };
        let waiting = match proto {
            Proto::Tcp => f[3] == "0A",
            Proto::Udp => f[3] == "07" && number < EPHEMERAL_START,
        };
        if waiting && number != 0 && inode != 0 && !loopback_address(addr) {
            out.push(Listening { port: Port { proto, number }, inode });
        }
    }
    out
}

/// The inode in the target of a `/proc/<pid>/fd/<n>` link (`socket:[12345]`); None for any other kind of file.
pub fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

/// The ports as the service writes them (`tcp/80`), comma separated.
pub fn format_ports(ports: &[Port]) -> String {
    ports.iter().map(Port::text).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0000000000000000 100 0 0 10 0
   1: 0100007F:0277 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 22222 1 0000000000000000 100 0 0 10 0
   2: 0100A8C0:C350 0100A8C0:01BB 01 00000000:00000000 00:00000000 00000000  1000        0 33333 1 0000000000000000 100 0 0 10 0
   3: 0100A8C0:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 44444 1 0000000000000000 100 0 0 10 0
";
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:1F91 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 55555 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000001000000:0CEA 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 66666 1 0000000000000000 100 0 0 10 0
   2: 0000000000000000FFFF00000100007F:0CEB 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 77777 1 0000000000000000 100 0 0 10 0
";
    const UDP: &str = "   sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
  100: 00000000:14E9 00000000:0000 07 00000000:00000000 00:00000000 00000000  1000        0 88888 2 0000000000000000 0
  101: 00000000:E000 00000000:0000 07 00000000:00000000 00:00000000 00000000  1000        0 99999 2 0000000000000000 0
  102: 0100A8C0:0044 0100A8C0:0035 01 00000000:00000000 00:00000000 00000000  1000        0 10101 2 0000000000000000 0
";

    #[test]
    fn tcp_listeners_skip_loopback_and_connected_sockets() {
        let got = parse_listeners(TCP, Proto::Tcp);
        assert_eq!(got.iter().map(|l| (l.port.number, l.inode)).collect::<Vec<_>>(), vec![(8080, 11111), (80, 44444)]);
        assert!(got.iter().all(|l| l.port.proto == Proto::Tcp));
    }

    #[test]
    fn tcp6_listeners_skip_the_loopback_forms() {
        let got = parse_listeners(TCP6, Proto::Tcp);
        assert_eq!(got.iter().map(|l| (l.port.number, l.inode)).collect::<Vec<_>>(), vec![(8081, 55555)]);
    }

    #[test]
    fn udp_listeners_are_the_unconnected_sockets_on_a_chosen_port() {
        let got = parse_listeners(UDP, Proto::Udp);
        assert_eq!(got.iter().map(|l| (l.port.number, l.inode)).collect::<Vec<_>>(), vec![(5353, 88888)], "an ephemeral client port and a connected socket wait for no one");
    }

    #[test]
    fn bad_lines_are_skipped_and_socket_links_parse() {
        assert!(parse_listeners("header\nnot a line\n 0: zz:zz 0:0 0A a b c d e f g\n", Proto::Tcp).is_empty());
        assert_eq!(socket_inode("socket:[12345]"), Some(12345));
        assert_eq!(socket_inode("pipe:[12345]"), None);
        assert_eq!(socket_inode("/dev/null"), None);
        assert_eq!(format_ports(&[Port { proto: Proto::Tcp, number: 80 }, Port { proto: Proto::Udp, number: 5353 }]), "tcp/80, udp/5353");
    }

    #[test]
    fn set_policy_replaces_an_app_and_refuses_a_conflict() {
        let mut ps = Vec::new();
        set_policy(&mut ps, app("a.b", Policy::Block, vec![tcp(80)])).unwrap();
        set_policy(&mut ps, app("a.b", Policy::Block, vec![tcp(81)])).unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].ports, vec![tcp(81)]);
        let clash = app("c.d", Policy::Allow, vec![tcp(81)]);
        assert!(matches!(set_policy(&mut ps, clash), Err(Error::Conflict { number: 81, .. })));
        assert_eq!(ps.len(), 1, "a refused change leaves the list as it was");
        assert!(matches!(set_policy(&mut ps, app("BAD", Policy::Allow, vec![])), Err(Error::BadAppId(_))));
        assert!(remove_policy(&mut ps, "a.b"));
        assert!(!remove_policy(&mut ps, "a.b"));
    }

    #[test]
    fn port_lists_parse_and_the_clear_script_names_the_table() {
        assert_eq!(parse_port_list("-").unwrap(), vec![]);
        assert_eq!(parse_port_list("tcp/22,udp/53").unwrap().len(), 2);
        assert!(parse_port_list("22").is_err());
        assert!(clear_script().contains(&format!("delete table inet {TABLE}")));
    }

    fn tcp(n: u16) -> Port {
        Port { proto: Proto::Tcp, number: n }
    }

    fn app(id: &str, policy: Policy, ports: Vec<Port>) -> AppPolicy {
        AppPolicy { app_id: id.to_string(), policy, ports }
    }

    #[test]
    fn app_ids_are_restricted_to_a_safe_set() {
        assert!(valid_app_id("is.olibuijr.notes"));
        assert!(valid_app_id("app-1_x"));
        assert!(!valid_app_id(""));
        assert!(!valid_app_id("Upper.case"));
        assert!(!valid_app_id("a\"; flush ruleset"));
        assert!(!valid_app_id(&"a".repeat(MAX_APP_ID + 1)));
    }

    #[test]
    fn with_no_policies_the_ruleset_only_sets_the_accept_default() {
        let text = ruleset(&[]).unwrap();
        assert!(text.contains("policy accept;"));
        assert!(!text.contains(" drop "), "nothing is dropped without a block policy");
    }

    #[test]
    fn a_blocked_app_drops_each_of_its_ports_in_sorted_order() {
        let text = ruleset(&[app("is.chat", Policy::Block, vec![Port { proto: Proto::Udp, number: 5353 }, tcp(8080)])]).unwrap();
        let tcp_rule = text.find("tcp dport 8080 drop comment \"is.chat\"").expect("tcp rule");
        let udp_rule = text.find("udp dport 5353 drop comment \"is.chat\"").expect("udp rule");
        assert!(tcp_rule < udp_rule, "tcp sorts before udp, so the rules come out in a fixed order");
    }

    #[test]
    fn the_ruleset_replaces_its_own_table_when_applied_again() {
        let text = ruleset(&[]).unwrap();
        assert!(text.starts_with(&format!("add table inet {TABLE}\ndelete table inet {TABLE}\n")));
    }

    #[test]
    fn an_allowed_app_adds_no_drop_rule() {
        let text = ruleset(&[app("is.mail", Policy::Allow, vec![tcp(993)])]).unwrap();
        assert!(!text.contains("dport 993"));
    }

    #[test]
    fn a_port_shared_by_allow_and_block_is_a_conflict() {
        let err = ruleset(&[app("is.a", Policy::Allow, vec![tcp(22)]), app("is.b", Policy::Block, vec![tcp(22)])]).unwrap_err();
        assert_eq!(err, Error::Conflict { number: 22, allowed_by: "is.a".into(), blocked_by: "is.b".into() });
    }

    #[test]
    fn a_bad_app_id_or_zero_port_is_refused() {
        assert_eq!(ruleset(&[app("Bad Id", Policy::Block, vec![])]).unwrap_err(), Error::BadAppId("Bad Id".into()));
        assert_eq!(
            ruleset(&[app("is.a", Policy::Block, vec![tcp(0)])]).unwrap_err(),
            Error::BadPort { app_id: "is.a".into(), number: 0 }
        );
    }

    #[test]
    fn the_policy_file_round_trips() {
        let policies = vec![app("is.chat", Policy::Block, vec![Port { proto: Proto::Udp, number: 5353 }, tcp(8080)]), app("is.none", Policy::Allow, vec![])];
        let text = format_policies(&policies);
        assert_eq!(text, "is.chat block udp/5353,tcp/8080\nis.none allow -\n");
        assert_eq!(parse_policies(&text).unwrap(), policies);
    }

    #[test]
    fn the_boot_plan_is_the_ruleset_for_the_file() {
        let text = plan_from_file("is.chat block tcp/8080\n").unwrap();
        assert!(text.contains("tcp dport 8080 drop comment \"is.chat\""));
        assert_eq!(plan_from_file("").unwrap(), ruleset(&[]).unwrap());
    }

    #[test]
    fn a_bad_file_or_a_conflict_gives_no_plan() {
        assert!(plan_from_file("is.a block tcp/x\n").unwrap_err().starts_with("line 1:"));
        let conflict = plan_from_file("is.a allow tcp/22\nis.b block tcp/22\n").unwrap_err();
        assert!(conflict.starts_with("Conflict"), "{conflict}");
    }

    #[test]
    fn the_policy_file_skips_comments_and_blank_lines() {
        let text = "# managed by settings\n\nis.mail allow tcp/993\n";
        assert_eq!(parse_policies(text).unwrap(), vec![app("is.mail", Policy::Allow, vec![tcp(993)])]);
    }

    #[test]
    fn modes_are_off_by_default_and_the_default_ruleset_is_unchanged() {
        assert_eq!(Modes::default(), Modes { block_all: false, stealth: false });
        assert_eq!(ruleset_with(&[], Modes::default()).unwrap(), ruleset(&[]).unwrap());
        assert!(!ruleset(&[]).unwrap().contains("ct state"));
    }

    #[test]
    fn stealth_drops_echo_requests_but_keeps_loopback() {
        let text = ruleset_with(&[], Modes { block_all: false, stealth: true }).unwrap();
        let lo = text.find("iifname \"lo\" accept").expect("loopback accept");
        let ping = text.find("icmp type echo-request drop").expect("ping drop");
        assert!(lo < ping, "loopback is let through before the drop");
        assert!(text.contains("icmpv6 type echo-request drop"));
        assert!(!text.contains("ct state new drop"), "stealth alone does not block connections");
    }

    #[test]
    fn block_all_keeps_replies_loopback_discovery_and_dhcp_then_drops_new_connections() {
        let text = ruleset_with(&[app("is.chat", Policy::Block, vec![tcp(8080)])], Modes { block_all: true, stealth: false }).unwrap();
        let positions: Vec<usize> = [
            "tcp dport 8080 drop",
            "iifname \"lo\" accept",
            "ct state established,related accept",
            "nd-neighbor-advert } accept",
            "udp dport { 68, 546 } accept",
            "ct state new drop",
        ]
        .iter()
        .map(|rule| text.find(rule).unwrap_or_else(|| panic!("missing {rule}")))
        .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "the rules come in a fixed order: {positions:?}");
        assert!(!text.contains("echo-request"), "stealth is its own mode");
    }

    #[test]
    fn both_modes_together_drop_pings_before_the_final_drop() {
        let text = ruleset_with(&[], Modes { block_all: true, stealth: true }).unwrap();
        assert!(text.find("icmp type echo-request drop").unwrap() < text.find("ct state new drop").unwrap());
    }

    #[test]
    fn mode_lines_are_read_written_and_skipped_by_the_policy_parser() {
        let text = "@mode block-all\n# note\n@mode stealth\nis.mail allow tcp/993\n";
        assert_eq!(parse_modes(text).unwrap(), Modes { block_all: true, stealth: true });
        assert_eq!(parse_policies(text).unwrap(), vec![app("is.mail", Policy::Allow, vec![tcp(993)])]);
        assert_eq!(parse_modes("is.mail allow tcp/993\n").unwrap(), Modes::default());
        let policies = vec![app("is.mail", Policy::Allow, vec![tcp(993)])];
        let modes = Modes { block_all: true, stealth: true };
        assert_eq!(format_file(&policies, modes), "@mode block-all\n@mode stealth\nis.mail allow tcp/993\n");
        assert_eq!(parse_modes(&format_file(&policies, modes)).unwrap(), modes);
        assert_eq!(parse_policies(&format_file(&policies, modes)).unwrap(), policies);
        assert_eq!(format_file(&policies, Modes::default()), format_policies(&policies), "no modes, no extra lines");
    }

    #[test]
    fn a_bad_mode_line_names_its_line_and_the_plan_refuses_it() {
        assert_eq!(parse_modes("is.a allow -\n@mode loud\n").unwrap_err(), "line 2: unknown mode loud");
        assert!(parse_modes("@mode\n").unwrap_err().starts_with("line 1:"));
        assert!(parse_modes("@mode stealth extra\n").unwrap_err().starts_with("line 1:"));
        assert!(parse_modes("@frob stealth\n").unwrap_err().starts_with("line 1:"));
        assert!(plan_from_file("@mode loud\n").is_err());
        assert!(plan_from_file("@mode block-all\n").unwrap().contains("ct state new drop"), "the boot plan carries the modes");
    }

    #[test]
    fn a_bad_policy_file_line_names_its_line() {
        assert!(parse_policies("is.a maybe -\n").unwrap_err().starts_with("line 1: policy must be allow or block"));
        assert!(parse_policies("is.a block tcp/x\n").unwrap_err().starts_with("line 1: port number x"));
        assert!(parse_policies("is.a block 22\n").unwrap_err().contains("needs a protocol"));
        assert!(parse_policies("is.a block sctp/22\n").unwrap_err().contains("unknown protocol sctp"));
        assert!(parse_policies("is.a block tcp/22 extra\n").unwrap_err().contains("too many fields"));
        assert!(parse_policies("ok block -\n\nis.b\n").unwrap_err().starts_with("line 3:"));
    }
}
