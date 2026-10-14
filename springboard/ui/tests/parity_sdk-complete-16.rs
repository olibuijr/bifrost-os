//! Parity sdk-complete-16: headless tests for name and postal address formatting. Each test builds an in-memory
//! store, fetches a contact with the keys the formatter needs and checks the formatted text. No display, no device.

use bifrost_ui::contact_address::{lines, single_line, Style};
use bifrost_ui::contact_book::{
    format_name, formatter_required_keys, Contact, ContactKey, ContactStore, Error, FetchedContact, LabeledValue,
    NameOrder, PostalAddress, SaveRequest,
};

fn fetched(contact: Contact, keys: &[ContactKey]) -> FetchedContact {
    let mut store = ContactStore::new();
    let mut request = SaveRequest::new();
    request.add_contact(contact, None);
    store.execute(&request).expect("the store accepts a new contact");
    store.contacts_matching(None, keys).expect("the fetch names valid keys").remove(0)
}

fn person(given: &str, family: &str) -> Contact {
    Contact { given_name: given.into(), family_name: family.into(), ..Contact::default() }
}

fn name_of(contact: Contact, order: NameOrder) -> String {
    let fetched = fetched(contact, &formatter_required_keys());
    format_name(&fetched, order).expect("every required key was fetched")
}

#[test]
fn headless_given_name_first_joins_prefix_given_middle_family_and_suffix() {
    let contact = Contact {
        name_prefix: "Dr".into(),
        given_name: "Ann".into(),
        middle_name: "Marie".into(),
        family_name: "Lee".into(),
        name_suffix: "Jr".into(),
        ..Contact::default()
    };
    assert_eq!(name_of(contact, NameOrder::GivenNameFirst), "Dr Ann Marie Lee Jr");
}

#[test]
fn headless_family_name_first_puts_the_family_name_before_a_comma() {
    let contact = Contact { given_name: "Ann".into(), middle_name: "Marie".into(), family_name: "Lee".into(), ..Contact::default() };
    assert_eq!(name_of(contact, NameOrder::FamilyNameFirst), "Lee, Ann Marie");
}

#[test]
fn headless_family_name_first_without_a_family_name_is_the_given_part_alone() {
    assert_eq!(name_of(person("Ann", ""), NameOrder::FamilyNameFirst), "Ann");
    assert_eq!(name_of(person("", "Lee"), NameOrder::FamilyNameFirst), "Lee");
}

#[test]
fn headless_blank_name_parts_leave_no_stray_spaces() {
    let contact = Contact { given_name: "  Ann ".into(), family_name: "   ".into(), ..Contact::default() };
    assert_eq!(name_of(contact, NameOrder::GivenNameFirst), "Ann");
}

#[test]
fn headless_a_contact_without_a_name_falls_back_to_the_nickname_then_the_organization() {
    let nick = Contact { nickname: "Annie".into(), organization_name: "Acme".into(), ..Contact::default() };
    assert_eq!(name_of(nick, NameOrder::GivenNameFirst), "Annie");
    let org = Contact { organization_name: "Acme".into(), ..Contact::default() };
    assert_eq!(name_of(org, NameOrder::FamilyNameFirst), "Acme");
    assert_eq!(name_of(Contact::default(), NameOrder::GivenNameFirst), "");
}

#[test]
fn headless_formatting_a_name_fails_for_a_key_that_was_not_fetched() {
    let contact = person("Ann", "Lee");
    let keys = [ContactKey::GivenName, ContactKey::MiddleName, ContactKey::NamePrefix];
    let fetched = fetched(contact, &keys);
    assert_eq!(format_name(&fetched, NameOrder::GivenNameFirst), Err(Error::PropertyNotFetched(ContactKey::FamilyName)));
}

#[test]
fn headless_the_formatter_requires_the_seven_name_keys() {
    let keys = formatter_required_keys();
    assert_eq!(keys.len(), 7);
    assert!(keys.contains(&ContactKey::OrganizationName));
    assert!(!keys.contains(&ContactKey::EmailAddresses));
}

fn reykjavik() -> PostalAddress {
    PostalAddress {
        street: "Laugavegur 12".into(),
        city: "Reykjavik".into(),
        postal_code: "101".into(),
        country: "Iceland".into(),
        ..PostalAddress::default()
    }
}

#[test]
fn headless_a_city_first_address_prints_one_line_per_part() {
    let address = PostalAddress {
        street: "1 Main St".into(),
        city: "Springfield".into(),
        state: "IL".into(),
        postal_code: "62701".into(),
        country: "USA".into(),
        ..PostalAddress::default()
    };
    assert_eq!(lines(&address, Style::CityFirst), ["1 Main St", "Springfield, IL 62701", "USA"]);
    assert_eq!(single_line(&address, Style::CityFirst), "1 Main St, Springfield, IL 62701, USA");
}

#[test]
fn headless_a_postal_first_address_puts_the_code_before_the_city() {
    assert_eq!(lines(&reykjavik(), Style::PostalFirst), ["Laugavegur 12", "101 Reykjavik", "Iceland"]);
}

#[test]
fn headless_an_address_with_only_a_city_prints_just_that_line() {
    let address = PostalAddress { city: "Oslo".into(), ..PostalAddress::default() };
    assert_eq!(lines(&address, Style::CityFirst), ["Oslo"]);
    assert_eq!(single_line(&PostalAddress::default(), Style::CityFirst), "");
}

#[test]
fn headless_a_fetched_postal_address_formats_from_the_store() {
    let mut contact = person("Ann", "Lee");
    contact.postal_addresses.push(LabeledValue::new(Some("home"), reykjavik()));
    let fetched = fetched(contact, &[ContactKey::PostalAddresses]);
    let stored = fetched.postal_addresses().expect("the postal addresses were fetched");
    assert_eq!(single_line(&stored[0].value, Style::PostalFirst), "Laugavegur 12, 101 Reykjavik, Iceland");
}
