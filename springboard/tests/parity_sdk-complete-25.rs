//! Parity sdk-complete-25: headless tests of the app permission and entitlement policy. The policy is a pure value
//! built from the manifest's permission names; no test launches a process, opens a sandbox or reads an account.

use springboard::bare::app_policy::Policy;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_manifest_with_no_permissions_grants_nothing() {
    let p = Policy::from_permissions("is.example.notes", &[]).unwrap();
    assert_eq!(p, Policy::default());
    assert!(!p.network && !p.storage && !p.audio && !p.notifications && !p.documents);
}

#[test]
fn each_declared_permission_sets_exactly_its_own_flag() {
    let network = Policy::from_permissions("is.example.notes", &names(&["network"])).unwrap();
    assert!(network.network);
    assert!(!network.storage && !network.keychain);

    let all = names(&[
        "network",
        "storage",
        "audio",
        "notifications",
        "documents",
        "keychain",
        "accessibility",
        "display-config",
    ]);
    let p = Policy::from_permissions("is.example.notes", &all).unwrap();
    assert!(p.network && p.storage && p.audio && p.notifications);
    assert!(p.documents && p.keychain && p.accessibility && p.display_config);
    assert!(!p.remote && !p.log_store, "remote and the journal are never set by a plain permission list");
}

#[test]
fn an_unknown_permission_name_is_refused_with_its_name() {
    let err = Policy::from_permissions("is.example.notes", &names(&["network", "location"])).unwrap_err();
    assert_eq!(err.to_string(), "Unsupported app permission: location");
}

#[test]
fn remote_desktop_is_only_for_the_first_party_remote_app() {
    let err = Policy::from_permissions("is.example.notes", &names(&["remote-desktop"])).unwrap_err();
    assert_eq!(err.to_string(), "Unsupported app permission: remote-desktop");
}

#[test]
fn remote_desktop_needs_network_and_storage_too() {
    let id = "is.olibuijr.remote.desktop";
    let missing_storage = Policy::from_permissions(id, &names(&["network", "remote-desktop"]));
    assert_eq!(missing_storage.unwrap_err().to_string(), "Remote sharing requires network and storage");
    let complete = Policy::from_permissions(id, &names(&["network", "storage", "remote-desktop"])).unwrap();
    assert!(complete.remote && complete.network && complete.storage);
}

#[test]
fn the_log_store_entitlement_needs_a_first_party_system_app() {
    let entitlements = names(&["log-store"]);
    let first_party_system = Policy::default().with_entitlements("is.olibuijr.console", "system-app", &entitlements);
    assert!(first_party_system.log_store);

    let store_package = Policy::default().with_entitlements("com.example.console", "system-app", &entitlements);
    assert!(!store_package.log_store, "a store package cannot grant itself the journal");

    let ordinary_app = Policy::default().with_entitlements("is.olibuijr.console", "app", &entitlements);
    assert!(!ordinary_app.log_store, "only a system app may hold the entitlement");
}

#[test]
fn an_entitlement_list_without_the_log_store_grants_no_journal() {
    let p = Policy::default().with_entitlements("is.olibuijr.console", "system-app", &names(&["other"]));
    assert!(!p.log_store);
}
