//! Parity sdk-tests-05: headless tests for the DNS-SD operation references (`bifrost_core::dnssd_ref`) and for the
//! local service browser (`bifrost_core::dns_browse`). Both are pure models: replies are pushed in by the test and no
//! socket, daemon or network is touched.

use bifrost_core::dns_browse::{BrowseError, Browser, Change, ServiceType, ServiceTypeError};
use bifrost_core::dnssd_ref::{Kind, RefError, Registry};
use bifrost_core::net_connection::Transport;
use bifrost_core::txt_record::{Presence, TxtRecord};

// Operation references.

#[test]
fn headless_a_new_registry_numbers_its_references_from_one() {
    let mut registry = Registry::new();
    let first: bifrost_core::dnssd_ref::Ref<u8> = registry.open(Kind::Browse);
    let second: bifrost_core::dnssd_ref::Ref<u8> = registry.open(Kind::Resolve);
    assert_eq!(first.number(), Some(1));
    assert_eq!(second.number(), Some(2), "each reference gets a fresh number");
    assert_eq!(first.kind(), Kind::Browse);
    assert_eq!(second.kind(), Kind::Resolve);
    assert!(first.is_open() && second.is_open());
}

#[test]
fn headless_replies_are_processed_oldest_first() {
    let mut registry = Registry::new();
    let mut r = registry.open::<u32>(Kind::Browse);
    r.push(10).unwrap();
    r.push(20).unwrap();
    r.push(30).unwrap();
    assert_eq!(r.pending(), 3);
    assert_eq!(r.process_result(), Ok(Some(10)));
    assert_eq!(r.process_result(), Ok(Some(20)));
    assert_eq!(r.pending(), 1);
}

#[test]
fn headless_processing_with_nothing_waiting_returns_no_reply_and_no_error() {
    let mut registry = Registry::new();
    let mut r = registry.open::<u32>(Kind::QueryRecord);
    assert_eq!(r.process_result(), Ok(None), "where the daemon call would block, nothing is returned");
}

#[test]
fn headless_deallocating_drops_the_pending_replies_and_retires_the_number() {
    let mut registry = Registry::new();
    let mut r = registry.open::<u32>(Kind::Register);
    r.push(1).unwrap();
    r.push(2).unwrap();
    assert_eq!(r.deallocate(), 2, "the two replies still waiting are dropped");
    assert_eq!(r.number(), None);
    assert!(!r.is_open());
    assert_eq!(r.pending(), 0);
}

#[test]
fn headless_a_deallocated_reference_refuses_replies_and_processing() {
    let mut registry = Registry::new();
    let mut r = registry.open::<u32>(Kind::Browse);
    r.deallocate();
    assert_eq!(r.push(5), Err(RefError::Deallocated));
    assert_eq!(r.process_result(), Err(RefError::Deallocated));
    assert_eq!(r.pending(), 0, "a refused reply is not kept");
}

#[test]
fn deallocating_twice_drops_nothing_the_second_time() {
    let mut registry = Registry::new();
    let mut r = registry.open::<u32>(Kind::Browse);
    r.push(7).unwrap();
    assert_eq!(r.deallocate(), 1);
    assert_eq!(r.deallocate(), 0);
}

#[test]
fn a_retired_number_is_not_reused_by_a_later_reference() {
    let mut registry = Registry::new();
    let mut first = registry.open::<u32>(Kind::Browse);
    first.deallocate();
    let second = registry.open::<u32>(Kind::Browse);
    assert_eq!(second.number(), Some(2), "numbers keep counting after a deallocation");
}

#[test]
fn the_refusal_names_the_deallocation() {
    assert_eq!(RefError::Deallocated.to_string(), "the DNS-SD reference was deallocated");
}

// Service types.

#[test]
fn headless_a_service_type_parses_its_transport_and_lowers_its_name() {
    let tcp = ServiceType::parse("_HTTP._tcp").unwrap();
    assert_eq!(tcp.name(), "http");
    assert_eq!(tcp.transport(), Transport::Tcp);
    assert_eq!(ServiceType::parse("_dns._udp").unwrap().transport(), Transport::Udp);
}

#[test]
fn headless_a_service_type_without_its_underscore_or_dot_is_a_format_error() {
    assert_eq!(ServiceType::parse("http._tcp"), Err(ServiceTypeError::Format));
    assert_eq!(ServiceType::parse("_http"), Err(ServiceTypeError::Format));
}

#[test]
fn headless_a_protocol_other_than_tcp_or_udp_is_refused() {
    assert_eq!(ServiceType::parse("_http.tcp"), Err(ServiceTypeError::BadTransport));
    assert_eq!(ServiceType::parse("_http._sctp"), Err(ServiceTypeError::BadTransport));
}

#[test]
fn headless_a_service_name_must_follow_the_character_rules() {
    assert_eq!(ServiceType::parse("_-http._tcp"), Err(ServiceTypeError::BadName), "no leading hyphen");
    assert_eq!(ServiceType::parse("_ht--tp._tcp"), Err(ServiceTypeError::BadName), "no double hyphen");
    assert_eq!(ServiceType::parse("_123._tcp"), Err(ServiceTypeError::BadName), "at least one letter");
    assert_eq!(ServiceType::parse("_._tcp"), Err(ServiceTypeError::BadName), "not empty");
    assert_eq!(ServiceType::parse("_abcdefghijklmnop._tcp"), Err(ServiceTypeError::BadName), "at most 15 bytes");
    assert_eq!(ServiceType::parse("_ht_tp._tcp"), Err(ServiceTypeError::BadName), "no underscore inside the name");
    assert!(ServiceType::parse("_abcdefghijklmno._tcp").is_ok(), "15 bytes is the longest name");
}

#[test]
fn a_service_type_builds_its_dns_name_under_the_domain() {
    let t = ServiceType::parse("_http._tcp").unwrap();
    assert_eq!(t.dns_name("local."), "_http._tcp.local.");
    assert_eq!(t.dns_name(""), "_http._tcp.local.", "an empty domain means local");
    assert_eq!(t.dns_name("example.net"), "_http._tcp.example.net.", "a trailing dot is optional");
}

// The service browser.

#[test]
fn headless_a_browser_reports_an_instance_once_as_added_and_once_as_removed() {
    let mut b = Browser::new(ServiceType::parse("_http._tcp").unwrap(), "");
    assert_eq!(b.browse_reply(true, "Kitchen"), Ok(Some(Change::Added("Kitchen".into()))));
    assert_eq!(b.browse_reply(true, "Kitchen"), Ok(None), "a repeated add changes nothing");
    assert_eq!(b.browse_reply(false, "kitchen"), Ok(Some(Change::Removed("Kitchen".into()))), "names compare without case");
    assert_eq!(b.browse_reply(false, "Kitchen"), Ok(None), "a removal of an unknown name changes nothing");
    assert!(b.instances().is_empty());
}

#[test]
fn headless_a_browse_name_must_be_present_and_at_most_63_bytes() {
    let mut b = Browser::new(ServiceType::parse("_http._tcp").unwrap(), "");
    assert_eq!(b.browse_reply(true, ""), Err(BrowseError::EmptyName));
    assert_eq!(b.browse_reply(true, &"a".repeat(64)), Err(BrowseError::NameTooLong));
    assert!(matches!(b.browse_reply(true, &"a".repeat(63)), Ok(Some(Change::Added(_)))), "63 bytes is the longest label");
}

#[test]
fn a_new_instance_is_not_resolved_until_its_resolve_reply_arrives() {
    let mut b = Browser::new(ServiceType::parse("_http._tcp").unwrap(), "local");
    b.browse_reply(true, "Kitchen").unwrap();
    let before = b.instance("Kitchen").unwrap();
    assert_eq!((before.host.as_deref(), before.port), (None, None));
    assert!(!before.is_resolved());
    assert!(before.txt.is_empty());
}

#[test]
fn headless_a_resolve_reply_sets_host_port_and_txt_of_a_known_instance() {
    let mut b = Browser::new(ServiceType::parse("_http._tcp").unwrap(), "local");
    b.browse_reply(true, "Kitchen").unwrap();
    let mut txt = TxtRecord::new();
    txt.set("path", Some(&b"/status"[..])).unwrap();
    assert!(b.resolve_reply("KITCHEN", "mac.local.", 631, txt));
    let i = b.instance("kitchen").unwrap();
    assert_eq!(i.host.as_deref(), Some("mac.local"), "the trailing dot is dropped");
    assert_eq!(i.port, Some(631));
    assert!(i.is_resolved());
    assert!(i.txt.contains_key("path"));
    assert_eq!(i.txt.get("path"), Presence::Value(&b"/status"[..]));
}

#[test]
fn headless_a_resolve_reply_for_an_unknown_instance_changes_nothing() {
    let mut b = Browser::new(ServiceType::parse("_http._tcp").unwrap(), "local");
    assert!(!b.resolve_reply("Ghost", "ghost.local.", 80, TxtRecord::new()));
    assert!(b.instances().is_empty());
}

#[test]
fn headless_instances_are_listed_sorted_without_regard_to_case() {
    let mut b = Browser::new(ServiceType::parse("_ipp._tcp").unwrap(), "local");
    for name in ["beta", "Alpha", "gamma"] {
        b.browse_reply(true, name).unwrap();
    }
    let names: Vec<&str> = b.instances().iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Alpha", "beta", "gamma"]);
}

#[test]
fn a_browser_names_its_instances_under_its_service_and_domain() {
    let b = Browser::new(ServiceType::parse("_ipp._tcp").unwrap(), "");
    assert_eq!(b.domain(), "local");
    assert_eq!(b.kind().name(), "ipp");
    assert_eq!(b.full_name("Printer"), "Printer._ipp._tcp.local.");
    let elsewhere = Browser::new(ServiceType::parse("_ipp._tcp").unwrap(), "example.net.");
    assert_eq!(elsewhere.domain(), "example.net", "a trailing dot is dropped from the domain");
    assert_eq!(elsewhere.full_name("Printer"), "Printer._ipp._tcp.example.net.");
}
