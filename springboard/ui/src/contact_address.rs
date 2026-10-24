//! Postal address formatter: the lines a postal address prints as on an envelope or a card. The model takes the parts
//! of a [`PostalAddress`] and returns non-empty lines only, so a missing part leaves no gap. Four layouts are modelled:
//! city first ("Springfield, IL 62701"), postal code first ("101 Reykjavik"), the postal code on a line of its own after
//! the city, and largest part first (postal code, region and city, then the street). [`Style::for_country`] picks the
//! layout from the address's ISO country code, as the reference formatter does; the layouts follow the public postal
//! conventions of each country, and a country without an entry uses city first. The administrative area below the city
//! is not printed. Country names are printed as stored.

use crate::contact_book::PostalAddress;

/// How the locality line is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// City, then state and postal code: "Springfield, IL 62701".
    CityFirst,
    /// Postal code, then city and state: "101 Reykjavik".
    PostalFirst,
    /// City (and state) on one line, then the postal code on its own line.
    PostalLine,
    /// Largest part first: the postal code, then the state, city and district, then the street.
    LargestFirst,
}

/// Countries whose locality line puts the postal code before the city.
const POSTAL_FIRST: [&str; 31] = [
    "AT", "BE", "BG", "CH", "CZ", "DE", "DK", "EE", "ES", "FI", "FO", "FR", "GL", "GR", "HR", "HU", "IS", "IT", "LI", "LT",
    "LU", "LV", "MC", "NL", "NO", "PL", "PT", "RO", "SE", "SI", "SK",
];
/// Countries that print the postal code on its own line below the city.
const POSTAL_LINE: [&str; 4] = ["GB", "IE", "IM", "JE"];
/// Countries that write the address from the largest part down.
const LARGEST_FIRST: [&str; 5] = ["CN", "JP", "KR", "TW", "HK"];

impl Style {
    /// The layout for an ISO 3166 country code (any case, surrounding spaces ignored); city first when the code is
    /// empty or has no entry.
    pub fn for_country(iso_code: &str) -> Style {
        let code = iso_code.trim().to_ascii_uppercase();
        let code = code.as_str();
        if POSTAL_FIRST.contains(&code) {
            Style::PostalFirst
        } else if POSTAL_LINE.contains(&code) {
            Style::PostalLine
        } else if LARGEST_FIRST.contains(&code) {
            Style::LargestFirst
        } else {
            Style::CityFirst
        }
    }
}

/// The lines of the address in the layout of its own country (its `iso_country_code`).
pub fn lines_for_country(address: &PostalAddress) -> Vec<String> {
    lines(address, Style::for_country(&address.iso_country_code))
}

/// The non-empty lines of the address, from the street down to the country.
pub fn lines(address: &PostalAddress, style: Style) -> Vec<String> {
    let mut out = Vec::new();
    if style == Style::LargestFirst {
        push_nonempty(&mut out, address.postal_code.trim());
        push_nonempty(&mut out, &join(&[&address.state, &address.city, &address.sub_locality], " "));
        push_nonempty(&mut out, address.street.trim());
        push_nonempty(&mut out, address.country.trim());
        return out;
    }
    for part in [&address.street, &address.sub_locality] {
        push_nonempty(&mut out, part.trim());
    }
    let locality = match style {
        Style::CityFirst => {
            let rest = join(&[&address.state, &address.postal_code], " ");
            join(&[&address.city, &rest], ", ")
        }
        Style::PostalFirst => join(&[&address.postal_code, &address.city, &address.state], " "),
        Style::PostalLine | Style::LargestFirst => join(&[&address.city, &address.state], " "),
    };
    push_nonempty(&mut out, &locality);
    if style == Style::PostalLine {
        push_nonempty(&mut out, address.postal_code.trim());
    }
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

    #[test]
    fn the_country_code_picks_the_layout() {
        assert_eq!(Style::for_country("is"), Style::PostalFirst);
        assert_eq!(Style::for_country(" DE "), Style::PostalFirst);
        assert_eq!(Style::for_country("US"), Style::CityFirst);
        assert_eq!(Style::for_country("GB"), Style::PostalLine);
        assert_eq!(Style::for_country("JP"), Style::LargestFirst);
        assert_eq!(Style::for_country(""), Style::CityFirst);
        assert_eq!(Style::for_country("zz"), Style::CityFirst);
    }

    #[test]
    fn the_postal_code_gets_its_own_line_or_leads_the_address() {
        let london = PostalAddress {
            street: "10 Downing Street".into(),
            city: "London".into(),
            postal_code: "SW1A 2AA".into(),
            country: "United Kingdom".into(),
            iso_country_code: "gb".into(),
            ..PostalAddress::default()
        };
        assert_eq!(lines_for_country(&london), vec!["10 Downing Street", "London", "SW1A 2AA", "United Kingdom"]);
        let tokyo = PostalAddress {
            street: "1-1 Chiyoda".into(),
            city: "Chiyoda-ku".into(),
            state: "Tokyo".into(),
            postal_code: "100-0001".into(),
            iso_country_code: "JP".into(),
            ..PostalAddress::default()
        };
        assert_eq!(lines_for_country(&tokyo), vec!["100-0001", "Tokyo Chiyoda-ku", "1-1 Chiyoda"]);
        let reykjavik = PostalAddress { city: "Reykjavik".into(), postal_code: "101".into(), iso_country_code: "IS".into(), ..PostalAddress::default() };
        assert_eq!(lines_for_country(&reykjavik), vec!["101 Reykjavik"]);
    }
}
