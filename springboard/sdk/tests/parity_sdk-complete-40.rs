//! Parity sdk-complete-40: headless tests for the remote device model (name, class, paired, connected) and for the
//! contact search items. They use the models only: no radio, no socket, no index service.

use bifrost_app::bluetooth_devices::{DeviceList, RemoteDevice};
use bifrost_app::contact_index::{self, DOMAIN};
use bifrost_core::bt_address::{BdAddr, ClassOfDevice};
use bifrost_ui::contact_book::{Contact, LabeledValue};

fn addr(n: u8) -> BdAddr {
    BdAddr([0, 0, 0, 0, 0, n])
}

fn class(raw: u32) -> Option<ClassOfDevice> {
    ClassOfDevice::new(raw)
}

#[test]
fn a_device_without_a_class_reads_unknown() {
    let d = RemoteDevice::new(addr(1));
    assert_eq!(d.type_label(), "Unknown");
}

#[test]
fn an_audio_video_headset_class_reads_headset() {
    let mut list = DeviceList::new();
    list.report(addr(2), class(0x240404), Some("Buds"), None);
    assert_eq!(list.get(addr(2)).unwrap().type_label(), "Headset");
}

#[test]
fn an_audio_video_loudspeaker_class_reads_speaker() {
    let mut list = DeviceList::new();
    list.report(addr(3), class(0x240414), Some("Kitchen"), None);
    assert_eq!(list.get(addr(3)).unwrap().type_label(), "Speaker");
}

#[test]
fn peripheral_classes_say_keyboard_or_pointing_device() {
    let mut list = DeviceList::new();
    list.report(addr(4), class(0x000540), None, None);
    list.report(addr(5), class(0x000580), None, None);
    assert_eq!(list.get(addr(4)).unwrap().type_label(), "Keyboard");
    assert_eq!(list.get(addr(5)).unwrap().type_label(), "Pointing Device");
}

#[test]
fn a_computer_class_reads_computer_and_an_uncategorised_class_reads_device() {
    let mut list = DeviceList::new();
    list.report(addr(6), class(0x000100), None, None);
    list.report(addr(7), class(0x001F00), None, None);
    assert_eq!(list.get(addr(6)).unwrap().type_label(), "Computer");
    assert_eq!(list.get(addr(7)).unwrap().type_label(), "Device");
}

#[test]
fn the_label_is_the_name_and_falls_back_to_the_address() {
    let mut list = DeviceList::new();
    list.report(addr(8), None, None, None);
    assert_eq!(list.get(addr(8)).unwrap().label(), "00:00:00:00:00:08");
    list.report(addr(8), None, Some("Desk speaker"), None);
    assert_eq!(list.get(addr(8)).unwrap().label(), "Desk speaker");
}

#[test]
fn a_paired_device_that_is_also_connected_appears_in_both_filters() {
    let mut list = DeviceList::new();
    list.set_paired(addr(9), true);
    list.set_connected(addr(9), true);
    list.report(addr(10), None, Some("Stranger"), None);
    assert_eq!(list.paired().len(), 1);
    assert_eq!(list.connected().len(), 1);
    assert_eq!(list.all().len(), 2);
    assert!(list.all()[0].paired, "paired devices come first");
}

#[test]
fn a_device_list_forgets_a_device_and_its_state() {
    let mut list = DeviceList::new();
    list.set_paired(addr(11), true);
    list.set_connected(addr(11), true);
    assert!(list.forget(addr(11)));
    assert!(list.is_empty());
    assert!(!list.forget(addr(11)), "a second forget finds nothing");
}

fn ada() -> Contact {
    Contact {
        identifier: "c-ada".into(),
        given_name: "Ada".into(),
        family_name: "Lovelace".into(),
        organization_name: "Analytical Engine Co".into(),
        phone_numbers: vec![LabeledValue::new(Some("mobile"), "+44 20 7946 0000".to_string())],
        email_addresses: vec![LabeledValue::new(Some("home"), "ada@example.com".to_string())],
        ..Contact::default()
    }
}

#[test]
fn a_contact_title_is_the_personal_name_in_order() {
    assert_eq!(contact_index::display_name(&ada()), "Ada Lovelace");
}

#[test]
fn a_contact_without_a_personal_name_is_titled_by_its_organization() {
    let company = Contact { identifier: "c-co".into(), organization_name: "Acme".into(), ..Contact::default() };
    assert_eq!(contact_index::display_name(&company), "Acme");
}

#[test]
fn a_searchable_item_is_identified_by_its_contact_and_grouped_under_the_contacts_domain() {
    let item = contact_index::searchable_item(&ada()).expect("a named contact is indexed");
    assert_eq!(item.unique_identifier, "contacts.c-ada");
    assert_eq!(item.domain_identifier, DOMAIN);
    assert_eq!(item.attribute_set.title, "Ada Lovelace");
    assert_eq!(item.attribute_set.content_type, "public.contact");
}

#[test]
fn a_searchable_item_finds_the_person_by_phone_email_and_organization() {
    let item = contact_index::searchable_item(&ada()).expect("a named contact is indexed");
    let keywords = &item.attribute_set.keywords;
    assert!(keywords.contains(&"ada@example.com".to_string()));
    assert!(keywords.contains(&"+44 20 7946 0000".to_string()));
    assert!(keywords.contains(&"Analytical Engine Co".to_string()));
    assert_eq!(keywords.len(), 3, "no blank nickname or job title is added");
}

#[test]
fn a_contact_with_no_name_or_no_identifier_is_not_indexed() {
    assert!(contact_index::searchable_item(&Contact { identifier: "c-x".into(), ..Contact::default() }).is_none());
    assert!(contact_index::searchable_item(&Contact { given_name: "Grace".into(), ..Contact::default() }).is_none());
}

#[test]
fn a_batch_keeps_the_indexable_contacts_in_order() {
    let unnamed = Contact { given_name: "Unsaved".into(), ..Contact::default() };
    let batch = [ada(), unnamed, Contact { identifier: "c-grace".into(), given_name: "Grace".into(), ..Contact::default() }];
    let items = contact_index::searchable_items(&batch);
    let ids: Vec<&str> = items.iter().map(|i| i.unique_identifier.as_str()).collect();
    assert_eq!(ids, vec!["contacts.c-ada", "contacts.c-grace"]);
}
