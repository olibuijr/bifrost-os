//! vCard import and export for contacts, in vCard 3.0 and 4.0.
//!
//! The bifrost-ui `vcard` model reads and writes vCard 3.0: names, one organization, labelled phones and emails, and the
//! note. This module adds what the app needs around it. It reads 4.0 syntax (quoted `TYPE` values, `tel:` and `mailto:`
//! URIs, `VALUE=uri` parameters), skips `KIND:group` cards (groups are not people), and keeps the postal address, which
//! the model does not parse. The store holds one phone, one email and one address per contact, so the first of each
//! is kept in the store and the later phones and emails are kept as extra values (`extras.rs`), up to `MAX_EXTRAS`
//! per contact; the rest are counted. The card fields in `details.rs` (nickname, job title, department, URL, birthday,
//! pronouns, phonetic name, related name, profile and the company flag) are read from and written to their own
//! properties. Pure std.

use crate::details::{self, CardDetails, Detail, Details};
use crate::extras::{Extra, Extras, ValueKind, MAX_EXTRAS};
use crate::names;
use bifrost_app::controls::vcard::{self, Card, Labeled};
use springboard::shell::contactstore::{Contact, LABELS};

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
    /// The extra values of each person in `contacts`, in the same order.
    pub values: Vec<Vec<Extra>>,
    /// The card fields of each person in `contacts`, in the same order.
    pub details: Vec<CardDetails>,
    /// Cards the contact store cannot hold: no name or company, or a field longer than the store allows.
    pub invalid: usize,
    /// Phone numbers and emails after the first of each, beyond the `MAX_EXTRAS` a contact keeps.
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
            Some((lines, address, card_details)) => {
                for line in lines {
                    normalized.push_str(&line);
                    normalized.push_str("\r\n");
                }
                addresses.push((address, card_details));
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
    for (card, (address, card_details)) in cards.iter().zip(addresses) {
        let (contact, values, dropped) = contact_of(card, &address);
        out.extra_values += dropped;
        if contact.valid() {
            out.contacts.push(contact);
            out.values.push(values);
            out.details.push(card_details);
        } else {
            out.invalid += 1;
        }
    }
    Ok(out)
}

/// What an export writes besides the contact's name and numbers (Settings > vCard).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    /// Write the notes of each card.
    pub notes: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { notes: true }
    }
}

/// Writes contacts as one vCard text, in the chosen version, with their extra values. Each card ends with CRLF.
pub fn export(contacts: &[Contact], values: &Extras, version: Version) -> String {
    export_with(contacts, values, &Details::default(), ExportOptions::default(), version)
}

/// Writes contacts as one vCard text with their extra values, card fields and the export options.
pub fn export_with(contacts: &[Contact], values: &Extras, card_details: &Details, options: ExportOptions, version: Version) -> String {
    let mut out = String::new();
    for contact in contacts {
        let extra = values.get(contact.id);
        let own = card_details.of(contact.id);
        let mut lines = logical_lines(&card_of(contact, extra, options.notes).to_vcard());
        // The addresses and the card fields go just before the last line, END:VCARD, in order.
        let mut added: Vec<String> = std::iter::once(contact.address.trim())
            .chain(extra.iter().filter(|e| e.kind == ValueKind::Address).map(|e| e.value.trim()))
            .filter(|address| !address.is_empty())
            .map(|address| format!("ADR:;;{};;;;", escape(address)))
            .collect();
        added.extend(detail_lines(&own));
        let department = own.get(Detail::Department);
        if !department.is_empty() {
            match lines.iter().position(|line| line.starts_with("ORG:")) {
                Some(at) => lines[at] = format!("{};{}", lines[at], escape(department)),
                None => added.push(format!("ORG:;{}", escape(department))),
            }
        }
        let end = lines.len().saturating_sub(1);
        let tail = lines.split_off(end);
        lines.extend(added);
        lines.extend(tail);
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

/// The properties for a card's fields and company flag (the department goes in `ORG`, written by the caller).
fn detail_lines(own: &CardDetails) -> Vec<String> {
    let mut lines = Vec::new();
    if own.company {
        lines.push("KIND:org".to_string());
    }
    for (detail, value) in own.filled() {
        let line = match detail {
            Detail::Nickname => format!("NICKNAME:{}", escape(value)),
            Detail::JobTitle => format!("TITLE:{}", escape(value)),
            Detail::Department => continue,
            Detail::Url => format!("URL:{value}"),
            Detail::Birthday => format!("BDAY:{value}"),
            Detail::Pronouns => format!("X-PRONOUNS:{}", escape(value)),
            Detail::Phonetic => format!("X-PHONETIC-NAME:{}", escape(value)),
            Detail::Related => format!("RELATED:{}", escape(value)),
            Detail::Profile => format!("X-SOCIALPROFILE:{value}"),
        };
        lines.push(line);
    }
    lines
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

/// One card's lines rewritten for the 3.0 model, its postal address and its card fields. `None` for a group card.
fn normalize(block: &[String]) -> Option<(Vec<String>, String, CardDetails)> {
    let mut lines = Vec::with_capacity(block.len());
    let mut address = String::new();
    let mut own = CardDetails::default();
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
        read_detail(&mut own, &name, raw);
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
    Some((lines, address, own))
}

/// Reads one property into the card fields. The first value of a field wins.
fn read_detail(own: &mut CardDetails, name: &str, raw: &str) {
    let field = match name {
        "NICKNAME" => Detail::Nickname,
        "TITLE" => Detail::JobTitle,
        "URL" => Detail::Url,
        "BDAY" => Detail::Birthday,
        "X-PRONOUNS" | "PRONOUNS" => Detail::Pronouns,
        "X-PHONETIC-NAME" => Detail::Phonetic,
        "RELATED" => Detail::Related,
        "X-SOCIALPROFILE" => Detail::Profile,
        "ORG" => {
            let department = structured(raw).get(1).cloned().unwrap_or_default();
            if own.get(Detail::Department).is_empty() {
                own.set(Detail::Department, &department);
            }
            return;
        }
        "KIND" => {
            if raw.trim().eq_ignore_ascii_case("org") {
                own.company = true;
            }
            return;
        }
        _ => return,
    };
    if !own.get(field).is_empty() {
        return;
    }
    let value = match field {
        Detail::Url | Detail::Profile => raw.trim().to_string(),
        Detail::Birthday => details::normalize_birthday(&unescape_text(raw)),
        _ => unescape_text(raw),
    };
    own.set(field, &value);
}

/// The `;`-separated components of a structured value, each unescaped.
fn structured(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') | Some('N') => cur.push('\n'),
                Some(other) => cur.push(other),
                None => {}
            },
            ';' => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// A single text value with its escapes undone; a bare `;` is kept.
fn unescape_text(raw: &str) -> String {
    structured(raw).join(";")
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

/// The store contact for a card, its extra values (phones and emails after the first of each, up to `MAX_EXTRAS`),
/// and how many extra values were dropped.
fn contact_of(card: &Card, address: &str) -> (Contact, Vec<Extra>, usize) {
    let person = [card.given_name.trim(), card.family_name.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let name = if person.is_empty() { card.formatted_name.trim().to_string() } else { person };
    let phone = card.phones.first();
    let email = card.emails.first();
    let spare = card
        .phones
        .iter()
        .skip(1)
        .map(|p| Extra { kind: ValueKind::Phone, label: LABELS[0][phone_index(&p.label) as usize].into(), value: p.value.trim().into() })
        .chain(card.emails.iter().skip(1).map(|e| Extra {
            kind: ValueKind::Email,
            label: LABELS[1][email_index(&e.label) as usize].into(),
            value: e.value.trim().into(),
        }));
    let mut values = Vec::new();
    let mut dropped = 0;
    for extra in spare {
        if extra.value.is_empty() {
            continue;
        }
        if values.len() < MAX_EXTRAS && extra.well_formed() {
            values.push(extra);
        } else {
            dropped += 1;
        }
    }
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
    (contact, values, dropped)
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

/// The vCard `TYPE` for a predefined label: `mobile` is written as `cell`, and `other` has no type.
fn type_of(label: &str) -> &str {
    match label {
        "mobile" => "cell",
        "other" => "",
        label => label,
    }
}

fn card_of(contact: &Contact, extra: &[Extra], notes: bool) -> Card {
    let (given, family) = names::split(&contact.name);
    let mut phones = if contact.phone.trim().is_empty() {
        vec![]
    } else {
        vec![Labeled { label: phone_label(contact.labels[0]).into(), value: contact.phone.trim().into() }]
    };
    let mut emails = if contact.email.trim().is_empty() {
        vec![]
    } else {
        vec![Labeled { label: email_label(contact.labels[1]).into(), value: contact.email.trim().into() }]
    };
    for value in extra {
        let labeled = Labeled { label: type_of(&value.label).into(), value: value.value.trim().into() };
        match value.kind {
            ValueKind::Phone => phones.push(labeled),
            ValueKind::Email => emails.push(labeled),
            ValueKind::Address => {}
        }
    }
    Card {
        formatted_name: contact.display_name().to_string(),
        family_name: family.to_string(),
        given_name: given.to_string(),
        organization: contact.company.trim().to_string(),
        phones,
        emails,
        note: if notes { contact.notes.clone() } else { String::new() },
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

/// Headless model tests: vCard text in, contacts out, and back. They read and write strings in memory.
#[cfg(test)]
mod headless_vcard_tests {
    use super::*;

    fn ada() -> Contact {
        Contact {
            id: 1,
            name: "Ada Lovelace".into(),
            phone: "555 1234".into(),
            email: "ada@example.test".into(),
            notes: String::new(),
            favorite: false,
            company: "Analytical".into(),
            address: String::new(),
            labels: [0; 3],
        }
    }

    #[test]
    fn a_3_0_card_is_read_into_a_contact_with_its_company_phone_and_email() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nN:Lovelace;Ada;;;\r\nORG:Analytical\r\nTEL;TYPE=CELL:555 1234\r\nEMAIL;TYPE=WORK:ada@example.test\r\nEND:VCARD\r\n";
        let got = import(text).unwrap();
        assert_eq!(got.invalid, 0);
        assert_eq!(got.contacts.len(), 1);
        let c = &got.contacts[0];
        assert!(c.name.contains("Lovelace") && c.name.contains("Ada"));
        assert_eq!(c.company, "Analytical");
        assert_eq!(c.phone, "555 1234");
        assert_eq!(c.email, "ada@example.test");
        assert_eq!(c.id, 0, "identifiers are assigned by the store");
    }

    #[test]
    fn a_card_with_no_name_or_company_is_counted_as_invalid() {
        let got = import("BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\n").unwrap();
        assert_eq!(got.invalid, 1);
        assert!(got.contacts.is_empty());
    }

    #[test]
    fn a_group_card_is_skipped_and_counted() {
        let got = import("BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Friends\r\nMEMBER:urn:uuid:0001\r\nEND:VCARD\r\n").unwrap();
        assert_eq!((got.groups, got.contacts.len()), (1, 0));
    }

    #[test]
    fn an_exported_card_names_its_version_and_reads_back_to_the_same_person() {
        let v3 = export(&[ada()], &Extras::default(), Version::V3);
        assert!(v3.starts_with("BEGIN:VCARD"));
        assert!(v3.contains("VERSION:3.0") && v3.contains("ORG:Analytical"));
        let v4 = export(&[ada()], &Extras::default(), Version::V4);
        assert!(v4.contains("VERSION:4.0"));
        let back = import(&v3).unwrap();
        assert_eq!(back.contacts.len(), 1);
        assert_eq!(back.contacts[0].email, "ada@example.test");
        assert_eq!(back.contacts[0].company, "Analytical");
    }
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
            let text = export(std::slice::from_ref(&original), &Extras::default(), version);
            let got = import(&text).unwrap();
            assert_eq!(got.contacts, vec![original.clone()], "{version:?}");
        }
    }

    #[test]
    fn version_4_0_export_quotes_types_and_says_4_0() {
        let text = export(&[sample()], &Extras::default(), Version::V4);
        assert!(text.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"), "{text}");
        assert!(text.contains("TEL;TYPE=\"work\":+354 555 1234\r\n"), "{text}");
        assert!(text.contains("EMAIL;TYPE=\"work\":olafur@example.is\r\n"), "{text}");
        let v3 = export(&[sample()], &Extras::default(), Version::V3);
        assert!(v3.contains("TEL;TYPE=work:+354 555 1234\r\n"), "{v3}");
    }

    #[test]
    fn long_lines_are_folded_to_75_bytes_and_read_back() {
        let mut long = sample();
        long.notes = "ä".repeat(120);
        let text = export(std::slice::from_ref(&long), &Extras::default(), Version::V3);
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
    fn extra_phones_and_emails_are_kept_under_their_labels() {
        let text = "BEGIN:VCARD\r\nFN:Cara Cole\r\nTEL:111\r\nTEL;TYPE=work:222\r\nEMAIL:a@x.is\r\nEMAIL;TYPE=home:b@x.is\r\nEND:VCARD\r\n";
        let got = import(text).unwrap();
        assert_eq!(got.extra_values, 0);
        assert_eq!(got.contacts[0].phone, "111");
        assert_eq!(
            got.values,
            vec![vec![
                Extra { kind: ValueKind::Phone, label: "work".into(), value: "222".into() },
                Extra { kind: ValueKind::Email, label: "home".into(), value: "b@x.is".into() },
            ]]
        );
    }

    #[test]
    fn a_contact_keeps_six_extra_values_and_counts_the_rest() {
        let mut text = String::from("BEGIN:VCARD\r\nFN:Cara Cole\r\n");
        for i in 1..=8 {
            text.push_str(&format!("TEL:10{i}\r\n"));
        }
        text.push_str("END:VCARD\r\n");
        let got = import(&text).unwrap();
        assert_eq!(got.extra_values, 1, "seven extras, six kept");
        assert_eq!(got.values[0].len(), 6);
        assert_eq!(got.values[0][5].value, "107");
    }

    #[test]
    fn extra_values_and_addresses_are_written_back_on_export() {
        let mut contact = sample();
        contact.address = "Gata 1".into();
        let mut values = Extras::default();
        assert!(values.add(contact.id, Extra { kind: ValueKind::Phone, label: "work".into(), value: "+354 555 9999".into() }));
        assert!(values.add(contact.id, Extra { kind: ValueKind::Address, label: "home".into(), value: "Gata 2".into() }));
        let text = export(std::slice::from_ref(&contact), &values, Version::V3);
        assert!(text.contains("TEL;TYPE=work:+354 555 9999\r\n"), "{text}");
        assert!(text.contains("ADR:;;Gata 1;;;;\r\n") && text.contains("ADR:;;Gata 2;;;;\r\n"), "{text}");
        let got = import(&text).unwrap();
        assert_eq!(got.values[0], vec![Extra { kind: ValueKind::Phone, label: "work".into(), value: "+354 555 9999".into() }]);
    }

    #[test]
    fn card_fields_are_read_from_their_properties() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Anna Lindqvist\r\nN:Lindqvist;Anna;;;\r\nORG:Acme;Sales\r\nNICKNAME:Annie\r\n\
TITLE:Engineer\r\nURL:https://example.org/anna\r\nBDAY:19900517\r\nX-PRONOUNS:she/her\r\nX-PHONETIC-NAME:AN-uh\r\n\
RELATED:Bo Berg\r\nX-SOCIALPROFILE;type=chat:https://chat.example/anna\r\nKIND:org\r\nEND:VCARD\r\n";
        let got = import(text).unwrap();
        let card = &got.details[0];
        assert_eq!(got.contacts[0].company, "Acme");
        assert_eq!(card.get(Detail::Department), "Sales");
        assert_eq!(card.get(Detail::Nickname), "Annie");
        assert_eq!(card.get(Detail::JobTitle), "Engineer");
        assert_eq!(card.get(Detail::Url), "https://example.org/anna");
        assert_eq!(card.get(Detail::Birthday), "1990-05-17");
        assert_eq!(card.get(Detail::Pronouns), "she/her");
        assert_eq!(card.get(Detail::Phonetic), "AN-uh");
        assert_eq!(card.get(Detail::Related), "Bo Berg");
        assert_eq!(card.get(Detail::Profile), "https://chat.example/anna");
        assert!(card.company, "KIND:org marks a company");
    }

    #[test]
    fn card_fields_round_trip_through_3_0_and_4_0() {
        let contact = sample();
        let mut details = Details::default();
        let mut card = CardDetails::default();
        card.company = true;
        for (field, value) in [
            (Detail::Nickname, "Óli, the dev"),
            (Detail::JobTitle, "Engineer"),
            (Detail::Department, "R&D; Labs"),
            (Detail::Url, "https://example.org/a,b"),
            (Detail::Birthday, "1990-05-17"),
            (Detail::Pronouns, "he/him"),
            (Detail::Phonetic, "OH-lee"),
            (Detail::Related, "Anna"),
            (Detail::Profile, "https://chat.example/oli"),
        ] {
            assert!(card.set(field, value));
        }
        details.set(contact.id, card.clone());
        for version in [Version::V3, Version::V4] {
            let text = export_with(std::slice::from_ref(&contact), &Extras::default(), &details, ExportOptions::default(), version);
            let got = import(&text).unwrap();
            assert_eq!(got.contacts, vec![contact.clone()], "{version:?}");
            assert_eq!(got.details, vec![card.clone()], "{version:?}\n{text}");
        }
    }

    #[test]
    fn a_department_without_a_company_still_gets_an_org_line() {
        let mut contact = sample();
        contact.company = String::new();
        let mut details = Details::default();
        let mut card = CardDetails::default();
        card.set(Detail::Department, "Sales");
        details.set(contact.id, card);
        let text = export_with(&[contact], &Extras::default(), &details, ExportOptions::default(), Version::V3);
        assert!(text.contains("ORG:;Sales\r\n"), "{text}");
    }

    #[test]
    fn leaving_the_notes_out_drops_the_note_line_only() {
        let options = ExportOptions { notes: false };
        let text = export_with(&[sample()], &Extras::default(), &Details::default(), options, Version::V3);
        assert!(!text.contains("NOTE"), "{text}");
        assert!(text.contains("TEL;TYPE=work:+354 555 1234\r\n"), "{text}");
        let with = export_with(&[sample()], &Extras::default(), &Details::default(), ExportOptions::default(), Version::V3);
        assert!(with.contains("NOTE:"), "{with}");
    }
}
