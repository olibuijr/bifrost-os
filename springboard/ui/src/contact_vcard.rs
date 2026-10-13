//! vCard 3.0 and 4.0 for the full contact model (CNContactVCardSerialization): `serialize` writes contacts,
//! `contacts` reads them. Covers names, organization, labelled phones, emails, addresses, URLs, relations, social
//! profiles, instant messages, birthday, dates and the photo (base64). Lines fold at 75 bytes and values escape
//! `\`, `,`, `;` and newlines. A label that is not a predefined one is written as an `itemN.X-ABLabel` group and
//! read back from it. Properties the model has no field for are skipped on read.

use crate::contact_book::{
    label, Contact, DateComponents, InstantMessageAddress, LabeledValue, PostalAddress, SocialProfile,
};
use crate::vcard::{escape, fields, fold, unfold, Error, MAX_BYTES, MAX_CARDS, MAX_VALUES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    V3,
    V4,
}

impl Version {
    fn text(self) -> &'static str {
        match self {
            Version::V3 => "3.0",
            Version::V4 => "4.0",
        }
    }
}

/// Writes the contacts as consecutive vCard objects with CRLF line endings.
pub fn serialize(contacts: &[Contact], version: Version) -> String {
    let mut out = String::new();
    for contact in contacts {
        for line in card_lines(contact, version) {
            fold(&line, &mut out);
        }
    }
    out
}

fn full_name(c: &Contact) -> String {
    let join = |parts: &[&str]| parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
    let person = join(&[c.name_prefix.as_str(), c.given_name.as_str(), c.middle_name.as_str(), c.family_name.as_str(), c.name_suffix.as_str()]);
    let name = [person.as_str(), c.nickname.trim(), c.organization_name.trim()].into_iter().find(|s| !s.is_empty()).unwrap_or("").to_string();
    name
}

/// The `TYPE` value for a predefined label's name.
fn type_of(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "mobile" => "cell".into(),
        "homefax" => "home,fax".into(),
        "workfax" => "work,fax".into(),
        "otherfax" => "fax".into(),
        other => other.to_string(),
    }
}

/// Keeps text that is safe inside a parameter value.
fn param_safe(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, ';' | ':' | ',' | '"' | '\r' | '\n')).collect()
}

struct Writer {
    lines: Vec<String>,
    items: u32,
}

impl Writer {
    /// One labelled property: a `TYPE` for a predefined label, an `itemN` group for a custom one.
    fn labelled(&mut self, prop: &str, label: &Option<String>, value: &str) {
        match label {
            None => self.lines.push(format!("{prop}:{value}")),
            Some(l) => match label::standard_name(l) {
                Some(name) => self.lines.push(format!("{prop};TYPE={}:{value}", type_of(name))),
                None => {
                    self.items += 1;
                    let group = format!("item{}", self.items);
                    self.lines.push(format!("{group}.{prop}:{value}"));
                    self.lines.push(format!("{group}.X-ABLabel:{}", escape(l)));
                }
            },
        }
    }
}

fn date_text(d: &DateComponents) -> String {
    match d.year {
        Some(y) => format!("{y:04}-{:02}-{:02}", d.month, d.day),
        None => format!("--{:02}-{:02}", d.month, d.day),
    }
}

fn card_lines(c: &Contact, version: Version) -> Vec<String> {
    let mut w = Writer { lines: vec!["BEGIN:VCARD".into(), format!("VERSION:{}", version.text())], items: 0 };
    w.lines.push(format!("FN:{}", escape(&full_name(c))));
    let n: Vec<String> = [&c.family_name, &c.given_name, &c.middle_name, &c.name_prefix, &c.name_suffix]
        .iter()
        .map(|p| escape(p.trim()))
        .collect();
    w.lines.push(format!("N:{}", n.join(";")));
    if !c.nickname.trim().is_empty() {
        w.lines.push(format!("NICKNAME:{}", escape(c.nickname.trim())));
    }
    if !c.organization_name.trim().is_empty() || !c.department_name.trim().is_empty() {
        let dept = c.department_name.trim();
        let tail = if dept.is_empty() { String::new() } else { format!(";{}", escape(dept)) };
        w.lines.push(format!("ORG:{}{tail}", escape(c.organization_name.trim())));
    }
    if !c.job_title.trim().is_empty() {
        w.lines.push(format!("TITLE:{}", escape(c.job_title.trim())));
    }
    for p in c.phone_numbers.iter().filter(|p| !p.value.trim().is_empty()) {
        w.labelled("TEL", &p.label, &escape(p.value.trim()));
    }
    for e in c.email_addresses.iter().filter(|e| !e.value.trim().is_empty()) {
        w.labelled("EMAIL", &e.label, &escape(e.value.trim()));
    }
    for a in &c.postal_addresses {
        let v = &a.value;
        let parts = ["", "", v.street.as_str(), v.city.as_str(), v.state.as_str(), v.postal_code.as_str(), v.country.as_str()];
        if parts.iter().all(|p| p.trim().is_empty()) {
            continue;
        }
        let value = parts.iter().map(|p| escape(p.trim())).collect::<Vec<_>>().join(";");
        w.labelled("ADR", &a.label, &value);
    }
    for u in c.url_addresses.iter().filter(|u| !u.value.trim().is_empty()) {
        w.labelled("URL", &u.label, &escape(u.value.trim()));
    }
    for r in c.contact_relations.iter().filter(|r| !r.value.trim().is_empty()) {
        w.labelled("RELATED", &r.label, &escape(r.value.trim()));
    }
    for s in &c.social_profiles {
        let v = &s.value;
        w.lines.push(format!(
            "X-SOCIALPROFILE;TYPE={};X-USER={}:{}",
            param_safe(&v.service),
            param_safe(&v.username),
            escape(&v.url_string)
        ));
    }
    for m in &c.instant_message_addresses {
        w.lines.push(format!("IMPP:{}:{}", escape(&m.value.service), escape(&m.value.username)));
    }
    if let Some(b) = &c.birthday {
        w.lines.push(format!("BDAY:{}", date_text(b)));
    }
    for d in &c.dates {
        w.labelled("X-ABDATE", &d.label, &date_text(&d.value));
    }
    if !c.note.trim().is_empty() {
        w.lines.push(format!("NOTE:{}", escape(c.note.trim())));
    }
    if let Some(image) = c.image_data.as_ref().filter(|i| !i.is_empty()) {
        let png = image.starts_with(&[0x89, b'P', b'N', b'G']);
        let data = base64_encode(image);
        w.lines.push(match (version, png) {
            (Version::V3, true) => format!("PHOTO;ENCODING=b;TYPE=PNG:{data}"),
            (Version::V3, false) => format!("PHOTO;ENCODING=b;TYPE=JPEG:{data}"),
            (Version::V4, true) => format!("PHOTO:data:image/png;base64,{data}"),
            (Version::V4, false) => format!("PHOTO:data:image/jpeg;base64,{data}"),
        });
    }
    w.lines.push("END:VCARD".into());
    w.lines
}

/// One property line: its group, name, parameters and raw value.
struct Prop {
    group: Option<String>,
    name: String,
    types: Vec<String>,
    other: Vec<(String, String)>,
    raw: String,
}

fn parse_prop(line: &str) -> Option<Prop> {
    let (head, raw) = line.split_once(':')?;
    let mut parts = head.split(';');
    let base = parts.next()?.trim();
    let (group, name) = match base.rsplit_once('.') {
        Some((g, n)) => (Some(g.to_ascii_lowercase()), n),
        None => (None, base),
    };
    let mut types = Vec::new();
    let mut other = Vec::new();
    for p in parts {
        match p.split_once('=') {
            Some((k, v)) => {
                let v = v.trim().trim_matches('"');
                if k.trim().eq_ignore_ascii_case("TYPE") {
                    types.extend(v.split(',').map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()));
                } else {
                    other.push((k.trim().to_ascii_uppercase(), v.to_string()));
                }
            }
            None if !p.trim().is_empty() => types.push(p.trim().to_ascii_lowercase()),
            None => {}
        }
    }
    Some(Prop { group, name: name.to_ascii_uppercase(), types, other, raw: raw.to_string() })
}

fn label_from_types(types: &[String]) -> Option<String> {
    let t: Vec<&str> = types
        .iter()
        .map(String::as_str)
        .filter(|t| !matches!(*t, "pref" | "voice" | "internet" | "text" | "x400"))
        .collect();
    let first = *t.first()?;
    if t.contains(&"fax") {
        let l = if t.contains(&"home") {
            label::HOME_FAX
        } else if t.contains(&"work") {
            label::WORK_FAX
        } else {
            label::OTHER_FAX
        };
        return Some(l.to_string());
    }
    Some(match first {
        "cell" => label::MOBILE.to_string(),
        other => label::from_standard_name(other).unwrap_or_else(|| other.to_string()),
    })
}

fn unescaped(raw: &str) -> String {
    fields(raw).join(";")
}

fn parse_date(raw: &str) -> Option<DateComponents> {
    let date = raw.trim().split('T').next()?;
    let flat: String = date.chars().filter(|c| *c != '-').collect();
    if !flat.is_ascii() {
        return None;
    }
    if date.starts_with("--") {
        if flat.len() != 4 {
            return None;
        }
        return DateComponents::new(None, flat[..2].parse().ok()?, flat[2..].parse().ok()?);
    }
    if flat.len() != 8 {
        return None;
    }
    DateComponents::new(Some(flat[..4].parse::<i32>().ok()?), flat[4..6].parse().ok()?, flat[6..].parse().ok()?)
}

fn push_capped<T>(list: &mut Vec<LabeledValue<T>>, label: Option<String>, value: T) {
    if list.len() < MAX_VALUES {
        list.push(LabeledValue { identifier: 0, label, value });
    }
}

/// Reads every card in `text` into contacts. Text without cards gives an empty list.
pub fn contacts(text: &str) -> Result<Vec<Contact>, Error> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let mut out = Vec::new();
    let mut open: Option<Vec<Prop>> = None;
    for line in unfold(text) {
        let Some(prop) = parse_prop(&line) else { continue };
        let vcard = prop.raw.trim().eq_ignore_ascii_case("VCARD");
        let name = prop.name.clone();
        match (name.as_str(), open.is_some()) {
            ("BEGIN", false) if vcard => open = Some(Vec::new()),
            ("END", true) if vcard => {
                if let Some(props) = open.take() {
                    out.push(build(props));
                }
                if out.len() > MAX_CARDS {
                    return Err(Error::TooLarge);
                }
            }
            ("BEGIN", _) | ("END", _) => return Err(Error::Malformed),
            (_, true) => {
                if let Some(props) = open.as_mut() {
                    props.push(prop);
                }
            }
            _ => {}
        }
    }
    if open.is_some() {
        return Err(Error::Malformed);
    }
    Ok(out)
}

fn build(props: Vec<Prop>) -> Contact {
    let custom = |p: &Prop| -> Option<String> {
        let group = p.group.as_ref()?;
        props
            .iter()
            .find(|o| o.name == "X-ABLABEL" && o.group.as_ref() == Some(group))
            .map(|o| unescaped(&o.raw))
    };
    let mut c = Contact::default();
    let mut formatted = String::new();
    for p in &props {
        let label = custom(p).or_else(|| label_from_types(&p.types));
        match p.name.as_str() {
            "FN" => formatted = unescaped(&p.raw).trim().to_string(),
            "N" => {
                let f = fields(&p.raw);
                let at = |i: usize| f.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
                (c.family_name, c.given_name, c.middle_name) = (at(0), at(1), at(2));
                (c.name_prefix, c.name_suffix) = (at(3), at(4));
            }
            "NICKNAME" => c.nickname = unescaped(&p.raw).trim().to_string(),
            "ORG" => {
                let f = fields(&p.raw);
                c.organization_name = f.first().map(|s| s.trim().to_string()).unwrap_or_default();
                c.department_name = f.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
            }
            "TITLE" => c.job_title = unescaped(&p.raw).trim().to_string(),
            "NOTE" => c.note = unescaped(&p.raw),
            "TEL" => {
                let v = unescaped(&p.raw);
                let v = v.trim().strip_prefix("tel:").unwrap_or(v.trim()).to_string();
                if !v.is_empty() {
                    push_capped(&mut c.phone_numbers, label, v);
                }
            }
            "EMAIL" => {
                let v = unescaped(&p.raw).trim().to_string();
                if !v.is_empty() {
                    push_capped(&mut c.email_addresses, label, v);
                }
            }
            "URL" => {
                let v = unescaped(&p.raw).trim().to_string();
                if !v.is_empty() {
                    push_capped(&mut c.url_addresses, label, v);
                }
            }
            "RELATED" => {
                let v = unescaped(&p.raw).trim().to_string();
                if !v.is_empty() {
                    push_capped(&mut c.contact_relations, label, v);
                }
            }
            "ADR" => {
                let f = fields(&p.raw);
                let at = |i: usize| f.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
                let address = PostalAddress {
                    street: at(2),
                    city: at(3),
                    state: at(4),
                    postal_code: at(5),
                    country: at(6),
                    ..PostalAddress::default()
                };
                push_capped(&mut c.postal_addresses, label, address);
            }
            "X-SOCIALPROFILE" => {
                let username = p.other.iter().find(|(k, _)| k == "X-USER").map(|(_, v)| v.clone()).unwrap_or_default();
                let profile = SocialProfile {
                    url_string: unescaped(&p.raw),
                    username,
                    user_identifier: String::new(),
                    service: p.types.first().cloned().unwrap_or_default(),
                };
                push_capped(&mut c.social_profiles, None, profile);
            }
            "IMPP" => {
                let text = unescaped(&p.raw);
                let (service, username) = text.split_once(':').unwrap_or(("", text.as_str()));
                let im = InstantMessageAddress { username: username.to_string(), service: service.to_string() };
                push_capped(&mut c.instant_message_addresses, None, im);
            }
            "BDAY" => c.birthday = parse_date(&p.raw),
            "ANNIVERSARY" => {
                if let Some(d) = parse_date(&p.raw) {
                    push_capped(&mut c.dates, Some(label::ANNIVERSARY.to_string()), d);
                }
            }
            "X-ABDATE" => {
                if let Some(d) = parse_date(&p.raw) {
                    push_capped(&mut c.dates, label, d);
                }
            }
            "PHOTO" => {
                let encoded = if p.other.iter().any(|(k, v)| k == "ENCODING" && matches!(v.to_ascii_lowercase().as_str(), "b" | "base64")) {
                    Some(p.raw.as_str())
                } else {
                    p.raw.strip_prefix("data:").and_then(|d| d.split_once("base64,")).map(|(_, data)| data)
                };
                c.image_data = encoded.and_then(base64_decode).filter(|d| !d.is_empty());
            }
            _ => {}
        }
    }
    if c.given_name.is_empty() && c.family_name.is_empty() && c.organization_name.is_empty() && !formatted.is_empty() {
        match formatted.rsplit_once(' ') {
            Some((g, f)) => (c.given_name, c.family_name) = (g.trim().to_string(), f.trim().to_string()),
            None => c.given_name = formatted,
        }
    }
    c
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes base64, ignoring whitespace; `None` for any other stray character.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|b| !b.is_ascii_whitespace()).take_while(|b| *b != b'=') {
        let v = B64.iter().position(|x| *x == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC6350: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Simon Perreault\r\nN:Perreault;Simon;;;ing. jr\\,M.Sc.\r\n\
BDAY:--0203\r\nANNIVERSARY:20090808T1430-0500\r\nGENDER:M\r\nLANG;PREF=1:fr\r\nORG;TYPE=work:Viagenie\r\n\
ADR;TYPE=work:;Suite D2-630;2875 Laurier;Quebec;QC;G1V 2M2;Canada\r\n\
TEL;VALUE=uri;PREF=1;TYPE=\"voice,home\":tel:+1-418-656-9254;ext=102\r\n\
TEL;VALUE=uri;TYPE=\"voice,work,cell,video,text\":tel:+1-418-262-6501\r\n\
EMAIL;TYPE=work:simon.perreault@viagenie.ca\r\nGEO;TYPE=work:geo:46.772673,-71.282945\r\nTZ:-0500\r\n\
URL;TYPE=home:http://nomis80.org\r\nEND:VCARD\r\n";

    fn full() -> Contact {
        let mut c = Contact {
            name_prefix: "Dr".into(),
            given_name: "Ada".into(),
            middle_name: "King".into(),
            family_name: "Lovelace".into(),
            name_suffix: "FRS".into(),
            nickname: "Countess".into(),
            organization_name: "Analytical, Inc.".into(),
            department_name: "Engines".into(),
            job_title: "Programmer".into(),
            note: "line one\nline two; with, punctuation \\ slash".into(),
            birthday: DateComponents::new(Some(1815), 12, 10),
            image_data: Some(vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4, 5]),
            ..Contact::default()
        };
        c.phone_numbers.push(LabeledValue::new(Some(label::MOBILE), "+354 555 1234".into()));
        c.phone_numbers.push(LabeledValue::new(Some("Boat"), "999".into()));
        c.phone_numbers.push(LabeledValue::new(Some(label::HOME_FAX), "111".into()));
        c.email_addresses.push(LabeledValue::new(Some(label::WORK), "ada@example.org".into()));
        c.postal_addresses.push(LabeledValue::new(
            Some(label::HOME),
            PostalAddress { street: "1 Main St".into(), city: "London".into(), state: "LDN".into(), postal_code: "N1".into(), country: "UK".into(), ..PostalAddress::default() },
        ));
        c.url_addresses.push(LabeledValue::new(Some(label::HOME_PAGE), "https://example.org/ada".into()));
        c.contact_relations.push(LabeledValue::new(Some(label::SPOUSE), "William".into()));
        c.social_profiles.push(LabeledValue::new(None, SocialProfile { url_string: "https://x.example/ada".into(), username: "ada".into(), user_identifier: String::new(), service: "twitter".into() }));
        c.instant_message_addresses.push(LabeledValue::new(None, InstantMessageAddress { username: "ada".into(), service: "skype".into() }));
        c.dates.push(LabeledValue::new(Some(label::ANNIVERSARY), DateComponents::new(None, 7, 8).unwrap()));
        c
    }

    #[test]
    fn an_rfc_6350_card_reads_with_labels_and_structured_values() {
        let got = contacts(RFC6350).unwrap();
        assert_eq!(got.len(), 1);
        let c = &got[0];
        assert_eq!((c.given_name.as_str(), c.family_name.as_str()), ("Simon", "Perreault"));
        assert_eq!(c.name_suffix, "ing. jr,M.Sc.");
        assert_eq!(c.birthday, DateComponents::new(None, 2, 3));
        assert_eq!(c.dates[0].value, DateComponents::new(Some(2009), 8, 8).unwrap());
        assert_eq!(c.dates[0].label.as_deref(), Some(label::ANNIVERSARY));
        assert_eq!(c.organization_name, "Viagenie");
        assert_eq!(c.phone_numbers[0].label.as_deref(), Some(label::HOME));
        assert_eq!(c.phone_numbers[0].value, "+1-418-656-9254;ext=102");
        assert_eq!(c.phone_numbers[1].label.as_deref(), Some(label::WORK));
        assert_eq!(c.email_addresses[0].label.as_deref(), Some(label::WORK));
        let a = &c.postal_addresses[0].value;
        assert_eq!((a.street.as_str(), a.city.as_str(), a.state.as_str()), ("2875 Laurier", "Quebec", "QC"));
        assert_eq!((a.postal_code.as_str(), a.country.as_str()), ("G1V 2M2", "Canada"));
        assert_eq!(c.url_addresses[0].label.as_deref(), Some(label::HOME));
    }

    #[test]
    fn a_full_contact_round_trips_in_both_versions() {
        for version in [Version::V3, Version::V4] {
            let text = serialize(&[full()], version);
            assert!(text.contains(&format!("VERSION:{}\r\n", version.text())));
            assert_eq!(contacts(&text).unwrap(), vec![full()], "{version:?}");
        }
    }

    #[test]
    fn lines_fold_at_75_bytes_and_unfold_again() {
        let mut c = full();
        c.note = "ä".repeat(150);
        c.image_data = Some((0..=255u8).cycle().take(400).collect());
        let text = serialize(&[c.clone()], Version::V3);
        for line in text.split("\r\n") {
            assert!(line.len() <= 75, "too long: {line:?}");
        }
        assert_eq!(contacts(&text).unwrap(), vec![c]);
    }

    #[test]
    fn photos_are_written_per_version_and_both_forms_are_read() {
        let mut c = Contact { given_name: "P".into(), ..Contact::default() };
        c.image_data = Some(vec![0xFF, 0xD8, 0xFF, 0xE0, 7]);
        let v3 = serialize(&[c.clone()], Version::V3).replace("\r\n ", "");
        assert!(v3.contains("PHOTO;ENCODING=b;TYPE=JPEG:/9j/4Ac="), "{v3}");
        let v4 = serialize(&[c.clone()], Version::V4).replace("\r\n ", "");
        assert!(v4.contains("PHOTO:data:image/jpeg;base64,/9j/4Ac="), "{v4}");
        assert_eq!(contacts(&v3).unwrap()[0].image_data, c.image_data);
        assert_eq!(contacts(&v4).unwrap()[0].image_data, c.image_data);
    }

    #[test]
    fn custom_labels_use_an_item_group_and_predefined_ones_use_type() {
        let text = serialize(&[full()], Version::V3);
        assert!(text.contains("item1.TEL:999\r\nitem1.X-ABLabel:Boat\r\n"), "{text}");
        assert!(text.contains("TEL;TYPE=cell:+354 555 1234\r\n"));
        assert!(text.contains("TEL;TYPE=home,fax:111\r\n"));
        assert!(text.contains("EMAIL;TYPE=work:ada@example.org\r\n"));
        assert!(text.contains("BDAY:1815-12-10\r\n"));
    }

    #[test]
    fn several_cards_blank_values_and_broken_structure() {
        let a = Contact { given_name: "A".into(), phone_numbers: vec![LabeledValue::new(None, "  ".into())], ..Contact::default() };
        let b = Contact { organization_name: "Only Org".into(), ..Contact::default() };
        let text = serialize(&[a, b], Version::V4);
        assert!(!text.contains("TEL"), "a blank phone is not written");
        let got = contacts(&text).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].organization_name, "Only Org");
        assert_eq!(contacts("VERSION:3.0\r\n").unwrap(), vec![]);
        assert_eq!(contacts("BEGIN:VCARD\r\nFN:x\r\n"), Err(Error::Malformed));
        assert_eq!(contacts("END:VCARD\r\n"), Err(Error::Malformed));
        assert_eq!(contacts("BEGIN:VCARD\r\nBEGIN:VCARD\r\n"), Err(Error::Malformed));
    }

    #[test]
    fn a_card_with_only_a_formatted_name_gets_a_split_name() {
        let got = contacts("BEGIN:VCARD\r\nFN:Grace Brewster Hopper\r\nEND:VCARD\r\n").unwrap();
        assert_eq!((got[0].given_name.as_str(), got[0].family_name.as_str()), ("Grace Brewster", "Hopper"));
    }

    #[test]
    fn dates_parse_in_every_vcard_form_and_reject_nonsense() {
        assert_eq!(parse_date("1990-05-04"), DateComponents::new(Some(1990), 5, 4));
        assert_eq!(parse_date("19900504"), DateComponents::new(Some(1990), 5, 4));
        assert_eq!(parse_date("--0504"), DateComponents::new(None, 5, 4));
        assert_eq!(parse_date("--05-04"), DateComponents::new(None, 5, 4));
        assert_eq!(parse_date("1990-13-04"), None);
        assert_eq!(parse_date("soon"), None);
    }

    #[test]
    fn base64_matches_the_standard_vectors() {
        for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded).unwrap(), plain.as_bytes());
        }
        assert_eq!(base64_decode("Zm9v\r\n YmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("Zm9*"), None);
    }
}
