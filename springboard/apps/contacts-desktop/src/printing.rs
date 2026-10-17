//! Text layouts for printing and PDF export: the contact list, one card, and address labels. The shell's print service
//! lays plain text out on the page (`bifrost_app::print::PrintOperation::text`), so these produce the text. Pure std.

use crate::details::Details;
use crate::extras::{Extras, ValueKind};
use springboard::shell::contactstore::{Contact, LABELS};

/// One card: the name, the company, the card fields, the phones, emails and addresses under their labels, and the notes.
pub fn card_text(contact: &Contact, extras: &Extras, details: &Details) -> String {
    let name = contact.display_name();
    let mut lines = vec![name.to_string()];
    let company = contact.company.trim();
    if !company.is_empty() && company != name {
        lines.push(company.to_string());
    }
    for (detail, value) in details.of(contact.id).filled() {
        lines.push(format!("{}: {value}", detail.title()));
    }
    let store = [(ValueKind::Phone, &contact.phone), (ValueKind::Email, &contact.email), (ValueKind::Address, &contact.address)];
    for (kind, value) in store {
        let value = value.trim();
        if !value.is_empty() {
            let labels = LABELS[kind.index()];
            let label = labels[contact.labels[kind.index()] as usize % labels.len()];
            lines.push(format!("{label} {}: {value}", kind_word(kind)));
        }
    }
    for extra in extras.get(contact.id) {
        lines.push(format!("{} {}: {}", extra.label, kind_word(extra.kind), extra.value));
    }
    let notes = contact.notes.trim();
    if !notes.is_empty() {
        lines.push(format!("Notes: {notes}"));
    }
    lines.join("\n")
}

fn kind_word(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Phone => "phone",
        ValueKind::Email => "email",
        ValueKind::Address => "address",
    }
}

/// The contact list: a title with the count, then each card, separated by blank lines.
pub fn list_text(contacts: &[&Contact], extras: &Extras, details: &Details) -> String {
    let mut out = format!("Contacts ({})\n", contacts.len());
    for contact in contacts {
        out.push('\n');
        out.push_str(&card_text(contact, extras, details));
        out.push('\n');
    }
    out
}

/// Address labels: the name, then the address one part per line, for each contact that has an address; blank lines
/// between them. The second value is the number of labels.
pub fn labels_text(contacts: &[&Contact], extras: &Extras) -> (String, usize) {
    let mut out = String::new();
    let mut count = 0;
    for contact in contacts {
        let addresses = std::iter::once(contact.address.as_str())
            .chain(extras.get(contact.id).iter().filter(|e| e.kind == ValueKind::Address).map(|e| e.value.as_str()))
            .map(str::trim)
            .filter(|a| !a.is_empty());
        for address in addresses {
            out.push_str(contact.display_name());
            out.push('\n');
            for part in address.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                out.push_str(part);
                out.push('\n');
            }
            out.push('\n');
            count += 1;
        }
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::details::{CardDetails, Detail};
    use crate::extras::Extra;

    fn anna() -> Contact {
        Contact {
            id: 1,
            name: "Anna Lindqvist".into(),
            phone: "555 0100".into(),
            email: "anna@acme.is".into(),
            notes: "Met at a fair".into(),
            company: "Acme".into(),
            address: "Gata 1, Reykjavik".into(),
            uid: String::new(),
            labels: [0, 1, 2],
            ..Default::default()
        }
    }

    #[test]
    fn a_card_lists_its_name_fields_values_and_notes() {
        let mut extras = Extras::default();
        assert!(extras.add(1, Extra { kind: ValueKind::Phone, label: "work".into(), value: "555 0200".into() }));
        let mut details = Details::default();
        let mut card = CardDetails::default();
        card.set(Detail::Nickname, "Annie");
        details.set(1, card);
        let text = card_text(&anna(), &extras, &details);
        assert_eq!(
            text,
            "Anna Lindqvist\nAcme\nNickname: Annie\nmobile phone: 555 0100\nwork email: anna@acme.is\nother address: Gata 1, Reykjavik\n\
work phone: 555 0200\nNotes: Met at a fair"
        );
    }

    #[test]
    fn a_company_card_does_not_repeat_its_name_as_the_company() {
        let contact = Contact { id: 2, company: "Acme".into(), ..Default::default() };
        assert_eq!(card_text(&contact, &Extras::default(), &Details::default()), "Acme");
    }

    #[test]
    fn the_list_has_a_count_and_one_block_per_card() {
        let a = anna();
        let b = Contact { id: 2, name: "Bo Berg".into(), ..Default::default() };
        let text = list_text(&[&a, &b], &Extras::default(), &Details::default());
        assert!(text.starts_with("Contacts (2)\n\nAnna Lindqvist\n"), "{text}");
        assert!(text.ends_with("\n\nBo Berg\n"), "{text}");
    }

    #[test]
    fn labels_are_made_for_contacts_with_an_address_only() {
        let a = anna();
        let b = Contact { id: 2, name: "Bo Berg".into(), ..Default::default() };
        let mut extras = Extras::default();
        assert!(extras.add(2, Extra { kind: ValueKind::Address, label: "home".into(), value: "Vegur 2, Akureyri".into() }));
        let (text, count) = labels_text(&[&a, &b], &extras);
        assert_eq!(count, 2);
        assert_eq!(text, "Anna Lindqvist\nGata 1\nReykjavik\n\nBo Berg\nVegur 2\nAkureyri\n\n");
        assert_eq!(labels_text(&[], &extras), (String::new(), 0));
    }
}
