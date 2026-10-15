//! Parity sdk-gaps-01: headless tests for the VLAN and bond command plans. Each test calls the public plan functions and
//! asserts the exact commands. No test runs a command or touches a network interface.

use bifrost_core::vlan_plan::{add_bond, add_vlan, remove_interface, BondMode, Refusal};

#[test]
fn vlan_plan_names_the_link_and_brings_it_up() {
    let plan = add_vlan("eth0", 200, None).unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].join(" "), "ip link add link eth0 name eth0.200 type vlan id 200");
    assert_eq!(plan[1].join(" "), "ip link set eth0.200 up");
}

#[test]
fn vlan_plan_takes_an_explicit_name() {
    let plan = add_vlan("eth0", 7, Some("guest")).unwrap();
    assert_eq!(plan[0].join(" "), "ip link add link eth0 name guest type vlan id 7");
}

#[test]
fn vlan_ids_outside_one_to_4094_are_refused() {
    assert_eq!(add_vlan("eth0", 0, None), Err(Refusal::BadVlanId));
    assert_eq!(add_vlan("eth0", 4095, None), Err(Refusal::BadVlanId));
    assert!(add_vlan("eth0", 4094, None).is_ok());
}

#[test]
fn removing_an_interface_deletes_that_link() {
    let plan = remove_interface("eth0.200").unwrap();
    assert_eq!(plan, vec![vec!["ip".to_string(), "link".to_string(), "delete".to_string(), "eth0.200".to_string()]]);
    assert_eq!(remove_interface("a/b"), Err(Refusal::BadName));
}

#[test]
fn bond_plan_enslaves_each_member_before_the_bond_comes_up() {
    let plan = add_bond("bond0", BondMode::Lacp, &["eth0", "eth1"], 100).unwrap();
    let text: Vec<String> = plan.iter().map(|c| c.join(" ")).collect();
    assert_eq!(text.len(), 1 + 2 * 2 + 2 + 1, "create, down and enslave each member, bring each member up, bring the bond up");
    assert_eq!(text[0], "ip link add bond0 type bond mode 802.3ad miimon 100");
    assert_eq!(text[1], "ip link set eth0 down");
    assert_eq!(text[2], "ip link set eth0 master bond0");
    assert_eq!(text[5], "ip link set eth0 up");
    assert_eq!(text[6], "ip link set eth1 up");
    assert_eq!(text[7], "ip link set bond0 up");
}

#[test]
fn bond_plan_clamps_the_link_monitor_interval() {
    let fast = add_bond("bond0", BondMode::ActiveBackup, &["eth0"], 1).unwrap();
    assert_eq!(fast[0].join(" "), "ip link add bond0 type bond mode active-backup miimon 10");
    let slow = add_bond("bond0", BondMode::ActiveBackup, &["eth0"], 99_999).unwrap();
    assert_eq!(slow[0].join(" "), "ip link add bond0 type bond mode active-backup miimon 10000");
}

#[test]
fn bond_refusals_cover_members_names_and_modes() {
    assert_eq!(add_bond("bond0", BondMode::Lacp, &[], 100), Err(Refusal::NoMembers));
    assert_eq!(add_bond("bond0", BondMode::Lacp, &["eth0"], 100), Err(Refusal::TooFewMembers));
    assert!(add_bond("bond0", BondMode::ActiveBackup, &["eth0"], 100).is_ok());
    assert_eq!(add_bond("bond0", BondMode::Lacp, &["eth0", "eth0"], 100), Err(Refusal::DuplicateMember));
    assert_eq!(add_bond("bond0", BondMode::Lacp, &["eth0", "bond1"], 100), Err(Refusal::MemberIsBond));
    assert_eq!(add_bond("bad name", BondMode::Lacp, &["eth0", "eth1"], 100), Err(Refusal::BadName));
}
