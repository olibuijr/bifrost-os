//! Ethernet bond and VLAN interfaces as command plans: checks a request (names, ids, modes, members) and returns
//! the `ip` commands that create or remove it, in order. Nothing is run here; the privileged network service runs
//! the plan. Pure std.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BondMode {
    BalanceRr,
    ActiveBackup,
    BalanceXor,
    Broadcast,
    Lacp,
    BalanceTlb,
    BalanceAlb,
}

impl BondMode {
    pub fn kernel_name(self) -> &'static str {
        match self {
            Self::BalanceRr => "balance-rr",
            Self::ActiveBackup => "active-backup",
            Self::BalanceXor => "balance-xor",
            Self::Broadcast => "broadcast",
            Self::Lacp => "802.3ad",
            Self::BalanceTlb => "balance-tlb",
            Self::BalanceAlb => "balance-alb",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    BadName,
    BadVlanId,
    NoMembers,
    TooFewMembers,
    DuplicateMember,
    MemberIsBond,
}

/// A Linux interface name: 1 to 15 characters of letters, digits, `.`, `-`, `_`; no `/` or whitespace; not `.` or `..`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name != "."
        && name != ".."
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub type Command = Vec<String>;

fn cmd(parts: &[&str]) -> Command {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Creates a VLAN `name` (tag 1-4094) on `parent`, brought up.
pub fn add_vlan(parent: &str, id: u16, name: Option<&str>) -> Result<Vec<Command>, Refusal> {
    if !valid_name(parent) {
        return Err(Refusal::BadName);
    }
    if !(1..=4094).contains(&id) {
        return Err(Refusal::BadVlanId);
    }
    let default = format!("{parent}.{id}");
    let name = name.unwrap_or(&default);
    if !valid_name(name) {
        return Err(Refusal::BadName);
    }
    let id_text = id.to_string();
    Ok(vec![
        cmd(&["ip", "link", "add", "link", parent, "name", name, "type", "vlan", "id", &id_text]),
        cmd(&["ip", "link", "set", name, "up"]),
    ])
}

pub fn remove_interface(name: &str) -> Result<Vec<Command>, Refusal> {
    if !valid_name(name) {
        return Err(Refusal::BadName);
    }
    Ok(vec![cmd(&["ip", "link", "delete", name])])
}

/// Creates a bond with the given members. Each member is taken down, enslaved, then brought up with the bond.
pub fn add_bond(name: &str, mode: BondMode, members: &[&str], miimon_ms: u32) -> Result<Vec<Command>, Refusal> {
    if !valid_name(name) {
        return Err(Refusal::BadName);
    }
    if members.is_empty() {
        return Err(Refusal::NoMembers);
    }
    if members.len() < 2 && mode != BondMode::ActiveBackup {
        return Err(Refusal::TooFewMembers);
    }
    for (i, m) in members.iter().enumerate() {
        if !valid_name(m) {
            return Err(Refusal::BadName);
        }
        if members[..i].contains(m) {
            return Err(Refusal::DuplicateMember);
        }
        if m.starts_with("bond") {
            return Err(Refusal::MemberIsBond);
        }
    }
    let miimon = miimon_ms.clamp(10, 10_000).to_string();
    let mut plan = vec![cmd(&["ip", "link", "add", name, "type", "bond", "mode", mode.kernel_name(), "miimon", &miimon])];
    for m in members {
        plan.push(cmd(&["ip", "link", "set", m, "down"]));
        plan.push(cmd(&["ip", "link", "set", m, "master", name]));
    }
    for m in members {
        plan.push(cmd(&["ip", "link", "set", m, "up"]));
    }
    plan.push(cmd(&["ip", "link", "set", name, "up"]));
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("eth0.100"));
        assert!(!valid_name(""));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("sixteen_chars_xx"));
        assert!(!valid_name(".."));
    }

    #[test]
    fn vlan_plan() {
        let p = add_vlan("eth0", 100, None).unwrap();
        assert_eq!(p[0].join(" "), "ip link add link eth0 name eth0.100 type vlan id 100");
        assert_eq!(p[1].join(" "), "ip link set eth0.100 up");
        assert_eq!(add_vlan("eth0", 0, None), Err(Refusal::BadVlanId));
        assert_eq!(add_vlan("eth0", 4095, None), Err(Refusal::BadVlanId));
        assert_eq!(add_vlan("bad name", 5, None), Err(Refusal::BadName));
        assert_eq!(add_vlan("eth0", 5, Some("lan")).unwrap()[1].join(" "), "ip link set lan up");
    }

    #[test]
    fn bond_plan_orders_the_steps() {
        let p = add_bond("bond0", BondMode::Lacp, &["eth0", "eth1"], 100).unwrap();
        assert_eq!(p[0].join(" "), "ip link add bond0 type bond mode 802.3ad miimon 100");
        assert_eq!(p[1].join(" "), "ip link set eth0 down");
        assert_eq!(p[2].join(" "), "ip link set eth0 master bond0");
        assert_eq!(p.last().unwrap().join(" "), "ip link set bond0 up");
        assert_eq!(p.len(), 1 + 4 + 2 + 1);
    }

    #[test]
    fn bond_refusals() {
        assert_eq!(add_bond("bond0", BondMode::Lacp, &[], 100), Err(Refusal::NoMembers));
        assert_eq!(add_bond("bond0", BondMode::Lacp, &["eth0"], 100), Err(Refusal::TooFewMembers));
        assert!(add_bond("bond0", BondMode::ActiveBackup, &["eth0"], 100).is_ok());
        assert_eq!(add_bond("bond0", BondMode::Lacp, &["eth0", "eth0"], 100), Err(Refusal::DuplicateMember));
        assert_eq!(add_bond("bond0", BondMode::Lacp, &["eth0", "bond1"], 100), Err(Refusal::MemberIsBond));
        assert_eq!(remove_interface("eth0.5").unwrap()[0].join(" "), "ip link delete eth0.5");
        assert_eq!(remove_interface("x/y"), Err(Refusal::BadName));
    }
}
