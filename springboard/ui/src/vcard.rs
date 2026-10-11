//! vCard 3.0 cards for contacts: read cards from text and write them back.
//! Reading unfolds continuation lines and unescapes values; writing escapes them and folds long lines to 75 bytes.
//! The model keeps names, the organization, labelled phone numbers and emails, and the note. Other properties
//! (addresses, dates, photos, custom fields) are skipped on read and never written.

/// Largest vCard text accepted, in bytes.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Most cards read from one text.
pub const MAX_CARDS: usize = 10_000;
/// Most phone numbers, and most emails, kept per card.
pub const MAX_VALUES: usize = 64;

/// Type parameters that describe a value's preference or encoding rather than its use (`TYPE=pref`).
const NOT_LABELS: [&str; 4] = ["pref", "voice", "internet", "text"];

/// A phone number or email with its label (the lower-case `TYPE` parameter, for example `home` or `cell`).
/// An empty label means the card gave none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Labeled {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Card {
    /// `FN`: the name as written on the card.
    pub formatted_name: String,
    /// `N`: family name and given name.
    pub family_name: String,
    pub given_name: String,
    /// `ORG`: the organization (the first component).
    pub organization: String,
    pub phones: Vec<Labeled>,
    pub emails: Vec<Labeled>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Text longer than `MAX_BYTES`, or more than `MAX_CARDS` cards.
    TooLarge,
    /// `END:VCARD` without a matching `BEGIN:VCARD`, a nested `BEGIN`, or a card left open at the end of the text.
    Malformed,
}

impl Card {
    /// The name shown for the card: the formatted name, else given and family name, else the organization.
    pub fn display_name(&self) -> String {
        let person = [self.given_name.trim(), self.family_name.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        [self.formatted_name.trim(), person.as_str(), self.organization.trim()]
            .into_iter()
            .find(|part| !part.is_empty())
            .unwrap_or("")
            .to_string()
    }

    /// Writes the card as one vCard 3.0 object with CRLF line endings.
    pub fn to_vcard(&self) -> String {
        let mut lines = vec![
            "BEGIN:VCARD".to_string(),
            "VERSION:3.0".to_string(),
            format!("FN:{}", escape(&self.display_name())),
            format!("N:{};{};;;", escape(self.family_name.trim()), escape(self.given_name.trim())),
        ];
        if !self.organization.trim().is_empty() {
            lines.push(format!("ORG:{}", escape(self.organization.trim())));
        }
        for phone in self.phones.iter().filter(|p| !p.value.trim().is_empty()) {
            lines.push(format!("TEL{}:{}", type_param(&phone.label), escape(phone.value.trim())));
        }
        for email in self.emails.iter().filter(|e| !e.value.trim().is_empty()) {
            lines.push(format!("EMAIL{}:{}", type_param(&email.label), escape(email.value.trim())));
        }
        if !self.note.trim().is_empty() {
            lines.push(format!("NOTE:{}", escape(self.note.trim())));
        }
        lines.push("END:VCARD".to_string());
        let mut out = String::new();
        for line in &lines {
            fold(line, &mut out);
        }
        out
    }
}

/// Reads every card in `text`. Text without cards gives an empty list.
pub fn parse(text: &str) -> Result<Vec<Card>, Error> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let mut cards = Vec::new();
    let mut open: Option<Card> = None;
    for line in unfold(text) {
        let Some((head, raw)) = line.split_once(':') else { continue };
        // `item1.TEL;TYPE=home` names the property TEL in the group item1.
        let base = head.split(';').next().unwrap_or("");
        let name = base.rsplit('.').next().unwrap_or("").trim().to_ascii_uppercase();
        let params: Vec<&str> = head.split(';').skip(1).collect();
        let is_open = open.is_some();
        match name.as_str() {
            "BEGIN" if !is_open && raw.trim().eq_ignore_ascii_case("VCARD") => open = Some(Card::default()),
            "END" if is_open && raw.trim().eq_ignore_ascii_case("VCARD") => {
                if let Some(card) = open.take() {
                    cards.push(card);
                }
                if cards.len() > MAX_CARDS {
                    return Err(Error::TooLarge);
                }
            }
            "BEGIN" | "END" => return Err(Error::Malformed),
            _ => {
                if let Some(card) = open.as_mut() {
                    apply(card, &name, &params, raw);
                }
            }
        }
    }
    if open.is_some() {
        return Err(Error::Malformed);
    }
    Ok(cards)
}

/// Joins continuation lines (starting with a space or a tab) to the line before them.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let continued = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t'));
        if let Some(rest) = continued {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        out.push(line.to_string());
    }
    out
}

fn apply(card: &mut Card, name: &str, params: &[&str], raw: &str) {
    match name {
        "FN" => card.formatted_name = text(raw).trim().to_string(),
        "N" => {
            let parts = fields(raw);
            card.family_name = field(&parts, 0);
            card.given_name = field(&parts, 1);
        }
        "ORG" => card.organization = field(&fields(raw), 0),
        "NOTE" => card.note = text(raw),
        "TEL" => push_value(&mut card.phones, params, raw),
        "EMAIL" => push_value(&mut card.emails, params, raw),
        _ => {}
    }
}

fn push_value(list: &mut Vec<Labeled>, params: &[&str], raw: &str) {
    let value = text(raw).trim().to_string();
    if value.is_empty() || list.len() >= MAX_VALUES {
        return;
    }
    list.push(Labeled { label: label(params), value });
}

/// The first `TYPE` value that describes use, lower case. Accepts `TYPE=work,voice` and the bare `;CELL` form
/// of older cards.
fn label(params: &[&str]) -> String {
    params
        .iter()
        .flat_map(|param| {
            let types = match param.split_once('=') {
                Some((key, value)) if key.eq_ignore_ascii_case("TYPE") => value,
                Some(_) => "",
                None => *param,
            };
            types.split(',').map(|t| t.trim().to_ascii_lowercase()).collect::<Vec<_>>()
        })
        .find(|t| !t.is_empty() && !NOT_LABELS.contains(&t.as_str()))
        .unwrap_or_default()
}

/// A single text value: escapes are undone, and a bare `;` (which the format requires to be escaped) is kept.
fn text(raw: &str) -> String {
    fields(raw).join(";")
}

/// The `;`-separated components of a structured value, each unescaped.
fn fields(raw: &str) -> Vec<String> {
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

fn field(parts: &[String], index: usize) -> String {
    parts.get(index).map(|part| part.trim().to_string()).unwrap_or_default()
}

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

/// `;TYPE=home` for a label made of letters, digits and `-`; nothing otherwise.
fn type_param(label: &str) -> String {
    let clean: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if clean.is_empty() {
        String::new()
    } else {
        format!(";TYPE={clean}")
    }
}

/// Appends `line` as CRLF-terminated physical lines of at most 75 bytes; continuation lines start with a space.
fn fold(line: &str, out: &mut String) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Anna Lindqvist\r\nN:Lindqvist;Anna;;;\r\n\
ORG:Acme\\, Inc.;Sales\r\nTEL;TYPE=CELL,VOICE:+354 555 1234\r\nEMAIL;TYPE=WORK:anna@example.org\r\n\
NOTE:Line one\\nLine two\\, with a comma\r\nADR;TYPE=HOME:;;Street 1;Reykjavik;;101;Iceland\r\nEND:VCARD\r\n";

    #[test]
    fn reads_names_organization_labelled_values_and_note() {
        let cards = parse(SAMPLE).unwrap();
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!(card.formatted_name, "Anna Lindqvist");
        assert_eq!((card.family_name.as_str(), card.given_name.as_str()), ("Lindqvist", "Anna"));
        assert_eq!(card.organization, "Acme, Inc.", "the escaped comma stays inside the first component");
        assert_eq!(card.phones, vec![Labeled { label: "cell".into(), value: "+354 555 1234".into() }]);
        assert_eq!(card.emails, vec![Labeled { label: "work".into(), value: "anna@example.org".into() }]);
        assert_eq!(card.note, "Line one\nLine two, with a comma");
        assert_eq!(card.display_name(), "Anna Lindqvist");
    }

    #[test]
    fn folded_lines_are_unfolded_and_a_written_card_reads_back_unchanged() {
        let card = Card {
            formatted_name: "Jón".into(),
            family_name: "Ólafsson".into(),
            given_name: "Ólafur Búi".into(),
            note: "ä".repeat(120),
            phones: vec![Labeled { label: "home".into(), value: "5551234".into() }],
            ..Card::default()
        };
        let text = card.to_vcard();
        assert!(text.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        for line in text.split("\r\n") {
            assert!(line.len() <= 75, "a physical line is at most 75 bytes: {line:?}");
        }
        let back = parse(&text).unwrap();
        assert_eq!(back, vec![card]);
    }

    #[test]
    fn separators_backslashes_and_newlines_survive_a_round_trip() {
        let card = Card {
            family_name: "O\\Brien; Jr".into(),
            given_name: "Pat,Ann".into(),
            note: "first\nsecond; third\\fourth".into(),
            ..Card::default()
        };
        let back = parse(&card.to_vcard()).unwrap().remove(0);
        assert_eq!(back.family_name, "O\\Brien; Jr");
        assert_eq!(back.given_name, "Pat,Ann");
        assert_eq!(back.note, "first\nsecond; third\\fourth");
    }

    #[test]
    fn several_cards_are_read_and_broken_structure_is_refused() {
        let two = format!("{SAMPLE}{SAMPLE}");
        assert_eq!(parse(&two).unwrap().len(), 2);
        assert_eq!(parse("VERSION:3.0\r\n").unwrap(), vec![], "text without a card is empty, not an error");
        assert_eq!(parse("BEGIN:VCARD\r\nFN:Open\r\n"), Err(Error::Malformed), "an open card is refused");
        assert_eq!(parse("END:VCARD\r\n"), Err(Error::Malformed), "an end without a begin is refused");
        assert_eq!(parse("BEGIN:VCARD\r\nBEGIN:VCARD\r\nEND:VCARD\r\n"), Err(Error::Malformed));
    }

    #[test]
    fn empty_values_unknown_properties_and_group_prefixes_are_handled() {
        let text = "BEGIN:VCARD\r\nitem1.TEL;CELL;PREF:+1 555\r\nTEL:   \r\nX-CUSTOM:ignored\r\n\
item2.EMAIL;TYPE=pref,home:a@example.org\r\nORG:Only Company\r\nEND:VCARD\r\n";
        let card = parse(text).unwrap().remove(0);
        assert_eq!(card.phones, vec![Labeled { label: "cell".into(), value: "+1 555".into() }]);
        assert_eq!(card.emails, vec![Labeled { label: "home".into(), value: "a@example.org".into() }]);
        assert_eq!(card.display_name(), "Only Company", "the organization names a card with no person");
    }

    #[test]
    fn written_labels_are_cleaned_and_empty_values_are_not_written() {
        let card = Card {
            family_name: "Doe".into(),
            phones: vec![
                Labeled { label: "we ird/cell".into(), value: "111".into() },
                Labeled { label: "".into(), value: "   ".into() },
            ],
            ..Card::default()
        };
        let text = card.to_vcard();
        assert!(text.contains("TEL;TYPE=weirdcell:111\r\n"));
        assert_eq!(text.matches("TEL").count(), 1, "a blank phone is not written");
        assert!(!text.contains("NOTE"), "an empty note is left out");
    }
}
