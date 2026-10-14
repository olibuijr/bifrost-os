//! Parity sdk-complete-08: headless tests for the guest USB passthrough policy. The tests decide and attach device
//! descriptors held in memory; no host USB bus is read and no guest is started.

use bifrost_core::guest_usb::{Action, Error, Passthrough, Rule, UsbDevice, CLASS_HUB, MAX_ATTACHED};

fn device(id: u32, vendor: u16, product: u16, class: u8) -> UsbDevice {
    UsbDevice { id, vendor, product, class }
}

fn allow_all() -> Rule {
    Rule { vendor: None, product: None, class: None, action: Action::Allow }
}

#[test]
fn a_rule_with_no_fields_matches_every_device() {
    let rule = allow_all();
    assert!(rule.matches(&device(1, 0x1234, 0x0001, 0x03)));
    assert!(rule.matches(&device(2, 0xffff, 0xffff, 0xfe)));
}

#[test]
fn a_rule_matches_the_vendor_product_and_class_it_names() {
    let rule = Rule { vendor: Some(0x1234), product: Some(0x0002), class: None, action: Action::Allow };
    assert!(rule.matches(&device(1, 0x1234, 0x0002, 0x08)));
    assert!(!rule.matches(&device(2, 0x1234, 0x0003, 0x08)), "another product");
    assert!(!rule.matches(&device(3, 0x5678, 0x0002, 0x08)), "another vendor");
}

#[test]
fn the_first_matching_rule_decides_even_when_a_later_rule_would_allow() {
    let policy = Passthrough::new(vec![
        Rule { vendor: Some(0x1234), product: None, class: None, action: Action::Deny },
        allow_all(),
    ]);
    assert_eq!(policy.decide(&device(1, 0x1234, 0x0001, 0x03)), Action::Deny);
    assert_eq!(policy.decide(&device(2, 0x9999, 0x0001, 0x03)), Action::Allow);
}

#[test]
fn a_device_no_rule_matches_is_denied() {
    let policy = Passthrough::new(vec![Rule { vendor: None, product: None, class: Some(0x08), action: Action::Allow }]);
    assert_eq!(policy.decide(&device(1, 0x1111, 0x0001, 0x03)), Action::Deny);
    assert_eq!(policy.decide(&device(2, 0x1111, 0x0001, 0x08)), Action::Allow);
}

#[test]
fn a_hub_is_denied_even_under_an_allow_everything_rule() {
    let mut policy = Passthrough::new(vec![allow_all()]);
    assert_eq!(policy.decide(&device(1, 0x0424, 0x2514, CLASS_HUB)), Action::Deny);
    assert_eq!(policy.attach(device(1, 0x0424, 0x2514, CLASS_HUB)), Err(Error::Denied));
    assert!(policy.attached().is_empty());
}

#[test]
fn a_refused_attach_leaves_the_attached_list_unchanged() {
    let mut policy = Passthrough::new(vec![Rule { vendor: Some(0x2222), product: None, class: None, action: Action::Allow }]);
    policy.attach(device(1, 0x2222, 0x0001, 0x03)).unwrap();
    assert_eq!(policy.attach(device(2, 0x3333, 0x0001, 0x03)), Err(Error::Denied));
    assert_eq!(policy.attached(), [device(1, 0x2222, 0x0001, 0x03)]);
}

#[test]
fn deciding_a_device_does_not_attach_it() {
    let policy = Passthrough::new(vec![allow_all()]);
    assert_eq!(policy.decide(&device(1, 0x1111, 0x0001, 0x03)), Action::Allow);
    assert!(policy.attached().is_empty());
}

#[test]
fn detaching_frees_a_slot_under_the_limit() {
    let mut policy = Passthrough::new(vec![allow_all()]);
    for id in 0..MAX_ATTACHED as u32 {
        policy.attach(device(id, 0x1111, 0x0001, 0x03)).unwrap();
    }
    assert_eq!(policy.attach(device(99, 0x1111, 0x0001, 0x03)), Err(Error::Limit(MAX_ATTACHED)));
    let gone = policy.detach(3).unwrap();
    assert_eq!(gone.id, 3);
    policy.attach(device(99, 0x1111, 0x0001, 0x03)).unwrap();
    assert_eq!(policy.attached().len(), MAX_ATTACHED);
}

#[test]
fn a_detached_device_can_be_attached_again() {
    let mut policy = Passthrough::new(vec![allow_all()]);
    let dev = device(7, 0x4444, 0x0009, 0x03);
    policy.attach(dev).unwrap();
    assert_eq!(policy.detach(7), Ok(dev));
    assert_eq!(policy.detach(7), Err(Error::NotAttached));
    policy.attach(dev).unwrap();
    assert_eq!(policy.attached(), [dev]);
}

#[test]
fn two_identical_devices_are_told_apart_by_their_host_key() {
    let mut policy = Passthrough::new(vec![allow_all()]);
    policy.attach(device(10, 0x5555, 0x0001, 0x03)).unwrap();
    policy.attach(device(11, 0x5555, 0x0001, 0x03)).unwrap();
    assert_eq!(policy.attach(device(10, 0x5555, 0x0001, 0x03)), Err(Error::AlreadyAttached));
    policy.detach(10).unwrap();
    assert_eq!(policy.attached(), [device(11, 0x5555, 0x0001, 0x03)]);
}
