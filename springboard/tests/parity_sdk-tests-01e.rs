//! Parity tests for the saved Wi-Fi network model: the security types, their supplicant fields, the password rules and the
//! springboard.conf round trip. Pure functions only; headless, no supplicant, radio or file is touched.

use springboard::net::wifi_config::{conf, move_network, parse_conf, priority_of, upsert, Eap, NetworkConfig, Security};

fn field(fields: &[(&'static str, String)], key: &str) -> Option<String> {
    fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
}

#[test]
fn security_labels_match_the_settings_sheet() {
    assert_eq!(Security::Open.label(), "None");
    assert_eq!(Security::Wpa2.label(), "WPA2 Personal");
    assert_eq!(Security::Wpa3.label(), "WPA3 Personal");
    assert_eq!(Security::Wpa2Wpa3.label(), "WPA2/WPA3 Personal");
    assert_eq!(Security::Enterprise(Eap::Ttls).label(), "WPA2 Enterprise");
    assert_eq!(Eap::Peap.label(), "PEAP");
    assert_eq!(Eap::Ttls.label(), "TTLS");
}

#[test]
fn personal_and_enterprise_are_told_apart() {
    assert!(Security::Wpa2.is_personal() && Security::Wpa3.is_personal() && Security::Wpa2Wpa3.is_personal());
    assert!(!Security::Open.is_personal());
    assert!(!Security::Enterprise(Eap::Peap).is_personal());
    assert!(Security::Enterprise(Eap::Peap).is_enterprise());
    assert!(!Security::Wpa2.is_enterprise());
}

#[test]
fn the_security_choice_cycles_through_the_five_choices() {
    assert_eq!(Security::Open.next(), Security::Wpa2);
    assert_eq!(Security::Wpa2.next(), Security::Wpa3);
    assert_eq!(Security::Wpa3.next(), Security::Wpa2Wpa3);
    assert_eq!(Security::Wpa2Wpa3.next(), Security::Enterprise(Eap::Peap));
    assert_eq!(Security::Enterprise(Eap::Peap).next(), Security::Open, "the cycle wraps");
    assert_eq!(Security::Enterprise(Eap::Ttls).next(), Security::Open, "TTLS is the same choice as PEAP");
    assert_eq!(Security::CHOICES.len(), 5);
}

#[test]
fn password_lengths_follow_the_standard_of_each_security() {
    assert_eq!(Security::Wpa2.password_len(), (8, 63));
    assert_eq!(Security::Wpa2Wpa3.password_len(), (8, 63));
    assert_eq!(Security::Wpa3.password_len(), (1, 128), "SAE takes any passphrase");
    assert_eq!(Security::Open.password_len(), (0, 128));
}

#[test]
fn a_network_must_have_a_name_and_the_credentials_its_security_needs() {
    assert_eq!(NetworkConfig::new("", Security::Open).problem(), Some("Enter the network name"));
    assert_eq!(NetworkConfig::new(&"x".repeat(33), Security::Open).problem(), Some("Enter the network name"));
    assert_eq!(NetworkConfig::new(&"x".repeat(32), Security::Open).problem(), None);

    let mut home = NetworkConfig::new("Home", Security::Wpa2);
    home.password = "short".into();
    assert_eq!(home.problem(), Some("Enter the password"));
    home.password = "longenough".into();
    assert_eq!(home.problem(), None);

    let mut office = NetworkConfig::new("Office", Security::Enterprise(Eap::Peap));
    office.password = "pw".into();
    assert_eq!(office.problem(), Some("Enter the username and password"), "enterprise needs an identity too");
    office.identity = "ola".into();
    assert_eq!(office.problem(), None);
}

#[test]
fn a_wpa2_network_writes_a_pre_shared_key_and_a_wpa3_one_writes_sae() {
    let mut wpa2 = NetworkConfig::new("Home", Security::Wpa2);
    wpa2.password = "longenough".into();
    let f = wpa2.fields(7);
    assert_eq!(field(&f, "key_mgmt"), Some("WPA-PSK".to_string()));
    assert_eq!(field(&f, "psk"), Some("\"longenough\"".to_string()));
    assert_eq!(field(&f, "priority"), Some("7".to_string()));
    assert_eq!(field(&f, "ssid"), Some("\"Home\"".to_string()));

    let mut wpa3 = NetworkConfig::new("Lab", Security::Wpa3);
    wpa3.password = "sae-pass".into();
    let f = wpa3.fields(1);
    assert_eq!(field(&f, "key_mgmt"), Some("SAE".to_string()));
    assert_eq!(field(&f, "sae_password"), Some("\"sae-pass\"".to_string()));
    assert_eq!(field(&f, "ieee80211w"), Some("2".to_string()), "WPA3 requires protected management frames");
}

#[test]
fn a_64_digit_hex_key_is_written_unquoted_as_a_raw_psk() {
    let mut net = NetworkConfig::new("Raw", Security::Wpa2);
    net.password = "ab".repeat(32);
    assert_eq!(field(&net.fields(1), "psk"), Some("ab".repeat(32)));
}

#[test]
fn an_enterprise_network_uses_the_inner_mschapv2_method_and_no_ca_check() {
    let mut net = NetworkConfig::new("Office", Security::Enterprise(Eap::Ttls));
    net.identity = "ola".into();
    net.anonymous = "anon".into();
    net.password = "secret".into();
    let f = net.fields(2);
    assert_eq!(field(&f, "key_mgmt"), Some("WPA-EAP".to_string()));
    assert_eq!(field(&f, "eap"), Some("TTLS".to_string()));
    assert_eq!(field(&f, "phase2"), Some("\"auth=MSCHAPV2\"".to_string()));
    assert_eq!(field(&f, "anonymous_identity"), Some("\"anon\"".to_string()));
    assert_eq!(field(&f, "ca_cert"), None, "no certificate check is configured");
}

#[test]
fn an_open_network_has_no_key_and_hidden_or_disabled_flags_are_written() {
    let mut open = NetworkConfig::new("Guest", Security::Open);
    open.hidden = true;
    open.auto_join = false;
    let f = open.fields(1);
    assert_eq!(field(&f, "key_mgmt"), Some("NONE".to_string()));
    assert_eq!(field(&f, "psk"), None);
    assert_eq!(field(&f, "scan_ssid"), Some("1".to_string()));
    assert_eq!(field(&f, "disabled"), Some("1".to_string()));
}

#[test]
fn the_config_file_reads_back_the_same_networks_in_joining_order() {
    let cafe = NetworkConfig::personal("Cafe", Some("hunter2hunter2"));
    let mut office = NetworkConfig::new("Office", Security::Enterprise(Eap::Peap));
    office.identity = "ola".into();
    office.password = "pw-1".into();
    let mut guest = NetworkConfig::new("Guest", Security::Open);
    guest.hidden = true;
    guest.auto_join = false;

    let text = conf("/run/wpa", &[cafe.clone(), office.clone(), guest.clone()]);
    assert!(text.starts_with("ctrl_interface=/run/wpa\n"));
    assert!(text.contains("wowlan_triggers=any"));
    assert_eq!(parse_conf(&text), vec![cafe, office, guest]);
}

#[test]
fn priorities_are_highest_for_the_first_network() {
    assert_eq!(priority_of(0, 3), 3);
    assert_eq!(priority_of(2, 3), 1);
}

#[test]
fn an_upsert_replaces_a_network_of_the_same_name_and_puts_it_first() {
    let mut nets = vec![NetworkConfig::personal("Cafe", Some("hunter2hunter2")), NetworkConfig::new("Office", Security::Open)];
    upsert(&mut nets, NetworkConfig::new("Office", Security::Wpa2));
    assert_eq!(nets.len(), 2);
    assert_eq!(nets[0].ssid, "Office");
    assert_eq!(nets[0].security, Security::Wpa2);
    assert_eq!(nets[1].ssid, "Cafe");
}

#[test]
fn move_network_swaps_with_a_neighbour_and_stops_at_the_ends() {
    let mut nets = vec![NetworkConfig::new("A", Security::Open), NetworkConfig::new("B", Security::Open), NetworkConfig::new("C", Security::Open)];
    assert!(!move_network(&mut nets, "A", true), "already first");
    assert!(move_network(&mut nets, "B", true));
    let order: Vec<&str> = nets.iter().map(|n| n.ssid.as_str()).collect();
    assert_eq!(order, vec!["B", "A", "C"]);
    assert!(!move_network(&mut nets, "C", false), "already last");
    assert!(!move_network(&mut nets, "missing", false), "unknown name");
}

#[test]
fn the_debug_form_never_shows_the_password() {
    let net = NetworkConfig::personal("Cafe", Some("hunter2hunter2"));
    let shown = format!("{net:?}");
    assert!(!shown.contains("hunter2hunter2"));
    assert!(shown.contains("<redacted>"));
    let bare = NetworkConfig::new("Open", Security::Open);
    assert!(format!("{bare:?}").contains("credentials: \"none\""));
}
