//! Parity sdk-os-04: headless host tests of the app-facing Wi-Fi, Bluetooth, nearby browsing and photo favourite models.
//! Each test drives the public API and checks concrete values. No test touches a radio, a service, a network or a display.

use bifrost_app::bluetooth_controller::{LocalName, LocalNameError};
use bifrost_app::bluetooth_devices::DeviceList;
use bifrost_app::bluetooth_events::{DeviceEvent, DeviceEventKind};
use bifrost_app::bluetooth_service::{Bluetooth, RadioState};
use bifrost_app::nearby_browsing::{NearbyBrowser, Phase};
use bifrost_app::nearby_service::{Error as ServiceTypeError, ServiceType};
use bifrost_app::photo_library::{PHAsset, PHAssetMediaType};
use bifrost_app::wifi_secrets::{Domain, Error as SecretError, WifiSecrets};
use bifrost_app::wifi_status::{signal_bars, WifiState, WifiStatus};
use bifrost_app::wlan::{self, Band, Phy, Sample, SecurityClass};
use bifrost_app::wlan_events::{Event, Snapshot, Watch};
use bifrost_app::wlan_keychain;
use bifrost_core::bt_address::BdAddr;
use bifrost_core::json;
use bifrost_ui::photo_request::{Error as PhotoRequestError, Plan, Request};

fn addr(last: u8) -> BdAddr {
    BdAddr([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, last])
}

fn parse(text: &str) -> json::Json {
    json::parse(text).expect("the test text is valid JSON")
}

fn sample(bssid: &str, freq: u32, rssi: i32, flags: &str, ssid: &str) -> Sample {
    Sample { bssid: bssid.into(), freq, rssi, flags: flags.into(), ssid: ssid.into() }
}

// --- Wi-Fi client, interfaces and status (core-wlan) -------------------------------------------------------------

#[test]
fn a_connected_status_needs_the_connected_code_and_an_address() {
    let joining = WifiStatus { state: Some(WifiState::from_code(3)), ssid: Some("Home".into()), ..WifiStatus::default() };
    assert_eq!(joining.state_or_off(), WifiState::Connecting);
    assert!(!joining.is_connected());
    assert!(!joining.has_address());

    let up = WifiStatus {
        state: Some(WifiState::from_code(4)),
        ssid: Some("Home".into()),
        ip: Some("192.168.1.20".into()),
        rssi: Some(-60),
        ..WifiStatus::default()
    };
    assert!(up.is_connected());
    assert!(up.has_address());
    assert_eq!(up.signal_bars(), 3, "-60 dBm is the third of four bars");
    assert_eq!(WifiStatus::default().state_or_off(), WifiState::Off);
}

#[test]
fn an_unknown_state_code_is_kept_for_a_newer_system() {
    let state = WifiState::from_code(9);
    assert_eq!(state, WifiState::Unknown(9));
    assert_eq!(state.code(), 9);
    assert_eq!(state.label(), "Unknown");
}

#[test]
fn signal_bars_cut_at_minus_55_65_75_and_85_dbm() {
    assert_eq!(signal_bars(Some(-55)), 4);
    assert_eq!(signal_bars(Some(-56)), 3);
    assert_eq!(signal_bars(Some(-85)), 1);
    assert_eq!(signal_bars(Some(-86)), 0);
    assert_eq!(signal_bars(None), 0);
}

#[test]
fn saved_networks_and_problem_notes_are_found_by_name() {
    let status = WifiStatus {
        saved: vec!["Office".into(), "Home".into()],
        problems: vec![("Home".into(), "Incorrect Password".into())],
        ..WifiStatus::default()
    };
    assert!(status.is_saved("Office"));
    assert!(!status.is_saved("Cafe"));
    assert_eq!(status.problem_for("Home"), Some("Incorrect Password"));
    assert_eq!(status.problem_for("Office"), None);
}

// --- Scan and network list (CWNetwork) ---------------------------------------------------------------------------

#[test]
fn a_scan_merges_access_points_into_networks_with_channel_band_and_phy() {
    let scan = [
        sample("02:00:00:00:00:01", 2462, -70, "[ESS]", "Guest"),
        sample("02:00:00:00:00:02", 5180, -50, "[WPA2-PSK-CCMP][VHT]", "Office"),
        sample("02:00:00:00:00:03", 5180, -58, "[WPA2-PSK-CCMP][VHT]", "Office"),
        sample("02:00:00:00:00:04", 2437, -90, "[ESS]", ""),
    ];
    let networks = wlan::merge(&scan);
    assert_eq!(networks.len(), 2, "the hidden access point with no name is dropped");

    let office = &networks[0];
    assert_eq!(office.ssid, "Office");
    assert_eq!(office.bssids, ["02:00:00:00:00:02", "02:00:00:00:00:03"]);
    assert_eq!(office.rssi, -50, "a network reports its strongest access point");
    assert_eq!(office.channel, Some(36));
    assert_eq!(office.band, Some(Band::Five));
    assert_eq!(office.phy, Phy::Ac);
    assert_eq!(office.phy.label(), "802.11ac");
    assert_eq!(office.security, SecurityClass::Wpa2Personal);

    let guest = &networks[1];
    assert_eq!(guest.channel, Some(11));
    assert_eq!(guest.band, Some(Band::TwoPointFour));
    assert_eq!(guest.phy, Phy::G);
    assert_eq!(guest.security, SecurityClass::Open);
    assert!(!guest.security.is_personal());
}

#[test]
fn security_classes_come_from_the_scan_flags() {
    assert_eq!(SecurityClass::from_flags("[SAE][ESS]"), SecurityClass::Wpa3Personal);
    assert_eq!(SecurityClass::from_flags("[WPA2-PSK+SAE-CCMP][ESS]"), SecurityClass::Wpa2Wpa3Personal);
    assert_eq!(SecurityClass::from_flags("[WEP][ESS]"), SecurityClass::Wep);
    assert_eq!(SecurityClass::from_flags("[WPA2-EAP-CCMP][ESS]"), SecurityClass::Enterprise);
    assert!(SecurityClass::Wep.is_personal());
    assert!(!SecurityClass::Enterprise.is_personal());
}

// --- Join, select and forget (passphrases and secrets) -------------------------------------------------------------

#[test]
fn a_personal_passphrase_is_checked_before_it_becomes_a_keychain_item() {
    assert_eq!(wlan_keychain::check_passphrase("short"), Err(wlan_keychain::Error::PasswordLength));
    assert_eq!(wlan_keychain::check_passphrase("correct horse"), Ok(()));
    assert_eq!(wlan_keychain::check_passphrase("tab\tkey-pass"), Err(wlan_keychain::Error::PasswordCharacters));
    assert_eq!(wlan_keychain::check_passphrase(&"f".repeat(64)), Ok(()), "64 hex digits are a raw key");

    let (account, value) = wlan_keychain::password_item("Home", "secret123").unwrap();
    assert_eq!(account, "password:486f6d65", "the network name is kept in hex");
    assert_eq!(value, b"secret123".to_vec());
    assert_eq!(wlan_keychain::password_item("", "secret123"), Err(wlan_keychain::Error::SsidLength));
}

#[test]
fn an_enterprise_network_stores_a_user_name_with_its_password() {
    let (account, value) = wlan_keychain::eap_item("Corp", "alice", "pw").unwrap();
    assert_eq!(account, "eap:436f7270");
    assert_eq!(value, b"alice\npw".to_vec());
    assert_eq!(wlan_keychain::parse_eap(&value), Some(("alice".to_string(), "pw".to_string())));
}

#[test]
fn a_shared_password_is_stored_per_domain_and_forgotten_on_request() {
    let mut secrets = WifiSecrets::new();
    secrets.set_password(Domain::User, "Home", "secret123").unwrap();
    assert_eq!(secrets.find_password(Domain::User, "Home"), Ok("secret123"));
    assert_eq!(secrets.find_password(Domain::System, "Home"), Err(SecretError::ItemNotFound));
    assert_eq!(secrets.set_password(Domain::None, "Home", "secret123"), Err(SecretError::Param));
    assert_eq!(secrets.set_password(Domain::User, &"n".repeat(33), "secret123"), Err(SecretError::Param));
    secrets.delete_password(Domain::User, "Home").unwrap();
    assert_eq!(secrets.find_password(Domain::User, "Home"), Err(SecretError::ItemNotFound));
}

// --- Wi-Fi change events (the client's notifications) -------------------------------------------------------------

#[test]
fn a_watch_reports_the_first_join_then_only_real_changes() {
    let mut watch = Watch::new();
    let joined = Snapshot { link_up: true, ssid: Some("Home".into()), rssi: Some(-61), scan: vec!["Home".into()] };
    assert_eq!(
        watch.update(joined.clone()),
        vec![
            Event::SsidChanged { from: None, to: Some("Home".into()) },
            Event::LinkChanged(true),
            Event::RssiChanged(Some(-61)),
            Event::ScanCacheUpdated,
        ]
    );
    let mut quieter = joined.clone();
    quieter.rssi = Some(-62);
    assert!(watch.update(quieter.clone()).is_empty(), "a 1 dBm move is below the step");
    quieter.rssi = Some(-64);
    assert_eq!(watch.update(quieter), vec![Event::RssiChanged(Some(-64))]);
}

// --- Bluetooth host and devices (iobluetooth) ----------------------------------------------------------------------

#[test]
fn a_status_reply_sets_the_radio_state_and_the_device_table() {
    let mut bt = Bluetooth::new();
    assert_eq!(bt.state, RadioState::Unavailable);
    let reply = parse(
        r#"{"state":"on","scanning":true,"busy":"AA:BB:CC:DD:EE:01","devices":[
            {"addr":"AA:BB:CC:DD:EE:01","name":"Keyboard","paired":true,"connected":false,"rssi":-40},
            {"addr":"AA:BB:CC:DD:EE:02","name":"","paired":false,"connected":false},
            {"addr":"not-an-address","name":"Bad"}]}"#,
    );
    bt.apply_status(&reply).unwrap();
    assert_eq!(bt.state, RadioState::On);
    assert!(bt.scanning);
    assert_eq!(bt.busy, Some(addr(1)));
    assert_eq!(bt.devices.len(), 2, "an entry whose address does not read is skipped");
    assert_eq!(bt.devices.get(addr(2)).unwrap().label(), "AA:BB:CC:DD:EE:02");
}

#[test]
fn a_status_reply_without_a_radio_state_is_refused_and_changes_nothing() {
    let mut bt = Bluetooth::new();
    let err = bt.apply_status(&parse("{}")).unwrap_err();
    assert_eq!(err, "status has no radio state");
    assert_eq!(bt.state, RadioState::Unavailable);
}

#[test]
fn connect_and_disconnect_events_update_the_device_and_queue_in_order() {
    let mut bt = Bluetooth::new();
    bt.apply_status(&parse(r#"{"state":"on","devices":[{"addr":"AA:BB:CC:DD:EE:01","name":"Mouse","paired":true}]}"#))
        .unwrap();
    let body = parse(r#"{"addr":"AA:BB:CC:DD:EE:01"}"#);
    assert!(bt.apply_event("connected", &body));
    assert!(bt.devices.get(addr(1)).unwrap().connected);
    assert!(bt.apply_event("disconnected", &body));
    assert!(!bt.devices.get(addr(1)).unwrap().connected);
    assert!(!bt.apply_event("pairing-request", &body), "other events are not Bluetooth connection events");
    assert_eq!(
        bt.take_events(),
        vec![
            DeviceEvent { kind: DeviceEventKind::Connected, address: addr(1) },
            DeviceEvent { kind: DeviceEventKind::Disconnected, address: addr(1) },
        ]
    );
    assert!(bt.take_events().is_empty(), "events are handed over once");
}

#[test]
fn paired_devices_are_listed_first_and_forgetting_removes_them() {
    let mut list = DeviceList::new();
    assert!(list.report(addr(1), None, Some("Zed"), None));
    assert!(list.report(addr(2), None, Some("alpha"), None));
    assert!(list.report(addr(3), None, Some("Beta"), None));
    list.set_paired(addr(1), true);
    list.set_paired(addr(3), true);
    let labels: Vec<String> = list.all().iter().map(|d| d.label()).collect();
    assert_eq!(labels, vec!["Beta", "Zed", "alpha"]);
    assert_eq!(list.paired().len(), 2);

    assert!(list.forget(addr(1)));
    assert!(!list.forget(addr(1)), "a device that is gone cannot be forgotten twice");
    assert_eq!(list.len(), 2);
}

#[test]
fn a_local_name_is_utf8_of_at_most_248_bytes_without_a_nul() {
    assert_eq!(LocalName::new("BifrOSt Desk").unwrap().as_str(), "BifrOSt Desk");
    assert_eq!(LocalName::new(""), Err(LocalNameError::Empty));
    assert!(LocalName::new(&"e".repeat(248)).is_ok());
    assert_eq!(LocalName::new(&"e".repeat(249)), Err(LocalNameError::TooLong));
    assert!(LocalName::new(&"é".repeat(124)).is_ok(), "an accented letter takes two bytes");
    assert_eq!(LocalName::new(&"é".repeat(125)), Err(LocalNameError::TooLong));
    assert_eq!(LocalName::new("desk\0name"), Err(LocalNameError::ContainsNul));
}

// --- Nearby browsing and background behaviour (multipeer-connectivity) --------------------------------------------

#[test]
fn a_service_type_is_checked_with_the_bonjour_rules() {
    assert_eq!(ServiceType::new("my-chat").unwrap().as_str(), "my-chat");
    assert_eq!(ServiceType::new("Chat"), Err(ServiceTypeError::BadCharacter));
    assert_eq!(ServiceType::new("a--b"), Err(ServiceTypeError::DoubleHyphen));
    assert_eq!(ServiceType::new("abcdefghijklmnop"), Err(ServiceTypeError::TooLong));
    assert_eq!(ServiceTypeError::TooLong.to_string(), "service type is longer than 15 bytes");
}

#[test]
fn browsing_pauses_in_the_background_and_resumes_in_the_foreground() {
    let mut browser = NearbyBrowser::new(ServiceType::new("my-chat").unwrap());
    assert_eq!(browser.service().as_str(), "my-chat");
    assert_eq!(browser.phase(), Phase::Stopped);

    browser.start();
    assert_eq!(browser.phase(), Phase::Browsing);
    assert!(browser.is_browsing());

    browser.did_enter_background();
    assert_eq!(browser.phase(), Phase::Suspended);
    assert!(!browser.is_browsing());

    browser.will_enter_foreground();
    assert_eq!(browser.phase(), Phase::Browsing);
}

#[test]
fn a_browser_started_in_the_background_waits_for_the_foreground() {
    let mut browser = NearbyBrowser::new(ServiceType::new("my-chat").unwrap());
    browser.did_enter_background();
    browser.start();
    assert_eq!(browser.phase(), Phase::Suspended, "no browsing while the app is in the background");
    browser.will_enter_foreground();
    assert_eq!(browser.phase(), Phase::Browsing);
}

#[test]
fn stopping_ends_browsing_for_good_even_across_a_background_cycle() {
    let mut browser = NearbyBrowser::new(ServiceType::new("my-chat").unwrap());
    browser.start();
    browser.did_enter_background();
    browser.stop();
    assert_eq!(browser.phase(), Phase::Stopped);
    browser.will_enter_foreground();
    assert_eq!(browser.phase(), Phase::Stopped, "a stopped browser does not restart by itself");
}

// --- Favourites (photokit) ----------------------------------------------------------------------------------------

#[test]
fn a_photo_asset_record_carries_its_favourite_flag() {
    let record = parse(
        r#"{"id":"A1","media_type":"image","width":4032,"height":3024,"created":1700000000,"favorite":true}"#,
    );
    let asset = PHAsset::decode(&record).unwrap();
    assert_eq!(asset.local_identifier, "A1");
    assert_eq!(asset.media_type, PHAssetMediaType::Image);
    assert_eq!((asset.pixel_width, asset.pixel_height), (4032, 3024));
    assert_eq!(asset.creation_date, 1700000000);
    assert!(asset.is_favorite);

    let plain = parse(r#"{"id":"B2","media_type":"video","width":1920,"height":1080,"created":5}"#);
    assert!(!PHAsset::decode(&plain).unwrap().is_favorite);
    let no_width = parse(r#"{"id":"C3","media_type":"image","height":1080,"created":5}"#);
    assert!(PHAsset::decode(&no_width).is_none());
}

#[test]
fn a_favourite_batch_plans_each_asset_once_and_names_unknown_ones() {
    let mut request = Request::new();
    request.set_favourite("A1", true).set_favourite("B2", false);
    assert_eq!(
        request.plan(&["A1", "B2"]),
        Ok(Plan { favourites: vec![("A1".into(), true), ("B2".into(), false)], ..Plan::default() })
    );

    let mut unknown = Request::new();
    unknown.set_favourite("B2", true);
    assert_eq!(unknown.plan(&["A1"]), Err(PhotoRequestError::UnknownAsset("B2".into())));

    let mut repeated = Request::new();
    repeated.set_favourite("A1", true).set_favourite("A1", false);
    assert_eq!(repeated.plan(&["A1"]), Err(PhotoRequestError::Repeated("A1".into())));
}
