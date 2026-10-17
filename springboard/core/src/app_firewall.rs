//! Application-level inbound firewall: a pure model that turns per-app inbound policies into one nftables ruleset.
//!
//! Each app has a policy: `allow` (its listening ports are accepted) or `block` (its listening ports are dropped). An
//! nftables rule can only match a port, not a process, so the policy is applied to the ports the app declares. A port
//! that two apps claim with opposite policies is a conflict and is refused rather than guessed. Default policy is
//! accept: the firewall only drops what a user blocked.
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
        if line.is_empty() || line.starts_with('#') {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_policy_file_skips_comments_and_blank_lines() {
        let text = "# managed by settings\n\nis.mail allow tcp/993\n";
        assert_eq!(parse_policies(text).unwrap(), vec![app("is.mail", Policy::Allow, vec![tcp(993)])]);
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
