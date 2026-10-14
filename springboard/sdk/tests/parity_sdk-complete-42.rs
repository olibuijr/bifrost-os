//! Parity sdk-complete-42: headless tests of the sign-in suggestions and save decisions for website logins, and of the
//! HID device counts and ignore list. The tests read device lists and login lists that they build themselves. No test
//! touches the radio, the keychain store or a display.

use bifrost_app::bluetooth_devices::DeviceList;
use bifrost_app::bluetooth_hid::{counts, is_hid, HidCounts, IgnoreList};
use bifrost_app::password_suggest::{host_of, save_decision, site_matches, suggestions, SaveDecision};
use bifrost_core::bt_address::{BdAddr, ClassOfDevice};
use springboard::credentials::Login;

fn login(site: &str, account: &str, modified: u64) -> Login {
    Login { site: site.into(), account: account.into(), modified }
}

#[test]
fn the_host_of_a_page_drops_scheme_www_port_path_and_user_information() {
    assert_eq!(host_of("https://WWW.Example.com:8443/login?next=1#top").as_deref(), Some("example.com"));
    assert_eq!(host_of("alice@intranet.local/path").as_deref(), Some("intranet.local"));
    assert_eq!(host_of("example.com.").as_deref(), Some("example.com"));
    assert_eq!(host_of("https://[::1]:8080/").as_deref(), Some("::1"));
}

#[test]
fn an_address_with_no_host_has_no_host() {
    assert_eq!(host_of(""), None);
    assert_eq!(host_of("https:///path-only"), None);
}

#[test]
fn a_saved_site_fits_its_own_host_and_the_subdomains_of_it() {
    assert!(site_matches("example.com", "https://login.example.com/"));
    assert!(site_matches("www.example.com", "example.com"));
    assert!(site_matches("example.com", "example.com:443"));
}

#[test]
fn a_saved_site_does_not_fit_a_host_that_only_ends_the_same_way() {
    assert!(!site_matches("example.com", "https://notexample.com/"));
    assert!(!site_matches("login.example.com", "example.com"));
    assert!(!site_matches("example.com", ""));
    assert!(!site_matches("", "example.com"));
}

#[test]
fn suggestions_are_the_fitting_logins_newest_first_then_by_account() {
    let logins = vec![
        login("example.com", "alice", 10),
        login("login.example.com", "bob", 30),
        login("other.org", "carol", 50),
        login("example.com", "anna", 10),
    ];
    let accounts: Vec<&str> = suggestions(&logins, "https://example.com/signin").iter().map(|l| l.account.as_str()).collect();
    assert_eq!(accounts, vec!["bob", "alice", "anna"]);
}

#[test]
fn a_page_with_no_fitting_login_gets_no_suggestions() {
    let logins = vec![login("example.com", "alice", 10)];
    assert!(suggestions(&logins, "https://elsewhere.org/").is_empty());
    assert!(suggestions(&[], "https://example.com/").is_empty());
}

#[test]
fn a_submitted_empty_password_is_never_saved() {
    assert_eq!(save_decision(Some("hunter2"), ""), SaveDecision::Nothing);
    assert_eq!(save_decision(None, ""), SaveDecision::Nothing);
}

#[test]
fn a_new_login_is_saved_and_an_unchanged_one_is_left_alone() {
    assert_eq!(save_decision(None, "hunter2"), SaveDecision::Save);
    assert_eq!(save_decision(Some("hunter2"), "hunter2"), SaveDecision::Unchanged);
}

#[test]
fn a_changed_password_is_an_update_and_the_comparison_is_exact() {
    assert_eq!(save_decision(Some("hunter2"), "Hunter2"), SaveDecision::Update);
    assert_eq!(save_decision(Some("hunter2"), "hunter2 "), SaveDecision::Update);
    assert_eq!(save_decision(Some(""), "hunter2"), SaveDecision::Update);
}

fn addr(n: u8) -> BdAddr {
    BdAddr([0, 0, 0, 0, 0, n])
}

fn class(raw: u32) -> Option<ClassOfDevice> {
    ClassOfDevice::new(raw)
}

/// Two keyboards (peripheral class with the keyboard bit), a headset (audio-video class) and a pointer.
fn devices() -> DeviceList {
    let mut list = DeviceList::new();
    list.report(addr(1), class(0x002540), Some("Keyboard"), None);
    list.report(addr(2), class(0x002540), Some("Second keyboard"), None);
    list.report(addr(3), class(0x240404), Some("Headset"), None);
    list.report(addr(4), class(0x002580), Some("Mouse"), None);
    list
}

#[test]
fn a_paired_but_disconnected_hid_device_counts_as_paired_only() {
    let mut list = devices();
    list.set_paired(addr(1), true);
    assert_eq!(counts(&list, &IgnoreList::new()), HidCounts { connected: 0, paired: 1 });
}

#[test]
fn an_audio_device_is_never_counted_as_hid_however_it_is_paired() {
    let mut list = devices();
    list.set_paired(addr(3), true);
    list.set_connected(addr(3), true);
    assert!(!is_hid(list.get(addr(3)).unwrap()));
    assert_eq!(counts(&list, &IgnoreList::new()), HidCounts::default());
}

#[test]
fn ignoring_one_of_two_connected_keyboards_leaves_the_other_counted() {
    let mut list = devices();
    for n in [1, 2] {
        list.set_paired(addr(n), true);
        list.set_connected(addr(n), true);
    }
    let mut ignore = IgnoreList::new();
    assert!(ignore.ignore(addr(2)));
    assert_eq!(counts(&list, &ignore), HidCounts { connected: 1, paired: 1 });
    assert!(ignore.is_ignored(addr(2)));
    assert!(!ignore.is_ignored(addr(1)));
}

#[test]
fn ignoring_the_same_address_twice_keeps_one_entry() {
    let mut ignore = IgnoreList::new();
    assert!(ignore.ignore(addr(5)));
    assert!(!ignore.ignore(addr(5)));
    assert_eq!(ignore.len(), 1);
    assert!(!ignore.is_empty());
}

#[test]
fn a_new_ignore_list_is_empty_and_ignores_nothing() {
    let ignore = IgnoreList::default();
    assert!(ignore.is_empty());
    assert_eq!(ignore.len(), 0);
    assert!(!ignore.is_ignored(addr(1)));
}

#[test]
fn an_ignored_address_that_is_forgotten_keeps_its_ignore_entry() {
    let mut list = devices();
    list.set_paired(addr(1), true);
    list.set_connected(addr(1), true);
    let mut ignore = IgnoreList::new();
    ignore.ignore(addr(1));
    assert!(list.forget(addr(1)));
    assert_eq!(counts(&list, &ignore), HidCounts::default());
    assert!(ignore.is_ignored(addr(1)), "forgetting a device does not change the ignore list");
    assert!(ignore.remove(addr(1)));
}
