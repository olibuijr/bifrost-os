//! Parity sdk-complete-01: headless tests for the hotspot models (`hotspot_configuration`, `accessory_hotspot`,
//! `hotspot_helper`). The tests check names, passphrases, consent, queues and responses in memory. They never touch a
//! radio, a supplicant or an accessory.
use bifrost_app::accessory_hotspot::{AccessoryHotspotJoin, JoinState};
use bifrost_app::hotspot_configuration::{HotspotConfiguration, HotspotError, HotspotManager, HotspotSecurity};
use bifrost_app::hotspot_helper::{Command, CommandKind, Confidence, Error as HelperError, Helper, Outcome};

fn wpa(ssid: &str, passphrase: &str) -> HotspotConfiguration {
    HotspotConfiguration::with_passphrase(ssid, passphrase, false)
}

fn wep(ssid: &str, key: &str) -> HotspotConfiguration {
    HotspotConfiguration::with_passphrase(ssid, key, true)
}

#[test]
fn an_open_network_needs_only_a_name_of_one_to_thirty_two_octets() {
    assert_eq!(HotspotConfiguration::open("Cafe").validate(), Ok(()));
    assert_eq!(HotspotConfiguration::open("a").security(), HotspotSecurity::Open);
    assert_eq!(HotspotConfiguration::open("a".repeat(32).as_str()).validate(), Ok(()));
    assert_eq!(HotspotConfiguration::open("a".repeat(33).as_str()).validate(), Err(HotspotError::InvalidSsid));
    assert_eq!(HotspotConfiguration::open("").validate(), Err(HotspotError::InvalidSsid));
}

#[test]
fn a_name_counts_octets_so_a_multibyte_name_is_longer_than_its_characters() {
    // Sixteen two-byte characters are 32 octets, so the name is still accepted.
    assert_eq!(HotspotConfiguration::open(&"é".repeat(16)).validate(), Ok(()));
    // Seventeen are 34 octets and are refused.
    assert_eq!(HotspotConfiguration::open(&"é".repeat(17)).validate(), Err(HotspotError::InvalidSsid));
}

#[test]
fn a_wpa_passphrase_is_eight_to_sixty_three_printable_characters_or_sixty_four_hex_digits() {
    assert_eq!(wpa("Home", "12345678").validate(), Ok(()));
    assert_eq!(wpa("Home", &"x".repeat(63)).validate(), Ok(()));
    assert_eq!(wpa("Home", &"f".repeat(64)).validate(), Ok(()));
    assert_eq!(wpa("Home", "1234567").validate(), Err(HotspotError::InvalidWpaPassphrase));
    assert_eq!(wpa("Home", &"x".repeat(64)).validate(), Err(HotspotError::InvalidWpaPassphrase));
    assert_eq!(wpa("Home", "pass\u{7f}word").validate(), Err(HotspotError::InvalidWpaPassphrase));
}

#[test]
fn a_wep_key_is_five_or_thirteen_characters_or_ten_or_twenty_six_hex_digits() {
    assert_eq!(wep("Old", "abcde").validate(), Ok(()));
    assert_eq!(wep("Old", "abcdefghijklm").validate(), Ok(()));
    assert_eq!(wep("Old", "0123456789").validate(), Ok(()));
    assert_eq!(wep("Old", &"0".repeat(26)).validate(), Ok(()));
    assert_eq!(wep("Old", "zzzzzzzzzz").validate(), Err(HotspotError::InvalidWepPassphrase));
    assert_eq!(wep("Old", "abcdef").validate(), Err(HotspotError::InvalidWepPassphrase));
}

#[test]
fn the_debug_text_of_a_configuration_never_shows_its_passphrase() {
    let text = format!("{:?}", wpa("Home", "supersecret"));
    assert!(text.contains("<redacted>"));
    assert!(!text.contains("supersecret"));
    assert!(format!("{:?}", HotspotConfiguration::open("Cafe")).contains("\"none\""));
}

#[test]
fn a_refused_consent_stores_nothing_and_a_valid_consent_stores_the_network() {
    let mut manager = HotspotManager::new();
    assert_eq!(manager.apply(wpa("Home", "12345678"), false), Err(HotspotError::UserDenied));
    assert!(manager.configured_ssids().is_empty());
    assert_eq!(manager.apply(wpa("Home", "12345678"), true), Ok(()));
    assert_eq!(manager.configured_ssids(), vec!["Home".to_string()]);
}

#[test]
fn validation_runs_before_consent_so_an_invalid_network_reports_its_own_error() {
    let mut manager = HotspotManager::new();
    assert_eq!(manager.apply(wpa("Home", "short"), false), Err(HotspotError::InvalidWpaPassphrase));
}

#[test]
fn applying_a_name_again_replaces_its_configuration_and_keeps_the_order() {
    let mut manager = HotspotManager::new();
    manager.apply(HotspotConfiguration::open("Cafe"), true).unwrap();
    manager.apply(wpa("Home", "12345678"), true).unwrap();
    manager.apply(wpa("Cafe", "12345678"), true).unwrap();
    assert_eq!(manager.configured_ssids(), vec!["Cafe".to_string(), "Home".to_string()]);
    assert_eq!(manager.configuration("Cafe").unwrap().security(), HotspotSecurity::WpaPersonal);
}

#[test]
fn a_manager_removes_only_the_networks_it_holds() {
    let mut manager = HotspotManager::new();
    manager.apply(HotspotConfiguration::open("Cafe"), true).unwrap();
    assert!(manager.remove_configuration_for_ssid("Cafe"));
    assert!(!manager.remove_configuration_for_ssid("Cafe"), "a second removal finds nothing");
    assert!(manager.configuration("Cafe").is_none());
}

#[test]
fn hotspot_error_messages_name_the_failed_check() {
    assert_eq!(HotspotError::UserDenied.to_string(), "The user refused the network change");
    assert_eq!(HotspotError::Pending.to_string(), "The network change has not completed");
}

#[test]
fn an_accessory_join_with_a_valid_passphrase_starts_and_then_reports_success() {
    let mut join = AccessoryHotspotJoin::new();
    assert_eq!(join.join("Thermostat", "Home", "12345678"), Ok(()));
    assert!(join.finish(Ok(())));
    assert_eq!(join.state(), &JoinState::Joined { accessory: "Thermostat".to_string(), ssid: "Home".to_string() });
    assert_eq!(join.state().to_string(), "Thermostat joined Home");
}

#[test]
fn an_accessory_join_reports_a_refused_outcome_as_failed() {
    let mut join = AccessoryHotspotJoin::new();
    join.join_without_security("Plug", "Guest").unwrap();
    assert!(join.finish(Err(HotspotError::InvalidSsid)));
    assert_eq!(join.state(), &JoinState::Failed(HotspotError::InvalidSsid));
}

#[test]
fn a_helper_queues_commands_in_order_and_answers_them() {
    let mut helper = Helper::register("Sign-in", &[CommandKind::Evaluate, CommandKind::Authenticate]).unwrap();
    helper.deliver(Command { kind: CommandKind::Evaluate, ssid: "Cafe".to_string() }).unwrap();
    helper.deliver(Command { kind: CommandKind::Authenticate, ssid: "Cafe".to_string() }).unwrap();
    let first = helper.take_command().unwrap();
    let response = helper.respond(&first, Outcome::Success, Confidence::High).unwrap();
    assert_eq!(response.confidence, Confidence::High);
    assert_eq!(helper.pending_count(), 1);
}

#[test]
fn a_helper_refuses_a_command_it_did_not_register_for() {
    let mut helper = Helper::register("Sign-in", &[CommandKind::Logoff]).unwrap();
    let command = Command { kind: CommandKind::Maintain, ssid: "Cafe".to_string() };
    assert_eq!(helper.deliver(command.clone()), Err(HelperError::NotRegistered));
    assert_eq!(helper.respond(&command, Outcome::Success, Confidence::None), Err(HelperError::NotRegistered));
}

#[test]
fn a_helper_gives_confidence_only_for_a_successful_evaluation() {
    let helper = Helper::register("Sign-in", &[CommandKind::Evaluate, CommandKind::Maintain]).unwrap();
    let evaluate = Command { kind: CommandKind::Evaluate, ssid: "Cafe".to_string() };
    let maintain = Command { kind: CommandKind::Maintain, ssid: "Cafe".to_string() };
    assert_eq!(helper.respond(&evaluate, Outcome::Failure, Confidence::Low), Err(HelperError::ConfidenceNotAllowed));
    assert_eq!(helper.respond(&maintain, Outcome::Success, Confidence::High), Err(HelperError::ConfidenceNotAllowed));
    assert!(helper.respond(&maintain, Outcome::Success, Confidence::None).is_ok());
}
