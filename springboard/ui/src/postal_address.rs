//! Structured postal addresses and their mailing layout. An address keeps the parts a person enters: street,
//! sub-locality, city, sub-administrative area (county), state, postal code, country and the ISO 3166-1 alpha-2
//! country code. `lines` lays them out in the order that country writes them; a country without its own rule gets
//! the default order. The sub-administrative area is kept on the address but not printed. Pure text: there is no
//! locale data here, only the four layouts below.

/// Longest value of one address part, in characters.
pub const MAX_PART: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Address {
    pub street: String,
    pub sub_locality: String,
    pub city: String,
    pub sub_administrative_area: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    /// ISO 3166-1 alpha-2, such as "IS". Compared without case; empty when unknown.
    pub iso_country_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A part is longer than `MAX_PART` characters.
    TooLong(&'static str),
    /// The country code is not two ASCII letters.
    BadCountryCode,
    /// A part holds a control character, such as a newline.
    ControlCharacter(&'static str),
}

impl Address {
    /// Checks every part for length and control characters, and the country code when one is given.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in self.parts() {
            if value.chars().count() > MAX_PART {
                return Err(Error::TooLong(name));
            }
            if value.chars().any(|c| c.is_control()) {
                return Err(Error::ControlCharacter(name));
            }
        }
        let code = self.iso_country_code.trim();
        if !code.is_empty() && !(code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())) {
            return Err(Error::BadCountryCode);
        }
        Ok(())
    }

    /// The printed lines, in the country's order. Blank parts are left out.
    pub fn lines(&self) -> Vec<String> {
        let code = self.iso_country_code.trim().to_ascii_uppercase();
        let mut out = Vec::new();
        push(&mut out, &self.street);
        push(&mut out, &self.sub_locality);
        match code.as_str() {
            "US" => {
                // "City, ST 12345", and the country is left off a US address.
                let city = self.city.trim();
                let state_zip = join(&[self.state.as_str(), self.postal_code.as_str()], " ");
                let line = match (city.is_empty(), state_zip.is_empty()) {
                    (false, false) => format!("{city}, {state_zip}"),
                    (false, true) => city.to_string(),
                    _ => state_zip,
                };
                push(&mut out, &line);
            }
            "GB" => {
                push(&mut out, &self.city);
                push(&mut out, &self.postal_code);
            }
            "IS" => {
                // Postal code before the town: "101 Reykjavik".
                push(&mut out, &join(&[self.postal_code.as_str(), self.city.as_str()], " "));
            }
            _ => {
                push(&mut out, &join(&[self.city.as_str(), self.state.as_str(), self.postal_code.as_str()], " "));
            }
        }
        if code != "US" {
            push(&mut out, &self.country);
        }
        out
    }

    /// The lines joined with newlines, ready for a label.
    pub fn format(&self) -> String {
        self.lines().join("\n")
    }

    /// The lines joined with commas, for a single-line field.
    pub fn single_line(&self) -> String {
        self.lines().join(", ")
    }

    fn parts(&self) -> [(&'static str, &str); 7] {
        [
            ("street", self.street.as_str()),
            ("sub_locality", self.sub_locality.as_str()),
            ("city", self.city.as_str()),
            ("sub_administrative_area", self.sub_administrative_area.as_str()),
            ("state", self.state.as_str()),
            ("postal_code", self.postal_code.as_str()),
            ("country", self.country.as_str()),
        ]
    }
}

/// Appends a trimmed line unless it is blank.
fn push(out: &mut Vec<String>, line: &str) {
    let line = line.trim();
    if !line.is_empty() {
        out.push(line.to_string());
    }
}

/// Joins the non-blank parts, trimmed, with `sep`.
fn join(parts: &[&str], sep: &str) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>()
        .join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(code: &str) -> Address {
        Address {
            street: "Laugavegur 1".into(),
            city: "Reykjavik".into(),
            postal_code: "101".into(),
            country: "Iceland".into(),
            iso_country_code: code.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_layout_puts_city_state_and_postal_code_on_one_line() {
        assert_eq!(sample("").lines(), vec!["Laugavegur 1", "Reykjavik 101", "Iceland"]);
    }

    #[test]
    fn a_us_address_is_city_comma_state_and_zip_with_no_country() {
        let a = Address {
            street: "1 Main St".into(),
            city: "Springfield".into(),
            state: "IL".into(),
            postal_code: "62701".into(),
            country: "United States".into(),
            iso_country_code: "us".into(),
            ..Default::default()
        };
        assert_eq!(a.lines(), vec!["1 Main St", "Springfield, IL 62701"]);
    }

    #[test]
    fn an_icelandic_address_puts_the_postal_code_before_the_town() {
        assert_eq!(sample("IS").lines(), vec!["Laugavegur 1", "101 Reykjavik", "Iceland"]);
        assert_eq!(sample("IS").single_line(), "Laugavegur 1, 101 Reykjavik, Iceland");
    }

    #[test]
    fn a_british_address_gives_town_and_postcode_on_their_own_lines() {
        let a = Address {
            street: "10 Downing Street".into(),
            city: "London".into(),
            postal_code: "SW1A 1AA".into(),
            country: "United Kingdom".into(),
            iso_country_code: "GB".into(),
            ..Default::default()
        };
        assert_eq!(a.lines(), vec!["10 Downing Street", "London", "SW1A 1AA", "United Kingdom"]);
        assert_eq!(a.format(), "10 Downing Street\nLondon\nSW1A 1AA\nUnited Kingdom");
    }

    #[test]
    fn blank_and_padded_parts_leave_no_empty_lines() {
        let a = Address { street: "   ".into(), city: "  Reykjavik  ".into(), ..Default::default() };
        assert_eq!(a.lines(), vec!["Reykjavik"]);
    }

    #[test]
    fn the_county_is_kept_but_not_printed() {
        let a = Address { sub_administrative_area: "Kjosarhreppur".into(), ..sample("") };
        assert!(!a.format().contains("Kjosarhreppur"));
        assert_eq!(a.sub_administrative_area, "Kjosarhreppur");
    }

    #[test]
    fn validation_checks_the_country_code_length_and_control_characters() {
        assert_eq!(sample("is").validate(), Ok(()));
        assert_eq!(sample("").validate(), Ok(()));
        assert_eq!(sample("Iceland").validate(), Err(Error::BadCountryCode));
        assert_eq!(sample("I1").validate(), Err(Error::BadCountryCode));
        let long = Address { street: "x".repeat(MAX_PART + 1), ..sample("") };
        assert_eq!(long.validate(), Err(Error::TooLong("street")));
        let newline = Address { city: "Reykja\nvik".into(), ..sample("") };
        assert_eq!(newline.validate(), Err(Error::ControlCharacter("city")));
    }
}
