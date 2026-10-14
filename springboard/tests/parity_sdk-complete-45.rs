//! Parity sdk-complete-45: the WEP and WPA Personal security types of saved Wi-Fi networks. Their supplicant fields,
//! the key rules, the springboard.conf round trip and the reading of a configuration the supplicant saved itself.
//! Pure functions only; no supplicant, radio or file is touched.

use springboard::net::wifi_config::{conf, parse_conf, NetworkConfig, Security};

fn field(fields: &[(&'static str, String)], key: &str) -> Option<String> {
    fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
}

#[test]
fn wep_and_wpa_personal_have_their_own_labels_and_count_as_personal() {
    assert_eq!(Security::Wep.label(), "WEP");
    assert_eq!(Security::Wpa.label(), "WPA Personal");
    assert!(Security::Wep.is_personal());
    assert!(Security::Wpa.is_personal());
    assert!(!Security::Wep.is_enterprise());
}

#[test]
fn a_wep_key_is_five_or_thirteen_ascii_characters_or_ten_or_twenty_six_hex_digits() {
    assert_eq!(NetworkConfig { password: "abcde".into(), ..NetworkConfig::new("Old", Security::Wep) }.problem(), None);
    assert_eq!(NetworkConfig { password: "abcdefghijklm".into(), ..NetworkConfig::new("Old", Security::Wep) }.problem(), None);
    assert_eq!(NetworkConfig { password: "0123456789".into(), ..NetworkConfig::new("Old", Security::Wep) }.problem(), None);
    assert_eq!(NetworkConfig { password: "0123456789abcdef0123456789".into(), ..NetworkConfig::new("Old", Security::Wep) }.problem(), None);
    assert_eq!(NetworkConfig { password: "abcd".into(), ..NetworkConfig::new("Old", Security::Wep) }.problem(), Some("Enter the password"));
    assert_eq!(NetworkConfig { password: "zzzzzzzzzz".into(), ..NetworkConfig::new("Old", Security::Wep) }.problem(), Some("Enter the password"));
    assert_eq!(NetworkConfig { password: "abcdef".into(), ..NetworkConfig::new("Old", Security::Wep) }.problem(), Some("Enter the password"));
}

#[test]
fn a_wpa_personal_passphrase_is_eight_to_sixty_three_characters() {
    assert_eq!(NetworkConfig { password: "legacy12".into(), ..NetworkConfig::new("Old", Security::Wpa) }.problem(), None);
    assert_eq!(NetworkConfig { password: "short".into(), ..NetworkConfig::new("Old", Security::Wpa) }.problem(), Some("Enter the password"));
}

#[test]
fn a_wep_network_writes_one_shared_key_with_open_authentication() {
    let f = NetworkConfig { password: "abcde".into(), ..NetworkConfig::new("Old Router", Security::Wep) }.fields(4);
    assert_eq!(field(&f, "key_mgmt").as_deref(), Some("NONE"));
    assert_eq!(field(&f, "wep_key0").as_deref(), Some("\"abcde\""), "an ASCII key is quoted");
    assert_eq!(field(&f, "wep_tx_keyidx").as_deref(), Some("0"));
    assert_eq!(field(&f, "auth_alg").as_deref(), Some("OPEN"));
    assert_eq!(field(&f, "priority").as_deref(), Some("4"));
    assert_eq!(field(&f, "psk"), None, "a WEP network has no pre-shared key");
}

#[test]
fn a_hexadecimal_wep_key_is_written_unquoted() {
    let f = NetworkConfig { password: "0123456789".into(), ..NetworkConfig::new("Old", Security::Wep) }.fields(1);
    assert_eq!(field(&f, "wep_key0").as_deref(), Some("0123456789"));
}

#[test]
fn a_wpa_personal_network_is_wpa_psk_with_tkip_and_no_rsn() {
    let f = NetworkConfig { password: "legacy12".into(), ..NetworkConfig::new("Legacy", Security::Wpa) }.fields(1);
    assert_eq!(field(&f, "key_mgmt").as_deref(), Some("WPA-PSK"));
    assert_eq!(field(&f, "proto").as_deref(), Some("WPA"));
    assert_eq!(field(&f, "pairwise").as_deref(), Some("TKIP"));
    assert_eq!(field(&f, "group").as_deref(), Some("TKIP"));
    assert_eq!(field(&f, "psk").as_deref(), Some("\"legacy12\""));
    assert_eq!(field(&f, "ieee80211w"), None, "WPA1 has no management frame protection");
}

#[test]
fn wep_and_wpa_networks_read_back_from_the_config_file_in_order() {
    let nets = vec![
        NetworkConfig { password: "abcde".into(), ..NetworkConfig::new("Old Router", Security::Wep) },
        NetworkConfig { password: "legacy12".into(), ..NetworkConfig::new("Legacy", Security::Wpa) },
        NetworkConfig { password: "secret12".into(), ..NetworkConfig::new("Home", Security::Wpa2) },
    ];
    let text = conf("/d/wpa", &nets);
    assert_eq!(parse_conf(&text), nets);
}

#[test]
fn a_wep_network_saved_by_the_supplicant_is_read_as_wep() {
    let text = "network={\n\tssid=\"Old Router\"\n\tkey_mgmt=NONE\n\twep_tx_keyidx=0\n\twep_key0=\"abcde\"\n\tauth_alg=OPEN\n}\n";
    let n = parse_conf(text);
    assert_eq!(n.len(), 1);
    assert_eq!((n[0].ssid.as_str(), n[0].security, n[0].password.as_str()), ("Old Router", Security::Wep, "abcde"));
}

#[test]
fn a_wpa1_network_saved_by_the_supplicant_is_read_as_wpa_and_a_wpa2_one_stays_wpa2() {
    let text = "network={\n\tssid=\"Legacy\"\n\tkey_mgmt=WPA-PSK\n\tproto=WPA\n\tpsk=\"legacy12\"\n}\nnetwork={\n\tssid=\"Home\"\n\tkey_mgmt=WPA-PSK\n\tproto=RSN WPA\n\tpsk=\"secret12\"\n}\n";
    let n = parse_conf(text);
    let security: Vec<(&str, Security)> = n.iter().map(|c| (c.ssid.as_str(), c.security)).collect();
    assert_eq!(security, [("Legacy", Security::Wpa), ("Home", Security::Wpa2)]);
}
