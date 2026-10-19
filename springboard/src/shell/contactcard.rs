//! vCard import and export for the contact store. The store keeps one name, one phone, one email and one address per
//! contact plus a sidecar of extras (`contactextras`); a card carries the full contact model. This module maps between
//! the two, so the `contacts` service can export the people an app may read and import cards the person gave it.
//!
//! Pure: no file access. `export` writes cards, `plan_import` reads cards and says what the book would become, and the
//! caller stores the plan only when it succeeded. Import never overwrites a contact: a card that matches a person
//! already in the book (same name and the same email or phone) is skipped and counted as a duplicate.
use super::contactextras::{Book as ExtraBook, Extras};
use super::contactstore::{self, Contact, MAX_NAME, MAX_NOTES, MAX_VALUE};
use super::services::contacts::{join_name, name_parts, NameParts};
use bifrost_ui::contact_book::{label, Contact as Card, DateComponents as CardDate, LabeledValue as CardValue, PostalAddress};
use bifrost_ui::contact_fetch::{DateComponents, LabeledValue};
use bifrost_ui::contact_vcard::{self, Version};
use std::collections::HashSet;

/// Longest text an extra value or label may hold in the sidecar.
const MAX_EXTRA: usize = 320;

/// Phone label names the store's label row can hold, in the order of `contactstore::LABELS[0]`.
const PHONE_LABELS: [&str; 4] = ["mobile", "home", "work", "other"];
/// Email and address label names, in the order of `contactstore::LABELS[1]` and `[2]`.
const PLACE_LABELS: [&str; 3] = ["home", "work", "other"];

/// A store label name as a card label: a predefined one when it has a standard counterpart, else free text.
fn card_label(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(label::from_standard_name(name).unwrap_or_else(|| name.to_string()))
}

/// A card label as a store label name: the lower-case standard name, free text as written, "other" when absent.
fn store_label(card: &Option<String>) -> String {
    match card {
        None => "other".into(),
        Some(l) => label::standard_name(l).map(str::to_lowercase).unwrap_or_else(|| l.clone()),
    }
}

fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')).take(max).collect()
}

fn index_of(set: &[&str], name: &str) -> u8 {
    let name = name.to_lowercase();
    set.iter().position(|l| *l == name).or_else(|| set.iter().position(|l| *l == "other")).unwrap_or(0) as u8
}

fn card_date(d: DateComponents) -> CardDate {
    CardDate { year: d.year, month: d.month, day: d.day }
}

fn store_date(d: CardDate) -> DateComponents {
    DateComponents { year: d.year, month: d.month, day: d.day }
}

/// The card of a store contact with its extras. The file's phone, email and address come first. `photo` is the
/// contact's picture bytes when it has one.
pub fn card_of(c: &Contact, extras: &Extras, photo: Option<Vec<u8>>) -> Card {
    let parts = name_parts(&c.name);
    let mut card = Card {
        identifier: c.id.to_string(),
        given_name: parts.given,
        middle_name: parts.middle,
        family_name: parts.family,
        name_suffix: parts.suffix,
        organization_name: c.company.clone(),
        note: c.notes.clone(),
        image_data: photo,
        ..Card::default()
    };
    if !c.phone.trim().is_empty() {
        let name = contactstore::LABELS[0].get(c.labels[0] as usize).copied().unwrap_or("other");
        card.phone_numbers.push(CardValue::new(card_label(name).as_deref(), c.phone.clone()));
    }
    for p in &extras.phones {
        card.phone_numbers.push(CardValue::new(card_label(&p.label).as_deref(), p.value.clone()));
    }
    if !c.email.trim().is_empty() {
        let name = contactstore::LABELS[1].get(c.labels[1] as usize).copied().unwrap_or("other");
        card.email_addresses.push(CardValue::new(card_label(name).as_deref(), c.email.clone()));
    }
    for e in &extras.emails {
        card.email_addresses.push(CardValue::new(card_label(&e.label).as_deref(), e.value.clone()));
    }
    for u in &extras.urls {
        card.url_addresses.push(CardValue::new(card_label(&u.label).as_deref(), u.value.clone()));
    }
    if !c.address.trim().is_empty() {
        let name = contactstore::LABELS[2].get(c.labels[2] as usize).copied().unwrap_or("other");
        let address = PostalAddress { street: c.address.clone(), ..PostalAddress::default() };
        card.postal_addresses.push(CardValue::new(card_label(name).as_deref(), address));
    }
    card.birthday = extras.birthday.map(card_date);
    for (name, d) in &extras.dates {
        card.dates.push(CardValue::new(card_label(name).as_deref(), card_date(*d)));
    }
    card
}

/// The cards of the contacts named by `ids` (every contact of `book` when `ids` is `None`), in book order, as vCard
/// text of `version`. `photo` looks a contact's picture up by id.
pub fn export(book: &[Contact], extras: &ExtraBook, ids: Option<&[u64]>, photo: &dyn Fn(u64) -> Option<Vec<u8>>, version: Version) -> String {
    let cards: Vec<Card> = book
        .iter()
        .filter(|c| ids.map_or(true, |ids| ids.contains(&c.id)))
        .map(|c| card_of(c, &extras.get(&c.id).cloned().unwrap_or_default(), photo(c.id)))
        .collect();
    contact_vcard::serialize(&cards, version)
}

/// One address line from a postal address: street, city, state and postal code, country, skipping empty parts.
fn address_line(a: &PostalAddress) -> String {
    [&a.street, &a.sub_locality, &a.city, &a.state, &a.postal_code, &a.country]
        .into_iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A store contact and extras read from a card, with the picture it carried. `None` when the card holds nothing the
/// store accepts (no name, company, address, phone or email, or a value over the store's limits).
fn read_card(card: &Card) -> Option<(Contact, Extras, Option<Vec<u8>>)> {
    let mut name = join_name(&NameParts {
        given: clean(card.given_name.trim(), MAX_NAME),
        middle: clean(card.middle_name.trim(), MAX_NAME),
        family: clean(card.family_name.trim(), MAX_NAME),
        suffix: clean(card.name_suffix.trim(), MAX_NAME),
    });
    if name.is_empty() {
        name = clean(card.nickname.trim(), MAX_NAME);
    }
    let mut contact = Contact {
        name: clean(&name, MAX_NAME),
        company: clean(card.organization_name.trim(), MAX_NAME),
        notes: clean(&card.note, MAX_NOTES),
        ..Contact::default()
    };
    let mut extras = Extras::default();
    for (i, p) in card.phone_numbers.iter().enumerate() {
        let value = clean(p.value.trim(), MAX_VALUE);
        if value.is_empty() {
            continue;
        }
        if contact.phone.is_empty() && i == card.phone_numbers.iter().position(|q| !q.value.trim().is_empty()).unwrap_or(0) {
            contact.phone = value;
            contact.labels[0] = index_of(&PHONE_LABELS, &store_label(&p.label));
        } else {
            extras.phones.push(LabeledValue::new(&clean(&store_label(&p.label), MAX_EXTRA), &value));
        }
    }
    for (i, e) in card.email_addresses.iter().enumerate() {
        let value = clean(e.value.trim(), MAX_VALUE);
        if value.is_empty() {
            continue;
        }
        if contact.email.is_empty() && i == card.email_addresses.iter().position(|q| !q.value.trim().is_empty()).unwrap_or(0) {
            contact.email = value;
            contact.labels[1] = index_of(&PLACE_LABELS, &store_label(&e.label));
        } else {
            extras.emails.push(LabeledValue::new(&clean(&store_label(&e.label), MAX_EXTRA), &value));
        }
    }
    for u in &card.url_addresses {
        let value = clean(u.value.trim(), MAX_EXTRA);
        if !value.is_empty() {
            extras.urls.push(LabeledValue::new(&clean(&store_label(&u.label), MAX_EXTRA), &value));
        }
    }
    if let Some(a) = card.postal_addresses.iter().find(|a| !address_line(&a.value).is_empty()) {
        contact.address = clean(&address_line(&a.value), MAX_VALUE);
        contact.labels[2] = index_of(&PLACE_LABELS, &store_label(&a.label));
    }
    extras.birthday = card.birthday.map(store_date);
    for d in &card.dates {
        extras.dates.push((clean(&store_label(&d.label), MAX_EXTRA), store_date(d.value)));
    }
    contact.valid().then_some((contact, extras, card.image_data.clone()))
}

fn digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// What makes two cards the same person: the lower-case name (or company), the lower-case email and the phone digits.
fn identity(c: &Contact) -> (String, String, String) {
    let who = if c.name.trim().is_empty() { &c.company } else { &c.name };
    (who.trim().to_lowercase(), c.email.trim().to_lowercase(), digits(&c.phone))
}

fn same_person(a: &(String, String, String), b: &(String, String, String)) -> bool {
    a.0 == b.0 && (a.1 == b.1 || (!a.2.is_empty() && a.2 == b.2))
}

/// What an import would do to the book. The caller stores `book`, `extras` and `photos` only when `plan_import` is `Ok`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The whole book after the import (the old contacts, then the new ones).
    pub book: Vec<Contact>,
    /// The ids the new contacts got, in card order.
    pub added: Vec<u64>,
    /// The extras of the new contacts that have any.
    pub extras: Vec<(u64, Extras)>,
    /// The pictures of the new contacts that carried one.
    pub photos: Vec<(u64, Vec<u8>)>,
    /// Cards the store cannot hold (nothing to name them by, or a value over its limits).
    pub skipped: usize,
    /// Cards of a person the book already holds, or that repeat an earlier card of the same text.
    pub duplicates: usize,
}

/// Reads the cards in `text` and plans their addition to `book`. A text that is not vCard, or is over the size cap, is
/// an error; a card the store cannot hold is skipped and counted. The new contacts get the next free ids and a new UID.
pub fn plan_import(text: &str, book: &[Contact]) -> Result<Plan, String> {
    let cards = contact_vcard::contacts(text).map_err(|e| format!("not a readable vCard file: {e:?}"))?;
    if cards.is_empty() {
        return Err("no vCard in the text".into());
    }
    let mut plan = Plan { book: book.to_vec(), ..Plan::default() };
    let mut seen: Vec<_> = book.iter().map(identity).collect();
    let mut taken: HashSet<String> = book.iter().map(|c| c.uid.clone()).collect();
    for card in &cards {
        let Some((mut contact, extras, photo)) = read_card(card) else {
            plan.skipped += 1;
            continue;
        };
        let who = identity(&contact);
        if seen.iter().any(|s| same_person(s, &who)) {
            plan.duplicates += 1;
            continue;
        }
        let id = contactstore::new_id(&plan.book).ok_or_else(|| "the book is full".to_string())?;
        contact.id = id;
        contact.uid = contactstore::new_uid(id);
        while !taken.insert(contact.uid.clone()) {
            contact.uid.push('x');
        }
        seen.push(who);
        if extras != Extras::default() {
            plan.extras.push((id, extras));
        }
        if let Some(bytes) = photo.filter(|b| !b.is_empty()) {
            plan.photos.push((id, bytes));
        }
        plan.added.push(id);
        plan.book.push(contact);
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ada() -> Contact {
        Contact {
            id: 4,
            name: "Ada Augusta Lovelace".into(),
            phone: "+354 555 0101".into(),
            email: "ada@x.is".into(),
            notes: "Wrote the first program".into(),
            company: "Analytical Engines".into(),
            address: "12 Bryn Road, London".into(),
            labels: [2, 1, 0],
            uid: "u-4".into(),
            ..Contact::default()
        }
    }

    fn ada_extras() -> Extras {
        Extras {
            phones: vec![LabeledValue::new("home", "+354 555 0202")],
            emails: vec![LabeledValue::new("work", "ada@work.is")],
            urls: vec![LabeledValue::new("other", "https://ada.is")],
            birthday: DateComponents::new(Some(1815), 12, 10),
            dates: vec![("anniversary".into(), DateComponents::new(Some(1835), 7, 8).unwrap())],
        }
    }

    fn no_photo(_: u64) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn a_card_holds_the_file_values_first_then_the_extras_under_standard_labels() {
        let card = card_of(&ada(), &ada_extras(), None);
        assert_eq!(card.given_name, "Ada");
        assert_eq!(card.middle_name, "Augusta");
        assert_eq!(card.family_name, "Lovelace");
        assert_eq!(card.phone_numbers.len(), 2);
        assert_eq!(card.phone_numbers[0].value, "+354 555 0101");
        assert_eq!(card.phone_numbers[0].label.as_deref(), Some(label::WORK));
        assert_eq!(card.phone_numbers[1].label.as_deref(), Some(label::HOME));
        assert_eq!(card.email_addresses[1].value, "ada@work.is");
        assert_eq!(card.postal_addresses[0].label.as_deref(), Some(label::HOME));
        assert_eq!(card.dates[0].label.as_deref(), Some(label::ANNIVERSARY));
    }

    #[test]
    fn export_writes_only_the_named_contacts_in_the_asked_version() {
        let mut bob = ada();
        bob.id = 5;
        bob.name = "Bob Smith".into();
        let book = vec![ada(), bob];
        let all = export(&book, &ExtraBook::new(), None, &no_photo, Version::V3);
        assert_eq!(all.matches("BEGIN:VCARD").count(), 2);
        assert!(all.contains("VERSION:3.0"));
        let one = export(&book, &ExtraBook::new(), Some(&[5]), &no_photo, Version::V4);
        assert_eq!(one.matches("BEGIN:VCARD").count(), 1);
        assert!(one.contains("VERSION:4.0") && one.contains("Bob") && !one.contains("Lovelace"));
    }

    #[test]
    fn an_exported_book_imports_back_to_the_same_person() {
        let mut extras = ExtraBook::new();
        extras.insert(4, ada_extras());
        for version in [Version::V3, Version::V4] {
            let text = export(&[ada()], &extras, None, &no_photo, version);
            let plan = plan_import(&text, &[]).unwrap();
            assert_eq!(plan.added, vec![1], "{version:?}");
            let back = &plan.book[0];
            assert_eq!(back.name, "Ada Augusta Lovelace");
            assert_eq!(back.phone, "+354 555 0101");
            assert_eq!(back.email, "ada@x.is");
            assert_eq!(back.company, "Analytical Engines");
            assert_eq!(back.labels[0], 2, "the phone stays a work phone");
            assert_eq!(back.labels[1], 1);
            assert!(back.address.contains("12 Bryn Road"));
            let (id, got) = &plan.extras[0];
            assert_eq!(*id, 1);
            assert_eq!(got.phones[0].value, "+354 555 0202");
            assert_eq!(got.emails[0].value, "ada@work.is");
            assert_eq!(got.urls[0].value, "https://ada.is");
            assert_eq!(got.birthday, DateComponents::new(Some(1815), 12, 10));
            assert_eq!(got.dates.len(), 1);
        }
    }

    #[test]
    fn import_gives_new_contacts_free_ids_and_unique_uids_and_keeps_the_old_book() {
        let text = export(&[ada()], &ExtraBook::new(), None, &no_photo, Version::V3);
        let mut other = ada();
        other.id = 1;
        other.name = "Grace Hopper".into();
        other.email = "grace@x.is".into();
        other.phone = String::new();
        let plan = plan_import(&text, &[other.clone()]).unwrap();
        assert_eq!(plan.book[0], other);
        assert_eq!(plan.added, vec![2]);
        assert!(!plan.book[1].uid.is_empty() && plan.book[1].uid != other.uid);
    }

    #[test]
    fn a_person_already_in_the_book_is_not_added_twice() {
        let text = export(&[ada()], &ExtraBook::new(), None, &no_photo, Version::V3);
        let plan = plan_import(&text, &[ada()]).unwrap();
        assert!(plan.added.is_empty());
        assert_eq!(plan.duplicates, 1);
        // the same card twice in one file is added once
        let doubled = format!("{text}{text}");
        let plan = plan_import(&doubled, &[]).unwrap();
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.duplicates, 1);
    }

    #[test]
    fn a_card_with_nothing_to_name_it_is_skipped_and_counted() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Bo Li\r\nN:Li;Bo;;;\r\nEND:VCARD\r\n";
        let plan = plan_import(text, &[]).unwrap();
        assert_eq!(plan.skipped, 1);
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.book[0].name, "Bo Li");
    }

    #[test]
    fn text_that_is_no_vcard_is_refused_and_changes_nothing() {
        assert!(plan_import("hello", &[]).is_err());
        assert!(plan_import("BEGIN:VCARD\r\nFN:Broken\r\n", &[]).is_err());
        assert!(plan_import("BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\n", &[]).unwrap().added.is_empty());
    }

    #[test]
    fn a_picture_travels_with_the_card() {
        let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4];
        let with = {
            let bytes = png.clone();
            move |id: u64| (id == 4).then(|| bytes.clone())
        };
        let text = export(&[ada()], &ExtraBook::new(), None, &with, Version::V3);
        assert!(text.contains("PHOTO"));
        let plan = plan_import(&text, &[]).unwrap();
        assert_eq!(plan.photos, vec![(1, png)]);
    }

    #[test]
    fn a_custom_label_and_an_unknown_phone_label_survive_or_fall_back_to_other() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Cy Young\r\nN:Young;Cy;;;\r\nitem1.TEL:555 0100\r\nitem1.X-ABLabel:Boat\r\nTEL;TYPE=work:555 0200\r\nEND:VCARD\r\n";
        let plan = plan_import(text, &[]).unwrap();
        let c = &plan.book[0];
        assert_eq!(c.phone, "555 0100");
        assert_eq!(c.labels[0], 3, "a free-text label falls back to the store's other");
        let extra = &plan.extras[0].1;
        assert_eq!(extra.phones.len(), 1);
        assert_eq!(extra.phones[0].label, "work");
    }
}
