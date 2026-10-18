//! Per-region phone number rules, as data, for the 20 most common regions. A rule holds the country calling code, the
//! range of national significant number lengths, and the trunk prefix a national number is written with (the `0` in
//! `020 7946 0958`). The lengths are the usual ranges for each region, not the full numbering plans, so a rare number
//! of an unusual length is refused rather than guessed at. The rules serve [`to_e164`] and [`is_valid`]; the phone
//! detector in [`crate::detect`] finds numbers in text and can hand each one to these functions.

/// The rule for one region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// ISO 3166-1 alpha-2 code of the region.
    pub region: &'static str,
    /// Country calling code, without the `+`.
    pub calling_code: &'static str,
    /// Shortest national significant number, in digits.
    pub national_min: usize,
    /// Longest national significant number, in digits.
    pub national_max: usize,
    /// The digit a national number is dialled with inside the country, if any.
    pub trunk: Option<char>,
}

/// The rules for the 20 most common regions, in no particular order.
pub const RULES: [Rule; 20] = [
    Rule { region: "US", calling_code: "1", national_min: 10, national_max: 10, trunk: None },
    Rule { region: "CA", calling_code: "1", national_min: 10, national_max: 10, trunk: None },
    Rule { region: "GB", calling_code: "44", national_min: 9, national_max: 10, trunk: Some('0') },
    Rule { region: "IE", calling_code: "353", national_min: 8, national_max: 9, trunk: Some('0') },
    Rule { region: "FR", calling_code: "33", national_min: 9, national_max: 9, trunk: Some('0') },
    Rule { region: "DE", calling_code: "49", national_min: 10, national_max: 11, trunk: Some('0') },
    Rule { region: "NL", calling_code: "31", national_min: 9, national_max: 9, trunk: Some('0') },
    Rule { region: "BE", calling_code: "32", national_min: 8, national_max: 9, trunk: Some('0') },
    Rule { region: "CH", calling_code: "41", national_min: 9, national_max: 9, trunk: Some('0') },
    Rule { region: "AT", calling_code: "43", national_min: 9, national_max: 12, trunk: Some('0') },
    Rule { region: "ES", calling_code: "34", national_min: 9, national_max: 9, trunk: None },
    Rule { region: "IT", calling_code: "39", national_min: 9, national_max: 10, trunk: None },
    Rule { region: "PT", calling_code: "351", national_min: 9, national_max: 9, trunk: None },
    Rule { region: "SE", calling_code: "46", national_min: 7, national_max: 9, trunk: Some('0') },
    Rule { region: "NO", calling_code: "47", national_min: 8, national_max: 8, trunk: None },
    Rule { region: "DK", calling_code: "45", national_min: 8, national_max: 8, trunk: None },
    Rule { region: "FI", calling_code: "358", national_min: 5, national_max: 10, trunk: Some('0') },
    Rule { region: "IS", calling_code: "354", national_min: 7, national_max: 7, trunk: None },
    Rule { region: "PL", calling_code: "48", national_min: 9, national_max: 9, trunk: None },
    Rule { region: "JP", calling_code: "81", national_min: 9, national_max: 10, trunk: Some('0') },
];

/// The rule for a region code (case-insensitive), or `None` for a region not in the table.
pub fn rule(region: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.region.eq_ignore_ascii_case(region))
}

/// The rule whose calling code starts the digits of an international number (no `+`). When two regions share a code
/// (the `1` of the United States and Canada) the first in the table is returned, so the national length decides
/// nothing here; use [`is_valid`] to check a whole number.
pub fn rule_for_digits(digits: &str) -> Option<&'static Rule> {
    RULES
        .iter()
        .filter(|r| digits.starts_with(r.calling_code))
        .max_by_key(|r| r.calling_code.len())
}

/// The digits of `text`, with the spaces, dashes, dots and brackets a person writes around a number removed. A leading
/// `+` is kept as the marker of an international number; anything else that is not a digit makes the text unusable.
fn normalise(text: &str) -> Option<(bool, String)> {
    let trimmed = text.trim();
    let international = trimmed.starts_with('+');
    let mut digits = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        match c {
            '0'..='9' => digits.push(c),
            '+' if digits.is_empty() && international => {}
            ' ' | '-' | '.' | '(' | ')' | '\u{a0}' => {}
            _ => return None,
        }
    }
    if digits.is_empty() {
        None
    } else {
        Some((international, digits))
    }
}

/// Writes a phone number as E.164 (`+` and the digits, no spaces). `text` may be international (`+354 555 1234`) or
/// national in `region` (`555 1234` for Iceland, `020 7946 0958` for the United Kingdom, where the trunk `0` is dropped).
/// Returns `None` when the region is unknown or the number does not have a valid length for it.
pub fn to_e164(text: &str, region: &str) -> Option<String> {
    let (international, digits) = normalise(text)?;
    let home = rule(region)?;
    let e164 = if international {
        format!("+{digits}")
    } else {
        let national: &str = match home.trunk {
            Some(t) => digits.strip_prefix(t).unwrap_or(&digits),
            None => &digits,
        };
        format!("+{}{}", home.calling_code, national)
    };
    is_valid(&e164).then_some(e164)
}

/// Whether `e164` (`+` then digits) is a number of a known region: its calling code belongs to a rule and the national
/// part has that rule's length.
pub fn is_valid(e164: &str) -> bool {
    let Some(digits) = e164.strip_prefix('+') else {
        return false;
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    RULES.iter().any(|r| {
        digits.strip_prefix(r.calling_code).is_some_and(|national| (r.national_min..=r.national_max).contains(&national.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_twenty_distinct_regions_with_sane_ranges() {
        let mut regions: Vec<&str> = RULES.iter().map(|r| r.region).collect();
        regions.sort_unstable();
        regions.dedup();
        assert_eq!(regions.len(), 20);
        for r in &RULES {
            assert!(r.national_min >= 1 && r.national_min <= r.national_max, "{}", r.region);
            assert!(r.calling_code.bytes().all(|b| b.is_ascii_digit()), "{}", r.region);
        }
    }

    #[test]
    fn regions_are_found_regardless_of_case() {
        assert_eq!(rule("is").map(|r| r.calling_code), Some("354"));
        assert!(rule("ZZ").is_none());
    }

    #[test]
    fn a_national_number_becomes_e164_with_its_trunk_dropped() {
        assert_eq!(to_e164("(020) 7946 0958", "GB").as_deref(), Some("+442079460958"));
        assert_eq!(to_e164("06 12 34 56 78", "FR").as_deref(), Some("+33612345678"));
        assert_eq!(to_e164("555 1234", "IS").as_deref(), Some("+3545551234"));
    }

    #[test]
    fn an_international_number_keeps_its_own_code() {
        assert_eq!(to_e164("+354 555 1234", "IS").as_deref(), Some("+3545551234"));
        assert_eq!(to_e164("+1 415 555 2671", "US").as_deref(), Some("+14155552671"));
        assert_eq!(to_e164("+49 1761 2345678", "GB").as_deref(), Some("+4917612345678"));
    }

    #[test]
    fn a_number_of_the_wrong_length_or_an_unknown_region_is_refused() {
        assert_eq!(to_e164("12", "IS"), None);
        assert_eq!(to_e164("555 12345678", "IS"), None);
        assert_eq!(to_e164("555 1234", "ZZ"), None);
        assert_eq!(to_e164("+99 12345", "US"), None);
        assert_eq!(to_e164("555 abc", "IS"), None);
    }

    #[test]
    fn validity_checks_the_calling_code_and_the_length() {
        assert!(is_valid("+3545551234"));
        assert!(!is_valid("+35455512345"), "eight digits is one too many for Iceland");
        assert!(!is_valid("3545551234"), "E.164 needs the +");
        assert!(!is_valid("+"));
        assert!(!is_valid("+35a5551234"));
    }

    #[test]
    fn a_calling_code_finds_its_rule() {
        assert_eq!(rule_for_digits("4917612345678").map(|r| r.region), Some("DE"));
        assert_eq!(rule_for_digits("3541234").map(|r| r.region), Some("IS"));
        assert!(rule_for_digits("999").is_none());
    }
}
