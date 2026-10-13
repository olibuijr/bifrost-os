//! Postal address formatter: the lines a postal address prints as on an envelope or a card. The model takes the parts
//! of a [`PostalAddress`] and returns non-empty lines only, so a missing part leaves no gap. Two layouts are modelled:
//! city first ("Springfield, IL 62701") and postal code first ("101 Reykjavik"). The administrative area below the city
//! is not printed. Country names are printed as stored; there is no country code lookup.

use crate::contact_book::PostalAddress;

/// How the locality line is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// City, then state and postal code: "Springfield, IL 62701".
    CityFirst,
    /// Postal code, then city and state: "101 Reykjavik".
    PostalFirst,
}

/// The non-empty lines of the address, from the street down to the country.
pub fn lines(address: &PostalAddress, style: Style) -> Vec<String> {
    let mut out = Vec::new();
    for part in [&address.street, &address.sub_locality] {
        push_nonempty(&mut out, part.trim());
    }
    let locality = match style {
        Style::CityFirst => {
            let rest = join(&[&address.state, &address.postal_code], " ");
            join(&[&address.city, &rest], ", ")
        }
        Style::PostalFirst => join(&[&address.postal_code, &address.city, &address.state], " "),
    };
    push_nonempty(&mut out, &locality);
    push_nonempty(&mut out, address.country.trim());
    out
}

/// The lines joined with ", ", for a field that holds one line of text.
pub fn single_line(address: &PostalAddress, style: Style) -> String {
    lines(address, style).join(", ")
}

fn push_nonempty(out: &mut Vec<String>, text: &str) {
    if !text.is_empty() {
        out.push(text.to_string());
    }
}

/// Joins the non-empty parts with a separator, trimming each part.
fn join(parts: &[&String], separator: &str) -> String {
    parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address() -> PostalAddress {
        PostalAddress {
            street: "1 Main St".into(),
            city: "Springfield".into(),
            state: "IL".into(),
            postal_code: "62701".into(),
            country: "USA".into(),
            ..PostalAddress::default()
        }
    }

    #[test]
    fn city_first_puts_state_and_postal_code_after_the_city() {
        assert_eq!(lines(&address(), Style::CityFirst), vec!["1 Main St", "Springfield, IL 62701", "USA"]);
    }

    #[test]
    fn postal_first_puts_the_postal_code_before_the_city() {
        let mut a = address();
        a.street = "Laugavegur 12".into();
        a.city = "Reykjavik".into();
        a.state = String::new();
        a.postal_code = "101".into();
        a.country = "Iceland".into();
        assert_eq!(lines(&a, Style::PostalFirst), vec!["Laugavegur 12", "101 Reykjavik", "Iceland"]);
    }

    #[test]
    fn missing_parts_leave_no_empty_lines_or_stray_separators() {
        let a = PostalAddress { city: "Springfield".into(), ..PostalAddress::default() };
        assert_eq!(lines(&a, Style::CityFirst), vec!["Springfield"]);
        let b = PostalAddress { postal_code: "62701".into(), ..PostalAddress::default() };
        assert_eq!(lines(&b, Style::CityFirst), vec!["62701"]);
        assert_eq!(lines(&b, Style::PostalFirst), vec!["62701"]);
    }

    #[test]
    fn sub_locality_sits_between_street_and_locality() {
        let mut a = address();
        a.sub_locality = "Downtown".into();
        assert_eq!(lines(&a, Style::CityFirst), vec!["1 Main St", "Downtown", "Springfield, IL 62701", "USA"]);
    }

    #[test]
    fn empty_address_has_no_lines() {
        assert!(lines(&PostalAddress::default(), Style::CityFirst).is_empty());
        assert_eq!(single_line(&PostalAddress::default(), Style::PostalFirst), "");
    }

    #[test]
    fn blank_parts_count_as_missing() {
        let a = PostalAddress { street: "   ".into(), city: " Oslo ".into(), ..PostalAddress::default() };
        assert_eq!(lines(&a, Style::CityFirst), vec!["Oslo"]);
    }

    #[test]
    fn single_line_joins_the_lines() {
        assert_eq!(single_line(&address(), Style::CityFirst), "1 Main St, Springfield, IL 62701, USA");
    }
}
