//! Parity sdk-complete-17: headless tests for the app launch policy. The manifest permissions map to the flags a
//! launched app receives, unknown permissions are refused, remote sharing needs its own app id and the storage and
//! network permissions, and the journal entitlement is first-party system-app only. No process is launched.

use springboard::bare::app_policy::Policy;

fn permissions(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn an_app_with_no_permissions_receives_no_flags() {
    let policy = Policy::from_permissions("com.example.plain", &[]).expect("no permissions is valid");
    assert_eq!(policy, Policy::default());
}

#[test]
fn each_listed_permission_sets_only_its_own_flag() {
    let policy = Policy::from_permissions("com.example.app", &permissions(&["network"])).expect("valid");
    assert_eq!(policy, Policy { network: true, ..Policy::default() });
}

#[test]
fn every_public_permission_name_maps_to_its_flag() {
    let names = ["storage", "audio", "notifications", "documents", "keychain", "accessibility", "display-config"];
    let policy = Policy::from_permissions("com.example.app", &permissions(&names)).expect("valid");
    assert!(policy.storage && policy.audio && policy.notifications && policy.documents);
    assert!(policy.keychain && policy.accessibility && policy.display_config);
    assert!(!policy.network && !policy.remote && !policy.log_store);
}

#[test]
fn an_unknown_permission_is_refused() {
    let err = Policy::from_permissions("com.example.app", &permissions(&["camera"])).expect_err("camera is not a permission");
    assert!(err.to_string().contains("camera"), "the error names the permission");
}

#[test]
fn remote_sharing_is_granted_only_to_the_remote_desktop_app_with_network_and_storage() {
    let granted = permissions(&["remote-desktop", "network", "storage"]);
    let policy = Policy::from_permissions("is.olibuijr.remote.desktop", &granted).expect("valid for the remote app");
    assert!(policy.remote);

    assert!(Policy::from_permissions("com.example.app", &granted).is_err(), "another app cannot ask for it");
    let without_storage = permissions(&["remote-desktop", "network"]);
    assert!(Policy::from_permissions("is.olibuijr.remote.desktop", &without_storage).is_err());
}

#[test]
fn the_log_store_entitlement_needs_a_first_party_system_app() {
    let entitlements = permissions(&["log-store"]);
    let base = Policy::default();
    assert!(base.with_entitlements("is.olibuijr.console", "system-app", &entitlements).log_store);
    assert!(!base.with_entitlements("com.example.console", "system-app", &entitlements).log_store, "a store id cannot take it");
    assert!(!base.with_entitlements("is.olibuijr.console", "store", &entitlements).log_store, "a store kind cannot take it");
    assert!(!base.with_entitlements("is.olibuijr.console", "system-app", &[]).log_store, "the entitlement must be declared");
}
