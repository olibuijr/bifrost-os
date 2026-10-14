//! Parity sdk-complete-09: headless tests for the entitlement queries of an app policy. The policy is built from the
//! manifest strings the shell reads; nothing launches an app.

use springboard::bare::app_policy::{Policy, ENTITLEMENT_NAMES};
use springboard::shell::kinds::SYSTEM_APP;

#[test]
fn a_default_policy_holds_no_entitlement() {
    let policy = Policy::default();
    assert!(policy.held().is_empty());
    assert!(ENTITLEMENT_NAMES.iter().all(|name| !policy.holds(name)));
}

#[test]
fn a_policy_from_manifest_permissions_holds_exactly_those_names_in_query_order() {
    let permissions = ["documents".to_string(), "network".to_string()];
    let policy = Policy::from_permissions("example.notes", &permissions).unwrap();
    assert_eq!(policy.held(), vec!["network", "documents"]);
    assert!(policy.holds("documents"));
    assert!(!policy.holds("storage"));
}

#[test]
fn an_unknown_name_is_held_by_no_one() {
    let policy = Policy::from_permissions("example.notes", &["network".to_string()]).unwrap();
    assert!(!policy.holds("system-update"));
    assert!(!policy.holds(""));
    assert!(!policy.holds("Network"), "names are matched exactly, as the manifest spells them");
}

#[test]
fn the_log_store_entitlement_is_held_only_by_a_first_party_system_app() {
    let granted = [bifrost_core::log::ENTITLEMENT.to_string()];
    let first_party = Policy::default().with_entitlements("is.olibuijr.console", SYSTEM_APP, &granted);
    assert!(first_party.holds(bifrost_core::log::ENTITLEMENT));
    assert_eq!(first_party.held(), vec![bifrost_core::log::ENTITLEMENT]);

    let store_app = Policy::default().with_entitlements("example.console", SYSTEM_APP, &granted);
    assert!(!store_app.holds(bifrost_core::log::ENTITLEMENT));
    assert!(store_app.held().is_empty());
}

#[test]
fn held_names_follow_the_order_of_the_entitlement_names() {
    let permissions = ["remote-desktop".to_string(), "storage".to_string(), "network".to_string()];
    let policy = Policy::from_permissions("is.olibuijr.remote.desktop", &permissions).unwrap();
    assert_eq!(policy.held(), vec!["network", "storage", "remote-desktop"]);
}
