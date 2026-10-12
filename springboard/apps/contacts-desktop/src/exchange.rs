//! vCard import and export for contacts, in vCard 3.0 and 4.0.
//!
//! The bifrost-ui `vcard` model reads and writes vCard 3.0: names, one organization, labelled phones and emails, and the
//! note. This module adds what the app needs around it. It reads 4.0 syntax (quoted `TYPE` values, `tel:` and `mailto:`
//! URIs, `VALUE=uri` parameters), skips `KIND:group` cards (groups are not people), and keeps the postal address, which
//! the model does not parse. A contact holds one phone and one email, so the first of each is kept and the rest are
//! counted. Pure std.

use crate::names;
use bifrost_app::controls::vcard::{self, Card, Labeled};
use springboard::shell::contactstore::Contact;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    V3,
    V4,
}

/// What an import read from a text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Import {
    /// The people read, with identifiers not yet assigned (0).
    pub contacts: Vec<Contact>,
    /// Cards the contact store cannot hold: no name or company, or a field longer than the store allows.
    pub invalid: usize,
    /// Phone numbers and emails after the first of each, which a contact does not hold.
    pub extra_values: usize,
    /// `KIND:group` cards, skipped.
    pub groups: usize,
}

/// Reads every card in a vCard 3.0 or 4.0 text.
pub fn import(text: &str) -> Result<Import, String> {
    let mut out = Import::default();
    let mut normalized = String::new();
    let mut addresses = Vec::new();
    for block in blocks(text) {
        match normalize(&block) {
            Some((lines, address)) => {
                for line in lines {
                    normalized.push_str(&line);
                    normalized.push_str("\r\n");
                }
                addresses.push(address);
            }
            None => out.groups += 1,
        }
    }
    // A block that does not begin and end properly makes the model refuse the whole text, so the addresses line up
    // with the cards it returns.
    let cards = vcard::parse(&normalized).map_err(|e| match e {
        vcard::Error::TooLarge => "The file is too large to import".to_string(),
        vcard::Error::Malformed => "A card in the file does not begin and end properly".to_string(),
    })?;
    for (card, address) in cards.iter().zip(addresses) {
        let (contact, extra) = contact_of(card, &address);
        out.extra_values += extra;
        if contact.valid() {
            out.contacts.push(contact);
        } else {
            out.invalid += 1;
        }
    }
    Ok(out)
}

/// Writes contacts as one vCard text, in the chosen version. Each card ends with CRLF.
pub fn export(contacts: &[Contact], version: Version) -> String {
    let mut out = String::new();
    for contact in contacts {
        let mut lines = logical_lines(&card_of(contact).to_vcard());
        if !contact.address.trim().is_empty() {
            // The last line is END:VCARD; the address goes just before it.
            let end = lines.len().saturating_sub(1);
            lines.insert(end, format!("ADR:;;{};;;;", escape(contact.address.trim())));
        }
        for line in lines {
            let line = match version {
                Version::V3 => line,
                Version::V4 => to_v4(&line),
            };
            out.push_str(&fold75(&line));
        }
    }
    out
}

/// Joins continuation lines (starting with a space or a tab) to the line before them.
fn logical_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        out.push(line.to_string());
    }
    out
}

/// The lines of each card, from BEGIN to END. Lines outside a card are dropped. A card left open, or an END with no
/// BEGIN, comes back as its own block so the model reports it.
fn blocks(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut open: Option<Vec<String>> = None;
    for line in logical_lines(text) {
        let upper = line.trim().to_ascii_uppercase();
        if upper == "BEGIN:VCARD" {
            if let Some(unfinished) = open.take() {
                out.push(unfinished);
            }
            open = Some(vec![line]);
        } else if upper == "END:VCARD" {
            match open.take() {
                Some(mut block) => {
                    block.push(line);
                    out.push(block);
                }
                None => out.push(vec![line]),
            }
        } else if let Some(block) = open.as_mut() {
            block.push(line);
        }
    }
    if let Some(unfinished) = open {
        out.push(unfinished);
    }
    out
}

/// One card's lines rewritten for the 3.0 model, and its postal address. `None` for a group card.
fn normalize(block: &[String]) -> Option<(Vec<String>, String)> {
    let mut lines = Vec::with_capacity(block.len());
    let mut address = String::new();
    for line in block {
        let Some((head, raw)) = line.split_once(':') else {
            lines.push(line.clone());
            continue;
        };
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or("").rsplit('.').next().unwrap_or("").trim().to_ascii_uppercase();
        if name == "KIND" && raw.trim().eq_ignore_ascii_case("group") {
            return None;
        }
        let mut out_head = name.clone();
        for param in parts {
            let param = param.replace('"', "");
            if param.to_ascii_uppercase().starts_with("VALUE=") {
                continue;
            }
            out_head.push(';');
            out_head.push_str(&param);
        }
        let value: &str = match name.as_str() {
            "TEL" | "EMAIL" => strip_uri(raw),
            "ADR" => {
                if address.is_empty() {
                    address = street_text(raw);
                }
                raw
            }
            _ => raw,
        };
        lines.push(format!("{out_head}:{value}"));
    }
    Some((lines, address))
}

/// Drops a `tel:` or `mailto:` scheme from a 4.0 URI value.
fn strip_uri(raw: &str) -> &str {
    for scheme in ["tel:", "mailto:"] {
        if raw.get(..scheme.len()).is_some_and(|p| p.eq_ignore_ascii_case(scheme)) {
            return &raw[scheme.len()..];
        }
    }
    raw
}

/// The non-empty components of an address value, joined on one line.
fn street_text(raw: &str) -> String {
    raw.split(';')
        .map(|part| part.replace("\\,", ",").replace("\\n", " ").trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn contact_of(card: &Card, address: &str) -> (Contact, usize) {
    let person = [card.given_name.trim(), card.family_name.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let name = if person.is_empty() { card.formatted_name.trim().to_string() } else { person };
    let phone = card.phones.first();
    let email = card.emails.first();
    let extra = card.phones.len().saturating_sub(1) + card.emails.len().saturating_sub(1);
    let contact = Contact {
        id: 0,
        name,
        phone: phone.map(|p| p.value.clone()).unwrap_or_default(),
        email: email.map(|e| e.value.clone()).unwrap_or_default(),
        notes: card.note.clone(),
        favorite: false,
        company: card.organization.clone(),
        address: address.to_string(),
        labels: [phone.map_or(3, |p| phone_index(&p.label)), email.map_or(2, |e| email_index(&e.label)), 2],
    };
    (contact, extra)
}

/// The store's phone labels are mobile, home, work, other.
fn phone_index(label: &str) -> u8 {
    match label {
        "cell" | "mobile" => 0,
        "home" => 1,
        "work" => 2,
        _ => 3,
    }
}

/// The store's email labels are home, work, other.
fn email_index(label: &str) -> u8 {
    match label {
        "home" => 0,
        "work" => 1,
        _ => 2,
    }
}

fn phone_label(index: u8) -> &'static str {
    match index {
        0 => "cell",
        1 => "home",
        2 => "work",
        _ => "",
    }
}

fn email_label(index: u8) -> &'static str {
    match index {
        0 => "home",
        1 => "work",
        _ => "",
    }
}

fn card_of(contact: &Contact) -> Card {
    let (given, family) = names::split(&contact.name);
    let phones = if contact.phone.trim().is_empty() {
        vec![]
    } else {
        vec![Labeled { label: phone_label(contact.labels[0]).into(), value: contact.phone.trim().into() }]
    };
    let emails = if contact.email.trim().is_empty() {
        vec![]
    } else {
        vec![Labeled { label: email_label(contact.labels[1]).into(), value: contact.email.trim().into() }]
    };
    Card {
        formatted_name: contact.display_name().to_string(),
        family_name: family.to_string(),
        given_name: given.to_string(),
        organization: contact.company.trim().to_string(),
        phones,
        emails,
        note: contact.notes.clone(),
    }
}

/// A 3.0 line rewritten for 4.0: the version, and quoted `TYPE` values.
fn to_v4(line: &str) -> String {
    if line == "VERSION:3.0" {
        return "VERSION:4.0".into();
    }
    let Some((head, value)) = line.split_once(':') else { return line.to_string() };
    let head: Vec<String> = head
        .split(';')
        .map(|param| match param.strip_prefix("TYPE=") {
            Some(types) => format!("TYPE=\"{types}\""),
            None => param.to_string(),
        })
        .collect();
    format!("{}:{}", head.join(";"), value)
}

/// Escapes a structured value the way the vCard format needs.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

/// One logical line as CRLF-ended physical lines of at most 75 bytes; continuation lines start with a space.
fn fold75(line: &str) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in line.chars() {
        let width = c.len_utf8();
        if used + width > 75 {
            out.push_str("\r\n ");
            used = 1;
        }
        out.push(c);
        used += width;
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const V3: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Anna Lindqvist\r\nN:Lindqvist;Anna;;;\r\nORG:Acme\\, Inc.;Sales\r\n\
TEL;TYPE=CELL,VOICE:+354 555 1234\r\nEMAIL;TYPE=WORK:anna@example.org\r\nNOTE:Line one\\nLine two\r\n\
ADR;TYPE=HOME:;;Street 1;Reykjavik;;101;Iceland\r\nEND:VCARD\r\n";

    const V4: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Bo Berg\r\nN:Berg;Bo;;;\r\nTEL;TYPE=\"cell\",voice;VALUE=uri:tel:+354-555-9999\r\n\
EMAIL;TYPE=\"work\";PREF=1:mailto:bo@example.org\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Friends\r\nMEMBER:urn:uuid:0001\r\nEND:VCARD\r\n";

    fn sample() -> Contact {
        Contact {
            id: 0,
            name: "Ólafur Búi Ólafsson".into(),
            phone: "+354 555 1234".into(),
            email: "olafur@example.is".into(),
            notes: "a;b\nc".into(),
            favorite: false,
            company: "Acme".into(),
            address: "Hafnarstræti 1, Reykjavík".into(),
            labels: [2, 1, 2],
        }
    }

    #[test]
    fn a_3_0_card_is_read_with_its_labels_and_address() {
        let got = import(V3).unwrap();
        assert_eq!(got.invalid, 0);
        assert_eq!(got.contacts.len(), 1);
        let c = &got.contacts[0];
        assert_eq!(c.name, "Anna Lindqvist");
        assert_eq!(c.company, "Acme, Inc.");
        assert_eq!(c.phone, "+354 555 1234");
        assert_eq!(c.email, "anna@example.org");
        assert_eq!(c.notes, "Line one\nLine two");
        assert_eq!(c.address, "Street 1, Reykjavik, 101, Iceland");
        assert_eq!(c.labels, [0, 1, 2]);
    }

    #[test]
    fn a_4_0_card_is_read_and_the_group_card_is_skipped() {
        let got = import(V4).unwrap();
        assert_eq!(got.groups, 1);
        assert_eq!(got.contacts.len(), 1);
        let c = &got.contacts[0];
        assert_eq!(c.name, "Bo Berg");
        assert_eq!(c.phone, "+354-555-9999", "the tel: scheme and VALUE=uri are dropped");
        assert_eq!(c.email, "bo@example.org", "the mailto: scheme is dropped");
        assert_eq!(c.labels, [0, 1, 2], "the quoted TYPE values are read");
    }

    #[test]
    fn a_contact_round_trips_through_3_0_and_4_0() {
        let original = sample();
        for version in [Version::V3, Version::V4] {
            let text = export(std::slice::from_ref(&original), version);
            let got = import(&text).unwrap();
            assert_eq!(got.contacts, vec![original.clone()], "{version:?}");
        }
    }

    #[test]
    fn version_4_0_export_quotes_types_and_says_4_0() {
        let text = export(&[sample()], Version::V4);
        assert!(text.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"), "{text}");
        assert!(text.contains("TEL;TYPE=\"work\":+354 555 1234\r\n"), "{text}");
        assert!(text.contains("EMAIL;TYPE=\"work\":olafur@example.is\r\n"), "{text}");
        let v3 = export(&[sample()], Version::V3);
        assert!(v3.contains("TEL;TYPE=work:+354 555 1234\r\n"), "{v3}");
    }

    #[test]
    fn long_lines_are_folded_to_75_bytes_and_read_back() {
        let mut long = sample();
        long.notes = "ä".repeat(120);
        let text = export(std::slice::from_ref(&long), Version::V3);
        for line in text.split("\r\n") {
            assert!(line.len() <= 75, "{line:?}");
        }
        assert_eq!(import(&text).unwrap().contacts, vec![long]);
    }

    #[test]
    fn empty_cards_are_counted_and_broken_text_is_refused() {
        let got = import("BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\n").unwrap();
        assert_eq!((got.contacts.len(), got.invalid), (0, 1));
        assert!(import("BEGIN:VCARD\r\nFN:Open\r\n").is_err());
        assert!(import("END:VCARD\r\n").is_err());
        assert_eq!(import("no cards here\r\n").unwrap(), Import::default());
    }

    #[test]
    fn extra_phones_and_emails_are_counted() {
        let text = "BEGIN:VCARD\r\nFN:Cara Cole\r\nTEL:111\r\nTEL:222\r\nEMAIL:a@x.is\r\nEMAIL:b@x.is\r\nEND:VCARD\r\n";
        let got = import(text).unwrap();
        assert_eq!(got.extra_values, 2);
        assert_eq!(got.contacts[0].phone, "111");
    }
}
