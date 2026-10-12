//! Contact display defaults: the sort order of the contact list and the default country for phone numbers.
//! The shape follows the usual contacts defaults: a sort order and a country code.
//! (none, user default, given name, family name).
//!
//! Pure: the caller keeps the two values in its own preferences and passes them in. There is no shared
//! singleton here; the shell owns one [`ContactsDefaults`] and hands it to the views that need it.
//! Names compare case-insensitively with surrounding spaces ignored. Equal keys keep their input order.

use std::cmp::Ordering;

/// Sort order: how contacts are sorted by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    /// Keep the order the contacts were given in.
    None,
    /// Use the system default (see [`SortOrder::resolve`]).
    UserDefault,
    /// Sort by given name, then family name.
    GivenName,
    /// Sort by family name, then given name.
    FamilyName,
}

impl SortOrder {
    /// The order a `UserDefault` choice stands for: `system_default`, or given name when that is not a
    /// concrete order either. Other choices return themselves.
    pub fn resolve(self, system_default: SortOrder) -> SortOrder {
        match self {
            SortOrder::UserDefault => match system_default {
                SortOrder::UserDefault => SortOrder::GivenName,
                other => other,
            },
            other => other,
        }
    }
}

/// Lower-cases and trims one name part, so that sorting ignores case and stray spaces.
fn fold(part: &str) -> String {
    part.trim().to_lowercase()
}

/// The sort key of one contact for a concrete order. `UserDefault` sorts as given name, and `None`
/// has no key ordering, so it sorts as given name too; [`sort_contacts`] never calls it for `None`.
pub fn sort_key(order: SortOrder, given: &str, family: &str) -> (String, String) {
    match order {
        SortOrder::FamilyName => (fold(family), fold(given)),
        _ => (fold(given), fold(family)),
    }
}

/// Compares two contacts' names under `order`. `None` compares every pair as equal.
pub fn compare_names(order: SortOrder, a: (&str, &str), b: (&str, &str)) -> Ordering {
    if order == SortOrder::None {
        return Ordering::Equal;
    }
    sort_key(order, a.0, a.1).cmp(&sort_key(order, b.0, b.1))
}

/// Sorts `items` in place by the name `names` returns for each one. `SortOrder::None` leaves the
/// slice as it is. The sort is stable.
pub fn sort_contacts<T>(items: &mut [T], order: SortOrder, names: impl Fn(&T) -> (&str, &str)) {
    if order == SortOrder::None {
        return;
    }
    items.sort_by_cached_key(|item| {
        let (given, family) = names(item);
        sort_key(order, given, family)
    });
}

/// `countryCode`: a two-letter ISO 3166 country code, stored in upper case. Returns `None` when the
/// text is not two ASCII letters once surrounding spaces are removed.
pub fn normalize_country_code(code: &str) -> Option<String> {
    let t = code.trim();
    if t.len() != 2 || !t.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(t.to_ascii_uppercase())
}

/// The contact display defaults the shell keeps for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactsDefaults {
    sort_order: SortOrder,
    country_code: Option<String>,
}

impl Default for ContactsDefaults {
    fn default() -> Self {
        ContactsDefaults { sort_order: SortOrder::UserDefault, country_code: None }
    }
}

impl ContactsDefaults {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stored sort choice, which may be `UserDefault`.
    pub fn sort_order(&self) -> SortOrder {
        self.sort_order
    }

    pub fn set_sort_order(&mut self, order: SortOrder) {
        self.sort_order = order;
    }

    /// The sort order to apply, with `UserDefault` resolved against `system_default`.
    pub fn effective_sort_order(&self, system_default: SortOrder) -> SortOrder {
        self.sort_order.resolve(system_default)
    }

    /// The default country code, if one is set.
    pub fn country_code(&self) -> Option<&str> {
        self.country_code.as_deref()
    }

    /// Sets the country code. Returns `false` and keeps the old value when `code` is not valid.
    pub fn set_country_code(&mut self, code: &str) -> bool {
        match normalize_country_code(code) {
            Some(c) => {
                self.country_code = Some(c);
                true
            }
            None => false,
        }
    }

    pub fn clear_country_code(&mut self) {
        self.country_code = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_default_resolves_against_the_system_choice() {
        assert_eq!(SortOrder::UserDefault.resolve(SortOrder::FamilyName), SortOrder::FamilyName);
        assert_eq!(SortOrder::UserDefault.resolve(SortOrder::UserDefault), SortOrder::GivenName);
        assert_eq!(SortOrder::GivenName.resolve(SortOrder::FamilyName), SortOrder::GivenName);
        assert_eq!(SortOrder::None.resolve(SortOrder::FamilyName), SortOrder::None);
    }

    #[test]
    fn country_code_is_two_letters_in_upper_case() {
        assert_eq!(normalize_country_code("is").as_deref(), Some("IS"));
        assert_eq!(normalize_country_code(" de ").as_deref(), Some("DE"));
        assert_eq!(normalize_country_code("usa"), None);
        assert_eq!(normalize_country_code("1x"), None);
        assert_eq!(normalize_country_code(""), None);
    }

    #[test]
    fn sorts_by_given_name_ignoring_case() {
        let mut people: Vec<(&str, &str)> = vec![("Zoe", "Adams"), ("anna", "Zimmer"), ("Anna", "Berg")];
        sort_contacts(&mut people, SortOrder::GivenName, |p| *p);
        assert_eq!(people, vec![("Anna", "Berg"), ("anna", "Zimmer"), ("Zoe", "Adams")]);
    }

    #[test]
    fn sorts_by_family_name_first() {
        let mut people: Vec<(&str, &str)> = vec![("Anna", "Zimmer"), ("Zoe", "Adams"), ("anna", "Berg")];
        sort_contacts(&mut people, SortOrder::FamilyName, |p| *p);
        assert_eq!(people, vec![("Zoe", "Adams"), ("anna", "Berg"), ("Anna", "Zimmer")]);
    }

    #[test]
    fn none_keeps_the_input_order() {
        let mut people: Vec<(&str, &str)> = vec![("Zoe", "Adams"), ("Anna", "Berg")];
        sort_contacts(&mut people, SortOrder::None, |p| *p);
        assert_eq!(people, vec![("Zoe", "Adams"), ("Anna", "Berg")]);
        assert_eq!(compare_names(SortOrder::None, ("Zoe", "A"), ("Anna", "B")), Ordering::Equal);
    }

    #[test]
    fn equal_names_keep_their_input_order() {
        let mut people: Vec<(&str, &str)> = vec![("Ann", "Lee"), ("ann", "lee"), ("Bo", "Lee")];
        sort_contacts(&mut people, SortOrder::GivenName, |p| *p);
        assert_eq!(people, vec![("Ann", "Lee"), ("ann", "lee"), ("Bo", "Lee")]);
    }

    #[test]
    fn defaults_keep_the_last_valid_country() {
        let mut d = ContactsDefaults::new();
        assert_eq!(d.sort_order(), SortOrder::UserDefault);
        assert_eq!(d.country_code(), None);
        assert!(d.set_country_code("is"));
        assert!(!d.set_country_code("islands"));
        assert_eq!(d.country_code(), Some("IS"));
        assert_eq!(d.effective_sort_order(SortOrder::FamilyName), SortOrder::FamilyName);
        d.set_sort_order(SortOrder::GivenName);
        assert_eq!(d.effective_sort_order(SortOrder::FamilyName), SortOrder::GivenName);
        d.clear_country_code();
        assert_eq!(d.country_code(), None);
    }
}
