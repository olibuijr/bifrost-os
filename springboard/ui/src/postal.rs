//! Structured postal addresses and their formatter: street, locality (city, state or region, postal code), country.
//! Pure model. The postal code goes before the city in the locality line when `postal_first` is set (the order many
//! European countries use); otherwise it follows the state. Empty parts are dropped, so a partly filled address
//! formats without stray separators.

/// A postal address with one value per part, as a contact card stores it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostalAddress {
    pub street: String,
    pub city: String,
    /// State, province or region.
    pub state: String,
    pub postal_code: String,
    pub country: String,
}

fn part(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(t)
}

impl PostalAddress {
    /// True when no part has text.
    pub fn is_empty(&self) -> bool {
        [&self.street, &self.city, &self.state, &self.postal_code, &self.country].iter().all(|s| s.trim().is_empty())
    }

    /// The address as lines in reading order: street, locality, country. Blank lines are left out.
    pub fn lines(&self, postal_first: bool) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(s) = part(&self.street) {
            out.push(s.to_string());
        }
        let locality = if postal_first {
            let head = [part(&self.postal_code), part(&self.city)];
            let joined = head.iter().flatten().copied().collect::<Vec<_>>().join(" ");
            let tail = part(&self.state).map(str::to_string);
            match (joined.is_empty(), tail) {
                (true, t) => t.unwrap_or_default(),
                (false, Some(t)) => format!("{joined}, {t}"),
                (false, None) => joined,
            }
        } else {
            let city_state = [part(&self.city), part(&self.state)].iter().flatten().copied().collect::<Vec<_>>().join(", ");
            match (city_state.is_empty(), part(&self.postal_code)) {
                (true, Some(p)) => p.to_string(),
                (true, None) => String::new(),
                (false, Some(p)) => format!("{city_state} {p}"),
                (false, None) => city_state,
            }
        };
        if !locality.is_empty() {
            out.push(locality);
        }
        if let Some(c) = part(&self.country) {
            out.push(c.to_string());
        }
        out
    }

    /// The address on one line, lines joined with ", ".
    pub fn one_line(&self, postal_first: bool) -> String {
        self.lines(postal_first).join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> PostalAddress {
        PostalAddress {
            street: "Hamraborg 1".into(),
            city: "Kópavogur".into(),
            state: "Capital Region".into(),
            postal_code: "200".into(),
            country: "Iceland".into(),
        }
    }

    #[test]
    fn an_empty_address_has_no_lines() {
        let a = PostalAddress::default();
        assert!(a.is_empty());
        assert!(a.lines(false).is_empty());
        assert_eq!(a.one_line(false), "");
    }

    #[test]
    fn the_standard_order_puts_the_postal_code_after_the_state() {
        assert_eq!(addr().lines(false), vec!["Hamraborg 1", "Kópavogur, Capital Region 200", "Iceland"]);
    }

    #[test]
    fn postal_first_puts_the_code_before_the_city() {
        let a = PostalAddress { street: "Laugavegur 1".into(), city: "Reykjavík".into(), postal_code: "101".into(), country: "Iceland".into(), ..Default::default() };
        assert_eq!(a.lines(true), vec!["Laugavegur 1", "101 Reykjavík", "Iceland"]);
        assert_eq!(a.one_line(true), "Laugavegur 1, 101 Reykjavík, Iceland");
    }

    #[test]
    fn a_partly_filled_address_leaves_no_stray_separators() {
        let a = PostalAddress { city: "  Akureyri ".into(), state: "   ".into(), postal_code: "600".into(), ..Default::default() };
        assert!(!a.is_empty());
        assert_eq!(a.lines(false), vec!["Akureyri 600"]);
        let b = PostalAddress { postal_code: "600".into(), ..Default::default() };
        assert_eq!(b.lines(true), vec!["600"]);
        assert_eq!(b.one_line(false), "600");
    }

    #[test]
    fn whitespace_only_parts_count_as_empty() {
        let a = PostalAddress { street: " \t".into(), country: "  ".into(), ..Default::default() };
        assert!(a.is_empty());
        assert!(a.lines(false).is_empty());
    }

    #[test]
    fn one_line_joins_the_lines_with_commas() {
        assert_eq!(addr().one_line(false), "Hamraborg 1, Kópavogur, Capital Region 200, Iceland");
    }
}
