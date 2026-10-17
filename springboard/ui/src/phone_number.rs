//! `PhoneNumberKit` (PhoneNumbers): parses a phone number typed or detected in text, normalises it to E.164 and
//! formats it for display, nationally or internationally. Pure functions with a small table of the regions Bifrost
//! offers in Language & Region (`region.rs`). Each region has its dial code, whether national numbers carry a trunk
//! prefix (`0`), the national lengths it accepts and the digit groups used when printing. This is not the full
//! numbering plan: a number that matches the length table is accepted, and a number outside the table is refused.

/// One region's numbering: how a national number is written and grouped.
struct Plan {
    /// ISO 3166 region code, as `region.rs` lists them.
    region: &'static str,
    /// The country calling code, without the `+`.
    dial: &'static str,
    /// Whether national numbers are written with a leading trunk prefix `0`.
    trunk: bool,
    /// The lengths of the national significant number (without the trunk prefix) that are valid.
    lengths: &'static [usize],
    /// Digit group sizes for printing; the last group takes the rest.
    groups: &'static [usize],
}

const PLANS: [Plan; 12] = [
    Plan { region: "IS", dial: "354", trunk: false, lengths: &[7], groups: &[3, 4] },
    Plan { region: "US", dial: "1", trunk: false, lengths: &[10], groups: &[3, 3] },
    Plan { region: "CA", dial: "1", trunk: false, lengths: &[10], groups: &[3, 3] },
    Plan { region: "GB", dial: "44", trunk: true, lengths: &[9, 10], groups: &[2, 4, 4] },
    Plan { region: "DK", dial: "45", trunk: false, lengths: &[8], groups: &[2, 2, 2, 2] },
    Plan { region: "NO", dial: "47", trunk: false, lengths: &[8], groups: &[3, 2, 3] },
    Plan { region: "SE", dial: "46", trunk: true, lengths: &[7, 8, 9], groups: &[2, 3, 2, 2] },
    Plan { region: "FI", dial: "358", trunk: true, lengths: &[8, 9, 10], groups: &[2, 3, 4] },
    Plan { region: "DE", dial: "49", trunk: true, lengths: &[10, 11], groups: &[3, 4] },
    Plan { region: "FR", dial: "33", trunk: true, lengths: &[9], groups: &[1, 2, 2, 2, 2] },
    Plan { region: "AU", dial: "61", trunk: true, lengths: &[9], groups: &[1, 4] },
    Plan { region: "JP", dial: "81", trunk: true, lengths: &[9, 10], groups: &[2, 4] },
];

/// Why a phone number was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneError {
    /// No digits at all.
    Empty,
    /// A character other than a digit, `+`, space, dash, dot or parentheses.
    InvalidCharacter,
    /// The number has the wrong length for its region.
    WrongLength,
    /// The region code is not one Bifrost has a numbering plan for.
    UnknownRegion,
    /// An international number whose calling code is not in the table.
    UnknownCallingCode,
}

/// A parsed, validated phone number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneNumber {
    /// The region the number belongs to.
    pub region: &'static str,
    /// The calling code, without the `+`.
    pub dial: &'static str,
    /// The national significant number: digits only, no trunk prefix.
    pub national: String,
}

impl PhoneNumber {
    /// E.164: `+` and the calling code and number, digits only, at most 15 digits.
    pub fn e164(&self) -> String {
        format!("+{}{}", self.dial, self.national)
    }

    /// The national format a person writes in `region`: trunk prefix and groups, with the North American
    /// `(NNN) NNN-NNNN` shape for US and CA.
    pub fn format_national(&self) -> String {
        let plan = plan_for(self.region).expect("a parsed number has a known region");
        let body = group(&self.national, plan.groups, " ");
        if plan.dial == "1" && self.national.len() == 10 {
            let n = &self.national;
            return format!("({}) {}-{}", &n[..3], &n[3..6], &n[6..]);
        }
        if plan.trunk {
            format!("0{body}")
        } else {
            body
        }
    }

    /// The international format: `+` the calling code and the grouped national number.
    pub fn format_international(&self) -> String {
        let plan = plan_for(self.region).expect("a parsed number has a known region");
        if plan.dial == "1" && self.national.len() == 10 {
            let n = &self.national;
            return format!("+1 ({}) {}-{}", &n[..3], &n[3..6], &n[6..]);
        }
        format!("+{} {}", self.dial, group(&self.national, plan.groups, " "))
    }
}

/// The numbering plan of a region code, if Bifrost has one.
fn plan_for(region: &str) -> Option<&'static Plan> {
    PLANS.iter().find(|p| p.region == region)
}

/// Splits `digits` into groups of the given sizes, the last group taking the remainder, joined by `sep`.
fn group(digits: &str, sizes: &[usize], sep: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut rest = digits;
    for (i, size) in sizes.iter().enumerate() {
        if rest.is_empty() {
            break;
        }
        if i + 1 == sizes.len() || rest.len() <= *size {
            out.push(rest);
            rest = "";
        } else {
            out.push(&rest[..*size]);
            rest = &rest[*size..];
        }
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out.join(sep)
}

/// Parses `input` as a phone number. An international number (leading `+` or `00`) is matched to its region by
/// calling code. A national number is read in `region`, where a leading `0` is the trunk prefix if the region has one.
pub fn parse(input: &str, region: &str) -> Result<PhoneNumber, PhoneError> {
    let mut international = false;
    let mut digits = String::new();
    for (i, c) in input.trim().chars().enumerate() {
        match c {
            '+' if i == 0 => international = true,
            '0'..='9' => digits.push(c),
            ' ' | '-' | '.' | '(' | ')' | '\u{a0}' => {}
            _ => return Err(PhoneError::InvalidCharacter),
        }
    }
    if digits.is_empty() {
        return Err(PhoneError::Empty);
    }
    if !international && input.trim().starts_with("00") {
        international = true;
        digits = digits[2..].to_string();
        if digits.is_empty() {
            return Err(PhoneError::Empty);
        }
    }
    if international {
        let plan = PLANS
            .iter()
            .filter(|p| digits.starts_with(p.dial))
            .max_by_key(|p| p.dial.len())
            .ok_or(PhoneError::UnknownCallingCode)?;
        return national_in(plan, &digits[plan.dial.len()..]);
    }
    let plan = plan_for(region).ok_or(PhoneError::UnknownRegion)?;
    let national = if plan.trunk && digits.starts_with('0') { &digits[1..] } else { digits.as_str() };
    national_in(plan, national)
}

fn national_in(plan: &Plan, national: &str) -> Result<PhoneNumber, PhoneError> {
    if !plan.lengths.contains(&national.len()) {
        return Err(PhoneError::WrongLength);
    }
    Ok(PhoneNumber { region: plan.region, dial: plan.dial, national: national.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icelandic_number_normalises_to_e164_and_formats_both_ways() {
        let n = parse("555 1234", "IS").unwrap();
        assert_eq!(n.e164(), "+3545551234");
        assert_eq!(n.format_national(), "555 1234");
        assert_eq!(n.format_international(), "+354 555 1234");
    }

    #[test]
    fn an_international_number_names_its_own_region_whatever_the_default_is() {
        let n = parse("+44 20 7946 0958", "IS").unwrap();
        assert_eq!(n.region, "GB");
        assert_eq!(n.e164(), "+442079460958");
        assert_eq!(n.format_national(), "020 7946 0958");
    }

    #[test]
    fn a_double_zero_prefix_is_the_same_as_plus() {
        assert_eq!(parse("0045 32 12 34 56", "IS").unwrap().e164(), "+4532123456");
    }

    #[test]
    fn a_trunk_prefix_is_dropped_when_the_region_has_one() {
        let n = parse("0 2 12 34 56 78", "FR").unwrap();
        assert_eq!(n.e164(), "+33212345678");
        assert_eq!(n.format_national(), "02 12 34 56 78");
    }

    #[test]
    fn north_american_numbers_use_the_parenthesised_shape() {
        let n = parse("(415) 555-0132", "US").unwrap();
        assert_eq!(n.e164(), "+14155550132");
        assert_eq!(n.format_national(), "(415) 555-0132");
        assert_eq!(n.format_international(), "+1 (415) 555-0132");
    }

    #[test]
    fn a_wrong_length_or_symbol_is_refused() {
        assert_eq!(parse("555 12", "IS"), Err(PhoneError::WrongLength));
        assert_eq!(parse("555 12x4", "IS"), Err(PhoneError::InvalidCharacter));
        assert_eq!(parse("  ", "IS"), Err(PhoneError::Empty));
        assert_eq!(parse("+999 1234", "IS"), Err(PhoneError::UnknownCallingCode));
        assert_eq!(parse("555 1234", "ZZ"), Err(PhoneError::UnknownRegion));
    }
}
