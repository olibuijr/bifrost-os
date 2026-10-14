//! Parity sdk-complete-02: headless tests for the VPN profile store. The store keeps named tunnel profiles, one of
//! which can be active, and converts to and from a text form. No file, interface or network is touched.

use bifrost_core::tunnel::{TunnelError, VpnProfile, VpnProtocol};
use bifrost_core::vpn_profiles::VpnProfileStore;

fn office() -> VpnProfile {
    VpnProfile::new("Office", "vpn.example.net", VpnProtocol::Ikev2)
}

fn cafe() -> VpnProfile {
    VpnProfile::new("Cafe", "10.0.0.1", VpnProtocol::IpsecIkev1)
}

#[test]
fn a_new_store_is_empty_with_no_active_profile() {
    let store = VpnProfileStore::new();
    assert!(store.is_empty());
    assert_eq!(store.len(), 0);
    assert!(store.active().is_none());
}

#[test]
fn a_valid_profile_is_added_and_found_without_regard_to_case() {
    let mut store = VpnProfileStore::new();
    store.add(office()).unwrap();
    assert_eq!(store.len(), 1);
    assert_eq!(store.get("office").unwrap().server, "vpn.example.net");
    assert!(store.get("Missing").is_none());
}

#[test]
fn a_second_profile_with_the_same_name_in_another_case_is_refused() {
    let mut store = VpnProfileStore::new();
    store.add(office()).unwrap();
    let clash = VpnProfile::new("OFFICE", "other.example.net", VpnProtocol::Tunnel);
    assert!(matches!(store.add(clash), Err(TunnelError::InvalidConfiguration(_))));
    assert_eq!(store.len(), 1, "a refused profile leaves the store as it was");
    assert_eq!(store.get("Office").unwrap().server, "vpn.example.net");
}

#[test]
fn a_profile_that_fails_validation_is_not_stored() {
    let mut store = VpnProfileStore::new();
    let bad_server = VpnProfile::new("Bad", "https://vpn.example.net/x", VpnProtocol::Ikev2);
    assert!(store.add(bad_server).is_err());
    let tab_name = VpnProfile::new("Tab\tName", "vpn.example.net", VpnProtocol::Ikev2);
    assert!(store.add(tab_name).is_err(), "a name with a tab would break the text form");
    assert!(store.is_empty());
}

#[test]
fn the_list_is_sorted_by_name_ignoring_case() {
    let mut store = VpnProfileStore::new();
    store.add(office()).unwrap();
    store.add(VpnProfile::new("alpha", "a.example.net", VpnProtocol::Tunnel)).unwrap();
    store.add(cafe()).unwrap();
    let names: Vec<&str> = store.list().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "Cafe", "Office"]);
}

#[test]
fn setting_an_active_profile_requires_a_stored_name() {
    let mut store = VpnProfileStore::new();
    store.add(office()).unwrap();
    assert!(store.set_active(Some("Nowhere")).is_err());
    store.set_active(Some("OFFICE")).unwrap();
    assert_eq!(store.active().unwrap().name, "Office");
    store.set_active(None).unwrap();
    assert!(store.active().is_none());
}

#[test]
fn replacing_a_profile_keeps_it_active_across_a_rename() {
    let mut store = VpnProfileStore::new();
    store.add(office()).unwrap();
    store.set_active(Some("Office")).unwrap();
    let renamed = VpnProfile::new("Work", "vpn.example.org", VpnProtocol::Ikev2);
    store.replace("Office", renamed).unwrap();
    assert!(store.get("Office").is_none());
    assert_eq!(store.active().unwrap().name, "Work");
    assert_eq!(store.active().unwrap().server, "vpn.example.org");
}

#[test]
fn a_rename_onto_another_profile_name_is_refused() {
    let mut store = VpnProfileStore::new();
    store.add(office()).unwrap();
    store.add(cafe()).unwrap();
    let clash = VpnProfile::new("cafe", "vpn.example.org", VpnProtocol::Ikev2);
    assert!(store.replace("Office", clash).is_err());
    assert_eq!(store.get("Office").unwrap().server, "vpn.example.net");
}

#[test]
fn removing_the_active_profile_leaves_none_active() {
    let mut store = VpnProfileStore::new();
    store.add(office()).unwrap();
    store.add(cafe()).unwrap();
    store.set_active(Some("Cafe")).unwrap();
    let removed = store.remove("cafe").unwrap();
    assert_eq!(removed.port, 500);
    assert!(store.active().is_none());
    assert_eq!(store.len(), 1);
    assert!(store.remove("cafe").is_none());
}

#[test]
fn the_text_form_round_trips_every_field_and_the_active_profile() {
    let mut store = VpnProfileStore::new();
    let mut relay = VpnProfile::new("Relay", "relay.example.net", VpnProtocol::Tunnel);
    relay.port = 8443;
    relay.on_demand = true;
    store.add(relay).unwrap();
    store.add(office()).unwrap();
    store.set_active(Some("Relay")).unwrap();

    let text = store.to_text();
    assert!(text.contains("profile\tRelay\trelay.example.net\t8443\ttunnel\t1\n"));
    assert!(text.ends_with("active\tRelay\n"));

    let back = VpnProfileStore::from_text(&text).unwrap();
    assert_eq!(back, store);
    assert_eq!(back.active().unwrap().port, 8443);
    assert!(back.active().unwrap().on_demand);
}

#[test]
fn a_store_without_an_active_profile_writes_no_active_line() {
    let mut store = VpnProfileStore::new();
    store.add(cafe()).unwrap();
    let text = store.to_text();
    assert_eq!(text, "bifrost-vpn-profiles 1\nprofile\tCafe\t10.0.0.1\t500\tikev1\t0\n");
    assert_eq!(VpnProfileStore::from_text(&text).unwrap(), store);
}

#[test]
fn a_file_with_an_unknown_header_is_refused() {
    assert!(VpnProfileStore::from_text("profiles 2\n").is_err());
    assert!(VpnProfileStore::from_text("").is_err());
}

#[test]
fn a_file_with_a_bad_profile_line_is_refused_as_a_whole() {
    let bad_port = "bifrost-vpn-profiles 1\nprofile\tOffice\tvpn.example.net\tsix\tikev2\t0\n";
    assert!(VpnProfileStore::from_text(bad_port).is_err());
    let bad_protocol = "bifrost-vpn-profiles 1\nprofile\tOffice\tvpn.example.net\t500\twireguard\t0\n";
    assert!(VpnProfileStore::from_text(bad_protocol).is_err());
    let bad_flag = "bifrost-vpn-profiles 1\nprofile\tOffice\tvpn.example.net\t500\tikev2\tyes\n";
    assert!(VpnProfileStore::from_text(bad_flag).is_err());
    let short_line = "bifrost-vpn-profiles 1\nprofile\tOffice\tvpn.example.net\n";
    assert!(VpnProfileStore::from_text(short_line).is_err());
}

#[test]
fn a_file_with_duplicate_names_or_an_unknown_active_name_is_refused() {
    let duplicate = "bifrost-vpn-profiles 1\nprofile\tOffice\ta.example.net\t500\tikev2\t0\nprofile\toffice\tb.example.net\t500\tikev2\t0\n";
    assert!(VpnProfileStore::from_text(duplicate).is_err());
    let dangling = "bifrost-vpn-profiles 1\nprofile\tOffice\ta.example.net\t500\tikev2\t0\nactive\tCafe\n";
    assert!(VpnProfileStore::from_text(dangling).is_err());
}

#[test]
fn blank_lines_in_the_file_are_ignored() {
    let text = "bifrost-vpn-profiles 1\n\nprofile\tCafe\t10.0.0.1\t500\tikev1\t0\n\n";
    let store = VpnProfileStore::from_text(text).unwrap();
    assert_eq!(store.len(), 1);
    assert!(store.active().is_none());
}
