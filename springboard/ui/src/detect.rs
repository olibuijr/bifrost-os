//! `NativeDataDetector` (Foundation): finds links, email addresses, dates, street addresses, money amounts, flight
//! numbers and phone numbers in plain text.
//! Results are byte ranges into the input, cut only at ASCII bytes, so they are always char boundaries.
//! Overlaps are resolved by kind priority (link, then date, then phone), so "2026-10-09" is a date, not a phone number.

use crate::phone_rules;
use std::ops::Range;

/// The kinds of content the detector knows, the `NativeTextCheckingType` subset Bifrost uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Link,
    Date,
    Address,
    Phone,
    /// `local@domain.tld`.
    Email,
    /// A number with a currency symbol or code, or `kr`: `$12.50`, `ISK 2.000`, `1.500 kr`, `12,99 EUR`.
    Money,
    /// An airline code and a flight number, `FI 552` or `BA117` (the official flight match kind).
    Flight,
}

/// One piece of text the detector recognised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub kind: Kind,
    pub range: Range<usize>,
}

impl Match {
    /// The matched text in `source`.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range.clone()]
    }
}

/// The E.164 form (`+` then digits) of a phone match, checked against the per-region rules in [`phone_rules`] for
/// `region` (a region code such as `IS`). `None` for a match of another kind or a number the rules do not accept.
pub fn phone_e164(m: &Match, source: &str, region: &str) -> Option<String> {
    (m.kind == Kind::Phone).then(|| phone_rules::to_e164(m.text(source), region)).flatten()
}

/// Finds every `kinds` item in `text`, sorted by position and without overlaps.
pub fn find(text: &str, kinds: &[Kind]) -> Vec<Match> {
    let mut accepted: Vec<Match> = Vec::new();
    // Priority order, whatever order the caller listed the kinds in. A link wins over an address inside it.
    for kind in [Kind::Link, Kind::Email, Kind::Date, Kind::Address, Kind::Money, Kind::Flight, Kind::Phone] {
        if !kinds.contains(&kind) {
            continue;
        }
        let found = match kind {
            Kind::Link => links(text),
            Kind::Email => emails(text),
            Kind::Date => dates(text),
            Kind::Address => addresses(text),
            Kind::Money => money(text),
            Kind::Flight => flights(text),
            Kind::Phone => phones(text),
        };
        for range in found {
            if !accepted.iter().any(|m| m.range.start < range.end && range.start < m.range.end) {
                accepted.push(Match { kind, range });
            }
        }
    }
    accepted.sort_by_key(|m| m.range.start);
    accepted
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `http://`, `https://` and `www.` links. The link ends at whitespace or a quote or bracket; sentence
/// punctuation after it is not part of it, and a closing bracket is only kept when it has an opener.
fn links(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    for prefix in ["https://", "http://", "www."] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(prefix).map(|i| i + from) {
            from = at + prefix.len();
            // a word or an `@` before the prefix means it is part of another token (`x@www.is` is an address)
            if at > 0 && (is_word(bytes[at - 1]) || bytes[at - 1] == b'@') {
                continue;
            }
            let mut end = at + prefix.len();
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() && !matches!(bytes[end], b'<' | b'>' | b'"' | b'\'' | b'`') {
                end += 1;
            }
            // trim sentence punctuation and unbalanced closing brackets
            loop {
                let slice = &bytes[at..end];
                let last = match slice.last() {
                    Some(&b) => b,
                    None => break,
                };
                let trim = match last {
                    b'.' | b',' | b';' | b':' | b'!' | b'?' => true,
                    b')' => slice.iter().filter(|&&b| b == b'(').count() < slice.iter().filter(|&&b| b == b')').count(),
                    b']' => slice.iter().filter(|&&b| b == b'[').count() < slice.iter().filter(|&&b| b == b']').count(),
                    _ => false,
                };
                if !trim {
                    break;
                }
                end -= 1;
            }
            // the host needs a dot and at least one character before it
            let host_start = at + prefix.len();
            if end <= host_start {
                continue;
            }
            let host_end = text[host_start..end].find(['/', ':', '?', '#']).map_or(end, |i| host_start + i);
            let host = &text[host_start..host_end];
            let dot = host.find('.');
            if dot.map_or(true, |d| d == 0) || host.ends_with('.') || host.len() < 3 {
                continue;
            }
            out.push(at..end);
            from = end;
        }
    }
    out
}

/// Dates written as `yyyy-mm-dd` or `d.m.yyyy` (the Icelandic and European order). Only real calendar
/// dates count, so "2026-13-01" is not a date.
fn dates(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || (i > 0 && (is_word(bytes[i - 1]) || (bytes[i - 1] == b'.' && i > 1 && bytes[i - 2].is_ascii_digit()))) {
            i += 1;
            continue;
        }
        if let Some(end) = iso_date(bytes, i).or_else(|| dotted_date(bytes, i)) {
            if end >= bytes.len() || !is_word(bytes[end]) {
                out.push(i..end);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn digits(bytes: &[u8], at: usize, n: usize) -> Option<u32> {
    let run = bytes.get(at..at + n)?;
    run.iter().all(u8::is_ascii_digit).then(|| run.iter().fold(0, |v, &b| v * 10 + u32::from(b - b'0')))
}

fn iso_date(bytes: &[u8], at: usize) -> Option<usize> {
    let y = digits(bytes, at, 4)?;
    if bytes.get(at + 4) != Some(&b'-') {
        return None;
    }
    let m = digits(bytes, at + 5, 2)?;
    if bytes.get(at + 7) != Some(&b'-') {
        return None;
    }
    let d = digits(bytes, at + 8, 2)?;
    valid(y, m, d).then_some(at + 10)
}

/// One or two digits at `at`: the value and the index after them.
fn small(bytes: &[u8], at: usize) -> Option<(u32, usize)> {
    let mut v = 0;
    let mut n = 0;
    while n < 2 && bytes.get(at + n).is_some_and(u8::is_ascii_digit) {
        v = v * 10 + u32::from(bytes[at + n] - b'0');
        n += 1;
    }
    (n > 0).then_some((v, at + n))
}

/// `d.m.yyyy` with one or two digits for the day and the month.
fn dotted_date(bytes: &[u8], at: usize) -> Option<usize> {
    let (d, p) = small(bytes, at)?;
    if bytes.get(p) != Some(&b'.') {
        return None;
    }
    let (m, p) = small(bytes, p + 1)?;
    if bytes.get(p) != Some(&b'.') {
        return None;
    }
    let y = digits(bytes, p + 1, 4)?;
    let end = p + 5;
    (valid(y, m, d) && !bytes.get(end).is_some_and(u8::is_ascii_digit)).then_some(end)
}

fn valid(y: u32, m: u32, d: u32) -> bool {
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    (1..=days).contains(&d) && y >= 1000
}

/// Street addresses: a street name followed by a house number, in either order and in English or Icelandic
/// form: "Laugavegur 12", "Hverfisgötu 4a", "Main Street 12", "12 Main Street". A street name needs its
/// house number (a bare "Main Street" is not an address). Capital letters mark the name in the separate-word forms.
fn addresses(text: &str) -> Vec<Range<usize>> {
    if !text.bytes().any(|b| b.is_ascii_digit()) {
        return Vec::new();
    }
    const ATTACHED: [&str; 8] = ["gata", "götu", "gotu", "vegur", "vegi", "braut", "stígur", "stræti"];
    const WORDS: [&str; 10] = ["street", "st", "road", "rd", "avenue", "ave", "lane", "drive", "boulevard", "blvd"];
    // word spans, without trailing sentence punctuation
    let mut toks: Vec<Range<usize>> = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        match (start, c.is_whitespace()) {
            (None, false) => start = Some(i),
            (Some(s0), true) => {
                let mut end = i;
                while end > s0 && text[..end].ends_with([',', '.', ';', ':', '!', '?']) {
                    end = text[..end].char_indices().last().map_or(s0, |(j, _)| j);
                }
                if end > s0 {
                    toks.push(s0..end);
                }
                start = None;
            }
            _ => {}
        }
    }
    let word = |i: usize| &text[toks[i].clone()];
    let is_number = |t: &str| t.starts_with(|c: char| c.is_ascii_digit()) && t.chars().all(|c| c.is_alphanumeric() || c == '-');
    let capital = |t: &str| t.starts_with(|c: char| c.is_uppercase());
    let attached_suffix = |w: &str| ATTACHED.iter().any(|s| w.len() > s.len() && w.ends_with(s) && !w[..w.len() - s.len()].ends_with(|c: char| !c.is_alphabetic()));
    let mut out = Vec::new();
    let mut i = 0;
    // every pattern needs a house number, so the cheap tests run before any lower-casing
    while i < toks.len() {
        // "Laugavegur 12"
        if i + 1 < toks.len() && is_number(word(i + 1)) && attached_suffix(&word(i).to_lowercase()) {
            out.push(toks[i].start..toks[i + 1].end);
            i += 2;
            continue;
        }
        // "Main Street 12"
        if i >= 1 && i + 1 < toks.len() && capital(word(i - 1)) && is_number(word(i + 1)) && WORDS.contains(&word(i).to_lowercase().as_str()) {
            out.push(toks[i - 1].start..toks[i + 1].end);
            i += 2;
            continue;
        }
        // "12 Main Street"
        if i + 2 < toks.len() && is_number(word(i)) && capital(word(i + 1)) && WORDS.contains(&word(i + 2).to_lowercase().as_str()) {
            out.push(toks[i].start..toks[i + 2].end);
            i += 3;
            continue;
        }
        i += 1;
    }
    out
}

/// Flight numbers: a two-character airline code in capitals (a capital, then a capital or a digit), an optional single
/// space, then one to four digits that end at a word boundary: `FI 552`, `BA117`. Lower-case letters do not count, so
/// `is 12` is not a flight. Two capitals before digits can still be an ordinary word, so this rule is a heuristic.
fn flights(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for at in 0..bytes.len() {
        if (at > 0 && is_word(bytes[at - 1])) || !bytes[at].is_ascii_uppercase() {
            continue;
        }
        let second = match bytes.get(at + 1) {
            Some(&b) => b,
            None => break,
        };
        if !(second.is_ascii_uppercase() || second.is_ascii_digit()) {
            continue;
        }
        let mut p = at + 2;
        if bytes.get(p) == Some(&b' ') {
            p += 1;
        }
        let digits_start = p;
        while p < bytes.len() && bytes[p].is_ascii_digit() && p - digits_start < 4 {
            p += 1;
        }
        if p == digits_start {
            continue;
        }
        // a fifth digit, or any letter after the number, means this is not a flight number
        if bytes.get(p).is_some_and(|&b| is_word(b)) {
            continue;
        }
        out.push(at..p);
    }
    out
}

/// The airline code and the flight number of a flight match: `("FI", "552")` for `FI 552`. `None` when the text has
/// no digits, or the code before the digits is not two characters.
pub fn flight_parts(text: &str) -> Option<(&str, &str)> {
    let at = text.find(|c: char| c.is_ascii_digit())?;
    let airline = text[..at].trim_end();
    let number = &text[at..];
    let valid = airline.len() == 2 && number.bytes().all(|b| b.is_ascii_digit());
    valid.then_some((airline, number))
}

/// Phone numbers: 7 to 15 digits (E.164 length) with spaces, dashes or brackets between them, and an
/// optional leading `+`. A bare run of digits is not a phone number here (it is usually an id or a version).
fn phones(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        if (i > 0 && is_word(bytes[i - 1])) || !(bytes[i].is_ascii_digit() || (bytes[i] == b'+' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) || (bytes[i] == b'(' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))) {
            i += 1;
            continue;
        }
        let plus = bytes[i] == b'+';
        let mut j = if plus { i + 1 } else { i };
        let mut last_digit_end = None;
        let mut count = 0;
        let mut separator = plus;
        while j < bytes.len() {
            let b = bytes[j];
            if b.is_ascii_digit() {
                count += 1;
                last_digit_end = Some(j + 1);
            } else if matches!(b, b' ' | b'-' | b'(' | b')') {
                separator = true;
            } else {
                break;
            }
            j += 1;
        }
        let end = last_digit_end.unwrap_or(start + 1);
        let trailing_word = bytes.get(end).is_some_and(|&b| is_word(b));
        if (7..=15).contains(&count) && separator && !trailing_word {
            out.push(start..end);
            i = end;
        } else {
            i = start + 1;
        }
    }
    out
}

/// Email addresses, `local@domain.tld`. The local part is ASCII letters, digits and `._%+-`; the domain is
/// dot-separated labels of letters, digits and `-`, and its last label is at least two letters, so "a@b" is no address.
fn emails(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let is_local = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-');
    let mut out = Vec::new();
    for at in (0..bytes.len()).filter(|&i| bytes[i] == b'@') {
        let mut start = at;
        while start > 0 && is_local(bytes[start - 1]) {
            start -= 1;
        }
        while start < at && bytes[start] == b'.' {
            start += 1;
        }
        if start == at || bytes[at - 1] == b'.' {
            continue;
        }
        let mut end = at + 1;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'.' | b'-')) {
            end += 1;
        }
        while end > at + 1 && bytes[end - 1] == b'.' {
            end -= 1;
        }
        let labels: Vec<&str> = text[at + 1..end].split('.').collect();
        let tld = labels.last().copied().unwrap_or("");
        let valid = labels.len() >= 2
            && labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-'))
            && tld.len() >= 2
            && tld.bytes().all(|b| b.is_ascii_alphabetic());
        if valid {
            out.push(start..end);
        }
    }
    out
}

/// Currency codes that may stand before or after an amount. `kr` (krónur) is handled apart: it is only a suffix.
const CURRENCY_CODES: [&str; 9] = ["ISK", "USD", "EUR", "GBP", "SEK", "DKK", "NOK", "JPY", "CHF"];
const CURRENCY_SYMBOLS: [&str; 4] = ["$", "€", "£", "¥"];

/// Money amounts: `$12.50`, `€ 3,99`, `ISK 2.000` (a symbol or code before) and `1.500 kr`, `12,99 EUR` (after).
/// Digits group with `.` or `,`, and a fraction follows a `.` or `,`. A bare number is not money.
fn money(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let boundary = i == 0 || !(is_word(bytes[i - 1]) || matches!(bytes[i - 1], b'.' | b','));
        match prefixed_money(bytes, i).or_else(|| if boundary { suffixed_money(bytes, i) } else { None }) {
            Some(end) => {
                out.push(i..end);
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

/// The ASCII `word` at `at`, ignoring case, that ends at a word boundary.
fn word_at(bytes: &[u8], at: usize, word: &str) -> bool {
    bytes.get(at..at + word.len()).is_some_and(|s| s.eq_ignore_ascii_case(word.as_bytes())) && !bytes.get(at + word.len()).is_some_and(|&b| is_word(b))
}

/// Digits with `.` or `,` group and fraction separators (`1.500`, `12,99`, `1.250.000,50`): the index after them.
fn amount(bytes: &[u8], at: usize) -> Option<usize> {
    let mut p = at;
    while bytes.get(p).is_some_and(u8::is_ascii_digit) {
        p += 1;
    }
    if p == at {
        return None;
    }
    while matches!(bytes.get(p).copied(), Some(b'.' | b',')) && bytes.get(p + 1).is_some_and(u8::is_ascii_digit) {
        p += 1;
        while bytes.get(p).is_some_and(u8::is_ascii_digit) {
            p += 1;
        }
    }
    Some(p)
}

/// A symbol or code, optional spaces, then the amount: the index after the amount.
fn prefixed_money(bytes: &[u8], at: usize) -> Option<usize> {
    let len = if let Some(symbol) = CURRENCY_SYMBOLS.iter().find(|s| bytes[at..].starts_with(s.as_bytes())) {
        symbol.len()
    } else {
        if at > 0 && is_word(bytes[at - 1]) {
            return None;
        }
        CURRENCY_CODES.iter().find(|c| word_at(bytes, at, c))?.len()
    };
    let mut p = at + len;
    while bytes.get(p) == Some(&b' ') {
        p += 1;
    }
    let end = amount(bytes, p)?;
    (!bytes.get(end).is_some_and(|&b| is_word(b))).then_some(end)
}

/// An amount, optional spaces, then `kr` or a currency code: the index after that word.
fn suffixed_money(bytes: &[u8], at: usize) -> Option<usize> {
    let end = amount(bytes, at)?;
    let mut p = end;
    while bytes.get(p) == Some(&b' ') {
        p += 1;
    }
    if word_at(bytes, p, "kr") {
        return Some(p + 2);
    }
    let code = CURRENCY_CODES.iter().find(|c| word_at(bytes, p, c))?;
    Some(p + code.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str, kinds: &[Kind]) -> Vec<(Kind, String)> {
        find(s, kinds).into_iter().map(|m| (m.kind, m.text(s).to_string())).collect()
    }

    #[test]
    fn links_stop_before_sentence_punctuation_and_unbalanced_brackets() {
        let all = [Kind::Link];
        assert_eq!(texts("See https://bifrost.olibuijr.com/docs.", &all), vec![(Kind::Link, "https://bifrost.olibuijr.com/docs".into())]);
        assert_eq!(texts("(read www.example.is/a_(b))!", &all), vec![(Kind::Link, "www.example.is/a_(b)".into())]);
        assert_eq!(texts("[http://a.is]", &all), vec![(Kind::Link, "http://a.is".into())]);
        assert_eq!(texts("<https://x.is/y>", &all), vec![(Kind::Link, "https://x.is/y".into())]);
    }

    #[test]
    fn links_need_a_real_host() {
        let all = [Kind::Link];
        assert!(texts("https://localhost", &all).is_empty(), "no dot in the host");
        assert!(texts("https://.is", &all).is_empty());
        assert!(texts("www.", &all).is_empty());
        assert!(texts("xhttps://a.is", &all).is_empty(), "a link must start at a word boundary");
    }

    #[test]
    fn dates_are_real_calendar_dates_in_two_orders() {
        let all = [Kind::Date];
        assert_eq!(texts("Fundur 2026-10-09 kl. 10", &all), vec![(Kind::Date, "2026-10-09".into())]);
        assert_eq!(texts("opnað 9.10.2026.", &all), vec![(Kind::Date, "9.10.2026".into())]);
        assert_eq!(texts("29.02.2026", &all), vec![], "2026 is not a leap year");
        assert_eq!(texts("29.02.2028", &all), vec![(Kind::Date, "29.02.2028".into())]);
        assert!(texts("2026-13-01", &all).is_empty());
        assert!(texts("0.4.70", &all).is_empty(), "version numbers are not dates");
    }

    #[test]
    fn street_addresses_need_a_house_number() {
        let all = [Kind::Address];
        assert_eq!(texts("Hitti á Laugavegur 12, 101 Reykjavík.", &all), vec![(Kind::Address, "Laugavegur 12".into())]);
        assert_eq!(texts("Hverfisgötu 4a", &all), vec![(Kind::Address, "Hverfisgötu 4a".into())]);
        assert_eq!(texts("Main Street 12 and 12 Oak Road", &all), vec![(Kind::Address, "Main Street 12".into()), (Kind::Address, "12 Oak Road".into())]);
        assert!(texts("Main Street", &all).is_empty(), "no house number, no address");
        assert!(texts("the vegur was long", &all).is_empty(), "a lower-case road word alone is not a name");
        assert!(texts("I walked the street 12 times", &all).is_empty(), "lower-case street name is not a street");
    }

    #[test]
    fn phone_numbers_need_seven_digits_and_a_separator() {
        let all = [Kind::Phone];
        assert_eq!(texts("Hringdu í 555 1234 eða +354 555-1234.", &all), vec![(Kind::Phone, "555 1234".into()), (Kind::Phone, "+354 555-1234".into())]);
        assert_eq!(texts("(021) 555-1234", &all), vec![(Kind::Phone, "(021) 555-1234".into())]);
        assert!(texts("id 12345678", &all).is_empty(), "a bare run of digits is not a phone number");
        assert!(texts("abc555 1234", &all).is_empty(), "digits glued to a word are not a number");
    }

    #[test]
    fn a_phone_match_gives_its_e164_form_for_a_known_region() {
        let s = "Hringdu í +354 555-1234 eða 555 1234";
        let found = find(s, &[Kind::Phone]);
        assert_eq!(found.len(), 2);
        assert_eq!(phone_e164(&found[0], s, "IS").as_deref(), Some("+3545551234"));
        assert_eq!(phone_e164(&found[1], s, "is").as_deref(), Some("+3545551234"), "the region code is case-insensitive");
        assert_eq!(phone_e164(&found[0], s, "XX"), None, "an unknown region gives no number");
    }

    #[test]
    fn a_match_of_another_kind_gives_no_phone_number() {
        let s = "https://x.is/555 1234";
        let found = find(s, &[Kind::Link]);
        assert_eq!(phone_e164(&found[0], s, "IS"), None);
    }

    #[test]
    fn links_win_over_dates_and_phones_and_results_are_sorted() {
        let found = find("https://x.is/2026-10-09 og 555 1234 og 2026-10-09", &[Kind::Phone, Kind::Date, Kind::Link]);
        let kinds: Vec<Kind> = found.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec![Kind::Link, Kind::Phone, Kind::Date]);
        assert!(found.windows(2).all(|w| w[0].range.start < w[1].range.start));
        // a disabled kind is not reported
        assert!(find("555 1234", &[Kind::Date]).is_empty());
    }

    #[test]
    fn non_ascii_text_keeps_ranges_on_char_boundaries() {
        let s = "Þórður: https://þjóð.is/ó, sími 555 1234";
        for m in find(s, &[Kind::Link, Kind::Phone]) {
            let _ = m.text(s); // would panic on a bad boundary
        }
        assert_eq!(texts(s, &[Kind::Phone]), vec![(Kind::Phone, "555 1234".into())]);
    }

    #[test]
    fn emails_need_a_local_part_and_a_domain_with_a_top_level_label() {
        let all = [Kind::Email];
        assert_eq!(texts("Skrifaðu á olibuijr@olibuijr.com, takk.", &all), vec![(Kind::Email, "olibuijr@olibuijr.com".into())]);
        assert_eq!(texts("a.b+tag@mail.example.is.", &all), vec![(Kind::Email, "a.b+tag@mail.example.is".into())]);
        assert!(texts("a@b.", &all).is_empty(), "a single-label domain is not an address");
        assert!(texts("a@b.c", &all).is_empty(), "the top-level label needs two letters");
        assert!(texts("@x.is", &all).is_empty(), "no local part");
        assert!(texts("a.@x.is", &all).is_empty(), "the local part cannot end in a dot");
    }

    #[test]
    fn a_link_keeps_its_at_sign_and_a_www_host_after_at_is_an_address() {
        assert_eq!(texts("https://user@host.is/x", &[Kind::Link, Kind::Email]), vec![(Kind::Link, "https://user@host.is/x".into())]);
        assert_eq!(texts("mail x@www.is now", &[Kind::Link, Kind::Email]), vec![(Kind::Email, "x@www.is".into())]);
    }

    #[test]
    fn money_takes_a_symbol_or_code_before_or_kr_and_codes_after() {
        let all = [Kind::Money];
        assert_eq!(
            texts("Verðið 1.500 kr. og $12.50 eða ISK 2.000, 12,99 EUR", &all),
            vec![(Kind::Money, "1.500 kr".into()), (Kind::Money, "$12.50".into()), (Kind::Money, "ISK 2.000".into()), (Kind::Money, "12,99 EUR".into())]
        );
        assert_eq!(texts("€ 3,99", &all), vec![(Kind::Money, "€ 3,99".into())]);
    }

    #[test]
    fn bare_numbers_and_glued_words_are_not_money() {
        let all = [Kind::Money];
        assert!(texts("Verð 1500 og 2026-10-09, US45, 12kronur, 12 kr2", &all).is_empty());
        assert!(texts("pöntun 7 stk", &all).is_empty());
    }

    #[test]
    fn email_and_money_share_the_text_with_the_other_kinds_without_overlap() {
        let s = "Sendu á hjalp@bifrost.is 1.500 kr https://x.is 2026-10-09 og 555 1234";
        let found = find(s, &[Kind::Phone, Kind::Money, Kind::Date, Kind::Email, Kind::Link]);
        let kinds: Vec<Kind> = found.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec![Kind::Email, Kind::Money, Kind::Link, Kind::Date, Kind::Phone]);
        assert!(found.windows(2).all(|w| w[0].range.end <= w[1].range.start));
        assert!(find(s, &[Kind::Date]).iter().all(|m| m.kind == Kind::Date));
    }

    #[test]
    fn flight_numbers_take_a_two_capital_code_and_one_to_four_digits() {
        let all = [Kind::Flight];
        assert_eq!(texts("Flugið FI 552 lendir kl. 10", &all), vec![(Kind::Flight, "FI 552".into())]);
        assert_eq!(texts("BA117 and XY 1234.", &all), vec![(Kind::Flight, "BA117".into()), (Kind::Flight, "XY 1234".into())]);
        assert!(texts("ABC 123", &all).is_empty(), "a three-letter code is not an airline code");
        assert!(texts("AB 12345", &all).is_empty(), "five digits is not a flight number");
        assert!(texts("is 12 times", &all).is_empty(), "lower-case code letters do not count");
        assert!(texts("EX-12", &all).is_empty(), "a hyphen is not part of the code");
    }

    #[test]
    fn flight_parts_split_the_code_from_the_number() {
        assert_eq!(flight_parts("FI 552"), Some(("FI", "552")));
        assert_eq!(flight_parts("BA117"), Some(("BA", "117")));
        assert_eq!(flight_parts("FI"), None, "no number");
        assert_eq!(flight_parts("ABC 12"), None, "the code is two characters");
    }

    #[test]
    fn flights_sit_beside_dates_and_are_reported_only_when_asked() {
        let found = texts("Dagur 2026-10-09 flug FI 552", &[Kind::Flight, Kind::Date]);
        assert_eq!(found, vec![(Kind::Date, "2026-10-09".into()), (Kind::Flight, "FI 552".into())]);
        assert!(texts("flug FI 552", &[Kind::Phone, Kind::Date]).is_empty(), "a kind that was not asked for is not reported");
    }
}
