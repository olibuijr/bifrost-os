//! Parity sdk-rest-03: headless tests for the network interface model (`net_interfaces`). The tests parse sysfs text and
//! build the list from entries in memory. They never read a live network class directory or touch an adapter.
use bifrost_app::net_interfaces::{classify, from_entries, parse_arp_type, parse_hardware_address, parse_operstate, Entry, InterfaceKind};

fn entry(name: &str, operstate: &str, address: &str, arp_type: &str, wireless: bool) -> Entry {
    Entry {
        name: name.into(),
        operstate: Some(operstate.into()),
        address: Some(address.into()),
        arp_type: Some(arp_type.into()),
        wireless,
    }
}

#[test]
fn the_link_type_number_is_read_from_its_attribute_text() {
    assert_eq!(parse_arp_type("1\n"), Some(1));
    assert_eq!(parse_arp_type("772"), Some(772));
    assert_eq!(parse_arp_type("ether"), None);
}

#[test]
fn an_up_or_unreported_link_is_operational_and_a_down_one_is_not() {
    assert_eq!(parse_operstate("up\n"), Some(true));
    assert_eq!(parse_operstate("unknown"), Some(true));
    assert_eq!(parse_operstate("down"), Some(false));
    assert_eq!(parse_operstate("lowerlayerdown"), Some(false));
    assert_eq!(parse_operstate("sideways"), None);
}

#[test]
fn a_hardware_address_is_read_as_lowercase_colon_octets() {
    assert_eq!(parse_hardware_address("AA:bb:0C:1d:2e:3F\n").as_deref(), Some("aa:bb:0c:1d:2e:3f"));
}

#[test]
fn a_malformed_or_all_zero_hardware_address_is_absent() {
    assert_eq!(parse_hardware_address("00:00:00:00:00:00"), None, "a device without hardware reports zeros");
    assert_eq!(parse_hardware_address("aa:bb:cc:dd:ee"), None, "five octets are not an address");
    assert_eq!(parse_hardware_address("aa:bb:cc:dd:ee:gg"), None, "non-hex digits are rejected");
    assert_eq!(parse_hardware_address("aa:bb:cc:dd:ee:fff"), None, "an octet longer than two digits is rejected");
}

#[test]
fn a_wireless_phy_makes_an_adapter_wireless_even_with_the_ethernet_type() {
    assert_eq!(classify(1, true), InterfaceKind::Wireless);
    assert_eq!(classify(1, false), InterfaceKind::Wired);
}

#[test]
fn the_loopback_type_is_classified_as_loopback_and_other_types_as_other() {
    assert_eq!(classify(772, false), InterfaceKind::Loopback);
    assert_eq!(classify(65534, false), InterfaceKind::Other, "a tunnel device has no hardware link type");
}

#[test]
fn the_interface_list_is_sorted_by_name_with_kind_state_and_address() {
    let entries = vec![
        entry("wlan0", "up", "f4:3e:61:00:11:22", "1", true),
        entry("eth0", "down", "70:85:c2:aa:bb:cc", "1", false),
        entry("lo", "unknown", "00:00:00:00:00:00", "772", false),
    ];
    let list = from_entries(&entries);
    let names: Vec<&str> = list.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["eth0", "lo", "wlan0"]);
    assert_eq!(list[0].kind, InterfaceKind::Wired);
    assert!(!list[0].up, "a cable that is unplugged is down");
    assert_eq!(list[0].hardware_address.as_deref(), Some("70:85:c2:aa:bb:cc"));
    assert_eq!(list[1].kind, InterfaceKind::Loopback);
    assert!(list[1].up, "loopback reports unknown and is usable");
    assert_eq!(list[1].hardware_address, None);
    assert_eq!(list[2].kind, InterfaceKind::Wireless);
    assert_eq!(list[2].hardware_address.as_deref(), Some("f4:3e:61:00:11:22"));
}

#[test]
fn point_to_point_helper_links_are_left_out_of_the_list() {
    let entries = vec![entry("p2p-wlan0-0", "up", "f4:3e:61:00:11:23", "1", true), entry("eth0", "up", "70:85:c2:aa:bb:cc", "1", false)];
    let list = from_entries(&entries);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "eth0");
}

#[test]
fn an_entry_without_a_readable_link_type_is_left_out_rather_than_guessed() {
    let unreadable = Entry { name: "tun9".into(), operstate: Some("up".into()), address: None, arp_type: None, wireless: false };
    assert!(from_entries(&[unreadable]).is_empty());
}

#[test]
fn a_missing_state_or_address_is_down_and_unaddressed_not_an_error() {
    let bare = Entry { name: "eth1".into(), operstate: None, address: None, arp_type: Some("1".into()), wireless: false };
    let list = from_entries(&[bare]);
    assert_eq!(list.len(), 1);
    assert!(!list[0].up);
    assert_eq!(list[0].hardware_address, None);
}
